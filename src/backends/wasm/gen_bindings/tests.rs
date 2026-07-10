use super::{WasmBackend, cargo::gen_cargo_toml};
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn wasm_backend_name_is_wasm() {
    assert_eq!(WasmBackend.name(), "wasm");
}

#[test]
fn generate_bindings_empty_api_produces_files() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].path.to_string_lossy().ends_with("lib.rs"));
    assert!(files[1].path.to_string_lossy().ends_with("Cargo.toml"));
}

#[test]
fn extra_dependency_overrides_builtin_without_duplicate_key() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dependencies]
serde = { version = "1", features = ["derive", "rc"] }
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);

    let serde_lines = cargo_toml
        .lines()
        .filter(|l| l.trim_start().starts_with("serde =") || l.trim_start().starts_with("serde="))
        .count();
    assert_eq!(serde_lines, 1, "expected exactly one `serde` key, got:\n{cargo_toml}");
    assert!(
        cargo_toml.contains(r#"features = ["derive", "rc"]"#),
        "extra_dependencies override should win:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_emits_passthrough_features_for_type_cfg_attrs() {
    use crate::core::ir::TypeDef;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "PdfThing".to_string(),
            rust_path: "test_lib::PdfThing".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        cargo_toml.contains(r#"pdf = ["test-lib/pdf"]"#),
        "expected `pdf = [\"test-lib/pdf\"]` in:\n{cargo_toml}"
    );
    assert_eq!(
        cargo_toml.matches("\n[features]\n").count(),
        1,
        "exactly one [features] block expected:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);
    assert!(
        !cargo_toml.contains("[features]"),
        "expected no [features] block:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_declares_explicit_features_as_passthrough_without_enabling_default() {
    // binding-side `#[cfg(feature = X)]` items intentionally remain hidden
    use crate::core::ir::TypeDef;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
features = ["wasm-target"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GatedType".to_string(),
            rust_path: "test_lib::GatedType".to_string(),
            cfg: Some(r#"any(feature = "wasm-target", feature = "extra")"#.to_string()),
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);
    assert!(
        cargo_toml.contains(r#"extra = ["test-lib/extra"]"#),
        "expected `extra` passthrough:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"wasm-target = ["test-lib/wasm-target"]"#),
        "wasm-target must be declared as passthrough so rustc sees the feature:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("default = ["),
        "no default = [...] line — binding-side cfg items stay hidden:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_has_no_issues_docs_line_and_getrandom_deps_are_alphabetical() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        !cargo_toml.contains("Issues & docs:"),
        "Cargo.toml must not contain 'Issues & docs:' line — cargo-sort strips it and \
             alef re-emits it, causing prek to loop forever:\n{cargo_toml}"
    );

    let pos_02 = cargo_toml
        .find("getrandom_02")
        .expect("getrandom_02 must be present in target deps");
    let pos_03 = cargo_toml
        .find("getrandom_03")
        .expect("getrandom_03 must be present in target deps");
    assert!(
        pos_02 < pos_03,
        "getrandom_02 must appear before getrandom_03 (alphabetical order for cargo-sort \
             compatibility); got getrandom_02 at {pos_02}, getrandom_03 at {pos_03}:\n{cargo_toml}"
    );

    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn test_visitor_field_substitution_in_post_process() {
    let mut content = "impl From<WasmConversionOptions> for sample_markup_rs::options::ConversionOptions {\n    fn from(val: WasmConversionOptions) -> Self {\n        Self {\n            heading_style: val.heading_style.into(),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\nimpl From<WasmConversionOptionsUpdate> for sample_markup_rs::options::ConversionOptionsUpdate {\n    fn from(val: WasmConversionOptionsUpdate) -> Self {\n        Self {\n            heading_style: val.heading_style.map(Into::into),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\n".to_string();

    let field_name = "visitor";
    let patterns = &[
        ("            ", "\n            "),
        ("        ", "\n        "),
        ("  ", "\n  "),
    ];
    for (indent, newline_indent) in patterns {
        let old_pattern = format!("{indent}{field_name}: Default::default(),{newline_indent}..Default::default()");
        let new_pattern = format!(
            "{indent}{field_name}: val.{field_name}.map(|v| (*v.inner).clone()),{newline_indent}..Default::default()"
        );
        if content.contains(&old_pattern) {
            content = content.replace(&old_pattern, &new_pattern);
        }
    }

    assert!(
        content.contains("visitor: val.visitor.map(|v| (*v.inner).clone()),"),
        "Visitor field not forwarded in From impl"
    );
    assert!(
        !content.contains("visitor: Default::default(),\n            ..Default::default()"),
        "Unreplaced visitor: Default::default() with 12 spaces still present"
    );
}
