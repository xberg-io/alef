//! The one lookup every manifest renderer uses for a managed dependency version.
//!
//! ~keep Renderers call `versions.get("cargo:pyo3")` instead of reading
//! `template_versions::cargo::PYO3` directly: the const is still the default, but it is no
//! longer the only possible answer, and a renderer that reads it straight silently ignores
//! the consumer's `[crates.<lang>.dependency_versions]` entry. There are 63 such lookups
//! across the five renderers, far past `codegen-structure`'s third-repetition bar --
//! `tests/scaffold_dependency_version_overrides.rs` asserts that per-renderer count so the
//! keys cannot drift out of the registry unnoticed.

use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::core::managed_versions;

/// Resolved managed versions for one language's manifests.
pub(crate) struct ManagedVersions<'a> {
    language: Language,
    overrides: &'a std::collections::BTreeMap<String, String>,
}

impl<'a> ManagedVersions<'a> {
    pub(crate) fn new(config: &'a ResolvedCrateConfig, language: Language) -> Self {
        Self {
            language,
            overrides: config.dependency_version_overrides(language),
        }
    }

    /// The literal to emit for `key`: the consumer's override, else the `template_versions`
    /// default.
    ///
    /// # Panics
    ///
    /// ~keep Panics when `key` is not registered for this language. A renderer key is a
    /// compile-time literal paired with a registry entry, so a miss is a programming error
    /// in alef, not consumer input -- and returning an empty string instead would emit a
    /// manifest that parses and installs the wrong thing. Consumer-supplied keys never
    /// reach here: `config::validation` rejects an unknown one at config load.
    pub(crate) fn get(&self, key: &'static str) -> &'a str {
        managed_versions::resolve(self.language, key, self.overrides).unwrap_or_else(|| {
            panic!(
                "`{key}` is not a registered managed version for {}; add it to \
                 `core::managed_versions::MANAGED_DEPENDENCIES` or fix the lookup",
                self.language
            )
        })
    }
}

/// The in-manifest notice pointing a reader at the override table, as bare body lines (no
/// comment marker, no `~keep`, no trailing newline).
///
/// ~keep No `--` anywhere in the text: these lines are rendered into an XML comment by
/// `java_pom.xml.jinja`, and `--` inside an XML comment is a well-formedness error every
/// Maven parse would reject.
pub(crate) fn override_notice_lines(language: Language) -> Vec<String> {
    vec![
        "Dependency versions here are alef-managed and are rewritten on every `alef build`.".to_string(),
        format!(
            "To raise one durably, set it in `{}`; a direct edit to this file is discarded.",
            managed_versions::override_table(language)
        ),
    ]
}

/// [`override_notice_lines`] as a `#`-comment block ending in a newline, keep-marked.
///
/// ~keep The marker is added here and not in [`override_notice_lines`] because it is only
/// meaningful where an uncomment pass runs. Poly strips unmarked `#` comments from the
/// manifests this renders into, between one regeneration and the next, so without it the
/// notice disappears in a commit that reads as unrelated formatting. The XML path takes the
/// bare lines instead: poly has no uncomment pass for XML, and
/// `scaffold::template_env::render` strips every `~keep` from template output by design
/// (see `core::keep_marker`), so a marker there would be removed anyway. One marker spares
/// the whole unbroken comment run, so the first line carries it for both.
pub(crate) fn override_notice_hash(language: Language) -> String {
    override_notice_lines(language)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                format!("# {line} ~keep\n")
            } else {
                format!("# {line}\n")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    const CONFIG: &str = r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ruby.dependency_versions]
"gem:rspec" = "~> 99.0"
"#;

    fn resolved() -> ResolvedCrateConfig {
        let parsed: NewAlefConfig = toml::from_str(CONFIG).expect("config parses");
        parsed.resolve().expect("config resolves").remove(0)
    }

    #[test]
    fn an_override_replaces_the_default_and_leaves_its_siblings_alone() {
        let config = resolved();
        let versions = ManagedVersions::new(&config, Language::Ruby);
        assert_eq!(versions.get("gem:rspec"), "~> 99.0");
        assert_eq!(versions.get("gem:steep"), crate::core::template_versions::gem::STEEP);
    }

    #[test]
    fn a_language_without_the_table_falls_back_to_every_default() {
        let config = resolved();
        let versions = ManagedVersions::new(&config, Language::Python);
        assert_eq!(versions.get("cargo:pyo3"), crate::core::template_versions::cargo::PYO3);
    }

    #[test]
    #[should_panic(expected = "is not a registered managed version")]
    fn an_unregistered_key_panics_rather_than_emitting_an_empty_literal() {
        let config = resolved();
        ManagedVersions::new(&config, Language::Ruby).get("gem:not-a-real-entry");
    }

    /// The notice must survive poly's uncomment pass, or the only in-manifest signal a
    /// consumer gets disappears on the next formatting run.
    #[test]
    fn the_notice_carries_a_keep_marker_and_names_the_real_table() {
        let notice = override_notice_hash(Language::Python);
        assert!(notice.contains("~keep"), "notice must be keep-marked, got:\n{notice}");
        assert!(
            notice.contains("[crates.python.dependency_versions]"),
            "notice must name the real table, got:\n{notice}"
        );
        for line in notice.lines() {
            assert_eq!(line.trim_end(), line, "notice line has trailing whitespace: {line:?}");
        }
    }
}
