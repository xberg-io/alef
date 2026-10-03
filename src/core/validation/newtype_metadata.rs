use crate::core::ir::{ApiSurface, FieldDef, MethodDef, NewtypeWrapper, ParamDef, TypeRef};

use super::{ValidationCode, ValidationDiagnostic};

pub(super) fn newtype_metadata_diagnostics(api: &ApiSurface) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    for typ in &api.types {
        for field in &typ.fields {
            validate_field(
                api,
                &format!("{}.{}", typ.rust_path, field.name),
                field,
                &mut diagnostics,
            );
        }
        for method in &typ.methods {
            validate_method(
                api,
                &format!("{}::{}", typ.rust_path, method.name),
                method,
                &mut diagnostics,
            );
        }
    }
    for function in &api.functions {
        for param in &function.params {
            validate_param(
                api,
                &format!("{} parameter `{}`", function.rust_path, param.name),
                param,
                &mut diagnostics,
            );
        }
        validate_slot(
            api,
            &format!("{} return", function.rust_path),
            &function.return_type,
            false,
            function.return_newtype_wrapper.as_deref(),
            &mut diagnostics,
        );
    }
    for enum_def in &api.enums {
        for variant in enum_def.variants.iter().chain(&enum_def.excluded_variants) {
            for field in &variant.fields {
                validate_field(
                    api,
                    &format!("{}::{}.{}", enum_def.rust_path, variant.name, field.name),
                    field,
                    &mut diagnostics,
                );
            }
        }
        for method in &enum_def.methods {
            validate_method(
                api,
                &format!("{}::{}", enum_def.rust_path, method.name),
                method,
                &mut diagnostics,
            );
        }
    }
    for error in &api.errors {
        for variant in &error.variants {
            for field in &variant.fields {
                validate_field(
                    api,
                    &format!("{}::{}.{}", error.rust_path, variant.name, field.name),
                    field,
                    &mut diagnostics,
                );
            }
        }
        for method in &error.methods {
            validate_method(
                api,
                &format!("{}::{}", error.rust_path, method.name),
                method,
                &mut diagnostics,
            );
        }
    }
    diagnostics
}

fn validate_field(api: &ApiSurface, item_path: &str, field: &FieldDef, diagnostics: &mut Vec<ValidationDiagnostic>) {
    validate_slot(
        api,
        item_path,
        &field.ty,
        field.optional,
        field.newtype_wrapper.as_deref(),
        diagnostics,
    );
}

fn validate_param(api: &ApiSurface, item_path: &str, param: &ParamDef, diagnostics: &mut Vec<ValidationDiagnostic>) {
    validate_slot(
        api,
        item_path,
        &param.ty,
        param.optional,
        param.newtype_wrapper.as_deref(),
        diagnostics,
    );
}

fn validate_method(api: &ApiSurface, item_path: &str, method: &MethodDef, diagnostics: &mut Vec<ValidationDiagnostic>) {
    for param in &method.params {
        validate_param(
            api,
            &format!("{item_path} parameter `{}`", param.name),
            param,
            diagnostics,
        );
    }
    validate_slot(
        api,
        &format!("{item_path} return"),
        &method.return_type,
        false,
        method.return_newtype_wrapper.as_deref(),
        diagnostics,
    );
}

fn validate_slot(
    api: &ApiSurface,
    item_path: &str,
    ty: &TypeRef,
    optional: bool,
    encoded: Option<&str>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let Some(encoded) = encoded else {
        return;
    };
    let result = NewtypeWrapper::decode(encoded).and_then(|wrapper| wrapper.validate_for_type(ty, optional));
    if let Err(reason) = result {
        diagnostics.push(ValidationDiagnostic::error(
            ValidationCode::InvalidNewtypeMetadata,
            api.crate_name.clone(),
            Some(item_path.to_string()),
            format!("invalid transparent newtype conversion metadata: {reason}"),
            "clear the IR cache and re-extract with the current Alef version".to_string(),
        ));
    }
}
