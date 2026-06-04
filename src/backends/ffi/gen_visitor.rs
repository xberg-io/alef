/// Generate visitor/callback FFI bindings.
///
/// This module produces the `#[repr(C)]` callback struct, an opaque `Visitor`
/// handle that bridges C function pointers into the Rust visitor trait,
/// and the three public FFI entry points:
///
/// - `{prefix}_visitor_create(callbacks: *const {Prefix}VisitorCallbacks) -> *mut {Prefix}Visitor`
/// - `{prefix}_visitor_free(visitor: *mut {Prefix}Visitor)`
/// - `{prefix}_options_set_visitor_handle(options, visitor)` — attach visitor to options before `{prefix}_convert`
///
/// # Coverage
///
/// All compatible visitor trait methods are covered. The callback struct field
/// order matches the trait definition order (and therefore the Go binding's
/// expected layout).
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

#[derive(Clone)]
struct VisitorProtocol {
    context_type: String,
    context_path: String,
    result_type: String,
    result_path: String,
}

impl VisitorProtocol {
    fn from_api(api: &ApiSurface, bridge_cfg: &TraitBridgeConfig) -> Option<Self> {
        let Some(context_type) = bridge_cfg.context_type.as_deref() else {
            eprintln!(
                "[alef] gen_visitor(ffi): trait bridge `{}` must configure context_type for visitor callbacks",
                bridge_cfg.trait_name
            );
            return None;
        };
        let Some(result_type) = bridge_cfg.result_type.as_deref() else {
            eprintln!(
                "[alef] gen_visitor(ffi): trait bridge `{}` must configure result_type for visitor callbacks",
                bridge_cfg.trait_name
            );
            return None;
        };
        let Some(context_def) = api.types.iter().find(|type_def| type_def.name == context_type) else {
            eprintln!(
                "[alef] gen_visitor(ffi): trait bridge `{}` context_type `{context_type}` is not present in IR",
                bridge_cfg.trait_name
            );
            return None;
        };
        let Some(result_def) = api.enums.iter().find(|enum_def| enum_def.name == result_type) else {
            eprintln!(
                "[alef] gen_visitor(ffi): trait bridge `{}` result_type `{result_type}` is not present in IR",
                bridge_cfg.trait_name
            );
            return None;
        };
        Some(Self {
            context_type: context_type.to_string(),
            context_path: context_def.rust_path.replace('-', "_"),
            result_type: result_type.to_string(),
            result_path: result_def.rust_path.replace('-', "_"),
        })
    }

    fn from_bridge_config(core_import: &str, bridge_cfg: Option<&TraitBridgeConfig>) -> Option<Self> {
        let bridge_cfg = bridge_cfg?;
        let context_type = bridge_cfg.context_type.as_deref()?;
        let result_type = bridge_cfg.result_type.as_deref()?;
        Some(Self {
            context_type: context_type.to_string(),
            context_path: format!("{core_import}::{context_type}"),
            result_type: result_type.to_string(),
            result_path: format!("{core_import}::{result_type}"),
        })
    }
}

struct ContextFieldSpec {
    name: String,
    c_type: &'static str,
    c_init: String,
    setup: String,
    doc: String,
}

// ---------------------------------------------------------------------------
// Data-driven callback specifications
// ---------------------------------------------------------------------------

/// The kind of a single callback parameter (beyond the common ctx/user_data/out
/// prefix that every callback shares).
enum ParamKind {
    /// Required `*const c_char` — converted from `&str` via `CString::new`.
    Str(String),
    /// Optional `*const c_char` — converted from `Option<&str>` via `opt_str_to_c`.
    OptStr(String),
    /// `i32` — converted from `bool` via `i32::from`.
    Bool(String),
    /// `u32` — passed through directly.
    U32(String),
    /// `usize` — passed through directly.
    Usize(String),
    /// `*const *const c_char` + `usize` (cell_count) — special for table rows.
    CellSlice(String),
}

/// Specification for one visitor callback.
pub(crate) struct CallbackSpec {
    name: String,
    doc: String,
    params: Vec<ParamKind>,
}

/// Build a `Vec<CallbackSpec>` from a trait's IR definition for the FFI backend.
///
/// Derives all FFI-specific fields (`ParamKind`) from `TypeRef` + `optional` flag.
/// Methods with unsupported parameter types are skipped with a warning.
/// Parameters whose type is `TypeRef::Named(_)` (the context threaded via FFI's
/// separate channel) are silently skipped — they do not become C parameters.
pub(crate) fn callback_specs_from_trait(
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
) -> Vec<CallbackSpec> {
    use crate::core::ir::{PrimitiveType, TypeRef};

    let Some(protocol) = VisitorProtocol::from_bridge_config("", bridge_cfg) else {
        eprintln!(
            "[alef] gen_visitor(ffi): visitor callbacks require configured context_type and result_type metadata"
        );
        return Vec::new();
    };
    let mut specs = Vec::with_capacity(trait_def.methods.len());
    'methods: for m in &trait_def.methods {
        if m.trait_source.is_some() {
            continue;
        }
        if !matches!(&m.return_type, TypeRef::Named(name) if name == &protocol.result_type) {
            eprintln!(
                "[alef] gen_visitor(ffi): skip method `{}` — visitor callbacks require `{}` return type",
                m.name, protocol.result_type
            );
            continue;
        }
        if !m
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Named(name) if name == &protocol.context_type))
        {
            eprintln!(
                "[alef] gen_visitor(ffi): skip method `{}` — visitor callbacks require `{}` parameter",
                m.name, protocol.context_type
            );
            continue;
        }
        let mut params = Vec::new();
        for p in &m.params {
            // Skip the context parameter — it is threaded via FFI's separate channel.
            if matches!(&p.ty, TypeRef::Named(name) if name == &protocol.context_type) {
                continue;
            }
            let param_name = p.name.trim_start_matches('_').to_string();
            match (&p.ty, p.optional) {
                (TypeRef::String, false) => {
                    params.push(ParamKind::Str(param_name));
                }
                (TypeRef::String, true) => {
                    params.push(ParamKind::OptStr(param_name));
                }
                (TypeRef::Primitive(PrimitiveType::Bool), false) => {
                    params.push(ParamKind::Bool(param_name));
                }
                (
                    TypeRef::Primitive(
                        PrimitiveType::U32
                        | PrimitiveType::I32
                        | PrimitiveType::U16
                        | PrimitiveType::I16
                        | PrimitiveType::U8
                        | PrimitiveType::I8,
                    ),
                    false,
                ) => {
                    params.push(ParamKind::U32(param_name));
                }
                (TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::I64), false) => {
                    params.push(ParamKind::Usize(param_name));
                }
                (TypeRef::Vec(inner), false) => match inner.as_ref() {
                    TypeRef::String => {
                        params.push(ParamKind::CellSlice(param_name));
                    }
                    _ => {
                        eprintln!(
                            "[alef] gen_visitor(ffi): skip method `{}` — unsupported Vec param `{}`",
                            m.name, p.name
                        );
                        continue 'methods;
                    }
                },
                _ => {
                    eprintln!(
                        "[alef] gen_visitor(ffi): skip method `{}` — unsupported param `{}: {:?}`",
                        m.name, p.name, p.ty
                    );
                    continue 'methods;
                }
            }
        }
        specs.push(CallbackSpec {
            name: m.name.clone(),
            doc: m.doc.clone(),
            params,
        });
    }
    specs
}

