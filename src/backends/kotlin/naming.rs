//! Kotlin-specific naming helpers for `ResolvedCrateConfig`.

use crate::core::config::{KotlinTarget, ResolvedCrateConfig};

/// Get the Kotlin target platform.
///
/// Returns `KotlinTarget::Jvm` (the default) when the `[kotlin]` section is absent or
/// `target` is not set.
pub fn kotlin_target(config: &ResolvedCrateConfig) -> KotlinTarget {
    config.kotlin.as_ref().map(|k| k.target).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn kotlin_target_defaults_to_jvm() {
        let r = minimal();
        assert_eq!(kotlin_target(&r), KotlinTarget::Jvm);
    }

    #[test]
    fn kotlin_target_explicit_native() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.kotlin]
target = "native"
"#,
        );
        assert_eq!(kotlin_target(&r), KotlinTarget::Native);
    }

    #[test]
    fn kotlin_target_explicit_multiplatform() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.kotlin]
target = "multiplatform"
"#,
        );
        assert_eq!(kotlin_target(&r), KotlinTarget::Multiplatform);
    }
}
