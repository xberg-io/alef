//! Reject an unknown `[crates.<lang>.dependency_versions]` key.
//!
//! ~keep A hard error rather than a warning, and deliberately the opposite of
//! `ManifestExtras`' "unknown keys warn, never block" contract. An override exists precisely
//! because the consumer's own bump would otherwise be written back over on the next
//! regeneration; ignoring a mistyped key would discard the bump again, with no signal, and
//! reproduce the exact failure the table was added to fix.

use crate::core::config::extras::Language;
use crate::core::config::resolved::ResolvedCrateConfig;
use crate::core::error::AlefError;
use crate::core::managed_versions;

/// Reject every declared override key that is not registered for its own language.
///
/// ~keep Scoped per language, not against the union of every language's keys: `cargo:rustler`
/// and `hex:rustler` are two different dependencies, and a `maven:junit` typed under
/// `[crates.python.dependency_versions]` must fail rather than be excused by Java's table.
pub(super) fn validate_dependency_version_overrides(config: &ResolvedCrateConfig) -> Result<(), AlefError> {
    for language in managed_versions::MANAGED_LANGUAGES {
        let declared = config.dependency_version_overrides(language);
        let mut unknown: Vec<&str> = declared
            .keys()
            .map(String::as_str)
            .filter(|key| managed_versions::lookup(language, key).is_none())
            .collect();
        if unknown.is_empty() {
            continue;
        }
        unknown.sort_unstable();
        return Err(AlefError::Config(unknown_key_message(config, language, &unknown)));
    }
    Ok(())
}

fn unknown_key_message(config: &ResolvedCrateConfig, language: Language, unknown: &[&str]) -> String {
    let table = managed_versions::override_table(language);
    format!(
        "unknown `{table}` key(s) for crate `{crate_name}`: {unknown}. An override for a key \
         alef does not emit would be silently discarded on the next regeneration, which is the \
         failure this table exists to prevent, so it is rejected instead. Valid keys for \
         {language}: {valid}.",
        crate_name = config.name,
        unknown = unknown.join(", "),
        valid = managed_versions::keys_for(language).join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    /// Deserialize, resolve, and run the whole `validate_resolved` gate -- the same entry
    /// point `bin_cli::helpers` calls on every config load, so a test that passes here
    /// proves the check is reachable from the CLI and not merely callable.
    fn resolve(toml_src: &str) -> Result<ResolvedCrateConfig, String> {
        let parsed: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
        let resolved = parsed.resolve().map_err(|error| error.to_string())?.remove(0);
        super::super::validate_resolved(&resolved).map_err(|error| error.to_string())?;
        Ok(resolved)
    }

    const VALID: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.python.dependency_versions]
"cargo:pyo3" = "0.99"
"pypi:pyrefly" = ">=99.0.0"
"#;

    const UNKNOWN_KEY: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.python.dependency_versions]
"cargo:pyo4" = "0.99"
"#;

    /// A key valid for another language must not be excused here: per-language scoping is
    /// the whole reason the keys are ecosystem-qualified.
    const FOREIGN_LANGUAGE_KEY: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.python.dependency_versions]
"maven:junit" = "99.0.0"
"#;

    #[test]
    fn known_keys_resolve() {
        let resolved = resolve(VALID).expect("a config declaring only registered keys must resolve");
        assert_eq!(resolved.managed_version(Language::Python, "cargo:pyo3"), Some("0.99"));
    }

    #[test]
    fn an_unknown_key_is_a_hard_error_naming_the_table_and_the_valid_keys() {
        let error = resolve(UNKNOWN_KEY).expect_err("an unknown key must fail resolution");
        assert!(
            error.contains("cargo:pyo4"),
            "the error must name the offending key, got: {error}"
        );
        assert!(
            error.contains("[crates.python.dependency_versions]"),
            "the error must name the real table, got: {error}"
        );
        assert!(
            error.contains("cargo:pyo3"),
            "the error must list the valid keys so the typo is fixable, got: {error}"
        );
    }

    #[test]
    fn a_key_registered_for_another_language_is_still_unknown() {
        let error = resolve(FOREIGN_LANGUAGE_KEY).expect_err("a foreign-language key must fail resolution");
        assert!(
            error.contains("maven:junit"),
            "the error must name the foreign key, got: {error}"
        );
    }
}
