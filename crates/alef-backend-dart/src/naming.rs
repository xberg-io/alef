//! Dart-specific naming helpers for `ResolvedCrateConfig`.

use alef_core::config::{DartStyle, ResolvedCrateConfig};
use alef_core::template_versions;

/// Get the Dart bridging style (`frb` or `ffi`).
pub fn dart_style(config: &ResolvedCrateConfig) -> DartStyle {
    config.dart.as_ref().map(|d| d.style).unwrap_or_default()
}

/// Get the flutter_rust_bridge version to pin.
///
/// Returns the per-crate override from `[crates.dart] frb_version` when set;
/// otherwise falls back to the compiled-in default constant.
pub fn dart_frb_version(config: &ResolvedCrateConfig) -> String {
    config
        .dart
        .as_ref()
        .and_then(|d| d.frb_version.as_ref())
        .cloned()
        .unwrap_or_else(|| template_versions::cargo::FLUTTER_RUST_BRIDGE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::new_config::NewAlefConfig;
    use alef_core::template_versions;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn dart_style_defaults_to_frb() {
        let r = minimal();
        assert_eq!(dart_style(&r), DartStyle::Frb);
    }

    #[test]
    fn dart_style_explicit_ffi() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.dart]
style = "ffi"
"#,
        );
        assert_eq!(dart_style(&r), DartStyle::Ffi);
    }

    #[test]
    fn dart_frb_version_defaults_to_constant() {
        let r = minimal();
        assert_eq!(
            dart_frb_version(&r),
            template_versions::cargo::FLUTTER_RUST_BRIDGE,
            "should fall back to compiled-in default"
        );
    }

    #[test]
    fn dart_frb_version_explicit_override_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.dart]
frb_version = "2.9.0"
"#,
        );
        assert_eq!(
            dart_frb_version(&r),
            "2.9.0",
            "explicit frb_version override should win"
        );
    }
}
