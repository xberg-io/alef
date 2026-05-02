//! WASM free-function and utility code generation.

use crate::type_map::WasmMapper;
use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_codegen::{generators, naming::to_node_name};
use alef_core::ir::{FunctionDef, TypeRef};

/// Format a doc string as rustdoc comment lines.
///
/// Returns an empty string when `doc` is empty, otherwise returns each line
/// prefixed with `/// ` and terminated with a newline, ready to prepend to an item.
pub(super) fn emit_rustdoc(doc: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for line in doc.lines() {
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Convert a `TypeRef` to its concrete Rust type string for use in serde deserialization
/// let-bindings. Unlike `WasmMapper::map_type`, this always returns a concrete Rust type
/// (e.g. `String`, `Vec<String>`) rather than `JsValue`. Used when emitting
/// `serde_wasm_bindgen::from_value::<T>(jsval)?` where T must be a concrete type.
pub(super) fn typeref_to_core_type_str(ty: &TypeRef) -> String {
    use alef_core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
            PrimitiveType::Usize => "usize".to_string(),
            PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_core_type_str(inner)),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_core_type_str(inner)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_core_type_str(k),
            typeref_to_core_type_str(v)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => n.to_string(),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Helper: format a parameter, prefixing with _ if unused
pub(super) fn format_param_unused(name: &str, ty: &str, unused: bool) -> String {
    let prefix = if unused { "_" } else { "" };
    format!("{}{}: {}", prefix, name, ty)
}

/// Generate a free function binding.
pub(super) fn gen_function(
    func: &FunctionDef,
    mapper: &WasmMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let can_delegate = alef_codegen::shared::can_auto_delegate_function(func, opaque_types);

    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate && !func.is_async)
        })
        .collect();

    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let mut attrs = emit_rustdoc(&func.doc);
    // Per-item clippy suppression: too_many_arguments when >7 params
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    if func.is_async {
        let let_bindings = if alef_codegen::generators::has_named_params(&func.params, opaque_types) {
            alef_codegen::generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        // Build the return expression: handle Vec<Named> with collect pattern (turbofish),
        // plain Named with From::from, and everything else as passthrough.
        let return_expr = match &func.return_type {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    format!(
                        "result.into_iter().map(|v| {} {{ inner: Arc::new(v) }}).collect::<Vec<_>>()",
                        mapper.map_type(inner)
                    )
                }
                TypeRef::Named(_) => {
                    let inner_mapped = mapper.map_type(inner);
                    format!("result.into_iter().map({inner_mapped}::from).collect::<Vec<_>>()")
                }
                _ => "result".to_string(),
            },
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let prefixed = mapper.map_type(&func.return_type);
                format!("{prefixed} {{ inner: Arc::new(result) }}")
            }
            TypeRef::Named(_) => {
                format!("{return_type}::from(result)")
            }
            TypeRef::Unit => "result".to_string(),
            _ => "result".to_string(),
        };
        let body = if func.error_type.is_some() {
            format!(
                "{let_bindings}let result = {core_call}.await\n        \
                 .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                 Ok({return_expr})"
            )
        } else {
            format!(
                "{let_bindings}let result = {core_call}.await;\n    \
                 {return_expr}"
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub async fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        )
    } else if can_delegate {
        let mut let_bindings = if alef_codegen::generators::has_named_params(&func.params, opaque_types) {
            alef_codegen::generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Nested Vec params (e.g. Vec<Vec<String>>) arrive as JsValue because wasm-bindgen
        // cannot pass them across the boundary directly. Emit a deserialization shadowing
        // binding so the core call sees a real `Vec<Vec<T>>`.
        let needs_result_wrap = func
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(_))))
            && func.error_type.is_none();
        for p in &func.params {
            if let TypeRef::Vec(outer_inner) = &p.ty
                && matches!(outer_inner.as_ref(), TypeRef::Vec(_))
            {
                let elem_ty = if let TypeRef::Vec(elem) = outer_inner.as_ref() {
                    typeref_to_core_type_str(elem.as_ref())
                } else {
                    "String".to_string()
                };
                let core_ty = format!("Vec<Vec<{elem_ty}>>");
                if p.optional {
                    let_bindings.push_str(&format!(
                        "let {n}: Option<{core_ty}> = {n}.map(|v| \
                         serde_wasm_bindgen::from_value::<{core_ty}>(v)\
                         .expect(\"deserialize {n}\")) ;\n    ",
                        n = p.name,
                    ));
                } else {
                    let_bindings.push_str(&format!(
                        "let {n}: {core_ty} = \
                         serde_wasm_bindgen::from_value::<{core_ty}>({n})\
                         .expect(\"deserialize {n}\");\n    ",
                        n = p.name,
                    ));
                }
            }
        }
        let _ = needs_result_wrap;
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let body = if func.error_type.is_some() {
            let wrap = wasm_wrap_return_fn(
                "result",
                &func.return_type,
                opaque_types,
                func.returns_ref,
                func.returns_cow,
                prefix,
            );
            format!(
                "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        } else {
            format!(
                "{let_bindings}{}",
                wasm_wrap_return_fn(
                    &core_call,
                    &func.return_type,
                    opaque_types,
                    func.returns_ref,
                    func.returns_cow,
                    prefix
                )
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        )
    } else if func.error_type.is_some()
        && (func.sanitized || alef_codegen::generators::has_named_params(&func.params, opaque_types))
    {
        // Serde recovery: accept Named non-opaque params as JsValue and deserialize
        // to core types via serde_wasm_bindgen. Also handles sanitized functions (Vec<tuple>).
        // WASM binding structs don't derive Serialize/Deserialize, so we can't round-trip
        // through the binding type; instead we accept raw JsValue/Vec<String> from JS and
        // deserialize directly to core types.
        let serde_params: Vec<String> = func
            .params
            .iter()
            .map(|p| match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    // Accept as JsValue so serde_wasm_bindgen::from_value can deserialize
                    let mapped_ty = if p.optional {
                        "Option<JsValue>".to_string()
                    } else {
                        "JsValue".to_string()
                    };
                    format!("{}: {}", p.name, mapped_ty)
                }
                TypeRef::Vec(inner) => {
                    // Sanitized Vec<tuple>: accept Vec<String> (JSON encoded)
                    if matches!(inner.as_ref(), TypeRef::Named(_)) {
                        if p.optional {
                            format!("{}: Option<Vec<String>>", p.name)
                        } else {
                            format!("{}: Vec<String>", p.name)
                        }
                    } else {
                        let ty = mapper.map_type(&p.ty);
                        let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                }
                _ => {
                    let ty = mapper.map_type(&p.ty);
                    let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                    format!("{}: {}", p.name, mapped_ty)
                }
            })
            .collect();

        // Generate serde_wasm_bindgen::from_value let-bindings for Named non-opaque params
        // and Vec<String> with is_ref=true (needs texts_refs intermediate)
        let mut serde_bindings = String::new();
        for p in &func.params {
            match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    let core_path = format!("{}::{}", core_import, name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&format!(
                            "let {n}_core: Option<{core_path}> = {n}.map(|v| \
                             serde_wasm_bindgen::from_value::<{core_path}>(v){err_conv})\
                             .transpose()?;\n    ",
                            n = p.name,
                            core_path = core_path,
                            err_conv = err_conv,
                        ));
                    } else {
                        serde_bindings.push_str(&format!(
                            "let {n}_core: {core_path} = \
                             serde_wasm_bindgen::from_value::<{core_path}>({n}){err_conv}?;\n    ",
                            n = p.name,
                            core_path = core_path,
                            err_conv = err_conv,
                        ));
                    }
                }
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    // Sanitized Vec<tuple>: deserialize from Vec<String> JSON
                    let inner_name = match inner.as_ref() {
                        TypeRef::Named(n) => n,
                        _ => "UnknownTuple",
                    };
                    let core_path = format!("{}::{}", core_import, inner_name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&format!(
                            "let {n}_core: Option<Vec<{core_path}>> = {n}.map(|strings| {{\n    \
                             strings.into_iter()\n    \
                             .map(|s| serde_json::from_str::<{core_path}>(&s){err_conv})\n    \
                             .collect::<Result<Vec<_>, _>>()\n    \
                             }}).transpose()?;\n    ",
                            n = p.name,
                            core_path = core_path,
                            err_conv = err_conv,
                        ));
                    } else {
                        serde_bindings.push_str(&format!(
                            "let {n}_core: Vec<{core_path}> = {n}.into_iter()\n    \
                             .map(|s| serde_json::from_str::<{core_path}>(&s){err_conv})\n    \
                             .collect::<Result<Vec<_>, _>>()?;\n    ",
                            n = p.name,
                            core_path = core_path,
                            err_conv = err_conv,
                        ));
                    }
                }
                TypeRef::Vec(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                        && p.sanitized
                        && p.original_type.is_some() =>
                {
                    // Sanitized Vec<tuple>: binding accepts Vec<String> (JSON-encoded tuple items).
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&format!(
                            "let {n}_core: Option<Vec<_>> = {n}.map(|strs| {{\n    \
                             strs.into_iter()\n    \
                             .map(|s| serde_json::from_str(&s){err_conv})\n    \
                             .collect::<Result<Vec<_>, _>>()\n    \
                             }}).transpose()?;\n    ",
                            n = p.name,
                            err_conv = err_conv,
                        ));
                    } else {
                        serde_bindings.push_str(&format!(
                            "let {n}_core: Vec<_> = {n}.into_iter()\n    \
                             .map(|s| serde_json::from_str(&s){err_conv})\n    \
                             .collect::<Result<Vec<_>, _>>()?;\n    ",
                            n = p.name,
                            err_conv = err_conv,
                        ));
                    }
                }
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                    // Vec<String> with is_ref=true: core expects &[&str].
                    // gen_call_args_with_let_bindings emits `&{name}_refs`, so we must create
                    // the intermediate Vec<&str> binding here.
                    if p.optional {
                        serde_bindings.push_str(&format!(
                            "let {n}_refs: Vec<&str> = {n}.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect()).unwrap_or_default();\n    ",
                            n = p.name,
                        ));
                    } else {
                        serde_bindings.push_str(&format!(
                            "let {n}_refs: Vec<&str> = {n}.iter().map(|s| s.as_str()).collect();\n    ",
                            n = p.name,
                        ));
                    }
                }
                TypeRef::Vec(outer_inner) if matches!(outer_inner.as_ref(), TypeRef::Vec(_)) => {
                    // Nested Vec (e.g. Vec<Vec<String>>): wasm-bindgen cannot pass this across
                    // the boundary directly, so the param arrives as JsValue. Deserialize via
                    // serde_wasm_bindgen and shadow the original binding so gen_call_args can
                    // still reference the parameter by its original name.
                    let elem_ty = if let TypeRef::Vec(elem) = outer_inner.as_ref() {
                        typeref_to_core_type_str(elem.as_ref())
                    } else {
                        "String".to_string()
                    };
                    let core_ty = format!("Vec<Vec<{elem_ty}>>");
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&format!(
                            "let {n}: Option<{core_ty}> = {n}.map(|v| \
                             serde_wasm_bindgen::from_value::<{core_ty}>(v){err_conv})\
                             .transpose()?;\n    ",
                            n = p.name,
                            core_ty = core_ty,
                            err_conv = err_conv,
                        ));
                    } else {
                        serde_bindings.push_str(&format!(
                            "let {n}: {core_ty} = \
                             serde_wasm_bindgen::from_value::<{core_ty}>({n}){err_conv}?;\n    ",
                            n = p.name,
                            core_ty = core_ty,
                            err_conv = err_conv,
                        ));
                    }
                }
                _ => {}
            }
        }

        let call_args = generators::gen_call_args_with_let_bindings(&func.params, opaque_types);
        let core_call = format!("{core_fn_path}({call_args})");
        let wrap = wasm_wrap_return_fn(
            "result",
            &func.return_type,
            opaque_types,
            func.returns_ref,
            func.returns_cow,
            prefix,
        );
        let body = if matches!(func.return_type, TypeRef::Unit) {
            format!("{serde_bindings}{core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok(())")
        } else {
            format!(
                "{serde_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            serde_params.join(", "),
            return_annotation
        )
    } else {
        let body = gen_wasm_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some());
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        )
    }
}

