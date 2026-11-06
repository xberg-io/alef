//! Emits one R visitor callback method (ADR 0019).
//!
//! Split out of `trait_bridge.rs` to keep that file under the size cap.

use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, visitor_param_type};
use crate::core::ir::{MethodDef, TypeRef};

use super::build_extendr_arg;

/// Generate a single visitor method that checks if the R list has an element with this name
/// and calls it as a function.
pub(super) fn gen_visitor_method_extendr(
    out: &mut String,
    method: &MethodDef,
    context_type: Option<&str>,
    type_paths: &std::collections::HashMap<String, String>,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
    accessor: Option<&crate::codegen::visitor_accessor::VisitorAccessorSpec>,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let signature = sig_parts.join(", ");

    let return_type = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let empty_args = method.params.is_empty();
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| build_extendr_arg(p, context_type))
        .collect();
    let args_pairs: Vec<String> = method
        .params
        .iter()
        .zip(args.iter())
        .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
        .collect();
    let args_pairs = args_pairs.join(", ");
    let (scope_call_prefix, scope_call_suffix) = scope_call_wrapper(accessor, method, context_type);

    out.push_str(&crate::backends::extendr::template_env::render(
        "visitor_method.jinja",
        crate::alef_context! {
            method_name => name,
            signature => signature,
            return_type => return_type,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&return_type, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &return_type,
                result_metadata,
                "s.to_string()",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            empty_args => empty_args,
            args_pairs => args_pairs,
            scope_call_prefix => scope_call_prefix,
            scope_call_suffix => scope_call_suffix,
        },
    ));
}

/// Wrap the host call in the callback-scoped reader when the context is opted in.
fn scope_call_wrapper(
    accessor: Option<&crate::codegen::visitor_accessor::VisitorAccessorSpec>,
    method: &MethodDef,
    context_type: Option<&str>,
) -> (String, String) {
    let Some(spec) = accessor else {
        return (String::new(), String::new());
    };
    let Some(param) = method
        .params
        .iter()
        .find(|p| matches!(&p.ty, TypeRef::Named(n) if Some(n.as_str()) == context_type))
    else {
        return (String::new(), String::new());
    };
    let borrow = if param.is_ref {
        param.name.clone()
    } else {
        format!("&{}", param.name)
    };
    (
        format!("alef_call_with_scope_{}({borrow}, || ", spec.ident()),
        ")".to_string(),
    )
}
