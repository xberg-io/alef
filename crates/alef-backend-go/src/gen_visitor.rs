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
/// `user_data` is the first argument; `ctx` is a JSON-encoded `NodeContext`; `out_result`
/// receives a heap-allocated C string when the visitor returns a Custom/Error result.
///
/// This differs from the legacy `VisitorCallbacks` pattern (FunctionParam bind_via), where
/// `user_data` was a FIELD on the struct and context was a typed `*NodeContext` pointer.
use alef_core::hash::{self, CommentStyle};
use serde_json;

/// Derive the cbindgen-generated C type name for a Rust FFI type.
///
/// cbindgen prepends the uppercased `ffi_prefix` to the Rust struct name verbatim.
/// Example: prefix="htm", Rust name="HtmHtmlVisitorVTable" → "HTMHtmHtmlVisitorVTable".
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

/// Build a `Vec<CallbackSpec>` from a trait's IR definition for the Go backend.
///
/// Derives all language-specific Go fields (method names, C types, decode expressions)
/// from `TypeRef` + `optional` flag. Methods with unsupported parameter types are
/// skipped with a warning.
fn callback_specs_from_trait(trait_def: &alef_core::ir::TypeDef) -> Vec<CallbackSpec> {
    use alef_core::ir::{PrimitiveType, TypeRef};
    use heck::ToPascalCase;

    let mut specs = Vec::with_capacity(trait_def.methods.len());
    'methods: for m in &trait_def.methods {
        if m.trait_source.is_some() {
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
            if matches!(&p.ty, TypeRef::Named(_)) {
                continue;
            }
            // strip leading underscore; keep snake_case for c_name (C extern),
            // but convert to lowerCamelCase for go_name (Go identifiers).
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
                        let decode = format!("decodeCellsJSON({param_name})");
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
        });
    }
    specs
}

