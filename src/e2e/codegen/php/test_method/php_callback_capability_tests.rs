use super::{render_unsupported_php_callback_method, uses_unsupported_php_callback};
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};

#[test]
fn lifecycle_only_bridge_does_not_emit_runnable_php_callback_test() {
    let mut config = ResolvedCrateConfig::default();
    config.replace_trait_bridges(vec![TraitBridgeConfig {
        trait_name: "EmbeddingBackend".into(),
        exclude_languages: vec!["php:callbacks".into()],
        ..TraitBridgeConfig::default()
    }]);
    let args = vec![crate::e2e::config::ArgMapping {
        name: "backend".into(),
        field: "input.backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some("EmbeddingBackend".into()),
    }];
    assert!(uses_unsupported_php_callback(&args, &config));
    config.replace_trait_bridges(vec![TraitBridgeConfig {
        trait_name: "EmbeddingBackend".into(),
        ..TraitBridgeConfig::default()
    }]);
    assert!(!uses_unsupported_php_callback(&args, &config));

    let method = render_unsupported_php_callback_method("register_embedding_backend", "callback fixture");
    assert!(method.contains("$this->markTestSkipped("));
    assert!(!method.contains("$result = ;"));
    assert!(!method.contains("$result ="));
}
