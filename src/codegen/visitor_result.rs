use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, EnumDef, TypeRef};

#[derive(Debug, Clone)]
pub(crate) struct VisitorResultVariant {
    pub name: String,
    pub wire_name: String,
    pub code: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct VisitorResultMetadata {
    pub default_variant: VisitorResultVariant,
    pub unit_variants: Vec<VisitorResultVariant>,
    pub string_payload_variants: Vec<VisitorResultVariant>,
}

pub(crate) fn visitor_result_metadata(
    api: &ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
) -> Option<VisitorResultMetadata> {
    let result_type = bridge_cfg.result_type.as_deref()?;
    let enum_def = api.enums.iter().find(|enum_def| enum_def.name == result_type)?;
    visitor_result_metadata_from_enum_checked(enum_def, &bridge_cfg.trait_name).ok()
}

pub(crate) fn required_visitor_result_metadata(
    api: &ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
) -> anyhow::Result<VisitorResultMetadata> {
    let result_type = bridge_cfg.result_type.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "trait bridge `{}` must configure result_type for visitor result conversion",
            bridge_cfg.trait_name
        )
    })?;
    let enum_def = api.enums.iter().find(|enum_def| enum_def.name == result_type).ok_or_else(|| {
        anyhow::anyhow!(
            "trait bridge `{}` configures result_type `{result_type}`, but no matching enum exists in the API surface",
            bridge_cfg.trait_name
        )
    })?;
    visitor_result_metadata_from_enum_checked(enum_def, &bridge_cfg.trait_name)
}

pub(crate) fn visitor_result_metadata_from_enum_checked(
    enum_def: &EnumDef,
    trait_name: &str,
) -> anyhow::Result<VisitorResultMetadata> {
    let unit_variants = enum_def
        .variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| variant.fields.is_empty() && !variant.originally_had_data_fields)
        .map(|(code, variant)| VisitorResultVariant {
            name: variant.name.clone(),
            wire_name: crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            ),
            code,
        })
        .collect::<Vec<_>>();

    let default_unit_variants = enum_def
        .variants
        .iter()
        .filter(|variant| variant.is_default && variant.fields.is_empty() && !variant.originally_had_data_fields)
        .collect::<Vec<_>>();

    let default_variant = match default_unit_variants.as_slice() {
        [variant] => VisitorResultVariant {
            name: variant.name.clone(),
            wire_name: crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            ),
            code: enum_def
                .variants
                .iter()
                .position(|candidate| candidate.name == variant.name)
                .unwrap_or_default(),
        },
        [] if unit_variants.len() == 1 => unit_variants[0].clone(),
        [] => anyhow::bail!(
            "trait bridge `{trait_name}` result_type `{}` must have exactly one #[default] unit variant, \
             or exactly one unit variant, to derive the default callback result",
            enum_def.name
        ),
        _ => anyhow::bail!(
            "trait bridge `{trait_name}` result_type `{}` has multiple #[default] unit variants; expected exactly one",
            enum_def.name
        ),
    };

    let string_payload_variants = enum_def
        .variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| variant.fields.len() == 1 && matches!(variant.fields[0].ty, TypeRef::String))
        .map(|(code, variant)| VisitorResultVariant {
            name: variant.name.clone(),
            wire_name: crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            ),
            code,
        })
        .collect();

    Ok(VisitorResultMetadata {
        default_variant,
        unit_variants,
        string_payload_variants,
    })
}

pub(crate) fn default_result_expr(return_type: &str, metadata: &VisitorResultMetadata) -> String {
    format!("{return_type}::{}", metadata.default_variant.name)
}

pub(crate) fn unknown_string_result_expr(
    return_type: &str,
    metadata: &VisitorResultMetadata,
    value_expr: &str,
) -> String {
    match metadata.string_payload_variants.as_slice() {
        [] => default_result_expr(return_type, metadata),
        [variant] => format!("{return_type}::{}({value_expr})", variant.name),
        variants => {
            let chosen = variants.iter().find(|v| v.name == "Custom").unwrap_or(&variants[0]);
            format!("{return_type}::{}({value_expr})", chosen.name)
        }
    }
}

pub(crate) fn variant_contexts(variants: &[VisitorResultVariant]) -> Vec<minijinja::Value> {
    variants
        .iter()
        .map(|variant| {
            minijinja::context! {
                name => variant.name.clone(),
                wire_name => variant.wire_name.clone(),
                code => variant.code,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(name: &str) -> VisitorResultVariant {
        VisitorResultVariant {
            name: name.to_string(),
            wire_name: name.to_string(),
            code: 0,
        }
    }

    fn metadata_with(string_payloads: Vec<VisitorResultVariant>) -> VisitorResultMetadata {
        VisitorResultMetadata {
            default_variant: variant("Continue"),
            unit_variants: vec![variant("Continue")],
            string_payload_variants: string_payloads,
        }
    }

    /// Two-payload case (`Custom`, `Error`) routes bare strings to `Custom` —
    /// the documented output channel; `Error` requires the explicit dict form.
    /// Regression for markdown-visitor v3.6.7 Python visitor tests where bare-string return
    /// was silently dropped to `Continue` (default).
    #[test]
    fn unknown_string_result_expr_prefers_custom_when_multiple_string_payloads() {
        let metadata = metadata_with(vec![variant("Custom"), variant("Error")]);
        assert_eq!(unknown_string_result_expr("VR", &metadata, "s"), "VR::Custom(s)");
    }

    #[test]
    fn unknown_string_result_expr_single_string_payload_uses_it() {
        let metadata = metadata_with(vec![variant("Replace")]);
        assert_eq!(unknown_string_result_expr("VR", &metadata, "s"), "VR::Replace(s)");
    }

    #[test]
    fn unknown_string_result_expr_no_string_payload_falls_back_to_default() {
        let metadata = metadata_with(vec![]);
        assert_eq!(unknown_string_result_expr("VR", &metadata, "s"), "VR::Continue");
    }

    #[test]
    fn unknown_string_result_expr_multiple_without_custom_uses_first() {
        let metadata = metadata_with(vec![variant("Replace"), variant("Warning")]);
        assert_eq!(unknown_string_result_expr("VR", &metadata, "s"), "VR::Replace(s)");
    }
}
