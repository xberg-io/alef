//! WASM struct method code generation.

use crate::type_map::WasmMapper;
use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_codegen::{generators, naming::to_node_name, shared};
use alef_core::ir::{MethodDef, TypeRef};

use super::functions::{emit_rustdoc, format_param_unused, wasm_wrap_return};

/// Generate a method binding for a struct method.
pub(super) fn gen_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
) -> String {
    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate && !method.is_async)
        })
        .collect();

    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let mut attrs = emit_rustdoc(&method.doc);
    // Per-item clippy suppression: too_many_arguments when >7 params (including &self for instance methods)
    let effective_param_count = if method.is_static {
        method.params.len()
    } else {
        method.params.len() + 1
    };
    if effective_param_count > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning methods
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    // Per-item clippy suppression: should_implement_trait for trait-conflicting names
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }

    if method.is_async {
        // For async methods with named params, use JsValue parameters to avoid _assertClass errors
        let has_named = alef_codegen::generators::has_named_params(&method.params, opaque_types);

        let async_params: Vec<String> = if has_named {
            method
                .params
                .iter()
                .map(|p| match &p.ty {
                    TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                        let mapped_ty = if p.optional {
                            "Option<wasm_bindgen::JsValue>".to_string()
                        } else {
                            "wasm_bindgen::JsValue".to_string()
                        };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                    _ => {
                        let ty = mapper.map_type(&p.ty);
                        let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                })
                .collect()
        } else {
            params.clone()
        };

        // Generate serde deserialization let-bindings for named non-opaque params
        let mut serde_bindings = String::new();
        if has_named {
            for p in &method.params {
                if let alef_core::ir::TypeRef::Named(name) = &p.ty {
                    if !opaque_types.contains(name.as_str()) {
                        let core_path = format!("{}::{}", core_import, name);
                        let err_conv = ".map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))";
                        if p.optional {
                            serde_bindings.push_str(&crate::template_env::render(
                                "serde_named_optional",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        } else {
                            serde_bindings.push_str(&crate::template_env::render(
                                "serde_named_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    }
                }
            }
        }

        let let_bindings = serde_bindings;
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&method.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
        };
        let core_call = format!(
            "{core_import}::{type_name}::from(self.clone()).{method_name}({call_args})",
            method_name = method.name
        );
        let body = if method.error_type.is_some() {
            format!(
                "{let_bindings}let result = {core_call}.await\n        \
                 .map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))?;\n    \
                 Ok({}::from(result))",
                return_type
            )
        } else {
            format!(
                "{let_bindings}let result = {core_call}.await;\n    \
                 Ok({}::from(result))",
                return_type
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub async fn {}(&self, {}) -> {} {{\n    \
             {body}\n}}",
            method.name,
            async_params.join(", "),
            return_annotation
        )
    } else if method.is_static {
        let body = if can_delegate {
            // WASM does not use optional promotion, so use gen_named_let_bindings_no_promote.
            let let_bindings = if alef_codegen::generators::has_named_params(&method.params, opaque_types) {
                alef_codegen::generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let call_args = if let_bindings.is_empty() {
                generators::gen_call_args(&method.params, opaque_types)
            } else {
                generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
            };
            let core_call = format!("{core_import}::{type_name}::{}({call_args})", method.name);
            if method.error_type.is_some() {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                );
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
                )
            } else {
                format!(
                    "{let_bindings}{}",
                    wasm_wrap_return(
                        &core_call,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                    )
                )
            }
        } else {
            super::functions::gen_wasm_unimplemented_body(
                &method.return_type,
                &method.name,
                method.error_type.is_some(),
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            method.name,
            params.join(", "),
            return_annotation
        )
    } else {
        let body = if can_delegate {
            // WASM does not use optional promotion, so use gen_named_let_bindings_no_promote.
            let let_bindings = if alef_codegen::generators::has_named_params(&method.params, opaque_types) {
                alef_codegen::generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let call_args = if let_bindings.is_empty() {
                generators::gen_call_args(&method.params, opaque_types)
            } else {
                generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
            };
            let core_call = format!(
                "{core_import}::{type_name}::from(self.clone()).{method_name}({call_args})",
                method_name = method.name
            );
            if method.error_type.is_some() {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                );
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
                )
            } else {
                format!(
                    "{let_bindings}{}",
                    wasm_wrap_return(
                        &core_call,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                    )
                )
            }
        } else {
            super::functions::gen_wasm_unimplemented_body(
                &method.return_type,
                &method.name,
                method.error_type.is_some(),
            )
        };
        format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}(&self, {}) -> {} {{\n    \
             {body}\n}}",
            method.name,
            params.join(", "),
            return_annotation
        )
    }
}
