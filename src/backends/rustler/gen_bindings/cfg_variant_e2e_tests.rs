//! End-to-end regression coverage for alef #544: a FOREIGN (dependency-owned) cfg-gated enum
//! variant run through the REAL `RustlerBackend::generate_bindings` path, not a direct
//! `conversions::gen_enum_from_*_cfg` / `types::gen_rustler_flat_data_enum_from_core` call.
//! Mirrors `backends::wasm::gen_bindings::cfg_variant_e2e_tests`, the pattern task #538
//! established for wasm.
//!
//! Rustler has TWO independent enum-conversion generators, and both carried this defect:
//! - the shared `codegen::conversions::gen_enum_from_core_to_binding_cfg` path (tagged/fieldless
//!   enums), reached via `rustler_conv_config` in `native.rs`, which never set
//!   `configured_features`;
//! - the bespoke `types::gen_rustler_flat_data_enum_from_core` (flat-struct data enums), which
//!   never took `configured_features` as an argument at all and computed its own
//!   `has_cfg_variants` locally -- the "second authority" this task's brief called out.
//!
//! Both get a negative/positive-control pair below.

use super::RustlerBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeRef};

fn rustler_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"elixir\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.elixir]\napp_name = \"test_lib\"\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

fn core_to_binding_conversion<'a>(lib_rs: &'a str, marker: &str) -> &'a str {
    let start = lib_rs
        .find(marker)
        .unwrap_or_else(|| panic!("generated crate must contain conversion impl starting with:\n{marker}"));
    let end = lib_rs[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("conversion impl must close");
    &lib_rs[start..end]
}

/// A different first path segment than the crate's own `core_import` ("test_lib") is what
/// `is_host_owned_rust_path` reads to classify this enum -- and every one of its cfg-gated
/// variants -- as FOREIGN. Fieldless variants only, so `is_flat_data_enum` is false and this
/// enum goes through the SHARED `codegen::conversions::gen_enum_from_core_to_binding_cfg` path
/// (`rustler_conv_config` in `native.rs`), not the bespoke flat-data-enum generator. ~keep
fn foreign_cfg_tagged_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "RoutingStrategy".to_string(),
            rust_path: "dep_crate::RoutingStrategy".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Primary".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Extra".to_string(),
                    cfg: Some(r#"feature = "extra-tier""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Like `foreign_cfg_tagged_enum_api`, but also declares a function taking the enum as a
/// PARAMETER (not just a return type) -- `impl From<BindingEnum> for CoreType` is only generated
/// for types `input_type_names` finds among parameter types, so the plain
/// `foreign_cfg_tagged_enum_api` fixture (return-type-only, implicit via no `functions` at all)
/// never exercises the binding->core direction at all. ~keep
fn foreign_cfg_tagged_enum_api_with_param_function() -> ApiSurface {
    let mut api = foreign_cfg_tagged_enum_api();
    api.functions.push(FunctionDef {
        name: "set_routing_strategy".to_string(),
        rust_path: "test_lib::set_routing_strategy".to_string(),
        params: vec![ParamDef {
            name: "strategy".to_string(),
            ty: TypeRef::Named("RoutingStrategy".to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::Unit,
        ..Default::default()
    });
    api
}

/// Same foreign-ownership shape as `foreign_cfg_tagged_enum_api`, but the non-cfg variant
/// carries a single TUPLE field, so every data-carrying variant is tuple-shaped and
/// `is_flat_data_enum` is true -- this enum routes through the bespoke
/// `types::gen_rustler_flat_data_enum_from_core` generator instead of the shared
/// `ConversionConfig`-driven path. ~keep
fn foreign_cfg_flat_data_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "PayloadKind".to_string(),
            rust_path: "dep_crate::PayloadKind".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Primary".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    is_tuple: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Extra".to_string(),
                    cfg: Some(r#"feature = "extra-tier""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// alef #544: `rustler_conv_config` (the `ConversionConfig` construction site feeding the shared
/// `gen_enum_from_core_to_binding_cfg`) never set `configured_features`, so
/// `codegen::conversions::enums::has_unresolved_foreign_cfg_variants` always saw `None` and had
/// to assume a foreign cfg-gated variant might still exist -- emitting a trailing
/// `_ => Default::default()` catch-all that is unreachable (a `cargo clippy -D warnings` failure)
/// once the binding's own feature set actually proves the foreign variant can never appear.
#[test]
fn generate_bindings_omits_unreachable_catch_all_for_tagged_enum_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_tagged_enum_api();
    let config = rustler_config_with_feature(None);
    let files = RustlerBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs, "impl From<dep_crate::RoutingStrategy> for RoutingStrategy {");

    assert!(
        !conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant proven unreachable by this binding's own configured feature \
         set must not leave behind an unreachable catch-all (a cargo clippy -D warnings failure), \
         got:\n{conversion}"
    );
}

/// Positive control for the test above.
#[test]
fn generate_bindings_keeps_catch_all_for_tagged_enum_foreign_variant_not_proven_unreachable_end_to_end() {
    let api = foreign_cfg_tagged_enum_api();
    let config = rustler_config_with_feature(Some("extra-tier"));
    let files = RustlerBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs, "impl From<dep_crate::RoutingStrategy> for RoutingStrategy {");

    assert!(
        conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable must keep the catch-all so the \
         match stays exhaustive, got:\n{conversion}"
    );
}

/// THE `unreachable_patterns` REGRESSION reproduced end to end against a downstream crate's
/// real `TierStrategy` (a downstream CI run 33428741012, `packages/elixir/native/<crate>_nif/src/lib.rs`):
/// Rustler's own unit/tagged enum declaration (`types::gen_enum`'s `declared_variants`, fed by
/// [`crate::codegen::conversions::enum_variant_declaration`]) DOES drop a FOREIGN cfg-gated
/// variant this binding's own configured feature set proves unreachable -- since alef
/// `589d67e8ab5a1db8c4427e20f4be0046e51f03bb`, `pub enum RoutingStrategy { Primary }` never
/// declares `Extra` here at all. `rustler_conv_config` never set
/// `declaration_drops_unreachable_foreign_variants` to match, so
/// `has_unresolved_foreign_cfg_variants` still assumed the declaration kept `Extra`
/// unconditionally and kept the catch-all -- unreachable the moment the 1-variant match is
/// already exhaustive: a `cargo clippy -D warnings` failure identical to the one that blocked
/// every Elixir e2e test from ever running. ~keep
#[test]
fn generate_bindings_omits_binding_to_core_catch_all_for_tagged_enum_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_tagged_enum_api_with_param_function();
    let config = rustler_config_with_feature(None);
    let files = RustlerBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs, "impl From<RoutingStrategy> for dep_crate::RoutingStrategy {");

    assert!(
        !conversion.contains("_ => Default::default(),"),
        "Rustler's own declaration drops a foreign cfg-gated variant proven unreachable by this \
         binding's own configured feature set, so the binding->core match it declares is already \
         exhaustive without a catch-all -- keeping one is unreachable_patterns under \
         -D warnings, got:\n{conversion}"
    );
}

