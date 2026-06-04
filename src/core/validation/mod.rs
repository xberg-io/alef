//! Central validation diagnostics for generation inputs and backend readiness.
//!
//! Validation should happen between config/extraction resolution and backend
//! emission. This module provides the shared diagnostic shape; individual
//! validators can stay close to the subsystem they understand while returning
//! structured reports here.

use crate::core::config::Language;
use crate::core::ir::{ApiSurface, FunctionDef, HandlerContractDef, MethodDef, ReceiverKind, ServiceDef, TypeRef};
use crate::extract::validation::sanitized_public_api_diagnostics;
use ahash::AHashSet;
use std::fmt;

/// Validation severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    Warning,
    Error,
}

impl fmt::Display for ValidationSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

/// Stable validation diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationCode {
    UnknownNamedType,
    UnsupportedGenericItem,
    LossySanitizedSurface,
    MissingPublishMetadata,
    UnsupportedBackendCapability,
    TraitBridgeCarrierUnavailable,
    SerdeMetadataIncomplete,
    JsonValueResolutionAmbiguous,
    BackendStubPath,
}

/// Diagnostics that are never safe to suppress globally.
///
/// These codes represent known lossy or ambiguous public API surfaces. Allowing
/// a crate-wide suppression would let generation proceed with plausible but
/// incorrect bindings.
pub fn is_critical_unsuppressible(code: ValidationCode) -> bool {
    matches!(
        code,
        ValidationCode::UnknownNamedType
            | ValidationCode::LossySanitizedSurface
            | ValidationCode::UnsupportedGenericItem
            | ValidationCode::JsonValueResolutionAmbiguous
            | ValidationCode::BackendStubPath
    )
}

impl fmt::Display for ValidationCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNamedType => f.write_str("unknown_named_type"),
            Self::UnsupportedGenericItem => f.write_str("unsupported_generic_item"),
            Self::LossySanitizedSurface => f.write_str("lossy_sanitized_surface"),
            Self::MissingPublishMetadata => f.write_str("missing_publish_metadata"),
            Self::UnsupportedBackendCapability => f.write_str("unsupported_backend_capability"),
            Self::TraitBridgeCarrierUnavailable => f.write_str("trait_bridge_carrier_unavailable"),
            Self::SerdeMetadataIncomplete => f.write_str("serde_metadata_incomplete"),
            Self::JsonValueResolutionAmbiguous => f.write_str("json_value_resolution_ambiguous"),
            Self::BackendStubPath => f.write_str("backend_stub_path"),
        }
    }
}

/// One structured validation issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDiagnostic {
    pub severity: ValidationSeverity,
    pub code: ValidationCode,
    pub crate_name: String,
    pub language: Option<Language>,
    pub item_path: Option<String>,
    pub reason: String,
    pub suggested_fix: String,
}

impl ValidationDiagnostic {
    pub fn error(
        code: ValidationCode,
        crate_name: impl Into<String>,
        item_path: impl Into<Option<String>>,
        reason: impl Into<String>,
        suggested_fix: impl Into<String>,
    ) -> Self {
        Self {
            severity: ValidationSeverity::Error,
            code,
            crate_name: crate_name.into(),
            language: None,
            item_path: item_path.into(),
            reason: reason.into(),
            suggested_fix: suggested_fix.into(),
        }
    }

    pub fn warning(
        code: ValidationCode,
        crate_name: impl Into<String>,
        language: Option<Language>,
        item_path: impl Into<Option<String>>,
        reason: impl Into<String>,
        suggested_fix: impl Into<String>,
    ) -> Self {
        Self {
            severity: ValidationSeverity::Warning,
            code,
            crate_name: crate_name.into(),
            language,
            item_path: item_path.into(),
            reason: reason.into(),
            suggested_fix: suggested_fix.into(),
        }
    }
}

impl fmt::Display for ValidationDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}:{}] crate `{}`", self.severity, self.code, self.crate_name)?;
        if let Some(language) = self.language {
            write!(f, " language `{language}`")?;
        }
        if let Some(item_path) = &self.item_path {
            write!(f, " item `{item_path}`")?;
        }
        write!(f, ": {} Suggested fix: {}", self.reason, self.suggested_fix)
    }
}

/// Collected validation diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ValidationReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diagnostic: ValidationDiagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn extend(&mut self, diagnostics: impl IntoIterator<Item = ValidationDiagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == ValidationSeverity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == ValidationSeverity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &ValidationDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == ValidationSeverity::Warning)
    }

    pub fn format_errors(&self) -> String {
        let mut message = String::from("validation failed");
        for diagnostic in self.errors() {
            message.push_str("\n- ");
            message.push_str(&diagnostic.to_string());
        }
        message
    }
}

