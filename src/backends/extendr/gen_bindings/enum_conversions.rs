use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::helpers::is_tuple_variant;
use crate::codegen::generators::type_paths::resolve_type_path;
use crate::core::ir::{EnumDef, EnumVariant};
use std::collections::HashMap;

/// A variant behind `#[cfg(...)]` needs different treatment depending on who owns the enum
/// (see [`is_host_owned_rust_path`], the same authority `codegen::cfg::collect_cfg_gates` uses
/// to decide which cfgs get a Cargo feature forwarded for them). A host-owned gated variant
/// keeps its match arm under a matching `#[cfg(...)]` guard -- forwarding already declared that
/// feature, so the gate is valid. A variant merged in from a foreign `[[crates.source_crates]]`
/// crate carries that crate's own cfg gate, which this generated crate's `Cargo.toml` never
/// declares as a feature; re-emitting it verbatim is an `unexpected cfg condition value` error,
/// so the arm is dropped entirely instead -- named and counted via `tracing::debug!`, not
/// silently; `codegen::foreign_cfg_variants` raises the same fact to WARN once for the whole run
/// rather than once per backend and direction -- mirroring `codegen::conversions::enums::emit_cfg_gated_arm` and
/// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper`. ~keep
fn emit_cfg_gated_arm(
    enum_def: &EnumDef,
    variant: &EnumVariant,
    is_host_enum: bool,
    pattern: &str,
    expression: &str,
    direction: &str,
) -> Option<String> {
    if variant.cfg.is_some() && !is_host_enum {
        tracing::debug!(
            enum_name = %enum_def.name,
            enum_rust_path = %enum_def.rust_path,
            variant_name = %variant.name,
            cfg = variant.cfg.as_deref().unwrap_or_default(),
            direction = direction,
            "dropping extendr enum conversion match arm for a foreign-crate variant behind a \
             #[cfg(...)] this generated crate cannot declare as a Cargo feature; the variant is \
             unreachable from this conversion"
        );
        return None;
    }
    Some(crate::backends::extendr::template_env::render(
        format!("enum_from_{direction}_arm.jinja").as_str(),
        crate::alef_context! {
            pattern => pattern,
            expression => expression,
            cfg => variant.cfg.as_deref(),
        },
    ))
}

pub(super) fn gen_from_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    type_paths: &HashMap<String, String>,
    configured_features: Option<&[String]>,
) -> String {
    let core_path = resolve_type_path(&enum_def.name, core_import, type_paths);
    let binding_name = enum_def.name.as_str();
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let arms: Vec<String> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let pattern = binding_pattern(binding_name, variant);
            let expression = core_expression(variant);
            emit_cfg_gated_arm(
                enum_def,
                variant,
                is_host_enum,
                &pattern,
                &expression,
                "binding_to_core",
            )
        })
        .collect();

    // This match is over the BINDING enum extendr itself declares (`generators::gen_enum`, used
    // whenever this bespoke module is reached). That declaration now drops a FOREIGN cfg-gated
    // variant this binding's own `configured_features` proves unreachable -- the same verdict
    // `enum_variant_declaration` computes -- so `declaration_may_drop_variant` is `true` here,
    // matching the core->binding direction below. See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    let catch_all = catch_all(enum_def, is_host_enum, configured_features, true).then(|| {
        crate::backends::extendr::template_env::render(
            "enum_from_binding_to_core_catch_all.jinja",
            crate::alef_context! {},
        )
    });

    crate::backends::extendr::template_env::render(
        "enum_from_binding_to_core_impl.jinja",
        crate::alef_context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            catch_all => catch_all,
        },
    )
}

