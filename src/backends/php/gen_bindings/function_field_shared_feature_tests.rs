//! Regression coverage for alef-issue #451: a cfg-gated top-level function removed its feature
//! from the PHP crate's `[features]` table even when a struct field gated on the SAME feature
//! still needed it declared.
//!
//! Mechanism: `php_declared_features` (`scaffold/languages/php.rs`) started every feature name
//! any cfg gate in the surface referenced, then unconditionally removed every name a top-level
//! function's cfg referenced (`php_function_referenced_feature_names`) -- correct in isolation,
//! because PHP never gates a function by cfg (it is always emitted into the facade; see
//! `rust_bindings.rs::generate_bindings`'s doc comment). But when a struct field's cfg named that
//! same feature, the field's gate is copied onto the generated `From` impl
//! (`ConversionConfig::restrict_field_gate`) and narrowed against that same declared set. With the
//! feature stripped, `restrict_cfg_gate_to_declared` found nothing declared for it
//! (`DeclaredCfgGate::Unreachable`) and fell back to re-emitting the original, now-undeclared
//! gate -- `unexpected_cfg_condition_value` under `-D warnings`, and once rustc evaluated the gate
//! false, a struct literal missing the field (E0063), because the binding struct declares the
//! field unconditionally (it is a trait-bridge "never skip" field here, same as the real report).
//!
//! Fixture names are neutral (`alpha`, `Config`, `alpha_reader`) -- the real report used a
//! consumer's actual crate/feature/function names, which must not appear in alef's own fixtures.

use super::PhpBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, TypeDef, TypeRef};

const FEATURE: &str = "alpha";

fn php_config_with_feature(feature: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        "[workspace]\nlanguages = [\"php\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.php]\nfeatures = [\"{feature}\"]\n"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("valid config");
    cfg.resolve().expect("resolve config").remove(0)
}

/// A HOST-owned, non-defaultable struct with a field gated on `FEATURE`, plus a top-level
/// function ALSO gated on `FEATURE` -- the exact shape from the report: the field needs `FEATURE`
/// declared for its copied `#[cfg(...)]` gate to compile, while the function's own cfg is a
/// reason `php_function_referenced_feature_names` wants to strip that same name.
fn shared_feature_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            fields: vec![
                FieldDef {
                    name: "always_on".to_string(),
                    ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                    ..Default::default()
                },
                FieldDef {
                    name: "alpha_option".to_string(),
                    ty: TypeRef::String,
                    cfg: Some(format!(r#"feature = "{FEATURE}""#)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "alpha_reader".to_string(),
            rust_path: "test_lib::alpha_reader".to_string(),
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            cfg: Some(format!(r#"feature = "{FEATURE}""#)),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn cargo_toml_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("scaffold_php_cargo must emit Cargo.toml")
        .content
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

/// Isolated unit coverage: `php_declared_features` must keep a name referenced by BOTH a
/// function and a field.
#[test]
fn php_declared_features_keeps_a_name_a_field_still_needs() {
    let api = shared_feature_api();
    let declared =
        crate::scaffold::languages::php::php_declared_features(&api, &[], &[], &std::collections::BTreeSet::new());
    assert!(
        declared.contains(FEATURE),
        "a feature name a struct field's cfg references must stay declared even when a \
         top-level function also references it; got {declared:?}"
    );
}

#[test]
fn php_declared_features_keeps_a_function_only_public_name() {
    let mut api = shared_feature_api();
    api.types.clear();
    let declared =
        crate::scaffold::languages::php::php_declared_features(&api, &[], &[], &std::collections::BTreeSet::new());
    assert!(
        declared.contains(FEATURE),
        "a public feature referenced by a top-level function must remain part of the wrapper \
         crate's Cargo feature contract; got {declared:?}"
    );
}

/// End-to-end: the invariant the whole mechanism must uphold. If the generated `From` impl
/// carries a `#[cfg(feature = "X")]` gate on a field, "X" must appear in the same crate's
/// generated `[features]` table -- otherwise the gate names an undeclared feature and rustc
/// evaluates it false, dropping the field from the struct literal it is required to fully
/// initialize (E0063), exactly the failure alef-issue #451 reports.
#[test]
fn generated_field_cfg_gate_names_a_feature_declared_in_the_same_crate() {
    let api = shared_feature_api();
    let config = php_config_with_feature(FEATURE);

    let cargo_files =
        crate::scaffold::languages::php::scaffold_php_cargo(&api, &config).expect("scaffold_php_cargo ok");
    let cargo_toml = cargo_toml_content(&cargo_files);

    let bindings = PhpBackend
        .generate_bindings(&api, &config)
        .expect("generate_bindings ok");
    let lib_rs = lib_rs_content(&bindings);

    let expected_gate = format!(r#"#[cfg(feature = "{FEATURE}")]"#);
    assert!(
        lib_rs.contains(&expected_gate),
        "expected the field conversion to still carry its cfg gate:\n{lib_rs}"
    );

    let declared_line = format!(r#"{FEATURE} = ["test-lib/{FEATURE}"]"#);
    assert!(
        cargo_toml.contains(&declared_line),
        "a struct field's cfg gate named `{FEATURE}` in the generated conversion, so the PHP \
         crate's own [features] table must declare it (`{declared_line}`); got:\n{cargo_toml}"
    );

    let default_line = cargo_toml
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert!(
        default_line.contains(FEATURE),
        "`{FEATURE}` is configured for this binding, so it must also be enabled by default; \
         got: {default_line}"
    );
}
