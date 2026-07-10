//! Scaffold-specific naming helpers derived from `ResolvedCrateConfig`.

use crate::core::config::ResolvedCrateConfig;

/// Get the PyPI package name used as `[project] name` in `pyproject.toml`.
///
/// Returns `[python] pip_name` if set, otherwise falls back to the crate name.
pub(crate) fn python_pip_name(config: &ResolvedCrateConfig) -> String {
    config
        .python
        .as_ref()
        .and_then(|p| p.pip_name.as_ref())
        .cloned()
        .unwrap_or_else(|| config.name.clone())
}

/// Get the resolved minimum macOS deployment target.
pub(crate) fn swift_min_macos(config: &ResolvedCrateConfig) -> String {
    config
        .swift
        .as_ref()
        .and_then(|s| s.min_macos_version.as_ref())
        .cloned()
        .unwrap_or_else(|| crate::core::template_versions::toolchain::SWIFT_MIN_MACOS.to_string())
}

/// Get the resolved minimum iOS deployment target.
pub(crate) fn swift_min_ios(config: &ResolvedCrateConfig) -> String {
    config
        .swift
        .as_ref()
        .and_then(|s| s.min_ios_version.as_ref())
        .cloned()
        .unwrap_or_else(|| crate::core::template_versions::toolchain::SWIFT_MIN_IOS.to_string())
}

/// Get the NuGet `<PackageId>` to publish under.
///
/// Defaults to `csharp_namespace()` when `[csharp].package_id` is unset.
pub(crate) fn csharp_package_id(config: &ResolvedCrateConfig) -> String {
    config
        .csharp
        .as_ref()
        .and_then(|c| c.package_id.as_ref())
        .cloned()
        .unwrap_or_else(|| config.csharp_namespace())
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
    fn python_pip_name_defaults_to_crate_name() {
        let r = minimal();
        assert_eq!(python_pip_name(&r), "test-lib");
    }

    #[test]
    fn python_pip_name_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
pip_name = "mypackage"
"#,
        );
        assert_eq!(python_pip_name(&r), "mypackage");
    }

    #[test]
    fn swift_min_macos_defaults_to_toolchain_constant() {
        let r = minimal();
        assert!(!swift_min_macos(&r).is_empty());
    }

    #[test]
    fn swift_min_ios_defaults_to_toolchain_constant() {
        let r = minimal();
        assert!(!swift_min_ios(&r).is_empty());
    }

    #[test]
    fn swift_min_macos_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.swift]
min_macos_version = "14.0"
"#,
        );
        assert_eq!(swift_min_macos(&r), "14.0");
    }

    #[test]
    fn swift_min_ios_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.swift]
min_ios_version = "17.0"
"#,
        );
        assert_eq!(swift_min_ios(&r), "17.0");
    }

    #[test]
    fn csharp_package_id_defaults_to_namespace() {
        let r = minimal();
        assert_eq!(csharp_package_id(&r), "TestLib");
    }

    #[test]
    fn csharp_package_id_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.csharp]
package_id = "MyCompany.TestLib"
"#,
        );
        assert_eq!(csharp_package_id(&r), "MyCompany.TestLib");
    }
}
