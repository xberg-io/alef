/// Generate Go visitor interface, CGo callback trampolines, and `ConvertWithVisitor`.
///
/// # CGo callback strategy
///
/// CGo does not allow passing Go function values as C function pointers. The generated
/// code uses a handle-table pattern:
///
/// - A package-level `sync.Map` keyed by `uintptr` stores active `Visitor` values.
/// - A counter provides unique IDs; the ID is cast to `unsafe.Pointer` and passed as
///   `user_data` to every C callback.
/// - Package-level `//export goVisit*` functions look up the visitor by ID and call
///   the appropriate method.
/// - A static C helper in the CGo preamble constructs the VTable by referencing all
///   exported Go trampolines — this is valid because CGo compiles the preamble together
///   with the Go file that carries the `//export` declarations.
/// - `ConvertWithVisitor` registers the visitor, builds the VTable via the static C
///   helper, then calls `{prefix}_{bridge_snake}_new` + `{prefix}_options_set_{field}`
///   + `{prefix}_convert` to perform conversion.
///
/// # VTable ABI
///
/// Each function pointer in the VTable has the signature:
///   `(user_data: void*, ctx: char* /* JSON */, ...extras..., out_result: char**) -> int32_t`
///
/// `user_data` is the first argument; `ctx` is a JSON-encoded context value; `out_result`
/// receives a heap-allocated C string when the visitor returns a Custom/Error result.
///
/// This differs from the legacy `VisitorCallbacks` pattern (FunctionParam bind_via), where
/// `user_data` was a FIELD on the struct and context was a typed pointer.
use crate::core::{
    config::TraitBridgeConfig,
    hash::{self, CommentStyle},
    ir::{ApiSurface, EnumDef, FunctionDef, TypeDef, TypeRef},
};
use serde_json;

/// Derive the cbindgen-generated C type name for a Rust FFI type.
///
/// cbindgen prepends the uppercased `ffi_prefix` to the Rust struct name verbatim.
/// Example: prefix="abc", Rust name="AbcRendererVTable" → "ABCAbcRendererVTable".
///
/// Note: the Rust struct name already includes the pascal-case prefix segment
/// (e.g. `Htm`), so only the uppercase prefix is prepended here.
pub(crate) fn ffi_c_type_name(ffi_prefix: &str, rust_basename: &str) -> String {
    let prefix_upper = ffi_prefix.to_uppercase();
    format!("{prefix_upper}{rust_basename}")
}

/// A single visitor callback specification.
struct CallbackSpec {
    /// Field name in the C VTable struct (snake_case).
    c_field: String,
    /// Exported Go function name (e.g. `"goVisitText"`).
    export_name: String,
    /// Go interface method name (PascalCase).
    go_method: String,
    /// Doc comment for the Go interface method.
    doc: String,
    /// Extra C parameters after `(user_data, ctx)` and before `(out_result)`.
    /// Each entry: (c_param_name, c_type, go_var_name, go_type_in_interface, decode_expr).
    /// `decode_expr` is the Go expression to convert the C parameter to the Go interface type.
    extra: Vec<ExtraParam>,
    /// If true, add an `isHeader C.int32_t` parameter (only for visit_table_row).
    has_is_header: bool,
    /// Named context type used by the visitor method.
    context_type: String,
    /// Named result type returned by the visitor method.
    result_type: String,
}

struct ExtraParam {
    c_name: String,
    c_type: String,
    go_name: String,
    /// Type as it appears in the Go interface method signature.
    go_iface_type: String,
    /// Expression to convert the C value to `go_iface_type`.
    decode: String,
}

/// Convert snake_case to lowerCamelCase (e.g. "tag_name" → "tagName").
fn snake_to_lower_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut next_upper = false;
    for ch in s.chars() {
        if ch == '_' {
            next_upper = true;
        } else if next_upper {
            result.extend(ch.to_uppercase());
            next_upper = false;
        } else {
            result.push(ch);
        }
    }
    result
}

struct VisitorAssociatedTypes {
    context_type: String,
    result_type: String,
}

#[derive(Clone, serde::Serialize)]
struct VisitorCodecMetadata {
    context_fields: Vec<ContextFieldMetadata>,
    result_variants: Vec<ResultVariantMetadata>,
    default_result_helper_name: String,
    default_result_wire_name: String,
}

