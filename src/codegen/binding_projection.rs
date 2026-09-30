use crate::core::ir::ApiSurface;

pub(crate) fn project(api: &ApiSurface) -> ApiSurface {
    project_owned(api.clone())
}

pub(crate) fn project_owned(mut api: ApiSurface) -> ApiSurface {
    api.functions.retain(|function| !function.binding_excluded);
    api.types.retain(|type_def| !type_def.binding_excluded);
    api.enums.retain(|enum_def| !enum_def.binding_excluded);
    api.errors.retain(|error_def| !error_def.binding_excluded);
    crate::cli::pipeline::sanitize_binding_projection(&mut api);
    api
}