// ---------------------------------------------------------------------------
// Code-generation helpers — each produces one section of the output
// ---------------------------------------------------------------------------

/// Build the C `extern "C" fn(...)` signature parameters for one callback.
fn c_param_list(spec: &CallbackSpec, pascal_prefix: &str) -> String {
    let mut parts = vec![
        format!("ctx: *const {pascal_prefix}Context"),
        "user_data: *mut std::ffi::c_void".to_string(),
    ];
    for p in &spec.params {
        match p {
            ParamKind::Str(n) | ParamKind::OptStr(n) => {
                parts.push(format!("{n}: *const std::ffi::c_char"));
            }
            ParamKind::Bool(n) => parts.push(format!("{n}: i32")),
            ParamKind::U32(n) => parts.push(format!("{n}: u32")),
            ParamKind::Usize(n) => parts.push(format!("{n}: usize")),
            ParamKind::CellSlice(n) => {
                parts.push(format!("{n}: *const *const std::ffi::c_char"));
                parts.push("cell_count: usize".to_string());
            }
        }
    }
    parts.push("out_custom: *mut *mut std::ffi::c_char".to_string());
    parts.push("out_len: *mut usize".to_string());
    parts.join(",\n            ")
}

/// Format a doc string so every line carries the `    ///` prefix.
///
/// Splits on `\n`, prepends `    /// ` to each line (trimming any existing
/// leading `///` a caller may have embedded), and rejoins with `\n`.
fn format_doc_comment(doc: &str) -> String {
    doc.lines()
        .map(|line| {
            // Strip any leading `///` the caller may have pre-pended so we
            // don't double-prefix embedded continuation lines.
            let stripped = line.trim_start_matches("///").trim_start();
            if stripped.is_empty() {
                "    ///".to_string()
            } else {
                format!("    /// {stripped}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate all `Option<unsafe extern "C" fn(...)>` struct fields.
fn gen_struct_fields(specs: &[CallbackSpec], pascal_prefix: &str) -> String {
    let mut out = String::new();
    for spec in specs {
        let doc_lines = format_doc_comment(&spec.doc);
        out.push_str(&crate::backends::ffi::template_env::render("formatted_line.jinja", minijinja::context! { content => format!("\n{doc_lines}\n    pub {name}: Option<\n        unsafe extern \"C\" fn(\n            {params}\n        ) -> i32,\n    >,\n", doc_lines = doc_lines, name = spec.name, params = c_param_list(spec, pascal_prefix)) }));
    }
    out
}

/// Build the Rust trait parameter list for a callback (the `&str`, `bool`, etc. side).
fn rust_param_list(spec: &CallbackSpec, protocol: &VisitorProtocol) -> String {
    let mut parts = vec!["&mut self".to_string(), format!("ctx: &{}", protocol.context_path)];
    for p in &spec.params {
        match p {
            ParamKind::Str(n) => parts.push(format!("{n}: &str")),
            ParamKind::OptStr(n) => parts.push(format!("{n}: Option<&str>")),
            ParamKind::Bool(n) => parts.push(format!("{n}: bool")),
            ParamKind::U32(n) => parts.push(format!("{n}: u32")),
            ParamKind::Usize(n) => parts.push(format!("{n}: usize")),
            ParamKind::CellSlice(n) => parts.push(format!("{n}: &[String]")),
        }
    }
    parts.join(", ")
}

/// Generate the body of one visitor trait impl method.
///
/// Produces local CString bindings, the `call_with_ctx` invocation, and the
/// callback argument forwarding.
fn gen_impl_body(spec: &CallbackSpec, _core_import: &str, protocol: &VisitorProtocol, default_result: &str) -> String {
    let mut bindings = String::new();
    let mut cb_args = Vec::new();
    let _ = protocol;

    for p in &spec.params {
        match p {
            ParamKind::Str(n) => {
                bindings.push_str(&crate::backends::ffi::template_env::render("formatted_line.jinja", minijinja::context! { content => format!("        let {n}_cs = match std::ffi::CString::new({n}) {{\n            Ok(s) => s,\n            Err(_) => return {default_result},\n        }};\n") }));
                cb_args.push(format!("{n}_cs.as_ptr()"));
            }
            ParamKind::OptStr(n) => {
                bindings.push_str(&crate::backends::ffi::template_env::render(
                    "formatted_line.jinja",
                    minijinja::context! { content => format!("        let ({n}_ptr, _{n}_cs) = opt_str_to_c({n});\n") },
                ));
                cb_args.push(format!("{n}_ptr"));
            }
            ParamKind::Bool(n) => {
                bindings.push_str(&crate::backends::ffi::template_env::render(
                    "formatted_line.jinja",
                    minijinja::context! { content => format!("        let {n}_i = i32::from({n});\n") },
                ));
                cb_args.push(format!("{n}_i"));
            }
            ParamKind::U32(n) | ParamKind::Usize(n) => {
                cb_args.push(n.clone());
            }
            ParamKind::CellSlice(n) => {
                bindings.push_str(&crate::backends::ffi::template_env::render("formatted_line.jinja", minijinja::context! { content => format!("        let {n}_cstrings: Vec<std::ffi::CString> = {n}\n            .iter()\n            .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())\n            .collect();\n        let {n}_ptrs: Vec<*const std::ffi::c_char> =\n            {n}_cstrings.iter().map(|cs| cs.as_ptr()).collect();\n        let cell_count = {n}_ptrs.len();\n") }));
                cb_args.push(format!("{n}_ptrs.as_ptr()"));
                cb_args.push("cell_count".to_string());
            }
        }
    }

    let args_str = if cb_args.is_empty() {
        "out_custom, out_len".to_string()
    } else {
        format!("{}, out_custom, out_len", cb_args.join(", "))
    };

    format!(
        "        let Some(cb) = self.callbacks.{name} else {{\n            return {default_result};\n        }};\n        let user_data = self.callbacks.user_data;\n{bindings}        // SAFETY: cb is a valid function pointer; all temporaries live for this call.\n        unsafe {{\n            call_with_ctx(ctx, |c_ctx, out_custom, out_len| {{\n                cb(c_ctx, user_data, {args_str})\n            }})\n        }}",
        name = spec.name,
    )
}

/// Generate all visitor trait impl methods.
fn gen_impl_methods(
    specs: &[CallbackSpec],
    pascal_prefix: &str,
    core_import: &str,
    protocol: &VisitorProtocol,
    default_result: &str,
) -> String {
    let mut out = String::new();
    let result_path = protocol.result_path.clone();
    for spec in specs {
        out.push_str(&crate::backends::ffi::template_env::render("formatted_line.jinja", minijinja::context! { content => format!("\n    fn {name}(\n        {params}\n    ) -> {result_path} {{\n{body}\n    }}\n", name = spec.name, params = rust_param_list(spec, protocol), body = gen_impl_body(spec, core_import, protocol, default_result)) }));
    }
    // Close the impl block — caller opens it.
    let _ = pascal_prefix; // used by caller
    out
}

/// Build the forwarding argument list for `VisitorRef` delegation.
fn visitor_ref_args(spec: &CallbackSpec) -> String {
    let mut args = vec!["ctx".to_string()];
    for p in &spec.params {
        match p {
            ParamKind::Str(n)
            | ParamKind::OptStr(n)
            | ParamKind::Bool(n)
            | ParamKind::U32(n)
            | ParamKind::Usize(n)
            | ParamKind::CellSlice(n) => args.push(n.clone()),
        }
    }
    args.join(", ")
}

/// Generate all `VisitorRef` forwarding methods.
fn gen_visitor_ref_methods(specs: &[CallbackSpec], _core_import: &str, protocol: &VisitorProtocol) -> String {
    let mut out = String::new();
    let result_path = protocol.result_path.clone();
    for spec in specs {
        let params = rust_param_list(spec, protocol);
        let args = visitor_ref_args(spec);
        out.push_str(&crate::backends::ffi::template_env::render(
            "vtable_delegation_method.jinja",
            minijinja::context! {
                method_name => spec.name.as_str(),
                all_params => params,
                ret => result_path.as_str(),
                arg_list => args,
            },
        ));
    }
    out
}

fn gen_result_decode_arms(
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
    default_result: &str,
) -> String {
    let mut seen_codes = std::collections::HashSet::new();
    let mut arms = String::new();
    for variant in &result_metadata.unit_variants {
        if seen_codes.insert(variant.code) {
            arms.push_str(&format!("        {} => VisitResult::{},\n", variant.code, variant.name));
        }
    }
    for variant in &result_metadata.string_payload_variants {
        if seen_codes.insert(variant.code) {
            arms.push_str(&format!(
                r#"        {} => {{
            let msg = if custom_ptr.is_null() {{
                String::new()
            }} else {{
                // SAFETY: caller guarantees this is a valid heap CString.
                let cstr = unsafe {{ std::ffi::CString::from_raw(custom_ptr) }};
                cstr.to_string_lossy().into_owned()
            }};
            VisitResult::{}(msg)
        }},
"#,
                variant.code, variant.name
            ));
        }
    }
    arms.push_str(&format!("        _ => {default_result},\n"));
    arms
}

fn context_c_type(field: &FieldDef) -> Option<&'static str> {
    match (&field.ty, field.optional) {
        (TypeRef::String, false | true) => Some("*const std::ffi::c_char"),
        (TypeRef::Primitive(PrimitiveType::Bool), false) => Some("i32"),
        (TypeRef::Primitive(PrimitiveType::U8), false) => Some("u8"),
        (TypeRef::Primitive(PrimitiveType::U16), false) => Some("u16"),
        (TypeRef::Primitive(PrimitiveType::U32), false) => Some("u32"),
        (TypeRef::Primitive(PrimitiveType::U64), false) => Some("u64"),
        (TypeRef::Primitive(PrimitiveType::I8), false) => Some("i8"),
        (TypeRef::Primitive(PrimitiveType::I16), false) => Some("i16"),
        (TypeRef::Primitive(PrimitiveType::I32), false) => Some("i32"),
        (TypeRef::Primitive(PrimitiveType::I64), false) => Some("i64"),
        (TypeRef::Primitive(PrimitiveType::Usize), false) => Some("usize"),
        (TypeRef::Primitive(PrimitiveType::Isize), false) => Some("isize"),
        _ => None,
    }
}

fn context_field_specs(context_def: &TypeDef) -> Vec<ContextFieldSpec> {
    context_def
        .fields
        .iter()
        .filter_map(|field| {
            let Some(c_type) = context_c_type(field) else {
                eprintln!(
                    "[alef] gen_visitor(ffi): skip context field `{}.{}` with unsupported type {:?}",
                    context_def.name, field.name, field.ty
                );
                return None;
            };
            let doc = field
                .doc
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or("Context field.")
                .to_string();
            let setup = match (&field.ty, field.optional) {
                (TypeRef::String, false) => format!(
                    "    let {name}_cstring = std::ffi::CString::new(ctx.{name}.as_str()).unwrap_or_default();\n",
                    name = field.name
                ),
                (TypeRef::String, true) => format!(
                    "    let {name}_cstring: Option<std::ffi::CString> = ctx\n        .{name}\n        .as_deref()\n        .and_then(|s| std::ffi::CString::new(s).ok());\n",
                    name = field.name
                ),
                _ => String::new(),
            };
            let c_init = match (&field.ty, field.optional) {
                (TypeRef::String, false) => format!("{name}: {name}_cstring.as_ptr()", name = field.name),
                (TypeRef::String, true) => format!(
                    "{name}: {name}_cstring.as_ref().map_or(std::ptr::null(), |c| c.as_ptr())",
                    name = field.name
                ),
                (TypeRef::Primitive(PrimitiveType::Bool), false) => {
                    format!("{}: i32::from(ctx.{})", field.name, field.name)
                }
                _ => format!("{}: ctx.{}", field.name, field.name),
            };
            Some(ContextFieldSpec {
                name: field.name.clone(),
                c_type,
                c_init,
                setup,
                doc,
            })
        })
        .collect()
}

fn gen_context_struct_fields(fields: &[ContextFieldSpec]) -> String {
    fields
        .iter()
        .map(|field| format!("    /// {}\n    pub {}: {},\n", field.doc, field.name, field.c_type))
        .collect()
}

fn gen_context_setup(fields: &[ContextFieldSpec]) -> String {
    fields.iter().map(|field| field.setup.as_str()).collect()
}

fn gen_context_inits(fields: &[ContextFieldSpec]) -> String {
    fields
        .iter()
        .map(|field| format!("        {},\n", field.c_init))
        .collect()
}

fn gen_result_constants(
    prefix: &str,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) -> String {
    let visit_prefix = prefix.to_uppercase();
    result_metadata
        .unit_variants
        .iter()
        .chain(result_metadata.string_payload_variants.iter())
        .map(|variant| {
            format!(
                "/// Visit-result code for `{}`.\npub const {}_VISIT_{}: i32 = {};\n",
                variant.name,
                visit_prefix,
                variant.name.to_shouty_snake_case(),
                variant.code
            )
        })
        .collect()
}

/// Generate the visitor FFI bindings block for `lib.rs`.
///
/// # Parameters
///
/// - `prefix`: the FFI function prefix (e.g. `"htm"`).
/// - `core_import`: the Rust `use` path for the core crate (e.g. `"sample_markdown_rs"`).
/// - `embed_visitor_in_options`: when `true`, the generated `{prefix}_convert_with_visitor`
///   embeds the visitor in `options.visitor` before calling the 2-argument `convert(html,
///   options)`.  Set `true` for the OptionsField bridge pattern; `false` for the legacy
///   FunctionParam pattern where `convert` takes a third visitor argument directly.
/// - `trait_def`: the IR `TypeDef` for the visitor trait, used to derive callback specs.
#[cfg(test)]
pub fn gen_visitor_bindings(
    prefix: &str,
    core_import: &str,
    embed_visitor_in_options: bool,
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
    function: Option<&FunctionDef>,
) -> String {
    gen_visitor_bindings_with_api(
        prefix,
        core_import,
        embed_visitor_in_options,
        trait_def,
        bridge_cfg,
        function,
        None,
    )
}

pub fn gen_visitor_bindings_with_api(
    prefix: &str,
    core_import: &str,
    embed_visitor_in_options: bool,
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
    function: Option<&FunctionDef>,
    api: Option<&crate::core::ir::ApiSurface>,
) -> String {
    let pascal_prefix = prefix.to_pascal_case();
    let Some(api) = api else {
        eprintln!("[alef] gen_visitor_bindings(ffi): visitor callbacks require API metadata");
        return String::new();
    };
    let Some(bridge_cfg) = bridge_cfg else {
        eprintln!("[alef] gen_visitor_bindings(ffi): visitor callbacks require trait_bridge metadata");
        return String::new();
    };
    let Some(protocol) = VisitorProtocol::from_api(api, bridge_cfg) else {
        return String::new();
    };
    let Some(context_def) = api.types.iter().find(|type_def| type_def.name == protocol.context_type) else {
        return String::new();
    };
    let Some(result_metadata) = crate::codegen::visitor_result::visitor_result_metadata(api, bridge_cfg) else {
        eprintln!(
            "[alef] gen_visitor_bindings(ffi): trait bridge `{}` result_type metadata is required",
            bridge_cfg.trait_name
        );
        return String::new();
    };
    let default_result = format!("{}::{}", protocol.result_path, result_metadata.default_variant.name);
    let result_decode_arms = gen_result_decode_arms(&result_metadata, &default_result);
    let specs = callback_specs_from_trait(trait_def, Some(bridge_cfg));
    if specs.is_empty() {
        eprintln!(
            "[alef] gen_visitor_bindings(ffi): trait `{}` has no `{}`/`{}` visitor callback methods, skipping visitor callbacks",
            trait_def.name, protocol.context_type, protocol.result_type
        );
        return String::new();
    }
    let callback_count = specs.len();
    let trait_path = trait_def.rust_path.replace('-', "_");
    let trait_name = &trait_def.name;
    let options_type = function
        .and_then(|func| visitor_options_param(func, Some(bridge_cfg)))
        .and_then(|param| named_type_ref(&param.ty))
        .or(bridge_cfg.options_type.as_deref());
    let Some(options_type) = options_type else {
        eprintln!(
            "[alef] gen_visitor_bindings(ffi): visitor callbacks require a configured or IR-derived options type, skipping visitor callbacks"
        );
        return String::new();
    };
    let options_field = bridge_cfg.resolved_options_field().unwrap_or("visitor");
    let options_path = format!("{core_import}::{options_type}");

    let context_fields = context_field_specs(context_def);
    if context_fields.is_empty() {
        eprintln!(
            "[alef] gen_visitor_bindings(ffi): context_type `{}` has no FFI-compatible fields",
            protocol.context_type
        );
        return String::new();
    }
    let result_constants = gen_result_constants(prefix, &result_metadata);
    let context_struct_fields = gen_context_struct_fields(&context_fields);
    let context_setup = gen_context_setup(&context_fields);
    let context_inits = gen_context_inits(&context_fields);
    let struct_fields = gen_struct_fields(&specs, &pascal_prefix);
    let impl_methods = gen_impl_methods(&specs, &pascal_prefix, core_import, &protocol, &default_result);
    let visitor_ref_methods = gen_visitor_ref_methods(&specs, core_import, &protocol);

    let visitor_function = function.and_then(|func| {
        visitor_function_spec(
            prefix,
            func,
            core_import,
            Some(bridge_cfg),
            embed_visitor_in_options,
            options_field,
        )
    });
    let context_path = protocol.context_path.clone();
    let result_path = protocol.result_path.clone();

    let mut out = format!(
        r#"// ---------------------------------------------------------------------------
// Visitor / callback FFI — {callback_count} {trait_name} methods
// ---------------------------------------------------------------------------

{result_constants}

/// Opaque context passed to every C callback.
///
/// Fields reflect `{context_type}` from the Rust core. All string pointers are
/// valid only for the duration of the callback invocation.
#[repr(C)]
pub struct {pascal_prefix}Context {{
{context_struct_fields}
}}

/// C-facing callback struct for the visitor pattern.
///
/// Populate the function-pointer fields you care about; leave the rest null.
/// The `user_data` pointer is forwarded unchanged to every callback — use it
/// to thread your own context through the conversion.
///
/// # Field order
///
/// The field order matches the Go binding's expected C layout exactly.
///
/// # Callback return protocol
///
/// Callbacks return an `i32` visit-result code.  When the code is
/// a string-payload variant, the callback must also write a heap-allocated,
/// null-terminated string into `*out_custom` and set `*out_len` to its byte
/// length (excluding the null terminator). The Rust side will read the string
/// and then call `free()` on the pointer.
///
/// For all other codes `out_custom` and `out_len` are not written.
///
/// # Callback signatures
///
/// All callbacks share the same leading parameters:
/// ```c
/// fn(ctx, user_data, out_custom, out_len, ...) -> i32
/// ```
/// followed by method-specific parameters documented on each field.
#[repr(C)]
pub struct {pascal_prefix}VisitorCallbacks {{
    /// Arbitrary caller context forwarded to every callback.
    pub user_data: *mut std::ffi::c_void,
{struct_fields}}}

// SAFETY: The `user_data` pointer is the caller's responsibility. We require
// callers to uphold thread-safety themselves (i.e. not share a visitor across
// threads without synchronisation). The callbacks themselves are `extern "C"`
// and therefore inherently `Send`.
unsafe impl Send for {pascal_prefix}VisitorCallbacks {{}}
// SAFETY: see Send impl above; the callbacks struct is effectively a POD vtable.
unsafe impl Sync for {pascal_prefix}VisitorCallbacks {{}}

/// Opaque handle wrapping a `{pascal_prefix}VisitorCallbacks` and implementing
/// the Rust `{trait_name}` trait.
///
/// Allocate with `{prefix}_visitor_create` and release with `{prefix}_visitor_free`.
/// The handle must NOT outlive the `{pascal_prefix}VisitorCallbacks` it was created from.
pub struct {pascal_prefix}Visitor {{
    callbacks: {pascal_prefix}VisitorCallbacks,
    /// CString storage for tag names / parent tags that we pass back to C.
    /// RefCell is used for interior mutability; it is Send (Vec<CString> is Send) and
    /// the outer Arc<Mutex> serialises all access, so Sync is not required on RefCell itself.
    _tag_scratch: std::cell::RefCell<Vec<std::ffi::CString>>,
}}

// SAFETY: {pascal_prefix}Visitor is only accessed through the outer Arc<Mutex<dyn {trait_name} + Send>>
// which serialises access. The `user_data` pointer is the caller's responsibility.
unsafe impl Send for {pascal_prefix}Visitor {{}}
// SAFETY: see Send impl above; Sync is safe because all mutation goes through Mutex.
unsafe impl Sync for {pascal_prefix}Visitor {{}}

impl std::fmt::Debug for {pascal_prefix}Visitor {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        f.debug_struct("{pascal_prefix}Visitor").finish_non_exhaustive()
    }}
}}

/// Map a visit-result integer code + optional custom string pointer back to
/// the Rust result enum.
///
/// # Safety
///
/// `custom_ptr` must be either null or a pointer to a heap-allocated
/// null-terminated string that this function will take ownership of (freeing
/// it after reading).
unsafe fn decode_visit_result(
    code: i32,
    custom_ptr: *mut std::ffi::c_char,
) -> {result_path} {{
    use {result_path} as VisitResult;
    match code {{
{result_decode_arms}
    }}
}}

/// Build a temporary `{pascal_prefix}Context` from a Rust `{context_type}`, invoke
/// the provided callback, and decode the result.
///
/// The context passed to the C callback is only valid for the duration
/// of this function call.
unsafe fn call_with_ctx<F>(
    ctx: &{context_path},
    callback: F,
) -> {result_path}
where
    F: FnOnce(
        *const {pascal_prefix}Context,
        *mut *mut std::ffi::c_char,
        *mut usize,
    ) -> i32,
{{
{context_setup}

    let c_ctx = {pascal_prefix}Context {{
{context_inits}
    }};

    let mut out_custom: *mut std::ffi::c_char = std::ptr::null_mut();
    let mut out_len: usize = 0;

    let code = callback(&c_ctx, &mut out_custom, &mut out_len);

    // SAFETY: decode_visit_result takes ownership of out_custom when non-null.
    unsafe {{ decode_visit_result(code, out_custom) }}
}}

/// Convert an `Option<&str>` to a C pointer: non-null CString when `Some`, null when `None`.
///
/// Returns `(ptr, Option<CString>)` — the `Option<CString>` must be kept alive
/// until after the pointer is consumed by the callback.
fn opt_str_to_c(s: Option<&str>) -> (*const std::ffi::c_char, Option<std::ffi::CString>) {{
    match s {{
        Some(val) => match std::ffi::CString::new(val) {{
            Ok(cs) => {{
                let ptr = cs.as_ptr();
                (ptr, Some(cs))
            }}
            Err(_) => (std::ptr::null(), None),
        }},
        None => (std::ptr::null(), None),
    }}
}}

impl {trait_path} for {pascal_prefix}Visitor {{
{impl_methods}}}

/// Create a new visitor handle from a callbacks struct.
///
/// The returned handle must be freed with `{prefix}_visitor_free`.
/// The `{pascal_prefix}VisitorCallbacks` struct is **copied** into the handle;
/// the caller may free it after this call returns.
///
/// Returns null on allocation failure.
///
/// # Safety
///
/// `callbacks` must point to a valid, fully initialised `{pascal_prefix}VisitorCallbacks`.
/// `user_data` (embedded in the struct) must remain valid and accessible from
/// any thread that calls `{prefix}_convert_with_visitor` until after
/// `{prefix}_visitor_free` is called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_visitor_create(
    callbacks: *const {pascal_prefix}VisitorCallbacks,
) -> *mut {pascal_prefix}Visitor {{
    if callbacks.is_null() {{
        return std::ptr::null_mut();
    }}
    // SAFETY: caller guarantees the pointer is valid.
    let cbs = unsafe {{ callbacks.read() }};
    let visitor = {pascal_prefix}Visitor {{
        callbacks: cbs,
        _tag_scratch: std::cell::RefCell::new(Vec::new()),
    }};
    Box::into_raw(Box::new(visitor))
}}

