//! Regression coverage for the PHP backend dropping every `#[cfg(feature = "...")]`-gated field
//! from the generated binding struct regardless of whether this binding's own configured feature
//! set actually satisfies the gate.
//!
//! `php_binding_keeps_field` (`types/structs/constructor_init.rs`) drops any field with
//! `field.cfg.is_some()` unless its name appears in `RustBindingConfig::never_skip_cfg_field_names`
//! -- and `rust_bindings.rs` populated that list only from active trait-bridge option fields,
//! never from the binding's actual enabled-feature set. Magnus (Ruby) has no equivalent filter at
//! all in its own struct generator, so a host-owned cfg-gated field like `pdf_options` on
//! `ExtractionConfig` renders in Ruby but silently vanished from PHP even though PHP's Cargo.toml
//! enables the same feature. See the "pdf" feature on a HOST-owned type below, which is exactly
//! this shape run through the real `PhpBackend::generate_bindings` path (not a direct
//! `codegen::generators::structs::gen_struct*` unit call).

use super::PhpBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

fn php_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"php\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.php]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A HOST-owned struct (`rust_path` starts with the crate's own name, "test_lib") with one
/// always-on field and one field gated on a feature this binding may or may not configure.
fn cfg_gated_field_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            fields: vec![
                FieldDef {
                    name: "use_cache".to_string(),
                    ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                    ..Default::default()
                },
                FieldDef {
                    name: "pdf_options".to_string(),
                    ty: TypeRef::String,
                    cfg: Some(r#"feature = "pdf""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

/// The defect: this binding's config enables "pdf", so `ExtractionConfig::pdf_options` is not
/// merely present on the core struct -- it is reachable and must be settable from PHP too. Before
/// the fix, `never_skip_cfg_field_names` never learned about satisfied host-owned cfg gates, so
/// `php_binding_keeps_field` dropped `pdf_options` from the generated PHP mirror unconditionally.
#[test]
fn generate_bindings_keeps_host_cfg_gated_field_when_feature_is_configured_end_to_end() {
    let api = cfg_gated_field_api();
    let config = php_config_with_feature(Some("pdf"));
    let files = PhpBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        lib_rs.contains("pdf_options"),
        "a cfg-gated field whose feature this PHP binding configures must still be emitted on the \
         generated mirror struct, got:\n{lib_rs}"
    );
}

/// Positive control for the test above: when "pdf" is NOT configured, the core struct itself
/// never has `pdf_options` compiled in, so the PHP mirror must not reference it either --
/// otherwise the fix would have overcorrected into "never gate a field," which breaks the build
/// for any binding that genuinely does not enable the feature.
#[test]
fn generate_bindings_drops_host_cfg_gated_field_when_feature_is_not_configured_end_to_end() {
    let api = cfg_gated_field_api();
    let config = php_config_with_feature(None);
    let files = PhpBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);

    assert!(
        !lib_rs.contains("pdf_options"),
        "a cfg-gated field whose feature this PHP binding does NOT configure must not appear on \
         the generated mirror struct (the core field does not exist to read/write), got:\n{lib_rs}"
    );
}
