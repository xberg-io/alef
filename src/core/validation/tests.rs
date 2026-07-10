use super::*;
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef,
    HandlerContractDef, MethodDef, ParamDef, ReceiverKind, RegistrationDef, ServiceDef, TypeDef, TypeRef,
    UnsupportedPublicItem,
};
use ahash::AHashSet;

fn function_def(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample_lib::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn field_def(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        original_type: None,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: Default::default(),
        vec_inner_core_wrapper: Default::default(),
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn method_def(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn api_surface_validation_reports_lossy_sanitized_fields() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![TypeDef {
            name: "Request".to_string(),
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: true,
                original_type: Some("FrameworkPayload".to_string()),
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: Default::default(),
                vec_inner_core_wrapper: Default::default(),
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert_eq!(report.diagnostics.len(), 1);
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.severity, ValidationSeverity::Error);
    assert_eq!(diagnostic.code, ValidationCode::LossySanitizedSurface);
    assert_eq!(diagnostic.crate_name, "sample-lib");
    assert_eq!(diagnostic.item_path.as_deref(), Some("field Request.payload"));
    assert!(
        diagnostic
            .reason
            .contains("field type `FrameworkPayload` was sanitized to `String`"),
        "{}",
        diagnostic.reason
    );
}

#[test]
fn report_formats_only_errors() {
    let mut report = ValidationReport::new();
    report.push(ValidationDiagnostic::warning(
        ValidationCode::SerdeMetadataIncomplete,
        "sample-lib",
        Some(Language::Dart),
        Some("Sample.field".to_string()),
        "serde default metadata is unavailable",
        "add explicit metadata",
    ));
    report.push(ValidationDiagnostic::error(
        ValidationCode::MissingPublishMetadata,
        "sample-lib",
        None,
        "missing package repository",
        "set package_metadata.repository",
    ));

    let formatted = report.format_errors();

    assert!(formatted.contains("validation failed"));
    assert!(formatted.contains("missing_publish_metadata"));
    assert!(!formatted.contains("serde_metadata_incomplete"));
}

#[test]
fn api_surface_validation_errors_for_unknown_named_types() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![function_def(
            "render",
            vec![ParamDef {
                name: "settings".to_string(),
                ty: TypeRef::Named("RenderSettings".to_string()),
                ..ParamDef::default()
            }],
            TypeRef::String,
        )],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::UnknownNamedType
            && diagnostic.item_path.as_deref() == Some("function render param settings")
            && diagnostic.reason.contains("RenderSettings")
    }));
}

#[test]
fn api_surface_validation_errors_for_unsupported_public_generics() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        unsupported_public_items: vec![UnsupportedPublicItem {
            item_kind: "function".to_string(),
            item_path: "sample_lib::render".to_string(),
            reason: "public function has generic parameters".to_string(),
            suggested_fix: "expose a concrete wrapper".to_string(),
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::UnsupportedGenericItem
            && diagnostic.item_path.as_deref() == Some("sample_lib::render")
            && diagnostic.reason.contains("generic parameters")
    }));
    assert!(is_critical_unsuppressible(ValidationCode::UnsupportedGenericItem));
}

#[test]
fn api_surface_validation_does_not_treat_excluded_types_as_publicly_known() {
    let mut api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![function_def(
            "render",
            vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("HiddenPayload".to_string()),
                ..ParamDef::default()
            }],
            TypeRef::String,
        )],
        ..ApiSurface::default()
    };
    api.excluded_type_paths.insert(
        "HiddenPayload".to_string(),
        "sample_lib::internal::HiddenPayload".to_string(),
    );

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ValidationCode::UnknownNamedType
            && diagnostic.item_path.as_deref() == Some("function render param payload")
            && diagnostic.reason.contains("HiddenPayload")
    }));
}

#[test]
fn api_surface_validation_allows_excluded_types_only_for_configured_bridged_traits() {
    let mut api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![TypeDef {
            name: "Renderer".to_string(),
            rust_path: "sample_lib::Renderer".to_string(),
            is_trait: true,
            methods: vec![method_def(
                "render",
                vec![ParamDef {
                    name: "payload".to_string(),
                    ty: TypeRef::Named("HiddenPayload".to_string()),
                    ..ParamDef::default()
                }],
                TypeRef::String,
            )],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    api.excluded_type_paths.insert(
        "HiddenPayload".to_string(),
        "sample_lib::internal::HiddenPayload".to_string(),
    );

    let unbridged = validate_api_surface(&api);
    assert!(
        unbridged
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::UnknownNamedType),
        "unconfigured trait methods must not treat excluded types as known"
    );

    let bridged = AHashSet::from(["Renderer"]);
    let bridged_report = validate_api_surface_with_bridged_traits(&api, &bridged);
    assert!(
        !bridged_report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == ValidationCode::UnknownNamedType),
        "configured bridged traits may substitute excluded types"
    );
}

