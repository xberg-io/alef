//! Swift-specific naming helpers for `ResolvedCrateConfig`.

use crate::core::config::ResolvedCrateConfig;
use crate::core::template_versions;

/// Get the swift-bridge version to pin in the generated `Cargo.toml`.
///
/// Returns the per-crate override from `[crates.swift] swift_bridge_version` when set;
/// otherwise falls back to the compiled-in default constant.
pub fn swift_bridge_version(config: &ResolvedCrateConfig) -> String {
    config
        .swift
        .as_ref()
        .and_then(|s| s.swift_bridge_version.as_ref())
        .cloned()
        .unwrap_or_else(|| template_versions::cargo::SWIFT_BRIDGE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::template_versions;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn swift_bridge_version_defaults_to_constant() {
        let r = minimal();
        assert_eq!(
            swift_bridge_version(&r),
            template_versions::cargo::SWIFT_BRIDGE,
            "should fall back to compiled-in default"
        );
    }

    #[test]
    fn swift_bridge_version_explicit_override_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.swift]
swift_bridge_version = "0.2.0"
"#,
        );
        assert_eq!(
            swift_bridge_version(&r),
            "0.2.0",
            "explicit swift_bridge_version override should win"
        );
    }
}