pub(super) fn gen_from_core_to_binding(
    enum_def: &EnumDef,
    core_import: &str,
    type_paths: &HashMap<String, String>,
    configured_features: Option<&[String]>,
) -> String {
    let core_path = resolve_type_path(&enum_def.name, core_import, type_paths);
    let binding_name = enum_def.name.as_str();
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let arms: Vec<String> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let pattern = core_pattern(&core_path, variant);
            let expression = binding_expression(variant);
            emit_cfg_gated_arm(
                enum_def,
                variant,
                is_host_enum,
                &pattern,
                &expression,
                "core_to_binding",
            )
        })
        .collect();

    // This match is over the real CORE type -- a shape extendr does not declare and cannot
    // influence, so `configured_features`' proof is already the complete answer and
    // `declaration_may_drop_variant` is `true` here too, matching the binding->core direction
    // above now that `generators::gen_enum` also drops a proven-unreachable foreign variant. ~keep
    let catch_all = catch_all(enum_def, is_host_enum, configured_features, true).then(|| {
        crate::backends::extendr::template_env::render(
            "enum_from_core_to_binding_catch_all.jinja",
            crate::alef_context! {},
        )
    });

    crate::backends::extendr::template_env::render(
        "enum_from_core_to_binding_impl.jinja",
        crate::alef_context! {
            binding_name => binding_name,
            core_path => core_path,
            arms => arms,
            catch_all => catch_all,
        },
    )
}

fn catch_all(
    enum_def: &EnumDef,
    is_host_enum: bool,
    configured_features: Option<&[String]>,
    declaration_may_drop_variant: bool,
) -> bool {
    let has_excluded_variants = !enum_def.excluded_variants.is_empty();
    let core_has_struct_variants = enum_def
        .variants
        .iter()
        .any(|variant| !variant.fields.is_empty() && !variant.is_tuple);
    let has_any_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    // A cfg-gated variant's arm is dropped entirely only when foreign-owned (see
    // `emit_cfg_gated_arm`) -- that really does leave the match non-exhaustive, UNLESS this
    // binding's own configured feature set proves the foreign variant unreachable, in which case
    // the gap closes and no catch-all is needed for it. A host-owned gated variant keeps its arm
    // under the identical `#[cfg(...)]` guard as the variant itself, so the two always compile in
    // or out together and the match stays exhaustive either way; triggering the catch-all on that
    // case alone made it unreachable under `-D warnings` the moment the gating feature was active
    // (the default once cfg features are forwarded, alef #464). That host/foreign distinction,
    // now refined by the configured feature set, is delegated to
    // `codegen::conversions::enum_conversion_needs_catch_all_for_features` rather than restated
    // here, so the rule cannot drift out of step with the other Rust-emitting backends (alef
    // #547). The two extra conditions below are extendr-specific and stay local. ~keep
    crate::codegen::conversions::enum_conversion_needs_catch_all_for_features(
        enum_def,
        is_host_enum,
        has_excluded_variants,
        configured_features,
        declaration_may_drop_variant,
    ) || core_has_struct_variants
        || has_any_data_variants
}

fn binding_pattern(binding_name: &str, variant: &EnumVariant) -> String {
    format!("{binding_name}::{}", variant.name)
}

fn core_pattern(core_path: &str, variant: &EnumVariant) -> String {
    if variant.fields.is_empty() {
        format!("{core_path}::{}", variant.name)
    } else if is_tuple_variant(&variant.fields) {
        format!("{core_path}::{}(..)", variant.name)
    } else {
        format!("{core_path}::{} {{ .. }}", variant.name)
    }
}

fn core_expression(variant: &EnumVariant) -> String {
    if variant.fields.is_empty() {
        format!("Self::{}", variant.name)
    } else if is_tuple_variant(&variant.fields) {
        let defaults = variant
            .fields
            .iter()
            .map(|_| "Default::default()")
            .collect::<Vec<_>>()
            .join(", ");
        format!("Self::{}({defaults})", variant.name)
    } else {
        let defaults = variant
            .fields
            .iter()
            .map(|field| format!("{}: Default::default()", field.name))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Self::{} {{ {defaults} }}", variant.name)
    }
}

fn binding_expression(variant: &EnumVariant) -> String {
    format!("Self::{}", variant.name)
}
