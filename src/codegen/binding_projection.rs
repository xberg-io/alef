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
    normalize_tuple_param_metadata(&mut api);
    api
}

fn normalize_tuple_param_metadata(api: &mut ApiSurface) {
    for function in &mut api.functions {
        normalize_params(&mut function.params);
    }
    for type_def in &mut api.types {
        for method in &mut type_def.methods {
            normalize_params(&mut method.params);
        }
    }
    for enum_def in &mut api.enums {
        for method in &mut enum_def.methods {
            normalize_params(&mut method.params);
        }
    }
}

fn normalize_params(params: &mut [crate::core::ir::ParamDef]) {
    for param in params {
        let Some(original) = param.original_type.as_deref() else {
            continue;
        };
        let Ok(source_type) = syn::parse_str::<syn::Type>(original) else {
            continue;
        };
        let resolved = match crate::extract::type_resolver::resolve_type(&source_type) {
            crate::core::ir::TypeRef::Optional(inner) => *inner,
            resolved => resolved,
        };
        if contains_tuple(&resolved) {
            param.original_type = Some(format!("{resolved:?}"));
        }
    }
}

fn contains_tuple(ty: &crate::core::ir::TypeRef) -> bool {
    match ty {
        crate::core::ir::TypeRef::Named(name) => name.starts_with('('),
        crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => contains_tuple(inner),
        crate::core::ir::TypeRef::Map(key, value) => contains_tuple(key) || contains_tuple(value),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::project_owned;
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

    #[test]
    fn projects_source_tuple_syntax_to_binding_tuple_metadata() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "record".to_string(),
                params: vec![
                    ParamDef {
                        name: "entries".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::String)),
                        sanitized: true,
                        is_ref: true,
                        original_type: Some("&[(PathBuf, Option<FileExtractionConfig>)]".to_string()),
                        ..ParamDef::default()
                    },
                    ParamDef {
                        name: "optional_entries".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::String)),
                        optional: true,
                        sanitized: true,
                        is_ref: true,
                        original_type: Some("Option<&[(PathBuf, Option<FileExtractionConfig>)]>".to_string()),
                        ..ParamDef::default()
                    },
                ],
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };

        let projected = project_owned(api);

        assert_eq!(
            projected.functions[0].params[0].original_type.as_deref(),
            Some("Vec(Named(\"(PathBuf, Option<FileExtractionConfig>)\"))")
        );
        assert_eq!(
            projected.functions[0].params[1].original_type.as_deref(),
            Some("Vec(Named(\"(PathBuf, Option<FileExtractionConfig>)\"))")
        );
    }
}
