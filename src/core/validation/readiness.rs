use super::{ValidationCode, ValidationDiagnostic};
use crate::core::ir::{ApiSurface, FunctionDef, HandlerContractDef, MethodDef, ReceiverKind, ServiceDef, TypeRef};
use ahash::AHashSet;

pub(super) fn backend_readiness_diagnostics(
    api: &ApiSurface,
    bridged_trait_names: &AHashSet<&str>,
) -> Vec<ValidationDiagnostic> {
    let known_names = known_type_names(api);
    let mut trait_known_names = known_names.clone();
    trait_known_names.extend(substitutable_type_names(api));
    let opaque_names = opaque_type_names(api);
    let mut diagnostics = Vec::new();

    for function in &api.functions {
        if function.binding_excluded {
            continue;
        }
        let item_path = format!("function {}", function.name);
        collect_function_diagnostics(api, &known_names, &opaque_names, &item_path, function, &mut diagnostics);
    }

    for typ in &api.types {
        if typ.binding_excluded {
            continue;
        }
        let method_known_names = if typ.is_trait && bridged_trait_names.contains(typ.name.as_str()) {
            &trait_known_names
        } else {
            &known_names
        };
        for method in &typ.methods {
            if method.binding_excluded {
                continue;
            }
            let item_path = format!("method {}.{}", typ.name, method.name);
            collect_method_diagnostics(
                api,
                method_known_names,
                &opaque_names,
                &item_path,
                method,
                typ.is_opaque,
                &mut diagnostics,
            );
        }
        for field in &typ.fields {
            if field.binding_excluded {
                continue;
            }
            collect_type_ref_diagnostics(
                api,
                &known_names,
                &format!("field {}.{}", typ.name, field.name),
                &field.ty,
                &mut diagnostics,
            );
        }
    }
    for enum_def in &api.enums {
        if enum_def.binding_excluded {
            continue;
        }
        for variant in &enum_def.variants {
            if variant.binding_excluded {
                continue;
            }
            for field in &variant.fields {
                if field.binding_excluded {
                    continue;
                }
                collect_type_ref_diagnostics(
                    api,
                    &known_names,
                    &format!("enum variant {}.{}", enum_def.name, variant.name),
                    &field.ty,
                    &mut diagnostics,
                );
            }
        }
    }
    for error_def in &api.errors {
        if error_def.binding_excluded {
            continue;
        }
        for method in &error_def.methods {
            if method.binding_excluded {
                continue;
            }
            let item_path = format!("error method {}.{}", error_def.name, method.name);
            collect_method_diagnostics(
                api,
                &known_names,
                &opaque_names,
                &item_path,
                method,
                false,
                &mut diagnostics,
            );
        }
        for variant in &error_def.variants {
            for field in &variant.fields {
                if field.binding_excluded {
                    continue;
                }
                collect_type_ref_diagnostics(
                    api,
                    &known_names,
                    &format!("error variant {}.{}", error_def.name, variant.name),
                    &field.ty,
                    &mut diagnostics,
                );
            }
        }
    }
    for service in &api.services {
        collect_service_diagnostics(api, &known_names, &opaque_names, service, &mut diagnostics);
    }
    for contract in &api.handler_contracts {
        collect_handler_contract_diagnostics(api, &known_names, &opaque_names, contract, &mut diagnostics);
    }

    diagnostics
}

fn known_type_names(api: &ApiSurface) -> AHashSet<&str> {
    api.types
        .iter()
        .map(|typ| typ.name.as_str())
        .chain(api.enums.iter().map(|item| item.name.as_str()))
        .chain(api.errors.iter().map(|item| item.name.as_str()))
        .collect()
}

/// Names of types that backends are expected to substitute at trait-bridge code-emit
/// time. Trait methods are allowed to reference these even though they no longer appear
/// in `api.types`, because the per-backend `trait_bridge.rs` swaps the reference for an
/// opaque JSON carrier (e.g. `json.RawMessage` in Go, `serde_json::Value` in Rust shims).
/// Free functions and fields can't be substituted that way, so they still hit
/// `unknown_named_type`.
fn substitutable_type_names(api: &ApiSurface) -> AHashSet<&str> {
    api.excluded_type_paths
        .keys()
        .map(String::as_str)
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect()
}

fn opaque_type_names(api: &ApiSurface) -> AHashSet<&str> {
    api.types
        .iter()
        .filter(|typ| typ.is_opaque)
        .map(|typ| typ.name.as_str())
        .collect()
}

