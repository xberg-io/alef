use crate::core::backend::GeneratedFile;
use crate::core::ir::{ApiSurface, ParamDef, TypeRef};
use heck::ToSnakeCase;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Serialize)]
pub(crate) struct OwnedInputType {
    pub name: String,
    pub clone_function: String,
    pub swift_clone_function: String,
    pub is_clone: bool,
    pub is_copy: bool,
    pub cfg: Option<String>,
}

pub(crate) fn element(param: &ParamDef) -> Option<&str> {
    if param.vec_inner_is_ref {
        return None;
    }
    let ty = match &param.ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        ty => ty,
    };
    match ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn collect(
    api: &ApiSurface,
    exclude_types: &HashSet<String>,
    exclude_functions: &HashSet<String>,
    is_dto: impl Fn(&str) -> bool,
) -> Vec<OwnedInputType> {
    let used: HashSet<&str> = api
        .functions
        .iter()
        .filter(|function| !function.binding_excluded && !exclude_functions.contains(&function.name))
        .flat_map(|function| function.params.iter())
        .filter_map(element)
        .collect();
    let mut inputs: Vec<_> = api
        .types
        .iter()
        .filter(|ty| !ty.is_trait && !ty.binding_excluded && !exclude_types.contains(&ty.name))
        .filter(|ty| used.contains(ty.name.as_str()) && !is_dto(&ty.name))
        .map(|ty| OwnedInputType {
            name: ty.name.clone(),
            clone_function: format!("__alef_clone_{}", ty.name.to_snake_case()),
            swift_clone_function: format!("__alefClone{}", ty.name),
            is_clone: ty.is_clone,
            is_copy: ty.is_copy,
            cfg: ty.cfg.clone(),
        })
        .collect();
    inputs.sort_by(|left, right| left.name.cmp(&right.name));
    inputs
}

pub(crate) fn externs(inputs: &[OwnedInputType]) -> String {
    super::template_env::render(
        "owned_vector_input_extern.rs.jinja",
        crate::alef_context! { inputs => inputs },
    )
}

pub(crate) fn implementations(inputs: &[OwnedInputType]) -> String {
    super::template_env::render(
        "owned_vector_input_clone.rs.jinja",
        crate::alef_context! { inputs => inputs },
    )
}

pub(crate) fn swift_file(inputs: &[OwnedInputType], sources: &Path) -> Option<GeneratedFile> {
    if inputs.is_empty() {
        return None;
    }
    Some(GeneratedFile {
        path: sources.join("ZSwiftOwnedVectorInputs.swift"),
        content: super::template_env::render(
            "owned_vector_inputs.swift.jinja",
            crate::alef_context! {
                inputs => inputs, header => crate::core::hash::SELF_MARKING_HEADER_LINE,
            },
        ),
        generated_header: false,
    })
}

pub(crate) fn conversion(param: &ParamDef, local: &str, swift_name: &str) -> String {
    super::template_env::render(
        "owned_vector_input_conversion.swift.jinja",
        crate::alef_context! {
            name => element(param).expect("qualified owned opaque vector"), local => local, parameter => swift_name,
            optional => param.optional || matches!(param.ty, TypeRef::Optional(_)),
        },
    )
    .trim_end()
    .to_string()
}
