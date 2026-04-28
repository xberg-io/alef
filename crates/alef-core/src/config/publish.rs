use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::output::StringOrVec;

/// Configuration for the `alef publish` command group.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublishConfig {
    /// Path to the core Rust crate directory to vendor.
    /// Auto-detected from `[crate].sources` if absent.
    pub core_crate: Option<String>,
    /// Per-language publish configuration overrides.
    #[serde(default)]
    pub languages: HashMap<String, PublishLanguageConfig>,
}

/// Per-language publish configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublishLanguageConfig {
    /// Shell command that must exit 0 for publish steps to run; skip with warning on failure.
    pub precondition: Option<String>,
    /// Command(s) to run before the main publish commands; aborts on failure.
    pub before: Option<StringOrVec>,
    /// Command(s) to run after the main publish commands; aborts on failure.
    pub after: Option<StringOrVec>,
    /// Vendoring strategy for this language.
    pub vendor_mode: Option<VendorMode>,
    /// Elixir NIF versions to build for (e.g. `["2.16", "2.17"]`).
    pub nif_versions: Option<Vec<String>>,
    /// Override the default build command for cross-compilation.
    pub build_command: Option<StringOrVec>,
    /// Override the default package command.
    pub package_command: Option<StringOrVec>,
    /// Archive format override (`"tar.gz"` or `"zip"`).
    pub archive_format: Option<String>,
    /// Generate a pkg-config `.pc` file (C FFI only).
    pub pkg_config: Option<bool>,
    /// Generate a CMake find module (C FFI only).
    pub cmake_config: Option<bool>,
    // --- New fields added in Phase 1 ---
    /// npm sub-package platform identifiers for Node.js NAPI builds.
    /// Each entry is a napi-rs platform string like `"linux-x64-gnu"`.
    /// Defaults to a standard set when absent.
    pub npm_subpackage_platforms: Option<Vec<String>>,
    /// Forward-looking: environment variables passed through to cibuildwheel
    /// when orchestrating Python cross-compilation builds.
    pub cibuildwheel_environment: Option<std::collections::HashMap<String, String>>,
    /// JNI classifier override for Java packaging (e.g. `"linux-x86_64"`).
    /// Derived from the target triple when absent.
    pub jni_classifier: Option<String>,
    /// NuGet RID override for C# packaging (e.g. `"linux-x64"`).
    /// Derived from the target triple when absent.
    pub csharp_rid: Option<String>,
    /// Build and include a Python wheel for this target (default: `true`).
    pub wheel: Option<bool>,
    /// Build and include a Python sdist (default: `true`).
    pub sdist: Option<bool>,
}

/// How to vendor the Rust core crate into a language package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum VendorMode {
    /// Copy only the core crate, rewrite path dependencies.
    /// Used by Ruby and Elixir.
    CoreOnly,
    /// Run `cargo vendor` for all transitive dependencies.
    /// Used by R/CRAN packages.
    Full,
    /// No vendoring needed (default for most languages).
    #[default]
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_config_deserializes_empty() {
        let toml_str = "";
        let cfg: PublishConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.core_crate.is_none());
        assert!(cfg.languages.is_empty());
    }

    #[test]
    fn publish_config_deserializes_with_languages() {
        let toml_str = r#"
core_crate = "crates/my-lib"

[languages.ruby]
vendor_mode = "core-only"

[languages.r]
vendor_mode = "full"

[languages.elixir]
vendor_mode = "core-only"
nif_versions = ["2.16", "2.17"]

[languages.c_ffi]
pkg_config = true
cmake_config = true
"#;
        let cfg: PublishConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.core_crate.as_deref(), Some("crates/my-lib"));

        let ruby = cfg.languages.get("ruby").unwrap();
        assert_eq!(ruby.vendor_mode, Some(VendorMode::CoreOnly));

        let r = cfg.languages.get("r").unwrap();
        assert_eq!(r.vendor_mode, Some(VendorMode::Full));

        let elixir = cfg.languages.get("elixir").unwrap();
        assert_eq!(
            elixir.nif_versions.as_deref(),
            Some(&["2.16".to_string(), "2.17".to_string()][..])
        );

        let c_ffi = cfg.languages.get("c_ffi").unwrap();
        assert_eq!(c_ffi.pkg_config, Some(true));
        assert_eq!(c_ffi.cmake_config, Some(true));
    }

    #[test]
    fn publish_language_config_with_commands() {
        let toml_str = r#"
precondition = "which cargo"
before = ["step1", "step2"]
build_command = "cross build --release"
package_command = "custom-packager"
archive_format = "zip"
"#;
        let cfg: PublishLanguageConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.precondition.as_deref(), Some("which cargo"));
        assert_eq!(cfg.before.unwrap().commands(), vec!["step1", "step2"]);
        assert_eq!(cfg.build_command.unwrap().commands(), vec!["cross build --release"]);
        assert_eq!(cfg.package_command.unwrap().commands(), vec!["custom-packager"]);
        assert_eq!(cfg.archive_format.as_deref(), Some("zip"));
    }

    #[test]
    fn vendor_mode_kebab_case() {
        assert_eq!(
            serde_json::from_str::<VendorMode>(r#""core-only""#).unwrap(),
            VendorMode::CoreOnly
        );
        assert_eq!(
            serde_json::from_str::<VendorMode>(r#""full""#).unwrap(),
            VendorMode::Full
        );
        assert_eq!(
            serde_json::from_str::<VendorMode>(r#""none""#).unwrap(),
            VendorMode::None
        );
    }

    #[test]
    fn publish_config_in_alef_config() {
        let toml_str = r#"
languages = ["python", "ruby"]

[crate]
name = "test-lib"
sources = ["src/lib.rs"]

[publish]
core_crate = "crates/test-lib"

[publish.languages.ruby]
vendor_mode = "core-only"
"#;
        let cfg: super::super::AlefConfig = toml::from_str(toml_str).unwrap();
        let publish = cfg.publish.unwrap();
        assert_eq!(publish.core_crate.as_deref(), Some("crates/test-lib"));
        let ruby = publish.languages.get("ruby").unwrap();
        assert_eq!(ruby.vendor_mode, Some(VendorMode::CoreOnly));
    }
}
