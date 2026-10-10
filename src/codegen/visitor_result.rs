use crate::codegen::cfg::is_host_owned_rust_path;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, EnumDef, TypeRef};

#[derive(Debug, Clone)]
pub(crate) struct VisitorResultVariant {
    pub name: String,
    pub wire_name: String,
    pub code: usize,
    /// `#[cfg(...)]` condition on the source variant, when the enum is host-owned (see
    /// `visitor_result_metadata_from_enum_checked`). A foreign-owned variant's cfg is never
    /// carried here -- such a variant is dropped from the metadata entirely, since forwarding a
    /// foreign crate's cfg as `#[cfg(feature = "...")]` in the binding crate references a
    /// feature that crate never declares. Consuming templates that emit a match arm or
    /// conditional block referencing `{{ variant.name }}` must gate it with this when present, or
    /// the generated code references a variant that does not exist in a build excluding that
    /// feature. ~keep
    pub cfg: Option<String>,
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
    visitor_result_metadata_from_enum_checked(enum_def, &bridge_cfg.trait_name, &api.crate_name).ok()
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
    visitor_result_metadata_from_enum_checked(enum_def, &bridge_cfg.trait_name, &api.crate_name)
}

/// Whether `variant` should appear in a visitor-result metadata list, and with which `cfg`.
///
/// A cfg-gated variant on a host-owned enum keeps its gate: the binding crate's `[features]`
/// table forwards that feature (see `codegen::cfg::collect_cfg_gates`), so a generated
/// `#[cfg(...)]` guard referencing it is valid. A cfg-gated variant on a FOREIGN-owned enum
/// (merged in from a `[[crates.source_crates]]` crate) carries that crate's own gate, which this
/// binding crate never declares as a Cargo feature -- forwarding it verbatim would be an
/// `unexpected cfg condition value` error, so such a variant is dropped from the metadata
/// entirely, the same policy `codegen::conversions::enums::emit_cfg_gated_arm` and
/// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper` already apply. ~keep
fn visitor_variant_cfg(
    enum_def: &EnumDef,
    variant: &crate::core::ir::EnumVariant,
    is_host_enum: bool,
) -> Result<Option<String>, ()> {
    let Some(cfg) = variant.cfg.as_deref() else {
        return Ok(None);
    };
    if !is_host_enum {
        tracing::debug!(
            enum_name = %enum_def.name,
            enum_rust_path = %enum_def.rust_path,
            variant_name = %variant.name,
            cfg = cfg,
            "dropping visitor-result variant for a foreign-crate enum variant behind a \
             #[cfg(...)] this binding crate cannot declare as a Cargo feature; the variant is \
             unreachable from generated visitor-bridge code"
        );
        return Err(());
    }
    Ok(Some(cfg.to_string()))
}