/// Generate WASM environment shims for wide-character C functions used by external scanners.
///
/// Some tree-sitter external scanners call C wide-character functions (`iswspace`, `iswalnum`,
/// etc.) that are not available in the WASM runtime. This emits `#[unsafe(no_mangle)] extern "C"`
/// shims that satisfy those link-time references using Rust's Unicode-aware char APIs.
///
/// Only shims whose names appear in `shim_names` are emitted.
pub(super) fn gen_env_shims(shim_names: &[String]) -> String {
    let mut out = String::from("// WASM environment shims for C scanner interop\n");

    for name in shim_names {
        let shim = match name.as_str() {
            "iswspace" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswspace(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_whitespace() as i32)\n",
                "}\n",
            ),
            "iswalnum" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswalnum(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_alphanumeric() as i32)\n",
                "}\n",
            ),
            "towupper" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn towupper(c: u32) -> u32 {\n",
                "    char::from_u32(c).map_or(c, |ch| ch.to_uppercase().next().unwrap_or(ch) as u32)\n",
                "}\n",
            ),
            "iswalpha" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswalpha(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_alphabetic() as i32)\n",
                "}\n",
            ),
            _ => continue,
        };
        out.push_str(shim);
    }

    // Trim trailing newline so the builder adds consistent spacing
    out.trim_end_matches('\n').to_string()
}

