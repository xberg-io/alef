//! Central validation diagnostics for generation inputs and backend readiness.
//!
//! Validation should happen between config/extraction resolution and backend
//! emission. This module provides the shared diagnostic shape; individual
//! validators can stay close to the subsystem they understand while returning
//! structured reports here.

use crate::core::config::Language;
use crate::core::ir::ApiSurface;
use crate::extract::validation::sanitized_public_api_diagnostics;
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

/// Validate the extracted public API surface before backend generation.
pub fn validate_api_surface(api: &ApiSurface) -> ValidationReport {
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
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

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
}