pub(crate) fn visitor_result_metadata_from_enum_checked(
    enum_def: &EnumDef,
    trait_name: &str,
    host_crate_name: &str,
) -> anyhow::Result<VisitorResultMetadata> {
    let is_host_enum = is_host_owned_rust_path(host_crate_name, &enum_def.rust_path);
    let unit_variants = enum_def
        .variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| variant.fields.is_empty() && !variant.originally_had_data_fields)
        .filter_map(|(code, variant)| {
            let cfg = visitor_variant_cfg(enum_def, variant, is_host_enum).ok()?;
            Some(VisitorResultVariant {
                name: variant.name.clone(),
                wire_name: crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ),
                code,
                cfg,
            })
        })
        .collect::<Vec<_>>();

    let default_unit_variants = enum_def
        .variants
        .iter()
        .filter(|variant| variant.is_default && variant.fields.is_empty() && !variant.originally_had_data_fields)
        .collect::<Vec<_>>();

    let default_variant = match default_unit_variants.as_slice() {
        // A `#[default]` variant's cfg is deliberately not applied to the drop-or-gate policy
        // `visitor_variant_cfg` uses for `unit_variants`/`string_payload_variants`: the default
        // must always be constructible, since `default_result_expr` is spliced into an
        // unconditional `return` statement, not a separately cfg-gated item, so there is no
        // expression-level way to gate it here. A cfg-gated `#[default]` variant is a pre-existing
        // design hazard this does not newly introduce or attempt to fix. ~keep
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
            cfg: variant.cfg.clone(),
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
        .filter_map(|(code, variant)| {
            let cfg = visitor_variant_cfg(enum_def, variant, is_host_enum).ok()?;
            Some(VisitorResultVariant {
                name: variant.name.clone(),
                wire_name: crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ),
                code,
                cfg,
            })
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
            crate::alef_context! {
                name => variant.name.clone(),
                wire_name => variant.wire_name.clone(),
                code => variant.code,
                // `None` renders falsy in a template `{% if variant.cfg %}` check, matching every
                // other backend's `source_cfg`/`arm_info.cfg` convention. ~keep
                cfg => variant.cfg.clone(),
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
            cfg: None,
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

    fn ir_variant(name: &str, cfg: Option<&str>, string_field: bool) -> crate::core::ir::EnumVariant {
        ir_variant_ex(name, cfg, string_field, false)
    }

    fn ir_variant_ex(
        name: &str,
        cfg: Option<&str>,
        string_field: bool,
        is_default: bool,
    ) -> crate::core::ir::EnumVariant {
        use crate::core::ir::{EnumVariant, FieldDef, TypeRef};
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            is_default,
            fields: if string_field {
                vec![FieldDef {
                    name: "0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }

    fn ir_enum(rust_path: &str, variants: Vec<crate::core::ir::EnumVariant>) -> EnumDef {
        EnumDef {
            name: "VisitorResult".to_string(),
            rust_path: rust_path.to_string(),
            variants,
            ..Default::default()
        }
    }

    /// The regression this task fixes: before `VisitorResultVariant` carried a `cfg` field,
    /// visitor-bridge templates (magnus's `visitor_method.rs.jinja` and siblings) emitted a
    /// match arm/`if let` block referencing every variant unconditionally, regardless of any
    /// `#[cfg(...)]` on the source variant -- an E0599 in a build excluding that feature. A
    /// host-owned cfg-gated variant must now be KEPT with its cfg carried through, in both the
    /// unit-variant and string-payload-variant lists.
    #[test]
    fn host_owned_cfg_variant_is_kept_with_its_cfg_in_both_lists() {
        let en = ir_enum(
            "mylib::VisitorResult",
            vec![
                ir_variant_ex("Continue", None, false, true),
                ir_variant("Thumbnail", Some(r#"feature = "thumbnails""#), false),
                ir_variant("Custom", Some(r#"feature = "thumbnails""#), true),
            ],
        );
        let metadata = visitor_result_metadata_from_enum_checked(&en, "MyTrait", "mylib").expect("valid metadata");

        let kept_unit = metadata
            .unit_variants
            .iter()
            .find(|v| v.name == "Thumbnail")
            .expect("host-owned cfg-gated unit variant must still be present");
        assert_eq!(kept_unit.cfg.as_deref(), Some(r#"feature = "thumbnails""#));

        let kept_payload = metadata
            .string_payload_variants
            .iter()
            .find(|v| v.name == "Custom")
            .expect("host-owned cfg-gated string-payload variant must still be present");
        assert_eq!(kept_payload.cfg.as_deref(), Some(r#"feature = "thumbnails""#));
    }

    /// A variant merged in from a foreign `[[crates.source_crates]]` crate (`rust_path` rooted in
    /// a crate other than the host) carries that crate's own cfg. Forwarding it as `#[cfg(...)]`
    /// names a feature this binding crate never declares -- an `unexpected cfg condition value`
    /// warning -- so the variant must be dropped from the metadata entirely (from both lists),
    /// mirroring `codegen::conversions::enums::emit_cfg_gated_arm` and
    /// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper`.
    #[test]
    fn foreign_owned_cfg_variant_is_dropped_from_both_lists() {
        let en = ir_enum(
            "dep_crate::VisitorResult",
            vec![
                ir_variant("Continue", None, false),
                ir_variant("Testkit", Some(r#"feature = "testkit""#), false),
                ir_variant("Custom", Some(r#"feature = "testkit""#), true),
            ],
        );
        let metadata = visitor_result_metadata_from_enum_checked(&en, "MyTrait", "mylib").expect("valid metadata");

        assert!(
            !metadata.unit_variants.iter().any(|v| v.name == "Testkit"),
            "foreign-crate cfg-gated unit variant must be dropped, got: {:?}",
            metadata.unit_variants
        );
        assert!(
            !metadata.string_payload_variants.iter().any(|v| v.name == "Custom"),
            "foreign-crate cfg-gated string-payload variant must be dropped, got: {:?}",
            metadata.string_payload_variants
        );
    }

    /// Negative control: an ungated enum (no variant carries a `cfg`) keeps every variant with
    /// `cfg: None`.
    #[test]
    fn ungated_enum_keeps_every_variant_with_no_cfg() {
        let en = ir_enum(
            "mylib::VisitorResult",
            vec![
                ir_variant_ex("Continue", None, false, true),
                ir_variant("Skip", None, false),
            ],
        );
        let metadata = visitor_result_metadata_from_enum_checked(&en, "MyTrait", "mylib").expect("valid metadata");

        assert_eq!(metadata.unit_variants.len(), 2, "both ungated variants must be kept");
        assert!(metadata.unit_variants.iter().all(|v| v.cfg.is_none()));
    }
}
