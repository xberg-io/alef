//! Regression coverage for `declared_features` narrowing a cfg-gated field's `#[cfg(...)]` gate
//! before it is copied onto the `Self { .. }` field initialiser in a core -> binding `From` impl.
//!
//! Without this, a backend that copies `field.cfg` verbatim can emit a gate naming a feature its
//! own binding crate's `Cargo.toml` never declares -- `unexpected_cfg_condition_value`, a hard
//! error under `-D warnings`. Mirrors a real PHP consumer's `ExtractionConfig.crawl` field,
//! whose gate named a feature the consumer's PHP crate never declared.

use super::gen_from_core_to_binding_cfg;
use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

fn type_with_fields(fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "test_lib::ExtractionConfig".to_string(),
        fields,
        ..TypeDef::default()
    }
}

fn crawl_field() -> FieldDef {
    FieldDef {
        name: "crawl".to_string(),
        ty: TypeRef::String,
        cfg: Some(r#"any(feature = "url-ingestion", feature = "url-config-types")"#.to_string()),
        ..FieldDef::default()
    }
}

/// CONTROL: when this binding declares every feature the gate names, the gate must survive
/// unchanged, byte for byte. Without this control, a fix that narrows or drops indiscriminately
/// would still pass the regression test below.
#[test]
fn gate_with_every_feature_declared_is_emitted_unchanged() {
    let field = crawl_field();
    let never_skip = vec!["crawl".to_string()];
    let declared: HashSet<&str> = ["url-ingestion", "url-config-types"].into_iter().collect();

    let config = ConversionConfig {
        never_skip_cfg_field_names: &never_skip,
        strip_cfg_fields_from_binding_struct: true,
        declared_features: Some(&declared),
        ..ConversionConfig::default()
    };

    let out = gen_from_core_to_binding_cfg(&type_with_fields(vec![field]), "test_lib", &AHashSet::new(), &config);

    assert!(
        out.contains(r#"#[cfg(any(feature = "url-ingestion", feature = "url-config-types"))]"#),
        "a gate whose every feature is declared must be copied verbatim, got:\n{out}"
    );
}

/// The reported defect: this binding declares "url-config-types" but not "url-ingestion" (a
/// real PHP consumer's shape). The copied gate must narrow to the single declared term,
/// not keep naming the undeclared one.
#[test]
fn gate_with_one_undeclared_feature_narrows_to_the_declared_term_alone() {
    let field = crawl_field();
    let never_skip = vec!["crawl".to_string()];
    let declared: HashSet<&str> = ["url-config-types"].into_iter().collect();

    let config = ConversionConfig {
        never_skip_cfg_field_names: &never_skip,
        strip_cfg_fields_from_binding_struct: true,
        declared_features: Some(&declared),
        ..ConversionConfig::default()
    };

    let out = gen_from_core_to_binding_cfg(&type_with_fields(vec![field]), "test_lib", &AHashSet::new(), &config);

    let crawl_line = out
        .lines()
        .find(|line| line.contains("crawl:"))
        .expect("crawl field initialiser present");
    let cfg_line = out
        .lines()
        .take_while(|line| !line.contains("crawl:"))
        .last()
        .expect("a preceding cfg attribute line exists");

    assert!(
        cfg_line.contains(r#"#[cfg(feature = "url-config-types")]"#),
        "gate must narrow to the single declared term, got cfg line:\n{cfg_line}\nfull output:\n{out}"
    );
    assert!(
        !cfg_line.contains("url-ingestion"),
        "the undeclared feature must not appear in the emitted cfg attribute, got:\n{cfg_line}"
    );
    assert!(crawl_line.contains("val.crawl"), "field initialiser missing, got:\n{out}");
}