#[test]
fn api_surface_validation_errors_for_ambiguous_bare_json_value() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![function_def(
            "decode",
            vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("Value".to_string()),
                ..ParamDef::default()
            }],
            TypeRef::String,
        )],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::JsonValueResolutionAmbiguous
            && diagnostic.item_path.as_deref() == Some("function decode param payload")
    }));
}

#[test]
fn api_surface_validation_errors_for_ambiguous_bare_json_value_alias() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![function_def(
            "decode",
            vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Named("JsonValue".to_string()),
                ..ParamDef::default()
            }],
            TypeRef::String,
        )],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::JsonValueResolutionAmbiguous
            && diagnostic.item_path.as_deref() == Some("function decode param payload")
    }));
}

#[test]
fn api_surface_validation_errors_for_ambiguous_bare_json_value_inside_map() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![function_def(
            "decode",
            vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Named("Value".to_string()))),
                ..ParamDef::default()
            }],
            TypeRef::String,
        )],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::JsonValueResolutionAmbiguous
            && diagnostic.item_path.as_deref() == Some("function decode param payload")
    }));
}

#[test]
fn api_surface_validation_errors_for_backend_stub_paths() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![FunctionDef {
            sanitized: true,
            ..function_def("render", vec![], TypeRef::String)
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::BackendStubPath
            && diagnostic.item_path.as_deref() == Some("function render")
    }));
}

#[test]
fn api_surface_validation_errors_for_non_delegatable_function_returning_opaque_type() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![TypeDef {
            name: "Session".to_string(),
            rust_path: "sample_lib::Session".to_string(),
            is_opaque: true,
            ..TypeDef::default()
        }],
        functions: vec![function_def(
            "lookup",
            vec![ParamDef {
                name: "key".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                sanitized: true,
                ..ParamDef::default()
            }],
            TypeRef::Named("Session".to_string()),
        )],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::BackendStubPath
            && diagnostic.item_path.as_deref() == Some("function lookup")
            && diagnostic.reason.contains("Session")
    }));
}

