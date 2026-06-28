// Tests for the JNI emitter. Kept in a separate file `include!`d last by `jni_emitter.rs` so the
// `#[cfg(test)]` module is the final item in the flattened module (the other `include!`d files
// contribute production items, which must not follow a test module — `clippy::items_after_test_module`).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{KotlinAndroidConfig, TraitBridgeConfig};

    #[test]
    fn jni_bridge_object_treats_android_trait_lifecycle_functions_as_managed() {
        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "Renderer".to_string(),
                register_fn: Some("register_renderer".to_string()),
                unregister_fn: Some("unregister_renderer".to_string()),
                clear_fn: Some("clear_renderers".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        assert!(trait_bridge_manages_jni_function("register_renderer", &config));
        assert!(trait_bridge_manages_jni_function("unregister_renderer", &config));
        assert!(trait_bridge_manages_jni_function("clear_renderers", &config));
        assert!(!trait_bridge_manages_jni_function("list_renderers", &config));
    }
}
