//! Public Swift component-manager wrappers.

use crate::core::config::ResolvedCrateConfig;

pub(super) fn generate(config: &ResolvedCrateConfig) -> Option<String> {
    if config.components.is_empty() {
        return None;
    }
    let mut generated = crate::backends::swift::template_env::render("components.swift.jinja", crate::alef_context! {});
    // Keep the separating blank line used when appending the component API. ~keep
    generated.push('\n');
    Some(generated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{ComponentConfig, ComponentProvidesConfig};

    #[test]
    fn emits_public_swift_api_with_explicit_mobile_exclusion() {
        let config = ResolvedCrateConfig {
            components: vec![ComponentConfig {
                producer_language: Default::default(),
                name: "fast".into(),
                provides: vec![ComponentProvidesConfig {
                    contract: "engine".into(),
                    implementation: "demo_core::FastEngine".into(),
                }],
                features: vec!["fast".into()],
                default_features: false,
                targets: Some(vec!["aarch64-apple-darwin".into()]),
                bundled_on: Vec::new(),
                source_manifest: None,
                producer_dir: None,
                extra_dependencies: Default::default(),
            }],
            ..ResolvedCrateConfig::default()
        };
        let generated = generate(&config).expect("component API");
        assert!(generated.contains("public func componentLoad"));
        assert!(generated.contains("public func componentPrefetch"));
        assert!(generated.contains("public func componentStatus"));
        assert!(generated.contains("public func componentCachePath"));
        assert!(generated.contains("#if os(iOS) || os(tvOS)"));
        assert!(generated.contains("RustBridge.component_load"));
        assert!(generated.contains("AlefComponentResponse<Value>"));
        use sha2::Digest as _;
        assert_eq!(
            sha2::Sha256::digest(generated.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            "b053585558b16dddab0945981fb60d4d9914414439b53ad55af2cdffd52f8407"
        );
    }
}