/// Free a visitor handle previously returned by `{prefix}_visitor_create`.
///
/// After this call the pointer is invalid and must not be used.
///
/// # Safety
///
/// `visitor` must have been returned by `{prefix}_visitor_create`, or be null.
/// Passing a null pointer is safe and has no effect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_visitor_free(visitor: *mut {pascal_prefix}Visitor) {{
    if !visitor.is_null() {{
        // SAFETY: visitor was created with Box::into_raw.
        unsafe {{ drop(Box::from_raw(visitor)); }}
    }}
}}

/// Attach a visitor to an options handle before calling `{prefix}_convert`.
///
/// The visitor will be invoked during conversion via the normal `{prefix}_convert` path.
/// The `visitor` pointer must remain valid until after `{prefix}_convert` returns.
///
/// Passing `null` for either argument is a no-op.
///
/// # Safety
///
/// `options` must be a non-null pointer returned by `{prefix}_conversion_options_from_json`,
/// valid for write access.  `visitor` must be a non-null pointer returned by
/// `{prefix}_visitor_create`, or null.  Both must remain valid for the duration of any
/// subsequent `{prefix}_convert` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_options_set_visitor_handle(
    options: *mut {options_path},
    visitor: *mut {pascal_prefix}Visitor,
) {{
    if options.is_null() || visitor.is_null() {{
        return;
    }}
    struct VisitorRef(*mut {pascal_prefix}Visitor);
    impl std::fmt::Debug for VisitorRef {{
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
            f.debug_struct("VisitorRef").finish_non_exhaustive()
        }}
    }}
    // SAFETY: VisitorRef is a thin wrapper around a raw pointer to {pascal_prefix}Visitor which
    // is itself Send + Sync. The caller guarantees the pointer remains valid during conversion.
    unsafe impl Send for VisitorRef {{}}
    // SAFETY: see Send impl above.
    unsafe impl Sync for VisitorRef {{}}
    impl {trait_path} for VisitorRef {{
{visitor_ref_methods}    }}
    // SAFETY: options is non-null (checked above); caller guarantees it is valid for write.
    let options_ref = unsafe {{ &mut *options }};
    options_ref.{options_field} = Some(std::sync::Arc::new(std::sync::Mutex::new(VisitorRef(visitor))));
}}"#,
        prefix = prefix,
        pascal_prefix = pascal_prefix,
        callback_count = callback_count,
        trait_name = trait_name,
        context_type = protocol.context_type,
        context_path = context_path,
        result_constants = result_constants,
        result_path = result_path,
        result_decode_arms = result_decode_arms,
        context_struct_fields = context_struct_fields,
        context_setup = context_setup,
        context_inits = context_inits,
        trait_path = trait_path,
        options_path = options_path,
        options_field = options_field,
        struct_fields = struct_fields,
        impl_methods = impl_methods,
        visitor_ref_methods = visitor_ref_methods,
    );

    if let Some(visitor_function) = visitor_function {
        out.push_str(&format!(
            r#"
/// Run conversion using a callback-based visitor.
///
/// Returns a heap-allocated result on success, or null on failure.
/// Check `{prefix}_last_error_code` / `{prefix}_last_error_context` for error details.
/// The returned pointer must be freed with the matching result free function.
///
/// # Safety
///
/// `html` must be a valid, non-null, null-terminated UTF-8 string.
/// `options` must be a valid pointer or null.
/// `visitor` must have been created with `{prefix}_visitor_create`, or be null.
/// Returned pointer must be freed with the matching result free function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {with_visitor_fn_name}(
    {params}
    visitor: *mut {pascal_prefix}Visitor,
) -> *mut {return_type} {{
    clear_last_error();

{param_conversions}
    struct VisitorRef(*mut {pascal_prefix}Visitor);
    impl std::fmt::Debug for VisitorRef {{
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
            f.debug_struct("VisitorRef").finish_non_exhaustive()
        }}
    }}
    // SAFETY: VisitorRef is a thin wrapper around a raw pointer to {pascal_prefix}Visitor which
    // is itself Send + Sync. The caller guarantees the pointer remains valid during conversion.
    unsafe impl Send for VisitorRef {{}}
    // SAFETY: see Send impl above.
    unsafe impl Sync for VisitorRef {{}}
    impl {trait_path} for VisitorRef {{
{visitor_ref_methods}    }}
    let visitor_handle: Option<std::sync::Arc<std::sync::Mutex<dyn {trait_path} + Send>>> = if visitor.is_null() {{
        None
    }} else {{
        Some(std::sync::Arc::new(std::sync::Mutex::new(VisitorRef(visitor))))
    }};

{call}
        Ok(result) => Box::into_raw(Box::new(result)),
        Err(e) => {{
            set_last_error(2, &e.to_string());
            std::ptr::null_mut()
        }}
    }}
}}
"#,
            prefix = prefix,
            with_visitor_fn_name = visitor_function.fn_name,
            pascal_prefix = pascal_prefix,
            trait_path = trait_path,
            visitor_ref_methods = visitor_ref_methods,
            params = visitor_function.ffi_params,
            param_conversions = visitor_function.param_conversions,
            return_type = visitor_function.return_type,
            call = visitor_function.call,
        ));
    }

    out
}

