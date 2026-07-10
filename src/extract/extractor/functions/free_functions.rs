use crate::core::ir::FunctionDef;

use super::super::helpers::{
    build_rust_path, extract_binding_exclusion_reason, extract_cfg_condition, extract_doc_comments,
    extract_version_annotation,
};
use super::returns::detect_cow_return;
use super::{extract_params, resolve_return_type, unwrap_future_return};

pub(crate) fn extract_function(item: &syn::ItemFn, crate_name: &str, module_path: &str) -> Option<FunctionDef> {
    if !super::super::helpers::is_pub(&item.vis) {
        return None;
    }

    if !item.sig.generics.params.is_empty() {
        return None;
    }

    let binding_exclusion_reason = extract_binding_exclusion_reason(&item.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let cfg = extract_cfg_condition(&item.attrs);
    let name = item.sig.ident.to_string();
    let doc = extract_doc_comments(&item.attrs);
    let mut is_async = item.sig.asyncness.is_some();

    let (mut return_type, mut error_type, returns_ref) = resolve_return_type(&item.sig.output);
    let returns_cow = detect_cow_return(&item.sig.output);

    if !is_async {
        let empty = ahash::AHashSet::new();
        if let Some((inner, future_error_type)) = unwrap_future_return(&item.sig.output, &empty) {
            is_async = true;
            return_type = inner;
            if future_error_type.is_some() {
                error_type = future_error_type;
            }
        }
    }

    let params = extract_params(&item.sig.inputs);
    let rust_path = build_rust_path(crate_name, module_path, &name);
    let sanitized = params.iter().any(|p| p.sanitized);

    Some(FunctionDef {
        rust_path,
        original_rust_path: String::new(),
        name,
        params,
        return_type,
        is_async,
        error_type,
        doc,
        cfg,
        sanitized,
        return_sanitized: false,
        returns_ref,
        returns_cow,
        return_newtype_wrapper: None,
        binding_excluded,
        binding_exclusion_reason,
        version: extract_version_annotation(&item.attrs),
    })
}
