use crate::core::ir::{ApiSurface, FieldDef, MethodDef, NewtypeConversion, NewtypeWrapper, ParamDef, TypeRef};

use super::{ValidationCode, ValidationDiagnostic};

pub(super) fn newtype_metadata_diagnostics(api: &ApiSurface) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    for typ in &api.types {
        for field in &typ.fields {
            validate_field(
                api,
                &format!("{}.{}", typ.rust_path, field.name),
                field,
                !typ.binding_excluded && !field.binding_excluded,
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
        validate_function(api, function, &mut diagnostics);
    }
    for enum_def in &api.enums {
        for variant in enum_def.variants.iter().chain(&enum_def.excluded_variants) {
            for field in &variant.fields {
                validate_field(
                    api,
                    &format!("{}::{}.{}", enum_def.rust_path, variant.name, field.name),
                    field,
                    false,
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
                    false,
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

fn validate_function(
    api: &ApiSurface,
    function: &crate::core::ir::FunctionDef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for param in &function.params {
        validate_param(
            api,
            &format!("{} parameter `{}`", function.rust_path, param.name),
            param,
            !function.binding_excluded,
            diagnostics,
        );
    }
    let return_path = format!("{} return", function.rust_path);
    validate_slot(
        api,
        &return_path,
        &function.return_type,
        false,
        function.return_newtype_wrapper.as_deref(),
        diagnostics,
    );
    if !function.binding_excluded {
        validate_borrowed_return(
            api,
            &return_path,
            function.returns_ref,
            function.return_newtype_wrapper.as_deref(),
            diagnostics,
        );
    }
}

fn validate_field(
    api: &ApiSurface,
    item_path: &str,
    field: &FieldDef,
    generated_accessor_clones_value: bool,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    validate_slot(
        api,
        item_path,
        &field.ty,
        field.optional,
        field.newtype_wrapper.as_deref(),
        diagnostics,
    );
    if generated_accessor_clones_value {
        validate_cloned_field_wrappers(api, item_path, field.newtype_wrapper.as_deref(), diagnostics);
    }
}

fn validate_cloned_field_wrappers(
    api: &ApiSurface,
    item_path: &str,
    encoded: Option<&str>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let Some(wrapper) = encoded.and_then(|value| NewtypeWrapper::decode(value).ok()) else {
        return;
    };
    let mut reported = std::collections::BTreeSet::new();
    for metadata in wrapper.explicit_paths() {
        let NewtypeConversion::TransparentString { wrapper_is_clone, .. } = &metadata.conversion else {
            continue;
        };
        if !reported.insert(metadata.rust_path.as_str()) {
            continue;
        }
        if *wrapper_is_clone != Some(false) {
            continue;
        }
        diagnostics.push(ValidationDiagnostic::error(
            ValidationCode::InvalidNewtypeMetadata,
            api.crate_name.clone(),
            Some(item_path.to_string()),
            format!(
                "transparent wrapper `{}` requires Clone because generated field accessors clone this value before calling its consuming `into` operation",
                metadata.rust_path
            ),
            "derive or implement Clone for the wrapper, or exclude the containing field/type from generated bindings"
                .to_string(),
        ));
    }
}

fn validate_param(
    api: &ApiSurface,
    item_path: &str,
    param: &ParamDef,
    enforce_supported_surface: bool,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    validate_slot(
        api,
        item_path,
        &param.ty,
        param.optional,
        param.newtype_wrapper.as_deref(),
        diagnostics,
    );
    if enforce_supported_surface && param.is_ref && param.is_mut {
        validate_mutable_param(api, item_path, param.newtype_wrapper.as_deref(), diagnostics);
    }
}

fn validate_method(api: &ApiSurface, item_path: &str, method: &MethodDef, diagnostics: &mut Vec<ValidationDiagnostic>) {
    for param in &method.params {
        validate_param(
            api,
            &format!("{item_path} parameter `{}`", param.name),
            param,
            !method.binding_excluded,
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
    if !method.binding_excluded {
        validate_borrowed_return(
            api,
            &format!("{item_path} return"),
            method.returns_ref,
            method.return_newtype_wrapper.as_deref(),
            diagnostics,
        );
    }
}

fn validate_borrowed_return(
    api: &ApiSurface,
    item_path: &str,
    returns_ref: bool,
    encoded: Option<&str>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if !returns_ref || !is_explicit_wrapper(encoded) {
        return;
    }
    diagnostics.push(ValidationDiagnostic::error(
        ValidationCode::InvalidNewtypeMetadata,
        api.crate_name.clone(),
        Some(item_path.to_string()),
        "borrowed transparent-wrapper return is unsupported because its configured `into` operation consumes the wrapper"
            .to_string(),
        "return an owned wrapper value or exclude this item from generated bindings".to_string(),
    ));
}

fn validate_mutable_param(
    api: &ApiSurface,
    item_path: &str,
    encoded: Option<&str>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if !is_explicit_wrapper(encoded) {
        return;
    }
    diagnostics.push(ValidationDiagnostic::error(
        ValidationCode::InvalidNewtypeMetadata,
        api.crate_name.clone(),
        Some(item_path.to_string()),
        "mutable transparent-wrapper parameter is unsupported because generated bindings cannot write the mutated wrapper back to the caller"
            .to_string(),
        "accept the wrapper by value or shared reference, return the updated value explicitly, or exclude this item from generated bindings"
            .to_string(),
    ));
}

fn is_explicit_wrapper(encoded: Option<&str>) -> bool {
    encoded
        .and_then(|value| NewtypeWrapper::decode(value).ok())
        .is_some_and(|wrapper| !wrapper.explicit_paths().is_empty())
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