/// Generate a type-appropriate unimplemented body for WASM (no todo!()).
pub(super) fn gen_wasm_unimplemented_body(return_type: &TypeRef, fn_name: &str, has_error: bool) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(JsValue::from_str(\"{err_msg}\"))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!("panic!(\"alef: {fn_name} not auto-delegatable\")"),
        }
    }
}

/// WASM-specific return wrapping for opaque methods (adds prefix for opaque Named returns).
#[allow(clippy::too_many_arguments)]
pub(super) fn wasm_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
) -> String {
    match return_type {
        // Self-returning opaque method
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        // Other opaque Named return: needs prefix
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        // Optional<opaque>: wrap with prefix
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        // Vec<opaque>: wrap with prefix
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        _ => generators::wrap_return(
            expr,
            return_type,
            type_name,
            opaque_types,
            self_is_opaque,
            returns_ref,
            returns_cow,
        ),
    }
}

/// WASM-specific return wrapping for free functions (no type_name context, adds prefix).
pub(super) fn wasm_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
) -> String {
    match return_type {
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_cow {
                format!("{expr}.into_owned().into()")
            } else if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
            if returns_cow && matches!(return_type, TypeRef::Bytes) {
                // Cow<[u8]> needs .into_owned() to become Vec<u8>
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.into_iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}