fn collect_function_diagnostics(
    api: &ApiSurface,
    known_names: &AHashSet<&str>,
    opaque_names: &AHashSet<&str>,
    item_path: &str,
    function: &FunctionDef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if function.sanitized {
        diagnostics.push(ValidationDiagnostic::error(
            ValidationCode::BackendStubPath,
            api.crate_name.clone(),
            Some(item_path.to_string()),
            "function signature was sanitized and may require backend stub generation",
            "exclude the item, configure an opaque/trait bridge, or expose a binding-safe DTO",
        ));
    }
    if fallback_body_would_require_opaque_return(
        function.sanitized,
        &function.params,
        &function.return_type,
        opaque_names,
    ) {
        diagnostics.push(opaque_stub_path_diagnostic(api, item_path, &function.return_type));
    }
    for param in &function.params {
        collect_type_ref_diagnostics(
            api,
            known_names,
            &format!("{item_path} param {}", param.name),
            &param.ty,
            diagnostics,
        );
    }
    collect_type_ref_diagnostics(api, known_names, item_path, &function.return_type, diagnostics);
}

fn collect_method_diagnostics(
    api: &ApiSurface,
    known_names: &AHashSet<&str>,
    opaque_names: &AHashSet<&str>,
    item_path: &str,
    method: &MethodDef,
    receiver_type_is_opaque: bool,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if method.sanitized {
        diagnostics.push(ValidationDiagnostic::error(
            ValidationCode::BackendStubPath,
            api.crate_name.clone(),
            Some(item_path.to_string()),
            "method signature was sanitized and may require backend stub generation",
            "exclude the item, configure an opaque/trait bridge, or expose a binding-safe DTO",
        ));
    }
    let non_delegatable_ref_mut = matches!(method.receiver, Some(ReceiverKind::RefMut))
        && method.trait_source.is_none()
        && !receiver_type_is_opaque;
    if (non_delegatable_ref_mut && returns_opaque(&method.return_type, opaque_names))
        || fallback_body_would_require_opaque_return(
            method.sanitized,
            &method.params,
            &method.return_type,
            opaque_names,
        )
    {
        diagnostics.push(opaque_stub_path_diagnostic(api, item_path, &method.return_type));
    }
    for param in &method.params {
        collect_type_ref_diagnostics(
            api,
            known_names,
            &format!("{item_path} param {}", param.name),
            &param.ty,
            diagnostics,
        );
    }
    collect_type_ref_diagnostics(api, known_names, item_path, &method.return_type, diagnostics);
}

fn fallback_body_would_require_opaque_return(
    sanitized: bool,
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    opaque_names: &AHashSet<&str>,
) -> bool {
    returns_opaque(return_type, opaque_names)
        && (sanitized
            || params
                .iter()
                .any(|param| param.sanitized || is_named_ref_param(param, opaque_names)))
}

fn is_named_ref_param(param: &crate::core::ir::ParamDef, opaque_names: &AHashSet<&str>) -> bool {
    if !param.is_ref {
        return false;
    }
    match &param.ty {
        TypeRef::Named(name) => !opaque_names.contains(name.as_str()),
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::String | TypeRef::Char),
        _ => false,
    }
}

fn returns_opaque(ty: &TypeRef, opaque_names: &AHashSet<&str>) -> bool {
    match ty {
        TypeRef::Named(name) => opaque_names.contains(name.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => returns_opaque(inner, opaque_names),
        TypeRef::Map(key, value) => returns_opaque(key, opaque_names) || returns_opaque(value, opaque_names),
        _ => false,
    }
}

fn opaque_stub_path_diagnostic(api: &ApiSurface, item_path: &str, return_type: &TypeRef) -> ValidationDiagnostic {
    ValidationDiagnostic::error(
        ValidationCode::BackendStubPath,
        api.crate_name.clone(),
        Some(item_path.to_string()),
        format!(
            "non-delegatable signature returns opaque type `{}`",
            type_ref_label(return_type)
        ),
        "exclude the item, add an adapter body, or change the API to return a binding-safe type",
    )
}

fn type_ref_label(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named(name) => name.clone(),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_label(inner),
        TypeRef::Map(_, value) => type_ref_label(value),
        _ => format!("{ty:?}"),
    }
}