#[derive(Clone, serde::Serialize)]
struct ContextFieldMetadata {
    doc: String,
    go_name: String,
    go_type: String,
    json_name: String,
}

#[derive(Clone, serde::Serialize)]
struct ResultVariantMetadata {
    code: i32,
    helper_name: String,
    name: String,
    wire_name: String,
    has_payload: bool,
    is_default: bool,
    payload_name: String,
    payload_go_name: String,
}

fn visitor_associated_types(
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: &TraitBridgeConfig,
) -> Option<VisitorAssociatedTypes> {
    let Some(context_type) = bridge_cfg.context_type.as_deref() else {
        eprintln!(
            "[alef] gen_visitor(go): trait bridge `{}` must configure context_type",
            bridge_cfg.trait_name
        );
        return None;
    };
    let Some(result_type) = bridge_cfg.result_type.as_deref() else {
        eprintln!(
            "[alef] gen_visitor(go): trait bridge `{}` must configure result_type",
            bridge_cfg.trait_name
        );
        return None;
    };

    let has_matching_method = trait_def.methods.iter().any(|method| {
        method.trait_source.is_none()
            && named_type_name(&method.return_type) == Some(result_type)
            && method
                .params
                .iter()
                .any(|param| named_type_name(&param.ty) == Some(context_type))
    });

    has_matching_method.then(|| VisitorAssociatedTypes {
        context_type: context_type.to_string(),
        result_type: result_type.to_string(),
    })
}

/// Build a `Vec<CallbackSpec>` from a trait's IR definition for the Go backend.
///
/// Derives all language-specific Go fields (method names, C types, decode expressions)
/// from `TypeRef` + `optional` flag. Methods with unsupported parameter types are
/// skipped with a warning.
fn callback_specs_from_trait(
    trait_def: &crate::core::ir::TypeDef,
    associated_types: &VisitorAssociatedTypes,
) -> Vec<CallbackSpec> {
    use crate::core::ir::{PrimitiveType, TypeRef};
    use heck::ToPascalCase;

    let mut specs = Vec::with_capacity(trait_def.methods.len());
    'methods: for m in &trait_def.methods {
        if m.trait_source.is_some() {
            continue;
        }
        if named_type_name(&m.return_type) != Some(associated_types.result_type.as_str()) {
            continue;
        }
        let has_context_param = m
            .params
            .iter()
            .any(|param| named_type_name(&param.ty) == Some(associated_types.context_type.as_str()));
        if !has_context_param {
            continue;
        }
        let go_method = m.name.to_pascal_case();
        let export_name = format!("go{go_method}");
        let first_line = m.doc.lines().next().unwrap_or("").trim().to_string();
        let doc = if first_line.is_empty() {
            format!("{go_method} visits this element.")
        } else {
            format!("{go_method} {}", first_line.to_lowercase().trim_end_matches('.'))
        };

        let mut extra = Vec::new();
        let mut has_is_header = false;

        for p in &m.params {
            if named_type_name(&p.ty) == Some(associated_types.context_type.as_str()) {
                continue;
            }
            let param_name = p.name.trim_start_matches('_').to_string();
            let go_param_name = snake_to_lower_camel(&param_name);

            match (&p.ty, p.optional) {
                (TypeRef::String, false) => {
                    let decode = format!("C.GoString({param_name})");
                    extra.push(ExtraParam {
                        c_name: param_name,
                        c_type: "*C.char".to_string(),
                        go_name: go_param_name,
                        go_iface_type: "string".to_string(),
                        decode,
                    });
                }
                (TypeRef::String, true) => {
                    let decode = format!("optGoString({param_name})");
                    extra.push(ExtraParam {
                        c_name: param_name,
                        c_type: "*C.char".to_string(),
                        go_name: go_param_name,
                        go_iface_type: "*string".to_string(),
                        decode,
                    });
                }
                (TypeRef::Primitive(PrimitiveType::Bool), false) => {
                    let decode = format!("{param_name} != 0");
                    extra.push(ExtraParam {
                        c_name: param_name.clone(),
                        c_type: "C.int32_t".to_string(),
                        go_name: go_param_name.clone(),
                        go_iface_type: "bool".to_string(),
                        decode,
                    });
                }
                (TypeRef::Primitive(PrimitiveType::U32 | PrimitiveType::I32), false) => {
                    let decode = format!("uint32({param_name})");
                    extra.push(ExtraParam {
                        c_name: param_name.clone(),
                        c_type: "C.uint32_t".to_string(),
                        go_name: go_param_name.clone(),
                        go_iface_type: "uint32".to_string(),
                        decode,
                    });
                }
                (TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64), false) => {
                    let decode = format!("uint({param_name})");
                    extra.push(ExtraParam {
                        c_name: param_name.clone(),
                        c_type: "C.uintptr_t".to_string(),
                        go_name: go_param_name.clone(),
                        go_iface_type: "uint".to_string(),
                        decode,
                    });
                }
                (TypeRef::Vec(inner), false) => match inner.as_ref() {
                    TypeRef::String => {
                        let decode = format!("decodeCellsSlice({param_name}, cellCount)");
                        extra.push(ExtraParam {
                            c_name: param_name.clone(),
                            c_type: "*C.char".to_string(),
                            go_name: go_param_name.clone(),
                            go_iface_type: "[]string".to_string(),
                            decode,
                        });
                        has_is_header = true;
                        break;
                    }
                    _ => {
                        eprintln!(
                            "[alef] gen_visitor(go): skip method `{}` — unsupported Vec param `{}`",
                            m.name, p.name
                        );
                        continue 'methods;
                    }
                },
                _ => {
                    eprintln!(
                        "[alef] gen_visitor(go): skip method `{}` — unsupported param `{}: {:?}`",
                        m.name, p.name, p.ty
                    );
                    continue 'methods;
                }
            }
        }

        specs.push(CallbackSpec {
            c_field: m.name.clone(),
            export_name,
            go_method,
            doc,
            extra,
            has_is_header,
            context_type: associated_types.context_type.clone(),
            result_type: associated_types.result_type.clone(),
        });
    }
    specs
}

