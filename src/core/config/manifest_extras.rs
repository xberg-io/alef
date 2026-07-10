//! Extra dependencies for language-native manifests (Phase 1: Node/Wasm `package.json`).
//!
//! Distinct from `ResolvedCrateConfig::extra_dependencies`, which
//! targets the binding crate's `Cargo.toml` only. This module's [`ManifestExtras`] targets
//! the host-language manifest emitted alongside each language binding — `package.json`,
//! `pyproject.toml`, `Gemfile`, `composer.json`, `pom.xml`, `*.csproj`, `pubspec.yaml`,
//! `Package.swift`, `go.mod`, `mix.exs`, `build.gradle.kts`, `build.zig.zon`.
//!
//! Two parallel surfaces:
//! - `[crates.<lang>.package_extras]` — applied to `packages/<lang>/<manifest>`
//! - `[crates.e2e.<lang>.harness_extras]` — applied to `e2e/<lang>/<manifest>`
//!
//! Both deserialize into [`ManifestExtras`], so per-language emitters need only one
//! injection helper that consumes the same struct.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Extra dependencies declared for a language-native manifest. `dependencies` and
/// `dev_dependencies` mirror the runtime / dev-test distinction present in
/// npm, Composer, Bundler, pubspec, Mix, etc. Languages without that distinction
/// (Go, Zig, Kotlin `testImplementation`) collapse both buckets into one at the
/// emitter level.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct ManifestExtras {
    /// Runtime dependencies (e.g. `dependencies` in package.json, `requires` in
    /// pyproject, `dependencies` in composer.json).
    #[serde(default)]
    pub dependencies: BTreeMap<String, ExtraDepSpec>,
    /// Dev / test dependencies (e.g. `devDependencies` in package.json,
    /// `require-dev` in composer.json, `group :test do` in Gemfile).
    #[serde(default)]
    pub dev_dependencies: BTreeMap<String, ExtraDepSpec>,
}

impl ManifestExtras {
    /// True when neither bucket has any entries.
    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty() && self.dev_dependencies.is_empty()
    }
}

/// A single extra-dependency declaration. Accepts either a bare version string
/// (`"tree-sitter" = "^0.25.0"`) or a free-form TOML table for source/feature
/// metadata (`"foo" = { version = "1", git = "https://…" }`).
///
/// Per-language emitters decide which table keys they understand. Unknown keys
/// are surfaced as warnings but never block emission — the goal is forward
/// compatibility as the alef.toml surface grows.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, JsonSchema)]
#[serde(untagged)]
pub enum ExtraDepSpec {
    /// Simple `"name" = "version"` form.
    Simple(String),
    /// Detailed `"name" = { … }` form.
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    Detailed(toml::Table),
}

impl ExtraDepSpec {
    /// Extract a `version` string when one is present, whether the spec is a
    /// bare `String` or a `Detailed` table with a `version = "…"` key.
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Simple(v) => Some(v),
            Self::Detailed(t) => t.get("version").and_then(|v| v.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_simple_form() {
        let toml_src = r#"
            [dependencies]
            "tree-sitter" = "^0.25.0"
        "#;
        let extras: ManifestExtras = toml::from_str(toml_src).expect("deserializes");
        assert_eq!(extras.dependencies.len(), 1);
        let spec = &extras.dependencies["tree-sitter"];
        assert_eq!(spec.version(), Some("^0.25.0"));
        assert!(matches!(spec, ExtraDepSpec::Simple(_)));
    }

    #[test]
    fn deserialize_detailed_form() {
        let toml_src = r#"
            [dev_dependencies]
            tracing = { version = "0.1", features = ["log"] }
        "#;
        let extras: ManifestExtras = toml::from_str(toml_src).expect("deserializes");
        let spec = &extras.dev_dependencies["tracing"];
        assert_eq!(spec.version(), Some("0.1"));
        if let ExtraDepSpec::Detailed(t) = spec {
            assert!(t.get("features").is_some());
        } else {
            panic!("expected Detailed form, got {spec:?}");
        }
    }

    #[test]
    fn defaults_are_empty() {
        let extras = ManifestExtras::default();
        assert!(extras.is_empty());
    }

    #[test]
    fn partial_tables_deserialize() {
        let toml_src = r#"
            [dev_dependencies]
            vitest = "^3.0.0"
        "#;
        let extras: ManifestExtras = toml::from_str(toml_src).expect("deserializes");
        assert!(extras.dependencies.is_empty());
        assert_eq!(extras.dev_dependencies.len(), 1);
    }

    #[test]
    fn round_trip_preserves_order_and_values() {
        let toml_src = r#"
            [dependencies]
            "a-pkg" = "1.0"
            "b-pkg" = { version = "2.0" }

            [dev_dependencies]
            "z-pkg" = "9.9"
        "#;
        let extras: ManifestExtras = toml::from_str(toml_src).expect("deserializes");
        let names: Vec<&str> = extras.dependencies.keys().map(String::as_str).collect();
        assert_eq!(names, vec!["a-pkg", "b-pkg"]);
        assert_eq!(extras.dependencies["b-pkg"].version(), Some("2.0"));
    }
}