/// Generate `{prefix}_convert` — the real no-visitor implementation of the core `convert`
/// function.
///
/// When `visitor_callbacks = true`, the core `convert` function has a visitor parameter
/// that causes the IR to sanitize the function (marking it as unimplementable via the normal
/// codegen path).  Instead of emitting a stub, the FFI generator calls this function to
/// produce a proper implementation that passes `None` for the visitor.
///
/// The generated function takes `html` and `options` (no visitor param) and returns a
/// heap-allocated result that the caller must free with the matching result free function.
pub fn gen_convert_no_visitor(
    prefix: &str,
    core_import: &str,
    bridge_cfg: Option<&TraitBridgeConfig>,
    function: Option<&FunctionDef>,
) -> String {
    let Some(function) = function else {
        eprintln!(
            "[alef] gen_convert_no_visitor(ffi): visitor callbacks require a matching public function, skipping no-visitor wrapper"
        );
        return String::new();
    };
    let visitor_function = no_visitor_function_spec(prefix, function, core_import, bridge_cfg);
    format!(
        r#"/// Run conversion.
///
/// Returns a heap-allocated result on success, or null on failure.
/// Check `{prefix}_last_error_code` / `{prefix}_last_error_context` for error details.
/// The returned pointer must be freed with the matching result free function.
///
/// # Arguments
///
/// - `html`: null-terminated, UTF-8 HTML input. Must not be null.
/// - `options`: optional function options; pass null for defaults.
///
/// # Safety
///
/// `html` must be a valid, non-null, null-terminated UTF-8 string.
/// `options` must be a valid pointer or null.
/// Returned pointer must be freed with the matching result free function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_name}(
    {params}
) -> *mut {return_type} {{
    clear_last_error();

{param_conversions}
{call}
        Ok(result) => Box::into_raw(Box::new(result)),
        Err(e) => {{
            set_last_error(2, &e.to_string());
            std::ptr::null_mut()
        }}
    }}
}}"#,
        prefix = prefix,
        fn_name = visitor_function.fn_name,
        params = visitor_function.ffi_params,
        return_type = visitor_function.return_type,
        param_conversions = visitor_function.param_conversions,
        call = visitor_function.call,
    )
}

