//! PyO3 `.pyi` stub coverage for trait-bridge registry functions.
//!
//! Split out of `backends_pyo3_gen_stubs_test.rs`, which is over the 1,000-line cap and may not
//! grow. Trait-bridge stub emission is its own concern: it is the only stub surface gated on the
//! shared emit gates (`registry_getter` present, bridged trait resolvable), so it changes for
//! different reasons than the field/enum/protocol stub tests it used to sit beside. ~keep

use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;

fn make_config_with_stubs() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/src/"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn test_pyi_includes_trait_bridge_registry_functions() {
    let backend = Pyo3Backend;
    let mut config = make_config_with_stubs();
    config.trait_bridges = vec![alef::core::config::TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        registry_getter: Some("test_lib::get_ocr_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()),
        clear_fn: Some("clear_ocr_backends".to_string()),
        ..Default::default()
    }];
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        // The bridged trait must resolve here and the bridge must carry a `registry_getter`, or
        // no `#[pyfunction]` is emitted and a stub declaring one would name a missing symbol. ~keep
        types: vec![TypeDef {
            name: "OcrBackend".to_string(),
            rust_path: "test_lib::OcrBackend".to_string(),
            is_trait: true,
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let content = backend.generate_type_stubs(&api, &config).unwrap()[0].content.clone();

    assert!(
        content.contains("def register_ocr_backend(backend: object) -> None: ...")
            && content.contains("def unregister_ocr_backend(name: str) -> None: ...")
            && content.contains("def clear_ocr_backends() -> None: ..."),
        "pyi must include trait bridge functions exported by runtime:\n{content}"
    );
}