fn collect_service_diagnostics(
    api: &ApiSurface,
    known_names: &AHashSet<&str>,
    opaque_names: &AHashSet<&str>,
    service: &ServiceDef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    collect_method_diagnostics(
        api,
        known_names,
        opaque_names,
        &format!("service {} constructor", service.name),
        &service.constructor,
        false,
        diagnostics,
    );
    for configurator in &service.configurators {
        collect_method_diagnostics(
            api,
            known_names,
            opaque_names,
            &format!("service {} configurator {}", service.name, configurator.name),
            configurator,
            false,
            diagnostics,
        );
    }
    for registration in &service.registrations {
        let item_path = format!("service {} registration {}", service.name, registration.method);
        for param in &registration.metadata_params {
            collect_type_ref_diagnostics(
                api,
                known_names,
                &format!("{item_path} metadata param {}", param.name),
                &param.ty,
                diagnostics,
            );
        }
        collect_type_ref_diagnostics(api, known_names, &item_path, &registration.return_type, diagnostics);
        for variant in &registration.variants {
            for param in &variant.signature_params {
                collect_type_ref_diagnostics(
                    api,
                    known_names,
                    &format!("{item_path} variant {} param {}", variant.name, param.name),
                    &param.ty,
                    diagnostics,
                );
            }
            if let Some(wrapper_call) = &variant.wrapper_call {
                for arg in &wrapper_call.args {
                    if let crate::core::ir::WrapperConstructorArg::Free { param } = arg {
                        collect_type_ref_diagnostics(
                            api,
                            known_names,
                            &format!("{item_path} variant {} wrapper param {}", variant.name, param.name),
                            &param.ty,
                            diagnostics,
                        );
                    }
                }
            }
        }
    }
    for entrypoint in &service.entrypoints {
        let item_path = format!("service {} entrypoint {}", service.name, entrypoint.method);
        for param in &entrypoint.params {
            collect_type_ref_diagnostics(
                api,
                known_names,
                &format!("{item_path} param {}", param.name),
                &param.ty,
                diagnostics,
            );
        }
        collect_type_ref_diagnostics(api, known_names, &item_path, &entrypoint.return_type, diagnostics);
    }
}

fn collect_handler_contract_diagnostics(
    api: &ApiSurface,
    known_names: &AHashSet<&str>,
    opaque_names: &AHashSet<&str>,
    contract: &HandlerContractDef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    collect_method_diagnostics(
        api,
        known_names,
        opaque_names,
        &format!("handler contract {} dispatch", contract.trait_name),
        &contract.dispatch,
        false,
        diagnostics,
    );
    for method in &contract.optional_methods {
        collect_method_diagnostics(
            api,
            known_names,
            opaque_names,
            &format!(
                "handler contract {} optional method {}",
                contract.trait_name, method.name
            ),
            method,
            false,
            diagnostics,
        );
    }
    for (label, maybe_type) in [
        ("wire request", contract.wire_request_type.as_deref()),
        ("wire response", contract.wire_response_type.as_deref()),
    ] {
        if let Some(type_name) = maybe_type {
            collect_type_ref_diagnostics(
                api,
                known_names,
                &format!("handler contract {} {label}", contract.trait_name),
                &TypeRef::Named(type_name.to_string()),
                diagnostics,
            );
        }
    }
}

fn collect_type_ref_diagnostics(
    api: &ApiSurface,
    known_names: &AHashSet<&str>,
    item_path: &str,
    ty: &TypeRef,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    match ty {
        TypeRef::Named(name) if name == "Value" || name == "JsonValue" => {
            diagnostics.push(ValidationDiagnostic::error(
                ValidationCode::JsonValueResolutionAmbiguous,
                api.crate_name.clone(),
                Some(item_path.to_string()),
                format!("bare `{name}` cannot prove it is serde_json::Value"),
                "import or expose serde_json::Value with a resolved path, or configure the type explicitly",
            ))
        }
        TypeRef::Named(name) if !known_names.contains(name.as_str()) => {
            diagnostics.push(ValidationDiagnostic::error(
                ValidationCode::UnknownNamedType,
                api.crate_name.clone(),
                Some(item_path.to_string()),
                format!("named type `{name}` is not present in the extracted API surface"),
                "include the type in the public API, configure it as opaque/excluded, or add a bridge rule",
            ));
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_type_ref_diagnostics(api, known_names, item_path, inner, diagnostics);
        }
        TypeRef::Map(key, value) => {
            collect_type_ref_diagnostics(api, known_names, item_path, key, diagnostics);
            collect_type_ref_diagnostics(api, known_names, item_path, value, diagnostics);
        }
        _ => {}
    }
}