struct LegacyVisitorFunctionSpec {
    fn_name: String,
    ffi_params: String,
    param_conversions: String,
    return_type: String,
    call: String,
}

struct LegacyNoVisitorFunctionSpec {
    fn_name: String,
    ffi_params: String,
    param_conversions: String,
    return_type: String,
    call: String,
}

fn visitor_function_spec(
    prefix: &str,
    func: &FunctionDef,
    core_import: &str,
    bridge_cfg: Option<&TraitBridgeConfig>,
    embed_visitor_in_options: bool,
    options_field: &str,
) -> Option<LegacyVisitorFunctionSpec> {
    let mut param_conversions = String::new();
    let mut call_args = Vec::new();
    let mut ffi_params = Vec::new();
    let options_param_name = visitor_options_param(func, bridge_cfg).map(|param| param.name.as_str());

    for param in &func.params {
        if is_bridge_param(param, bridge_cfg) {
            call_args.push("visitor_handle".to_string());
            continue;
        }
        ffi_params.push(ffi_param_decl(param, core_import));
        param_conversions.push_str(&param_conversion(param, core_import));
        call_args.push(rust_call_arg(param));
    }

    let call = if embed_visitor_in_options {
        if let Some(options_param_name) = options_param_name {
            let options_local = format!("{options_param_name}_rs");
            let Some(options_path) = visitor_options_param(func, bridge_cfg)
                .and_then(|param| named_type_ref(&param.ty))
                .map(|name| rust_named_path(core_import, name))
            else {
                eprintln!(
                    "[alef] gen_visitor_bindings(ffi): options-field visitor wrapper requires an options parameter, skipping with-visitor wrapper"
                );
                return None;
            };
            for arg in &mut call_args {
                if arg == &options_local {
                    *arg = "options_with_visitor".to_string();
                }
            }
            format!(
                "    let mut options_with_visitor: Option<{options_path}> = {options_local};\n\
                 if visitor_handle.is_some() {{\n\
                     let opts = options_with_visitor.get_or_insert_with({options_path}::default);\n\
                     opts.{options_field} = visitor_handle;\n\
                 }}\n\
                 match {core_import}::{function_name}({call_args}) {{",
                function_name = func.name,
                call_args = call_args.join(", "),
            )
        } else {
            format!(
                "    match {core_import}::{function_name}({call_args}) {{",
                function_name = func.name,
                call_args = call_args.join(", "),
            )
        }
    } else {
        format!(
            "    match {core_import}::{function_name}({call_args}) {{",
            function_name = func.name,
            call_args = call_args.join(", "),
        )
    };

    Some(LegacyVisitorFunctionSpec {
        fn_name: format!("{}_{}_with_visitor", prefix, func.name.to_snake_case()),
        ffi_params: if ffi_params.is_empty() {
            String::new()
        } else {
            format!("{},\n   ", ffi_params.join(",\n    "))
        },
        param_conversions,
        return_type: return_type_path(&func.return_type, core_import),
        call,
    })
}