/// Borrowed API surface that has passed central validation.
///
/// Public generation facades should accept this type so callers cannot
/// accidentally feed lossy or unsupported IR directly into backend emission.
#[derive(Debug, Clone, Copy)]
pub struct ValidatedApiSurface<'a> {
    api: &'a ApiSurface,
}

impl<'a> ValidatedApiSurface<'a> {
    pub fn new(api: &'a ApiSurface, suppress_codes: &[String]) -> Result<Self, ValidationReport> {
        Self::new_with_bridged_traits(api, suppress_codes, &AHashSet::new())
    }

    pub fn new_with_bridged_traits(
        api: &'a ApiSurface,
        suppress_codes: &[String],
        bridged_trait_names: &AHashSet<&str>,
    ) -> Result<Self, ValidationReport> {
        let report = validate_api_surface_with_bridged_traits(api, bridged_trait_names);
        let fatal = report.errors().any(|diagnostic| {
            is_critical_unsuppressible(diagnostic.code)
                || !suppress_codes.iter().any(|code| code == &diagnostic.code.to_string())
        });
        if fatal { Err(report) } else { Ok(Self { api }) }
    }

    pub fn api(&self) -> &'a ApiSurface {
        self.api
    }
}

/// Validate the extracted public API surface before backend generation.
pub fn validate_api_surface(api: &ApiSurface) -> ValidationReport {
    validate_api_surface_with_bridged_traits(api, &AHashSet::new())
}

/// Validate the extracted public API surface before backend generation, allowing
/// excluded-type substitution only for traits with explicit bridge config.
pub fn validate_api_surface_with_bridged_traits(
    api: &ApiSurface,
    bridged_trait_names: &AHashSet<&str>,
) -> ValidationReport {
    let mut report = ValidationReport::new();
    report.extend(sanitized_public_api_diagnostics(api).into_iter().map(|diagnostic| {
        ValidationDiagnostic::error(
            ValidationCode::LossySanitizedSurface,
            api.crate_name.clone(),
            Some(diagnostic.item_path),
            diagnostic.reason,
            diagnostic.suggested_fix,
        )
    }));
    report.extend(api.unsupported_public_items.iter().map(|item| {
        ValidationDiagnostic::error(
            ValidationCode::UnsupportedGenericItem,
            api.crate_name.clone(),
            Some(item.item_path.clone()),
            format!(
                "{} `{}` is unsupported: {}",
                item.item_kind, item.item_path, item.reason
            ),
            item.suggested_fix.clone(),
        )
    }));
    report.extend(backend_readiness_diagnostics(api, bridged_trait_names));
    report
}

fn backend_readiness_diagnostics(api: &ApiSurface, bridged_trait_names: &AHashSet<&str>) -> Vec<ValidationDiagnostic> {
    let known_names = known_type_names(api);
    // Trait method signatures may reference per-language-excluded types because backend
    // `trait_bridge.rs` substitutes them with a JSON opaque carrier at code-emit. Free
    // functions, fields, and inherent-method signatures don't get that substitution.
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
            // Error types are never opaque — they are always transparent error enums/structs.
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
    // A &mut self method without a trait source cannot be straightforwardly delegated to a
    // free function by the backend's default path. However, when the *receiver* type is itself
    // opaque, the backend already handles this via the direct inner-call path
    // (e.g. `self.inner.method()`), so returning another opaque handle from such a method
    // is a well-supported pattern. Only emit the diagnostic when the receiver is non-opaque
    // and therefore has no inner-call path available.
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
    // Service constructors and configurators are static-like factory methods — the receiver
    // type (the service builder) is not an opaque handle.
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
    // Handler contracts are trait dispatch methods — no opaque receiver context applies.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        ApiSurface, EntrypointDef, EntrypointKind, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef,
        HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeDef, TypeRef, UnsupportedPublicItem,
    };

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
        // Parser::parse(&mut self) -> Option<Tree> is the canonical pattern: an opaque type's
        // &mut self method returns another opaque handle. All backends handle this via the
        // direct inner-call path (e.g. `self.inner.parse()`), so no stub is required.
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
        // A &mut self method on a *non-opaque* type returning an opaque handle cannot be
        // delegated by the backend — there is no inner-call path — so this must still error.
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
}
