//! Central validation diagnostics for generation inputs and backend readiness.
//!
//! Validation should happen between config/extraction resolution and backend
//! emission. This module provides the shared diagnostic shape; individual
//! validators can stay close to the subsystem they understand while returning
//! structured reports here.

use crate::core::config::Language;
use crate::core::ir::ApiSurface;
use crate::extract::validation::sanitized_public_api_diagnostics;
use ahash::AHashSet;

mod readiness;

use readiness::backend_readiness_diagnostics;
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
    UnconsumedConfig,
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
            Self::UnconsumedConfig => f.write_str("unconsumed_config"),
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

#[cfg(test)]
mod tests;