fn no_visitor_function_spec(
    prefix: &str,
    func: &FunctionDef,
    core_import: &str,
    bridge_cfg: Option<&TraitBridgeConfig>,
) -> LegacyNoVisitorFunctionSpec {
    let mut param_conversions = String::new();
    let mut call_args = Vec::new();
    let mut ffi_params = Vec::new();

    for param in &func.params {
        if is_bridge_param(param, bridge_cfg) {
            call_args.push("None".to_string());
            continue;
        }
        ffi_params.push(ffi_param_decl(param, core_import));
        param_conversions.push_str(&param_conversion(param, core_import));
        call_args.push(rust_call_arg(param));
    }

    LegacyNoVisitorFunctionSpec {
        fn_name: format!("{}_{}", prefix, func.name.to_snake_case()),
        ffi_params: ffi_params.join(",\n    "),
        param_conversions,
        return_type: return_type_path(&func.return_type, core_import),
        call: format!(
            "    match {core_import}::{function_name}({call_args}) {{",
            function_name = func.name,
            call_args = call_args.join(", "),
        ),
    }
}

fn named_type_ref(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => named_type_ref(inner),
        _ => None,
    }
}

fn rust_named_path(core_import: &str, name: &str) -> String {
    format!("{core_import}::{name}")
}