/// Generate the complete visitor.go file content for the options-field VTable ABI.
///
/// # Parameters
///
/// - `pkg_name`: Go package name (e.g. `"htmltomarkdown"`).
/// - `ffi_prefix`: C function prefix (e.g. `"htm"`).
/// - `ffi_header`: C header filename (e.g. `"html_to_markdown.h"`).
/// - `ffi_crate_dir`: path from go output dir to the FFI crate dir.
/// - `to_root`: relative path from go output dir to the repo root.
/// - `vtable_trait_name`: Rust trait name used to derive the VTable struct name
///   (e.g. `"HtmlVisitor"` → `"HtmHtmlVisitorVTable"`).
/// - `options_field`: field name on `ConversionOptions` that holds the bridge
///   (e.g. `"visitor"`).
pub fn gen_visitor_file(
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
    vtable_trait_name: &str,
    options_field: &str,
    trait_def: &alef_core::ir::TypeDef,
) -> String {
    let specs = callback_specs_from_trait(trait_def);
    let mut out = String::with_capacity(32_768);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));

    // Derive C type names.
    // VTable: {PREFIX_UPPER}{PascalPrefix}{TraitName}VTable  e.g. HTMHtmHtmlVisitorVTable
    // Bridge: {PREFIX_UPPER}{PascalPrefix}{TraitName}Bridge  e.g. HTMHtmHtmlVisitorBridge
    let pascal_prefix = {
        let mut chars = ffi_prefix.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };
    let prefix_upper = ffi_prefix.to_uppercase();
    let vtable_rust_name = format!("{pascal_prefix}{vtable_trait_name}VTable");
    let bridge_rust_name = format!("{pascal_prefix}{vtable_trait_name}Bridge");
    let vtable_c_type = ffi_c_type_name(ffi_prefix, &vtable_rust_name);
    let bridge_c_type = ffi_c_type_name(ffi_prefix, &bridge_rust_name);
    let conversion_options_type = format!("{prefix_upper}ConversionOptions");

    // Derive bridge_snake from bridge_rust_name for fn names.
    // e.g. "HtmHtmlVisitorBridge" → "htm_html_visitor_bridge"
    let bridge_snake = to_snake_case(&bridge_rust_name);
    let fn_bridge_new = format!("{ffi_prefix}_{bridge_snake}_new");
    let fn_bridge_free = format!("{ffi_prefix}_{bridge_snake}_free");
    let fn_options_set_visitor = format!("{ffi_prefix}_options_set_{options_field}");
    let fn_options_free = format!("{ffi_prefix}_conversion_options_free");
    let fn_options_from_json = format!("{ffi_prefix}_conversion_options_from_json");
    let fn_convert = format!("{ffi_prefix}_convert");
    let fn_result_free = format!("{ffi_prefix}_conversion_result_free");

    // -------------------------------------------------------------------------
    // CGo preamble
    // -------------------------------------------------------------------------
    let callbacks: Vec<_> = specs
        .iter()
        .map(|spec| {
            minijinja::Value::from_serialize(serde_json::json!({
                "export_name": spec.export_name,
                "c_sig": c_signature(spec),
                "c_field": spec.c_field,
            }))
        })
        .collect();

    out.push_str(&crate::template_env::render(
        "visitor_preamble.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            to_root => to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_header => ffi_header,
            callbacks => callbacks,
            vtable_c_type => vtable_c_type.clone(),
        },
    ));

    // NOTE: NodeType is defined in binding.go as `type NodeType string`.
    // Do NOT re-declare it here — that would cause a redeclaration compile error.
    out.push_str(&crate::template_env::render(
        "visitor_node_context_and_result.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // -------------------------------------------------------------------------
    // Visitor interface
    // -------------------------------------------------------------------------
    out.push_str(&crate::template_env::render(
        "visitor_interface_header.jinja",
        minijinja::Value::default(),
    ));
    for spec in &specs {
        let param_str = iface_param_str(spec);
        out.push_str(&crate::template_env::render(
            "visitor_interface_method.jinja",
            minijinja::context! {
                doc => spec.doc,
                method => spec.go_method,
                params => param_str,
            },
        ));
    }
    out.push_str(&crate::template_env::render(
        "visitor_interface_footer.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // -------------------------------------------------------------------------
    // BaseVisitor — no-op defaults
    // -------------------------------------------------------------------------
    out.push_str(&crate::template_env::render(
        "base_visitor_header.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');
    for spec in &specs {
        let param_str = iface_param_str(spec);
        // Build blank identifiers to suppress "declared but not used" errors.
        let blank_ids: Vec<String> = iface_param_names(spec).into_iter().collect();
        out.push_str(&crate::template_env::render(
            "base_visitor_method.jinja",
            minijinja::context! {
                doc => spec.go_method,
                method_name => spec.go_method,
                params => param_str,
                blank_ids => blank_ids,
            },
        ));
        out.push('\n');
    }

    // -------------------------------------------------------------------------
    // Visitor registry
    // -------------------------------------------------------------------------
    out.push_str(&crate::template_env::render(
        "visitor_registry_block.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // -------------------------------------------------------------------------
    // Shared helpers
    // -------------------------------------------------------------------------

    // decodeNodeContext: decode from JSON string (VTable ABI passes ctx as *const c_char JSON)
    out.push_str(&crate::template_env::render(
        "decode_node_context.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // encodeVisitResult: write serde-native JSON into *out_result so the Rust trait bridge
    // can deserialize it with serde_json::from_str::<VisitResult>.
    //
    // Rust serde-derived enum serialisation:
    //   Continue     → "Continue"
    //   Skip         → "Skip"
    //   PreserveHtml → "PreserveHtml"
    //   Custom(s)    → {"Custom":"<s>"}
    //   Error(s)     → {"Error":"<s>"}
    //
    // The return code still carries the numeric variant tag so callers that only
    // inspect the code (and don't read out_result) remain compatible.
    out.push_str(&crate::template_env::render(
        "encode_visit_result.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    out.push_str(&crate::template_env::render(
        "opt_go_string.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // decodeCellsJSON: cells is a JSON-encoded []string in the VTable ABI.
    out.push_str(&crate::template_env::render(
        "decode_cells_json.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // -------------------------------------------------------------------------
    // //export trampolines
    // -------------------------------------------------------------------------
    for spec in &specs {
        gen_trampoline(&mut out, spec);
    }

    // -----------------------------------------------------------------------
    // Internal helper: convertWithVisitorHelper
    // -----------------------------------------------------------------------
    // This helper is called by Convert() in binding.go when options.Visitor is not nil.
    // It registers the visitor, builds the VTable, creates a bridge, attaches it to
    // options, calls the FFI convert function, and cleans up.
    let fn_result_to_json = fn_result_free.replace("_free", "_to_json");
    out.push_str(&crate::template_env::render(
        "convert_with_visitor_helper.jinja",
        minijinja::context! {
            conversion_options_type => conversion_options_type,
            fn_options_from_json => fn_options_from_json,
            fn_options_free => fn_options_free,
            fn_bridge_new => fn_bridge_new,
            fn_bridge_free => fn_bridge_free,
            fn_options_set_visitor => fn_options_set_visitor,
            bridge_c_type => bridge_c_type,
            fn_convert => fn_convert,
            fn_result_to_json => fn_result_to_json,
            fn_result_free => fn_result_free,
        },
    ));
    out.push('\n');
    out.push('\n');

    out
}

/// Build the C parameter list string for the extern declaration of an exported Go function.
///
/// VTable ABI: `(void* user_data, char* ctx, ...extras..., int32_t isHeader?, char** out_result)`
fn c_signature(spec: &CallbackSpec) -> String {
    let mut parts = vec!["void* user_data".to_string(), "char* ctx".to_string()];
    for ep in &spec.extra {
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
    parts.push("char** out_result".to_string());
    parts.join(", ")
}

/// Build the Go interface method parameter string.
fn iface_param_str(spec: &CallbackSpec) -> String {
    let mut params = vec!["ctx NodeContext".to_string()];
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

/// Generate one `//export goVisit*` C callback trampoline for the VTable ABI.
///
/// VTable ABI signature: `(user_data unsafe.Pointer, ctx *C.char, ...extras..., outResult **C.char) C.int32_t`
fn gen_trampoline(out: &mut String, spec: &CallbackSpec) {
    // Build Go function parameter list (CGo types).
    // VTable ABI: user_data first, then ctx (JSON string), then extras, then out_result.
    let mut go_params = vec!["userData unsafe.Pointer".to_string(), "ctx *C.char".to_string()];
    for ep in &spec.extra {
        go_params.push(format!("{} {}", ep.c_name, ep.c_type));
    }
    if spec.has_is_header {
        go_params.push("isHeader C.int32_t".to_string());
    }
    go_params.push("outResult **C.char".to_string());

    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => spec.export_name,
        },
    ));
    out.push_str(&crate::template_env::render(
        "trampoline_func_signature.jinja",
        minijinja::context! {
            name => spec.export_name,
            params => go_params.join(", "),
        },
    ));
    out.push_str(&crate::template_env::render(
        "trampoline_lookup.jinja",
        minijinja::Value::default(),
    ));

    // Decode each extra parameter.
    for ep in &spec.extra {
        out.push_str(&crate::template_env::render(
            "trampoline_param_decode.jinja",
            minijinja::context! {
                name => capitalize(&ep.go_name),
                decode => ep.decode,
            },
        ));
    }
    if spec.has_is_header {
        out.push_str(&crate::template_env::render(
            "trampoline_is_header.jinja",
            minijinja::Value::default(),
        ));
    }

    // Build call args.
    let mut call_args = vec!["nodeCtx".to_string()];
    for ep in &spec.extra {
        call_args.push(format!("go{}", capitalize(&ep.go_name)));
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    out.push_str(&crate::template_env::render(
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
/// 5. Call `{fn_convert}(cHTML, cOptions)` to run conversion.
/// 6. Free bridge and options after conversion completes.
///
/// NOTE: This function is no longer used. The same logic is now inlined in
/// `convertWithVisitorHelper` and generated directly in `gen_visitor_file`.
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
    out.push_str(&crate::template_env::render(
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

/// Convert PascalCase to snake_case (e.g. "HtmHtmlVisitorBridge" → "htm_html_visitor_bridge").
fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}