#[test]
fn api_surface_validation_allows_opaque_receiver_mut_method_returning_opaque_type() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![
            TypeDef {
                name: "Parser".to_string(),
                rust_path: "sample_lib::Parser".to_string(),
                is_opaque: true,
                methods: vec![MethodDef {
                    receiver: Some(ReceiverKind::RefMut),
                    ..method_def(
                        "parse",
                        vec![],
                        TypeRef::Optional(Box::new(TypeRef::Named("Tree".to_string()))),
                    )
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Tree".to_string(),
                rust_path: "sample_lib::Tree".to_string(),
                is_opaque: true,
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(
        !report.has_errors(),
        "&mut self on opaque type returning opaque handle must not trigger BackendStubPath: {report:?}"
    );
}

#[test]
fn api_surface_validation_errors_for_non_opaque_receiver_mut_method_returning_opaque_type() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![
            TypeDef {
                name: "Builder".to_string(),
                rust_path: "sample_lib::Builder".to_string(),
                is_opaque: false,
                methods: vec![MethodDef {
                    receiver: Some(ReceiverKind::RefMut),
                    ..method_def("build", vec![], TypeRef::Named("Session".to_string()))
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Session".to_string(),
                rust_path: "sample_lib::Session".to_string(),
                is_opaque: true,
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::BackendStubPath
            && diagnostic.item_path.as_deref() == Some("method Builder.build")
            && diagnostic.reason.contains("Session")
    }));
}

#[test]
fn critical_validation_codes_are_not_globally_suppressible() {
    for code in [
        ValidationCode::UnknownNamedType,
        ValidationCode::LossySanitizedSurface,
        ValidationCode::JsonValueResolutionAmbiguous,
        ValidationCode::BackendStubPath,
    ] {
        assert!(is_critical_unsuppressible(code), "{code} must be fatal");
    }

    assert!(!is_critical_unsuppressible(ValidationCode::MissingPublishMetadata));
}

#[test]
fn api_surface_validation_skips_binding_excluded_functions() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![FunctionDef {
            sanitized: true,
            binding_excluded: true,
            ..function_def(
                "stream",
                vec![ParamDef {
                    name: "payload".to_string(),
                    ty: TypeRef::Named("Value".to_string()),
                    ..ParamDef::default()
                }],
                TypeRef::Named("Hidden".to_string()),
            )
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(
        !report.has_errors(),
        "excluded functions must not block generation: {report:?}"
    );
}

#[test]
fn api_surface_validation_skips_adapter_excluded_sanitized_methods() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            methods: vec![MethodDef {
                sanitized: true,
                binding_excluded: true,
                binding_exclusion_reason: Some("handled by adapter".to_string()),
                ..method_def(
                    "stream",
                    vec![ParamDef {
                        name: "payload".to_string(),
                        ty: TypeRef::Named("Value".to_string()),
                        ..ParamDef::default()
                    }],
                    TypeRef::Named("Hidden".to_string()),
                )
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(
        !report.has_errors(),
        "adapter-excluded sanitized methods must not block generation: {report:?}"
    );
}

#[test]
fn api_surface_validation_checks_enum_and_error_variant_fields() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        enums: vec![EnumDef {
            name: "Event".to_string(),
            variants: vec![EnumVariant {
                name: "Created".to_string(),
                fields: vec![field_def("payload", TypeRef::Named("MissingPayload".to_string()))],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }],
        errors: vec![ErrorDef {
            name: "SampleError".to_string(),
            rust_path: "sample_lib::SampleError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Invalid".to_string(),
                fields: vec![field_def("metadata", TypeRef::Named("JsonValue".to_string()))],
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::UnknownNamedType
            && diagnostic.item_path.as_deref() == Some("enum variant Event.Created")
            && diagnostic.reason.contains("MissingPayload")
    }));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == ValidationSeverity::Error
            && diagnostic.code == ValidationCode::JsonValueResolutionAmbiguous
            && diagnostic.item_path.as_deref() == Some("error variant SampleError.Invalid")
    }));
}

#[test]
fn api_surface_validation_skips_binding_excluded_variant_fields() {
    let mut excluded_field = field_def("metadata", TypeRef::Named("JsonValue".to_string()));
    excluded_field.binding_excluded = true;
    excluded_field.binding_exclusion_reason = Some("alef(skip)".to_string());

    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        enums: vec![EnumDef {
            name: "Event".to_string(),
            variants: vec![EnumVariant {
                name: "Created".to_string(),
                fields: vec![excluded_field.clone()],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }],
        errors: vec![ErrorDef {
            name: "SampleError".to_string(),
            rust_path: "sample_lib::SampleError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Invalid".to_string(),
                fields: vec![excluded_field],
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(
        !report.has_errors(),
        "excluded variant fields must not block generation: {report:?}"
    );
}

#[test]
fn api_surface_validation_checks_service_ir_types() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        services: vec![ServiceDef {
            name: "App".to_string(),
            rust_path: "sample_lib::App".to_string(),
            constructor: method_def("new", vec![], TypeRef::Named("App".to_string())),
            configurators: vec![method_def(
                "with_state",
                vec![ParamDef {
                    name: "state".to_string(),
                    ty: TypeRef::Named("MissingState".to_string()),
                    ..ParamDef::default()
                }],
                TypeRef::Named("App".to_string()),
            )],
            registrations: vec![RegistrationDef {
                method: "route".to_string(),
                callback_param: "handler".to_string(),
                callback_contract: "Handler".to_string(),
                metadata_params: vec![ParamDef {
                    name: "metadata".to_string(),
                    ty: TypeRef::Named("Value".to_string()),
                    ..ParamDef::default()
                }],
                receiver: None,
                return_type: TypeRef::Named("App".to_string()),
                error_type: None,
                doc: String::new(),
                variants: vec![],
                ..Default::default()
            }],
            entrypoints: vec![EntrypointDef {
                method: "run".to_string(),
                kind: EntrypointKind::Run,
                is_async: true,
                params: vec![ParamDef {
                    name: "addr".to_string(),
                    ty: TypeRef::Named("SocketAddr".to_string()),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Unit,
                error_type: None,
                doc: String::new(),
            }],
            doc: String::new(),
            cfg: None,
        }],
        types: vec![TypeDef {
            name: "App".to_string(),
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ValidationCode::UnknownNamedType
            && diagnostic.item_path.as_deref() == Some("service App configurator with_state param state")
            && diagnostic.reason.contains("MissingState")
    }));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ValidationCode::JsonValueResolutionAmbiguous
            && diagnostic.item_path.as_deref() == Some("service App registration route metadata param metadata")
    }));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == ValidationCode::UnknownNamedType
            && diagnostic.item_path.as_deref() == Some("service App entrypoint run param addr")
            && diagnostic.reason.contains("SocketAddr")
    }));
}

#[test]
fn api_surface_validation_checks_handler_contract_ir_types() {
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        handler_contracts: vec![HandlerContractDef {
            trait_name: "Handler".to_string(),
            rust_path: "sample_lib::Handler".to_string(),
            dispatch: method_def(
                "handle",
                vec![ParamDef {
                    name: "request".to_string(),
                    ty: TypeRef::Named("MissingRequest".to_string()),
                    ..ParamDef::default()
                }],
                TypeRef::Named("MissingResponse".to_string()),
            ),
            optional_methods: vec![method_def("metadata", vec![], TypeRef::Named("JsonValue".to_string()))],
            wire_request_type: Some("WireRequest".to_string()),
            wire_response_type: Some("WireResponse".to_string()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: String::new(),
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(report.has_errors());
    for expected in [
        "handler contract Handler dispatch param request",
        "handler contract Handler dispatch",
        "handler contract Handler optional method metadata",
        "handler contract Handler wire request",
        "handler contract Handler wire response",
    ] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.item_path.as_deref() == Some(expected)),
            "missing diagnostic for {expected}: {report:?}"
        );
    }
}