fn return_type_path(ty: &TypeRef, core_import: &str) -> String {
    named_type_ref(ty)
        .map(|name| rust_named_path(core_import, name))
        .unwrap_or_else(|| "()".to_string())
}

fn is_bridge_param(param: &ParamDef, bridge_cfg: Option<&TraitBridgeConfig>) -> bool {
    let Some(bridge_cfg) = bridge_cfg else {
        return false;
    };
    bridge_cfg.param_name.as_deref() == Some(param.name.as_str())
        || bridge_cfg.type_alias.as_deref() == named_type_ref(&param.ty)
}

fn visitor_options_param<'a>(func: &'a FunctionDef, bridge_cfg: Option<&TraitBridgeConfig>) -> Option<&'a ParamDef> {
    if let Some(options_type) = bridge_cfg.and_then(|cfg| cfg.options_type.as_deref()) {
        return func
            .params
            .iter()
            .find(|param| named_type_ref(&param.ty) == Some(options_type));
    }
    func.params
        .iter()
        .find(|param| !is_bridge_param(param, bridge_cfg) && named_type_ref(&param.ty).is_some())
}

fn ffi_param_decl(param: &ParamDef, core_import: &str) -> String {
    match &param.ty {
        TypeRef::String | TypeRef::Path => format!("{}: *const std::ffi::c_char", param.name),
        TypeRef::Named(name) => {
            format!("{}: *const {}", param.name, rust_named_path(core_import, name))
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => {
                format!("{}: *const {}", param.name, rust_named_path(core_import, name))
            }
            _ => format!("{}: *const std::ffi::c_void", param.name),
        },
        _ => format!("{}: *const std::ffi::c_void", param.name),
    }
}

fn param_conversion(param: &ParamDef, core_import: &str) -> String {
    match &param.ty {
        TypeRef::String | TypeRef::Path => format!(
            r#"    if {name}.is_null() {{
        set_last_error(1, "Null pointer passed for {name}");
        return std::ptr::null_mut();
    }}
    // SAFETY: null check above guarantees {name} is a valid pointer.
    let {name}_rs = match unsafe {{ std::ffi::CStr::from_ptr({name}) }}.to_str() {{
        Ok(s) => s,
        Err(_) => {{
            set_last_error(1, "Invalid UTF-8 in {name} parameter");
            return std::ptr::null_mut();
        }}
    }};
"#,
            name = param.name,
        ),
        TypeRef::Named(name) => named_param_conversion(&param.name, core_import, name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => named_param_conversion(&param.name, core_import, name),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn named_param_conversion(param_name: &str, core_import: &str, type_name: &str) -> String {
    let path = rust_named_path(core_import, type_name);
    format!(
        r#"    let {name}_rs: Option<{path}> = if {name}.is_null() {{
        None
    }} else {{
        // SAFETY: {name} is a valid pointer guaranteed by the caller.
        Some(unsafe {{ &*{name} }}.clone())
    }};
"#,
        name = param_name,
        path = path,
    )
}