/// Positive control for the test above: when the gating feature IS configured, Rustler's
/// declaration keeps `Extra` (unconditionally, with no per-variant `#[cfg(...)]` it could attach
/// -- `enum_variant_declaration` never resolves a `Keep` to carry a cfg for a foreign variant),
/// while `emit_cfg_gated_arm` still unconditionally drops any foreign cfg-gated arm regardless of
/// features. The match is therefore genuinely short one arm and the catch-all must stay, or
/// omitting it trades the unreachable-pattern bug for `error[E0004]: non-exhaustive patterns`.
/// ~keep
#[test]
fn generate_bindings_keeps_binding_to_core_catch_all_for_tagged_enum_foreign_variant_not_proven_unreachable_end_to_end()
{
    let api = foreign_cfg_tagged_enum_api_with_param_function();
    let config = rustler_config_with_feature(Some("extra-tier"));
    let files = RustlerBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs, "impl From<RoutingStrategy> for dep_crate::RoutingStrategy {");

    assert!(
        conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable is still declared \
         unconditionally while its conversion arm is still dropped, so the catch-all must stay \
         to cover it -- omitting it is error[E0004]: non-exhaustive patterns, got:\n{conversion}"
    );
}

/// alef #544, the flat-data-enum half (the "second authority" this task's brief called out):
/// `types::gen_rustler_flat_data_enum_from_core` never took `configured_features` at all and
/// computed its own `has_cfg_variants` from "does any variant carry a cfg," host-owned or
/// foreign, ignoring the binding's own feature set entirely -- the same unreachable catch-all
/// defect, reached through the bespoke generator instead of the shared one.
#[test]
fn generate_bindings_omits_unreachable_catch_all_for_flat_data_enum_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_flat_data_enum_api();
    let config = rustler_config_with_feature(None);
    let files = RustlerBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs, "impl From<dep_crate::PayloadKind> for PayloadKind {");

    assert!(
        !conversion.contains("_ => Self::default(),"),
        "a foreign cfg-gated variant proven unreachable by this binding's own configured feature \
         set must not leave behind an unreachable catch-all (a cargo clippy -D warnings failure), \
         got:\n{conversion}"
    );
}

/// Positive control for the test above.
#[test]
fn generate_bindings_keeps_catch_all_for_flat_data_enum_foreign_variant_not_proven_unreachable_end_to_end() {
    let api = foreign_cfg_flat_data_enum_api();
    let config = rustler_config_with_feature(Some("extra-tier"));
    let files = RustlerBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs, "impl From<dep_crate::PayloadKind> for PayloadKind {");

    assert!(
        conversion.contains("_ => Self::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable must keep the catch-all so the \
         match stays exhaustive, got:\n{conversion}"
    );
}
