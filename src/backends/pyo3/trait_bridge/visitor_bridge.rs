use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, visitor_param_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef};
use std::collections::HashMap;

pub(super) fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> anyhow::Result<String> {
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Pyo3,
    )?;

    // Emit a helper function for converting the configured visitor context to a Python dict.
    let helper_fn = crate::backends::pyo3::template_env::render(
        "trait_bridge/nodecontext_to_py_dict.jinja",
        minijinja::context! {
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
        },
    );

    // Struct with only the Python object — no cached fields needed
    let struct_def = crate::backends::pyo3::template_env::render(
        "trait_bridge/visitor_struct.jinja",
        minijinja::context! {
            struct_name => struct_name,
        },
    );

    // Trait impl — collect all methods
    let mut methods_code = String::new();
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method(
            &mut methods_code,
            method,
            trait_path,
            bridge_cfg,
            type_paths,
            struct_name,
            &result_metadata,
        );
    }

    let mut out = String::with_capacity(4096);
    out.push_str(&helper_fn);
    out.push_str(&struct_def);
    out.push_str(&crate::backends::pyo3::template_env::render(
        "trait_bridge/impl_header.jinja",
        minijinja::context! { trait_path => trait_path, struct_name => struct_name },
    ));
    out.push_str(&methods_code);
    out.push_str("}\n");
    Ok(out)
}

/// Generate a single visitor-style trait method that tries Python dispatch, falls back to default.
///
/// For each method the generated code:
/// 1. Checks if the Python object has an attribute with this method's name.
/// 2. If yes, calls the method with converted arguments and converts the Python return value
///    to the appropriate Rust return type.
/// 3. If no (attribute absent), returns the configured default result variant.
fn gen_visitor_method(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    bridge_cfg: &TraitBridgeConfig,
    type_paths: &HashMap<String, String>,
    struct_name: &str,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) {
    use crate::core::ir::TypeRef;

    let name = &method.name;

    // Build the &mut self signature using the same helper used for plugin methods.
    // For visitor methods the IR may encode `Option<&str>` as `ty=String, optional=true, is_ref=true`
    // and `&[String]` as `ty=Vec<String>, is_ref=true`.
    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    // Determine the return type for this visitor method.
    // Visitor-style methods may return a named type from the core crate.
    // Use the fully-qualified path from type_paths when available.
    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths.get(n).cloned().unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    // Build argument expressions for the Python call
    let py_args = build_visitor_py_args(method, bridge_cfg);

    let py_call = if py_args.is_empty() {
        format!("obj.call_method0(\"{name}\")")
    } else {
        format!("obj.call_method1(\"{name}\", ({py_args}))")
    };

    let method_code = crate::backends::pyo3::template_env::render(
        "trait_bridge/visitor_method.jinja",
        minijinja::context! {
            wrapper => struct_name,
            method_name => name,
            sig => sig,
            ret_ty => ret_ty,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&ret_ty, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &ret_ty,
                result_metadata,
                "s",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            py_call => py_call,
        },
    );

    out.push_str(&method_code);
}

/// Build Python call argument expressions for a visitor method.
///
/// - configured context params: converted to a Python dict via `nodecontext_to_py_dict`
/// - `&str` params: passed directly (PyO3 handles `&str` → Python str coercion)
/// - `Option<&str>` params: passed as `Option<&str>` (PyO3 maps `None` → Python `None`)
/// - `bool` and integer params: passed directly
/// - `&[String]` / `Vec<String>` params: passed as Python lists
fn build_visitor_py_args(method: &MethodDef, bridge_cfg: &TraitBridgeConfig) -> String {
    use crate::core::ir::TypeRef;
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            // context_type param: convert to Python dict
            if let TypeRef::Named(n) = &p.ty {
                if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
                    return if p.is_ref {
                        format!("nodecontext_to_py_dict(py, {})", p.name)
                    } else {
                        format!("nodecontext_to_py_dict(py, &{})", p.name)
                    };
                }
            }
            // `Option<&str>`: IR collapses to String + optional + is_ref — pass directly
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                return p.name.clone();
            }
            // `&[String]`: IR collapses to Vec<String> + is_ref — pass directly (slice → PyList)
            if p.is_ref {
                if let TypeRef::Vec(inner) = &p.ty {
                    if matches!(inner.as_ref(), TypeRef::String) {
                        return p.name.clone();
                    }
                }
            }
            // Owned Vec<String>: convert to list
            if let TypeRef::Vec(inner) = &p.ty {
                if matches!(inner.as_ref(), TypeRef::String) {
                    return format!("{}.to_vec()", p.name);
                }
            }
            // Option<&str> encoded as Optional<String>
            if let TypeRef::Optional(inner) = &p.ty {
                if matches!(inner.as_ref(), TypeRef::String) {
                    return p.name.clone();
                }
            }
            // &str: pass directly
            if matches!(&p.ty, TypeRef::String) && p.is_ref {
                return p.name.clone();
            }
            if matches!(&p.ty, TypeRef::String) {
                return format!("{}.as_str()", p.name);
            }
            // Primitives and everything else: pass directly
            p.name.clone()
        })
        .collect();
    if args.len() == 1 {
        format!("{},", args[0])
    } else {
        args.join(", ")
    }
}