/// Generate the complete visitor.go file content for the options-field VTable ABI.
///
/// # Parameters
///
/// - `pkg_name`: Go package name (e.g. `"samplemarkdown"`).
/// - `ffi_prefix`: C function prefix (e.g. `"htm"`).
/// - `ffi_header`: C header filename.
/// - `ffi_crate_dir`: path from go output dir to the FFI crate dir.
/// - `to_root`: relative path from go output dir to the repo root.
/// - `vtable_trait_name`: Rust trait name used to derive the VTable struct name
///   (e.g. `"Renderer"` → `"AbcRendererVTable"`).
/// - `options_field`: configured field name on the options type that holds the bridge.
#[allow(clippy::too_many_arguments)]
pub fn gen_visitor_file(
    api: &ApiSurface,
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
    vtable_trait_name: &str,
    options_field: &str,
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    bridge_func: &FunctionDef,
) -> String {
    let Some(associated_types) = visitor_associated_types(trait_def, bridge_cfg) else {
        eprintln!(
            "[alef] gen_visitor(go): bridge `{}` has no compatible visitor callback methods, skipping visitor.go",
            bridge_cfg.trait_name
        );
        return String::new();
    };
    let specs = callback_specs_from_trait(trait_def, &associated_types);
    if specs.is_empty() {
        eprintln!(
            "[alef] gen_visitor(go): bridge `{}` has no supported visitor callback methods, skipping visitor.go",
            bridge_cfg.trait_name
        );
        return String::new();
    }
    let Some(codec_metadata) = visitor_codec_metadata(api, &associated_types) else {
        eprintln!(
            "[alef] gen_visitor(go): bridge `{}` requires IR metadata for context_type `{}` and result_type `{}`",
            bridge_cfg.trait_name, associated_types.context_type, associated_types.result_type
        );
        return String::new();
    };
    let mut out = String::with_capacity(32_768);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));

    let pascal_prefix = {
        let mut chars = ffi_prefix.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };
    let prefix_upper = ffi_prefix.to_uppercase();
    let _ = vtable_trait_name;
    let visitor_handle_rust_name = format!("{pascal_prefix}Visitor");
    let visitor_handle_c_type = ffi_c_type_name(ffi_prefix, &visitor_handle_rust_name);
    let callbacks_rust_name = format!("{pascal_prefix}VisitorCallbacks");
    let callbacks_c_type = ffi_c_type_name(ffi_prefix, &callbacks_rust_name);
    let context_rust_name = format!("{pascal_prefix}Context");
    let context_c_type = ffi_c_type_name(ffi_prefix, &context_rust_name);
    let options_type = bridge_cfg
        .options_type
        .as_deref()
        .expect("go options-field visitor bridge requires options_type");
    let options_type_snake = go_visitor_bridge_function_component(options_type);
    let return_type = named_type_name(&bridge_func.return_type)
        .expect("go options-field visitor bridge currently requires a named return type");
    let return_type_snake = go_visitor_bridge_function_component(return_type);
    let conversion_options_type = format!("{prefix_upper}{options_type}");

    let fn_visitor_create = format!("{ffi_prefix}_visitor_create");
    let fn_visitor_free = format!("{ffi_prefix}_visitor_free");
    let fn_options_set_visitor = format!("{ffi_prefix}_options_set_{options_field}");
    let fn_options_free = format!("{ffi_prefix}_{options_type_snake}_free");
    let fn_options_from_json = format!("{ffi_prefix}_{options_type_snake}_from_json");
    let fn_convert = format!(
        "{ffi_prefix}_{}",
        go_visitor_bridge_function_component(&bridge_func.name)
    );
    let fn_result_free = format!("{ffi_prefix}_{return_type_snake}_free");

    let callbacks: Vec<_> = specs
        .iter()
        .map(|spec| {
            minijinja::Value::from_serialize(serde_json::json!({
                "export_name": spec.export_name,
                "c_sig": c_signature(spec, &context_c_type),
                "c_field": spec.c_field,
            }))
        })
        .collect();

    out.push_str(&crate::backends::go::template_env::render(
        "visitor_preamble.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            to_root => to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_header => ffi_header,
            callbacks => callbacks,
            callbacks_c_type => callbacks_c_type.clone(),
        },
    ));

    // NOTE: NodeType is defined in binding.go as `type NodeType string`.
    out.push_str(&crate::backends::go::template_env::render(
        "visitor_node_context_and_result.jinja",
        minijinja::context! {
            context_type => associated_types.context_type.as_str(),
            result_type => associated_types.result_type.as_str(),
            context_fields => codec_metadata.context_fields,
            result_variants => codec_metadata.result_variants,
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "visitor_interface_header.jinja",
        minijinja::Value::default(),
    ));
    for spec in &specs {
        let param_str = iface_param_str(spec);
        out.push_str(&crate::backends::go::template_env::render(
            "visitor_interface_method.jinja",
            minijinja::context! {
                doc => spec.doc,
                method => spec.go_method,
                params => param_str,
                return_type => spec.result_type.as_str(),
            },
        ));
    }
    out.push_str(&crate::backends::go::template_env::render(
        "visitor_interface_footer.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "base_visitor_header.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');
    for spec in &specs {
        let param_str = iface_param_str(spec);
        let blank_ids: Vec<String> = iface_param_names(spec).into_iter().collect();
        out.push_str(&crate::backends::go::template_env::render(
            "base_visitor_method.jinja",
            minijinja::context! {
                doc => spec.go_method,
                method_name => spec.go_method,
                params => param_str,
                blank_ids => blank_ids,
                return_type => spec.result_type.as_str(),
                default_result_fn => codec_metadata.default_result_helper_name.as_str(),
            },
        ));
        out.push('\n');
    }

    out.push_str(&crate::backends::go::template_env::render(
        "visitor_registry_block.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    let context_fields_for_decode = context_fields_for_decode(&codec_metadata.context_fields);
    out.push_str(&crate::backends::go::template_env::render(
        "decode_node_context.jinja",
        minijinja::context! {
            context_type => associated_types.context_type.as_str(),
            context_c_type => context_c_type.clone(),
            context_fields => context_fields_for_decode,
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "encode_visit_result.jinja",
        minijinja::context! {
            result_type => associated_types.result_type.as_str(),
            result_variants => codec_metadata.result_variants,
            default_result_wire_name => codec_metadata.default_result_wire_name,
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "opt_go_string.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "decode_cells_json.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    for spec in &specs {
        gen_trampoline(&mut out, spec, &context_c_type);
    }

    let fn_result_to_json = fn_result_free.replace("_free", "_to_json");
    let helper_name = format!(
        "{}WithVisitorHelper",
        go_visitor_bridge_function_component(&bridge_func.name)
    );
    let helper_params = helper_params(bridge_func);
    let helper_setup = helper_c_param_setup(bridge_func, options_type);
    let helper_call_args = helper_call_args(bridge_func, options_type);
    let options_var = bridge_func
        .params
        .iter()
        .find(|param| named_type_name(&param.ty) == Some(options_type))
        .map(|param| crate::codegen::naming::go_param_name(&param.name))
        .unwrap_or_else(|| "options".to_string());
    out.push_str(&crate::backends::go::template_env::render(
        "convert_with_visitor_helper.jinja",
        minijinja::context! {
            helper_name => helper_name,
            helper_params => helper_params,
            helper_setup => helper_setup,
            helper_call_args => helper_call_args,
            options_var => options_var,
            options_type => options_type,
            conversion_options_type => conversion_options_type,
            fn_options_from_json => fn_options_from_json,
            fn_options_free => fn_options_free,
            fn_visitor_create => fn_visitor_create,
            fn_visitor_free => fn_visitor_free,
            fn_options_set_visitor => fn_options_set_visitor,
            visitor_handle_c_type => visitor_handle_c_type.clone(),
            fn_convert => fn_convert,
            fn_result_to_json => fn_result_to_json,
            fn_result_free => fn_result_free,
            result_type => return_type,
        },
    ));
    out.push('\n');
    out.push('\n');

    out
}

/// Build the C parameter list string for the extern declaration of an exported Go function.
///
/// VisitorCallbacks ABI: `(const HtmContext* ctx, void* user_data, ...extras..., int32_t isHeader?, char** out_custom, uintptr_t* out_len)`
///
/// For the `visit_table_row` case the cells parameter is emitted as
/// `const char* const* cells, uintptr_t cell_count` (matching the FFI struct)
/// instead of the JSON-encoded single-string form used by the old VTable ABI.
fn c_signature(spec: &CallbackSpec, context_c_type: &str) -> String {
    let mut parts = vec![format!("{context_c_type}* ctx"), "void* user_data".to_string()];
    for ep in &spec.extra {
        if spec.has_is_header && ep.c_type == "*C.char" {
            parts.push(format!("char** {}", ep.c_name));
            parts.push("uintptr_t cell_count".to_string());
            continue;
        }
        let ctype = match ep.c_type.as_str() {
            "*C.char" => "char*",
            "C.int32_t" => "int32_t",
            "C.uint32_t" => "uint32_t",
            "C.uintptr_t" => "uintptr_t",
            _ => "void*",
        };
        parts.push(format!("{ctype} {}", ep.c_name));
    }
    if spec.has_is_header {
        parts.push("int32_t isHeader".to_string());
    }
    parts.push("char** out_custom".to_string());
    parts.push("uintptr_t* out_len".to_string());
    parts.join(", ")
}

/// Build the Go interface method parameter string.
fn iface_param_str(spec: &CallbackSpec) -> String {
    let mut params = vec![format!("ctx {}", spec.context_type)];
    for ep in &spec.extra {
        params.push(format!("{} {}", ep.go_name, ep.go_iface_type));
    }
    if spec.has_is_header {
        params.push("isHeader bool".to_string());
    }
    params.join(", ")
}

/// Return just the parameter names for a Go interface method.
fn iface_param_names(spec: &CallbackSpec) -> Vec<String> {
    let mut names = vec!["ctx".to_string()];
    for ep in &spec.extra {
        names.push(ep.go_name.clone());
    }
    if spec.has_is_header {
        names.push("isHeader".to_string());
    }
    names
}

/// Generate one `//export goVisit*` C callback trampoline for the VisitorCallbacks ABI.
///
/// VisitorCallbacks ABI signature: `(ctx *C.{{ context_c_type }}, userData unsafe.Pointer, ...extras..., outCustom **C.char, outLen *C.uintptr_t) C.int32_t`
fn gen_trampoline(out: &mut String, spec: &CallbackSpec, context_c_type: &str) {
    let mut go_params = vec![
        format!("ctx *C.{context_c_type}"),
        "userData unsafe.Pointer".to_string(),
    ];
    for ep in &spec.extra {
        if spec.has_is_header && ep.c_type == "*C.char" {
            go_params.push(format!("{} **C.char", ep.c_name));
            go_params.push("cellCount C.uintptr_t".to_string());
        } else {
            go_params.push(format!("{} {}", ep.c_name, ep.c_type));
        }
    }
    if spec.has_is_header {
        go_params.push("isHeader C.int32_t".to_string());
    }
    go_params.push("outCustom **C.char".to_string());
    go_params.push("outLen *C.uintptr_t".to_string());

    out.push_str(&crate::backends::go::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => spec.export_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "trampoline_func_signature.jinja",
        minijinja::context! {
            name => spec.export_name,
            params => go_params.join(", "),
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "trampoline_lookup.jinja",
        minijinja::Value::default(),
    ));

    for ep in &spec.extra {
        out.push_str(&crate::backends::go::template_env::render(
            "trampoline_param_decode.jinja",
            minijinja::context! {
                name => capitalize(&ep.go_name),
                decode => ep.decode,
            },
        ));
    }
    if spec.has_is_header {
        out.push_str(&crate::backends::go::template_env::render(
            "trampoline_is_header.jinja",
            minijinja::Value::default(),
        ));
    }

    let mut call_args = vec!["nodeCtx".to_string()];
    for ep in &spec.extra {
        call_args.push(format!("go{}", capitalize(&ep.go_name)));
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    out.push_str(&crate::backends::go::template_env::render(
        "trampoline_func_call.jinja",
        minijinja::context! {
            method => spec.go_method,
            args => call_args.join(", "),
        },
    ));
    out.push_str("}\n");
    out.push('\n');
}

/// Generate the `ConvertWithVisitor` function for the options-field VTable pattern.
///
/// Flow:
/// 1. Register the Go visitor in the global table; get a numeric ID as user_data.
/// 2. Build the VTable via the static C helper (all fn pointers set, no user_data field).
/// 3. Create a bridge via `{fn_bridge_new}(&vtbl, unsafe.Pointer(id))`.
/// 4. Attach the bridge to options via `{fn_options_set_visitor}(cOptions, bridge)`.
/// 5. Call `{fn_convert}` to run conversion.
/// 6. Free bridge and options after conversion completes.
///
/// NOTE: This function is no longer used. The same logic is now inlined in
/// the generated helper and generated directly in `gen_visitor_file`.
#[allow(clippy::too_many_arguments, dead_code)]
fn gen_convert_with_visitor(
    out: &mut String,
    _ffi_prefix: &str,
    _conversion_options_type: &str,
    _vtable_c_type: &str,
    _bridge_c_type: &str,
    _fn_bridge_new: &str,
    _fn_bridge_free: &str,
    _fn_options_set_visitor: &str,
    _fn_options_free: &str,
    _fn_options_from_json: &str,
    _fn_convert: &str,
    _fn_result_free: &str,
) {
    out.push_str(&crate::backends::go::template_env::render(
        "convert_with_visitor_wrapper.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Convert a generated visitor bridge type name into its Go wrapper function component.
fn go_visitor_bridge_function_component(name: &str) -> String {
    crate::codegen::naming::pascal_to_snake(name)
}

fn result_helper_name(result_type: &str, variant: &str) -> String {
    format!("{result_type}{variant}")
}

fn visitor_codec_metadata(api: &ApiSurface, associated_types: &VisitorAssociatedTypes) -> Option<VisitorCodecMetadata> {
    let context_fields = api
        .types
        .iter()
        .find(|type_def| type_def.name == associated_types.context_type)
        .map(context_fields_from_type)?;
    let enum_def = api
        .enums
        .iter()
        .find(|enum_def| enum_def.name == associated_types.result_type)?;
    let result_variants = result_variants_from_enum(enum_def, &associated_types.result_type);
    let unit_variants = result_variants
        .iter()
        .filter(|variant| !variant.has_payload)
        .collect::<Vec<_>>();
    let default_result_variant = result_variants
        .iter()
        .find(|variant| variant.is_default)
        .or_else(|| (unit_variants.len() == 1).then_some(unit_variants[0]))?;
    let default_result_helper_name = default_result_variant.helper_name.clone();
    let default_result_wire_name = default_result_variant.wire_name.clone();
    Some(VisitorCodecMetadata {
        context_fields,
        default_result_helper_name,
        default_result_wire_name,
        result_variants,
    })
}

/// Project context fields to the form the decode template needs: a `kind` tag plus
/// the JSON / Go names. Fields whose Go type does not correspond to a scalar C
/// field (e.g. a typed-alias enum) are tagged `"unsupported"` so the template
/// can skip them — those stay at their zero value after decoding.
#[derive(Clone, serde::Serialize)]
struct ContextFieldDecode {
    json_name: String,
    go_name: String,
    kind: &'static str,
}

fn context_fields_for_decode(fields: &[ContextFieldMetadata]) -> Vec<ContextFieldDecode> {
    fields
        .iter()
        .map(|f| {
            let kind = match f.go_type.as_str() {
                "string" => "string",
                "*string" => "optional_string",
                "uint" => "uint",
                "uint32" => "uint32",
                "bool" => "bool",
                _ => "unsupported",
            };
            ContextFieldDecode {
                json_name: f.json_name.clone(),
                go_name: f.go_name.clone(),
                kind,
            }
        })
        .collect()
}

fn context_fields_from_type(type_def: &TypeDef) -> Vec<ContextFieldMetadata> {
    type_def
        .fields
        .iter()
        .map(|field| {
            let go_name = crate::codegen::naming::to_go_name(&field.name);
            let go_type = if field.optional {
                crate::backends::go::type_map::go_optional_type(&field.ty).into_owned()
            } else {
                crate::backends::go::type_map::go_type(&field.ty).into_owned()
            };
            let json_name =
                crate::codegen::naming::wire_field_name(&field.name, None, type_def.serde_rename_all.as_deref());
            ContextFieldMetadata {
                doc: field.doc.lines().next().unwrap_or("").trim().to_string(),
                go_name,
                go_type,
                json_name,
            }
        })
        .collect()
}

fn result_variants_from_enum(enum_def: &EnumDef, result_type: &str) -> Vec<ResultVariantMetadata> {
    enum_def
        .variants
        .iter()
        .enumerate()
        .map(|(code, variant)| {
            let has_payload = !variant.fields.is_empty();
            let payload_name = variant
                .fields
                .first()
                .map(|field| {
                    let name = field.name.as_str();
                    let trimmed = name.trim_start_matches('_');
                    if trimmed.parse::<usize>().is_ok() && !trimmed.is_empty() {
                        "value"
                    } else {
                        name
                    }
                })
                .unwrap_or("value")
                .to_string();
            ResultVariantMetadata {
                code: code as i32,
                helper_name: result_helper_name(result_type, &crate::codegen::naming::to_go_name(&variant.name)),
                name: crate::codegen::naming::to_go_name(&variant.name),
                wire_name: crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ),
                has_payload,
                is_default: variant.is_default,
                payload_go_name: crate::codegen::naming::go_param_name(&payload_name),
                payload_name,
            }
        })
        .collect()
}

fn named_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => named_type_name(inner),
        _ => None,
    }
}

fn helper_params(func: &FunctionDef) -> String {
    let mut params = func
        .params
        .iter()
        .map(|param| {
            let go_name = crate::codegen::naming::go_param_name(&param.name);
            let go_type: String = if param.optional {
                crate::backends::go::type_map::go_optional_type(&param.ty).into_owned()
            } else if let TypeRef::Named(_) = &param.ty {
                format!("*{}", crate::backends::go::type_map::go_type(&param.ty))
            } else {
                crate::backends::go::type_map::go_type(&param.ty).into_owned()
            };
            format!("{go_name} {go_type}")
        })
        .collect::<Vec<_>>();
    params.push("visitor Visitor".to_string());
    params.join(", ")
}

fn helper_c_param_setup(func: &FunctionDef, options_type: &str) -> String {
    let mut out = String::new();
    for param in &func.params {
        if named_type_name(&param.ty) == Some(options_type) {
            continue;
        }
        if matches!(param.ty, TypeRef::String | TypeRef::Path) {
            let go_name = crate::codegen::naming::go_param_name(&param.name);
            let c_name = crate::codegen::naming::go_param_name(&format!("c_{}", param.name));
            out.push_str(&crate::backends::go::template_env::render(
                "c_string_arg_setup.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_name => &go_name,
                },
            ));
        }
    }
    out
}

fn helper_call_args(func: &FunctionDef, options_type: &str) -> String {
    func.params
        .iter()
        .map(|param| {
            if named_type_name(&param.ty) == Some(options_type) {
                "cOptions".to_string()
            } else if matches!(param.ty, TypeRef::String | TypeRef::Path) {
                crate::codegen::naming::go_param_name(&format!("c_{}", param.name))
            } else {
                crate::codegen::naming::go_param_name(&param.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
