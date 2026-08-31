//! Flag an `options_field` trait bridge whose carrier field was excluded from bindings.
//!
//! `bind_via = "options_field"` (see [`crate::core::config::TraitBridgeConfig::bind_via`]) tells
//! every backend that emits this bridge to attach the callback/visitor handle to
//! `options.<field>` on a configured `options_type` before the underlying core call. That field
//! is a real struct field the extractor tracks like any other -- including whether it carries
//! `#[cfg_attr(alef, alef(skip))]`. The options-mirror struct emitter already honors a skipped
//! field (it simply omits it from the generated mirror struct), but the bridge emitters do not:
//! they unconditionally read `options.<field>` because their only job is to attach the bridge to
//! *some* options field, and they have no reason to expect that field is missing. Skipping the
//! field therefore does not shrink the binding's API -- it breaks compilation, because the
//! bridge keeps reading a field the mirror struct emitter just removed.
//!
//! This check runs once at extraction time (language-independent, so it fires regardless of
//! which backends a given run targets) and refuses the configuration outright rather than
//! letting any backend generate code that cannot compile.

use super::{ValidationCode, ValidationDiagnostic};
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::ApiSurface;

/// Collect one fatal diagnostic per `options_field` trait bridge whose configured carrier field
/// exists on `options_type` but is `binding_excluded` (typically via `alef(skip)`).
///
/// Silently returns no diagnostic when the bridge or its carrier field cannot be resolved at
/// all (unknown `options_type`, unset `options_field`/`param_name`, or a field name that never
/// existed) -- those are different defects, already surfaced by other validation (unknown named
/// types, missing config) or by the backend that fails to find the field during codegen.
pub fn trait_bridge_carrier_diagnostics(
    api: &ApiSurface,
    trait_bridges: &[TraitBridgeConfig],
) -> Vec<ValidationDiagnostic> {
    trait_bridges
        .iter()
        .filter_map(|bridge| excluded_carrier_diagnostic(api, bridge))
        .collect()
}

fn excluded_carrier_diagnostic(api: &ApiSurface, bridge: &TraitBridgeConfig) -> Option<ValidationDiagnostic> {
    if bridge.bind_via != BridgeBinding::OptionsField {
        return None;
    }
    let options_type_name = bridge.options_type.as_deref()?;
    let field_name = bridge.resolved_options_field()?;
    let typ = api.types.iter().find(|t| t.name == options_type_name)?;
    let field = typ.fields.iter().find(|f| f.name == field_name)?;
    if !field.binding_excluded {
        return None;
    }

    let exclusion_reason = field.binding_exclusion_reason.as_deref().unwrap_or("alef(skip)");
    Some(ValidationDiagnostic::error(
        ValidationCode::TraitBridgeCarrierUnavailable,
        api.crate_name.clone(),
        Some(format!("field {options_type_name}.{field_name}")),
        format!(
            "trait bridge `{}` binds via `options_field` to `{options_type_name}.{field_name}`, but \
             that field is excluded from bindings ({exclusion_reason}); every backend that emits this \
             bridge reads `options.{field_name}` off the generated `{options_type_name}` mirror struct, \
             which no longer has that field once it is skipped -- the binding would have no way to \
             attach a visitor/callback at all, and generated code that still reads the field would fail \
             to compile",
            bridge.trait_name,
        ),
        format!(
            "remove `alef(skip)` from `{options_type_name}.{field_name}`, or remove/reconfigure the \
             `[[crates.trait_bridges]]` entry for `{}` -- this bridge cannot function without that field",
            bridge.trait_name
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ValidationSeverity;
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    fn options_type(field_binding_excluded: bool) -> TypeDef {
        TypeDef {
            name: "ConversionOptions".to_string(),
            rust_path: "sample_lib::ConversionOptions".to_string(),
            fields: vec![FieldDef {
                name: "visitor".to_string(),
                ty: TypeRef::String,
                binding_excluded: field_binding_excluded,
                binding_exclusion_reason: field_binding_excluded.then(|| "alef(skip)".to_string()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }
    }

    fn options_field_bridge() -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            param_name: Some("visitor".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some("ConversionOptions".to_string()),
            ..TraitBridgeConfig::default()
        }
    }

    #[test]
    fn skipped_carrier_field_is_fatal() {
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            types: vec![options_type(true)],
            ..ApiSurface::default()
        };

        let diagnostics = trait_bridge_carrier_diagnostics(&api, &[options_field_bridge()]);

        assert_eq!(diagnostics.len(), 1, "exactly one diagnostic for the skipped carrier field");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, ValidationSeverity::Error);
        assert_eq!(diagnostic.code, ValidationCode::TraitBridgeCarrierUnavailable);
        assert_eq!(diagnostic.item_path.as_deref(), Some("field ConversionOptions.visitor"));
        assert!(
            diagnostic.reason.contains("HtmlVisitor") && diagnostic.reason.contains("ConversionOptions.visitor"),
            "reason must name both the bridge and the field: {}",
            diagnostic.reason
        );
    }

    #[test]
    fn non_excluded_carrier_field_produces_no_diagnostic() {
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            types: vec![options_type(false)],
            ..ApiSurface::default()
        };

        let diagnostics = trait_bridge_carrier_diagnostics(&api, &[options_field_bridge()]);

        assert!(
            diagnostics.is_empty(),
            "a carrier field that survived extraction must not be flagged"
        );
    }

    #[test]
    fn function_param_bridge_is_never_checked() {
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            types: vec![options_type(true)],
            ..ApiSurface::default()
        };
        let mut bridge = options_field_bridge();
        bridge.bind_via = BridgeBinding::FunctionParam;

        let diagnostics = trait_bridge_carrier_diagnostics(&api, &[bridge]);

        assert!(
            diagnostics.is_empty(),
            "function_param binding never reads an options-struct field, so skip is irrelevant"
        );
    }

    #[test]
    fn missing_carrier_field_produces_no_diagnostic() {
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            types: vec![TypeDef {
                name: "ConversionOptions".to_string(),
                rust_path: "sample_lib::ConversionOptions".to_string(),
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };

        let diagnostics = trait_bridge_carrier_diagnostics(&api, &[options_field_bridge()]);

        assert!(
            diagnostics.is_empty(),
            "a field name that never existed is a different defect, surfaced elsewhere"
        );
    }
}
