use crate::codegen::generators::trait_bridge::{TraitBridgeSpec, format_type_ref};
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

pub(super) struct MethodWriteback {
    pub(super) param_name: String,
    pub(super) binding_type: String,
    pub(super) core_type: String,
    pub(super) merge_fn: String,
}

pub(super) fn method_shape(
    method: &MethodDef,
    spec: &TraitBridgeSpec<'_>,
    struct_param_types: &std::collections::HashSet<String>,
) -> Option<MethodWriteback> {
    if !matches!(method.return_type, TypeRef::Unit) {
        return None;
    }
    let param = method.params.iter().find(|param| param.is_mut)?;
    let TypeRef::Named(type_name) = &param.ty else {
        return None;
    };
    if !struct_param_types.contains(type_name) {
        return None;
    }
    Some(MethodWriteback {
        param_name: param.name.clone(),
        binding_type: type_name.clone(),
        core_type: format_type_ref(&param.ty, &spec.type_paths),
        merge_fn: merge_fn_name(spec, method),
    })
}

pub(super) fn gen_merge_helpers(
    trait_type: &TypeDef,
    spec: &TraitBridgeSpec<'_>,
    api: &ApiSurface,
    struct_param_types: &std::collections::HashSet<String>,
    pyclass_absent_types: &ahash::AHashSet<String>,
) -> String {
    let excluded_types: Vec<String> = pyclass_absent_types.iter().cloned().collect();
    trait_type
        .methods
        .iter()
        .filter_map(|method| {
            let writeback = method_shape(method, spec, struct_param_types)?;
            let type_def = api.types.iter().find(|typ| typ.name == writeback.binding_type)?;
            let fields: Vec<minijinja::Value> = type_def
                .fields
                .iter()
                .filter(|field| !field.binding_excluded)
                .filter(|field| !field.serde_skip)
                .filter(|field| !field.sanitized || field.core_wrapper == crate::core::ir::CoreWrapper::Cow)
                .filter(|field| {
                    !crate::codegen::conversions::field_references_excluded_type(&field.ty, &excluded_types)
                })
                .map(|field| {
                    crate::alef_context! {
                        name => crate::codegen::naming::internal_rust_identifier(&field.name),
                        cfg => field.cfg.as_deref(),
                    }
                })
                .collect();
            let has_fields = !fields.is_empty();
            Some(crate::backends::pyo3::template_env::render(
                "trait_bridge/mut_writeback_merge_fn.jinja",
                crate::alef_context! {
                    merge_fn => writeback.merge_fn,
                    core_type => writeback.core_type,
                    fields => fields,
                    has_fields => has_fields,
                },
            ))
        })
        .collect()
}

fn merge_fn_name(spec: &TraitBridgeSpec<'_>, method: &MethodDef) -> String {
    let wrapper = crate::codegen::naming::pascal_to_snake(&spec.wrapper_name());
    let method = crate::codegen::naming::pascal_to_snake(&method.name);
    crate::codegen::naming::internal_rust_identifier(&format!("__alef_{wrapper}_{method}_merge_public_fields"))
}