fn rust_call_arg(param: &ParamDef) -> String {
    match &param.ty {
        TypeRef::String | TypeRef::Path if param.is_ref => format!("&{}_rs", param.name),
        TypeRef::String | TypeRef::Path => format!("{}_rs", param.name),
        TypeRef::Named(_) | TypeRef::Optional(_) => format!("{}_rs", param.name),
        _ => param.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ReceiverKind, TypeDef};

    fn param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            is_ref,
            ..ParamDef::default()
        }
    }

    fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Callback method.".to_string(),
            receiver: Some(ReceiverKind::RefMut),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn visitor_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("my_lib::visitor::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn bridge_config(
        trait_name: &str,
        options_type: &str,
        context_type: Option<&str>,
        result_type: Option<&str>,
    ) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            type_alias: Some("VisitorHandle".to_string()),
            param_name: Some("visitor".to_string()),
            options_type: Some(options_type.to_string()),
            context_type: context_type.map(str::to_string),
            result_type: result_type.map(str::to_string),
            ..TraitBridgeConfig::default()
        }
    }

    fn protocol_api(context_name: &str, result_name: &str, default_variant: &str) -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: context_name.to_string(),
                rust_path: format!("my_lib::visitor::{context_name}"),
                fields: vec![
                    FieldDef {
                        name: "tag_name".to_string(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "depth".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Usize),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            }],
            functions: vec![],
            enums: vec![EnumDef {
                name: result_name.to_string(),
                rust_path: format!("my_lib::visitor::{result_name}"),
                variants: vec![
                    EnumVariant {
                        name: "Continue".to_string(),
                        is_default: default_variant == "Continue",
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "Proceed".to_string(),
                        is_default: default_variant == "Proceed",
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "Custom".to_string(),
                        fields: vec![FieldDef {
                            name: "value".to_string(),
                            ty: TypeRef::String,
                            ..FieldDef::default()
                        }],
                        ..EnumVariant::default()
                    },
                ],
                ..EnumDef::default()
            }],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: Default::default(),
            services: vec![],
            handler_contracts: vec![],
                unsupported_public_items: Vec::new(),
    }

    #[test]
    fn visitor_bindings_use_trait_name_and_callback_count_from_ir() {
        let trait_def = visitor_trait(
            "MarkdownVisitor",
            vec![method(
                "visit_text",
                vec![
                    param("ctx", TypeRef::Named("NodeContext".to_string()), true),
                    param("text", TypeRef::String, true),
                ],
                TypeRef::Named("VisitResult".to_string()),
            )],
        );

        let bridge_cfg = bridge_config(
            "MarkdownVisitor",
            "RenderOptions",
            Some("NodeContext"),
            Some("VisitResult"),
        );
        let api = protocol_api("NodeContext", "VisitResult", "Continue");
        let code =
            gen_visitor_bindings_with_api("md", "my_lib", false, &trait_def, Some(&bridge_cfg), None, Some(&api));

        assert!(code.contains("// Visitor / callback FFI — 1 MarkdownVisitor methods"));
        assert!(code.contains("dyn MarkdownVisitor + Send"));
        assert!(code.contains("options: *mut my_lib::RenderOptions"));
        assert!(code.contains("MD_VISIT_CUSTOM"));
        assert!(!code.contains("fn md_convert_with_visitor"));
        assert!(!code.contains("all 42 HtmlVisitor methods"));
        assert!(!code.contains("dyn HtmlVisitor + Send"));
        assert!(!code.contains("`HTM_VISIT_CUSTOM`"));
    }

    #[test]
    fn visitor_bindings_skip_traits_without_node_context_visit_result_protocol() {
        let trait_def = visitor_trait(
            "PlainVisitor",
            vec![method(
                "visit_text",
                vec![
                    param("context", TypeRef::Named("OtherContext".to_string()), true),
                    param("text", TypeRef::String, true),
                ],
                TypeRef::String,
            )],
        );

        let bridge_cfg = bridge_config("PlainVisitor", "PlainOptions", None, None);
        let code = gen_visitor_bindings("pln", "my_lib", false, &trait_def, Some(&bridge_cfg), None);

        assert!(code.is_empty());
    }

    #[test]
    fn visitor_bindings_use_configured_context_and_result_type_names() {
        let trait_def = visitor_trait(
            "RenderVisitor",
            vec![method(
                "visit_text",
                vec![
                    param("context", TypeRef::Named("RenderContext".to_string()), true),
                    param("text", TypeRef::String, true),
                ],
                TypeRef::Named("RenderDecision".to_string()),
            )],
        );
        let bridge_cfg = bridge_config(
            "RenderVisitor",
            "RenderOptions",
            Some("RenderContext"),
            Some("RenderDecision"),
        );
        let api = protocol_api("RenderContext", "RenderDecision", "Continue");

        let code =
            gen_visitor_bindings_with_api("doc", "my_lib", false, &trait_def, Some(&bridge_cfg), None, Some(&api));

        assert!(code.contains("ctx: &my_lib::visitor::RenderContext"));
        assert!(code.contains(") -> my_lib::visitor::RenderDecision"));
        assert!(code.contains("use my_lib::visitor::RenderDecision as VisitResult"));
        assert!(code.contains("return my_lib::visitor::RenderDecision::Continue"));
        assert!(!code.contains("ctx: &my_lib::visitor::NodeContext"));
        assert!(!code.contains(") -> my_lib::visitor::VisitResult"));
    }

    #[test]
    fn visitor_bindings_use_derived_default_result_variant() {
        let trait_def = visitor_trait(
            "RenderVisitor",
            vec![method(
                "visit_text",
                vec![param("context", TypeRef::Named("RenderContext".to_string()), true)],
                TypeRef::Named("RenderDecision".to_string()),
            )],
        );
        let bridge_cfg = bridge_config(
            "RenderVisitor",
            "RenderOptions",
            Some("RenderContext"),
            Some("RenderDecision"),
        );
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "RenderContext".to_string(),
                rust_path: "my_lib::visitor::RenderContext".to_string(),
                fields: vec![FieldDef {
                    name: "tag_name".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            }],
            functions: vec![],
            enums: vec![EnumDef {
                name: "RenderDecision".to_string(),
                rust_path: "my_lib::visitor::RenderDecision".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "Proceed".to_string(),
                        is_default: true,
                        ..EnumVariant::default()
                    },
                    EnumVariant {
                        name: "ReplaceWith".to_string(),
                        fields: vec![FieldDef {
                            name: "value".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
                            original_type: None,
                        }],
                        is_tuple: true,
                        ..EnumVariant::default()
                    },
                ],
                ..EnumDef::default()
            }],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: Default::default(),
            services: vec![],
            handler_contracts: vec![],
                unsupported_public_items: Vec::new(),
};

        let code =
            gen_visitor_bindings_with_api("doc", "my_lib", false, &trait_def, Some(&bridge_cfg), None, Some(&api));

        assert!(code.contains("return my_lib::visitor::RenderDecision::Proceed"));
        assert!(code.contains("_ => my_lib::visitor::RenderDecision::Proceed"));
        assert!(!code.contains("RenderDecision::Continue"));
    }
}
