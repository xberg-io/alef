//! The registry of dependency versions a consumer may override in `alef.toml`.
//!
//! [`crate::core::template_versions`] is how *alef* carries the versions it emits, with
//! Renovate markers keeping them current. This module is how a *consumer* raises one of them
//! for their own tree without opening a PR against alef.
//!
//! ~keep Every `default` here **references** the `template_versions` const rather than
//! repeating its literal. `tests/template_versions_renovate_coverage.rs` reads
//! `src/core/template_versions.rs` by literal path, so the consts must stay where they are;
//! copying a literal into this file would give Renovate one value to bump and the emitters
//! another to read.
//!
//! ~keep Keys are ecosystem-qualified (`cargo:rustler` vs `hex:rustler`) because the same
//! package name means two different dependencies in two different ecosystems, and the Elixir
//! manifests emit both. The registry is keyed by `(language, key)`: an override declared under
//! `[crates.python.dependency_versions]` is only ever matched against Python's entries, so an
//! unknown Python key cannot be excused by a Java key that happens to spell the same thing.
//!
//! ~keep A `default` is a raw literal fragment emitted verbatim, not a semver to parse.
//! `gem:rb_sys`'s value is `"\">= 0.9.130\", \"< 0.10\""` — two Ruby requirement arguments,
//! because the gemspec DSL rejects a comma-joined single string. An override replaces the
//! fragment; alef performs no comparison against it.

use crate::core::config::extras::Language;
use crate::core::template_versions as tv;

/// One overridable dependency version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedDependency {
    /// The ecosystem-qualified key a consumer writes in
    /// `[crates.<lang>.dependency_versions]`.
    pub key: &'static str,
    /// The language whose `dependency_versions` table this key belongs to.
    pub language: Language,
    /// The `template_versions` path this entry's default comes from, for error and report
    /// output.
    pub const_name: &'static str,
    /// The literal alef emits when the consumer declares no override.
    pub default: &'static str,
}

/// Which generated manifest a language's overrides reach.
///
/// Stated per language rather than per entry: every entry for a language lands in one of the
/// two files named here, and the pair is what the `alef verify` finding and the in-manifest
/// notice print.
pub struct ManagedManifests {
    /// The host-language package manifest (`pyproject.toml`, `pom.xml`, `mix.exs`,
    /// `<gem>.gemspec` + `Gemfile`, `DESCRIPTION`).
    pub package_manifest: &'static str,
    /// The binding crate's `Cargo.toml`, when this language emits one carrying managed
    /// versions.
    pub binding_manifest: Option<&'static str>,
}

/// The `alef.toml` table an override for `language` is declared in.
pub fn override_table(language: Language) -> String {
    format!("[crates.{language}.dependency_versions]")
}

/// The manifests a language's overrides reach, or `None` for a language with no managed
/// versions.
pub fn manifests_for(language: Language) -> Option<ManagedManifests> {
    let manifests = match language {
        Language::Python => ManagedManifests {
            package_manifest: "pyproject.toml",
            binding_manifest: Some("Cargo.toml"),
        },
        Language::Java => ManagedManifests {
            package_manifest: "pom.xml",
            binding_manifest: None,
        },
        Language::Elixir => ManagedManifests {
            package_manifest: "mix.exs",
            binding_manifest: Some("Cargo.toml"),
        },
        Language::Ruby => ManagedManifests {
            package_manifest: "<gem>.gemspec + Gemfile",
            binding_manifest: None,
        },
        Language::R => ManagedManifests {
            package_manifest: "DESCRIPTION",
            binding_manifest: Some("Cargo.toml"),
        },
        _ => return None,
    };
    Some(manifests)
}

/// Every language carrying managed versions, in a stable order.
pub const MANAGED_LANGUAGES: [Language; 5] = [
    Language::Python,
    Language::Java,
    Language::Elixir,
    Language::Ruby,
    Language::R,
];

macro_rules! managed {
    ($language:expr, $key:literal, $path:path) => {
        ManagedDependency {
            key: $key,
            language: $language,
            const_name: stringify!($path),
            default: $path,
        }
    };
}

/// Every overridable dependency version, grouped by language and sorted by key within a
/// group.
pub static MANAGED_DEPENDENCIES: &[ManagedDependency] = &[
    managed!(Language::Python, "cargo:async-trait", tv::cargo::ASYNC_TRAIT),
    managed!(Language::Python, "cargo:futures", tv::cargo::FUTURES),
    managed!(Language::Python, "cargo:pyo3", tv::cargo::PYO3),
    managed!(
        Language::Python,
        "cargo:pyo3-async-runtimes",
        tv::cargo::PYO3_ASYNC_RUNTIMES
    ),
    managed!(Language::Python, "cargo:serde", tv::cargo::SERDE),
    managed!(Language::Python, "cargo:serde_json", tv::cargo::SERDE_JSON),
    managed!(Language::Python, "cargo:tracing", tv::cargo::TRACING),
    // ~keep The value is a full PEP 508 requirement (`maturin>=1.0,<2.0`), not a bare
    // specifier: it is emitted as one entry of `build-system.requires`, where the package
    // name is part of the string. An override must include the name too.
    managed!(Language::Python, "pypi:maturin", tv::pypi::MATURIN_BUILD_REQUIRES),
    managed!(Language::Python, "pypi:pyrefly", tv::pypi::PYREFLY),
    managed!(Language::Java, "maven:assertj", tv::maven::ASSERTJ),
    managed!(
        Language::Java,
        "maven:central-publishing-maven-plugin",
        tv::maven::CENTRAL_PUBLISHING_PLUGIN
    ),
    managed!(Language::Java, "maven:checkstyle", tv::maven::CHECKSTYLE),
    managed!(Language::Java, "maven:jackson", tv::maven::JACKSON),
    managed!(
        Language::Java,
        "maven:jacoco-maven-plugin",
        tv::maven::JACOCO_MAVEN_PLUGIN
    ),
    managed!(Language::Java, "maven:jspecify", tv::maven::JSPECIFY),
    managed!(Language::Java, "maven:junit", tv::maven::JUNIT),
    managed!(
        Language::Java,
        "maven:maven-checkstyle-plugin",
        tv::maven::MAVEN_CHECKSTYLE_PLUGIN
    ),
    managed!(
        Language::Java,
        "maven:maven-clean-plugin",
        tv::maven::MAVEN_CLEAN_PLUGIN
    ),
    managed!(
        Language::Java,
        "maven:maven-compiler-plugin",
        tv::maven::MAVEN_COMPILER_PLUGIN
    ),
    managed!(Language::Java, "maven:maven-core", tv::maven::MAVEN_CORE),
    managed!(
        Language::Java,
        "maven:maven-deploy-plugin",
        tv::maven::MAVEN_DEPLOY_PLUGIN
    ),
    managed!(
        Language::Java,
        "maven:maven-enforcer-plugin",
        tv::maven::MAVEN_ENFORCER_PLUGIN
    ),
    managed!(Language::Java, "maven:maven-gpg-plugin", tv::maven::MAVEN_GPG_PLUGIN),
    managed!(
        Language::Java,
        "maven:maven-install-plugin",
        tv::maven::MAVEN_INSTALL_PLUGIN
    ),
    managed!(Language::Java, "maven:maven-jar-plugin", tv::maven::MAVEN_JAR_PLUGIN),
    managed!(
        Language::Java,
        "maven:maven-javadoc-plugin",
        tv::maven::MAVEN_JAVADOC_PLUGIN
    ),
    managed!(
        Language::Java,
        "maven:maven-resources-plugin",
        tv::maven::MAVEN_RESOURCES_PLUGIN
    ),
    managed!(Language::Java, "maven:maven-site-plugin", tv::maven::MAVEN_SITE_PLUGIN),
    managed!(
        Language::Java,
        "maven:maven-source-plugin",
        tv::maven::MAVEN_SOURCE_PLUGIN
    ),
    managed!(
        Language::Java,
        "maven:maven-surefire-plugin",
        tv::maven::MAVEN_SUREFIRE_PLUGIN
    ),
    managed!(
        Language::Java,
        "maven:versions-maven-plugin",
        tv::maven::VERSIONS_MAVEN_PLUGIN
    ),
    managed!(Language::Elixir, "cargo:ahash", tv::cargo::AHASH),
    managed!(Language::Elixir, "cargo:alloc-no-stdlib", tv::cargo::ALLOC_NO_STDLIB),
    managed!(Language::Elixir, "cargo:alloc-stdlib", tv::cargo::ALLOC_STDLIB),
    managed!(Language::Elixir, "cargo:async-trait", tv::cargo::ASYNC_TRAIT),
    managed!(
        Language::Elixir,
        "cargo:brotli-decompressor",
        tv::cargo::BROTLI_DECOMPRESSOR
    ),
    managed!(Language::Elixir, "cargo:futures-util", tv::cargo::FUTURES_UTIL),
    managed!(Language::Elixir, "cargo:rustler", tv::cargo::RUSTLER),
    managed!(Language::Elixir, "cargo:serde", tv::cargo::SERDE),
    managed!(Language::Elixir, "cargo:serde_json", tv::cargo::SERDE_JSON),
    managed!(Language::Elixir, "cargo:tokio", tv::cargo::TOKIO),
    managed!(Language::Elixir, "cargo:tracing", tv::cargo::TRACING),
    managed!(Language::Elixir, "hex:credo", tv::hex::CREDO),
    managed!(Language::Elixir, "hex:ex_doc", tv::hex::EX_DOC),
    managed!(Language::Elixir, "hex:jason", tv::hex::JASON),
    managed!(Language::Elixir, "hex:rustler", tv::hex::RUSTLER),
    managed!(
        Language::Elixir,
        "hex:rustler_precompiled",
        tv::hex::RUSTLER_PRECOMPILED
    ),
    managed!(Language::Ruby, "gem:rake-compiler", tv::gem::RAKE_COMPILER),
    // ~keep Two Ruby requirement arguments (`">= 0.9.130", "< 0.10"`), emitted verbatim after
    // the `gem`/`add_dependency` name — the gemspec DSL rejects the comma-joined single string
    // every other entry here is. An override must supply the same shape, quotes included.
    managed!(Language::Ruby, "gem:rb_sys", tv::gem::RB_SYS),
    managed!(Language::Ruby, "gem:rspec", tv::gem::RSPEC_SCAFFOLD),
    managed!(Language::Ruby, "gem:rubocop", tv::gem::RUBOCOP_SCAFFOLD),
    managed!(Language::Ruby, "gem:rubocop-performance", tv::gem::RUBOCOP_PERFORMANCE),
    managed!(Language::Ruby, "gem:rubocop-rspec", tv::gem::RUBOCOP_RSPEC_SCAFFOLD),
    managed!(Language::Ruby, "gem:sorbet-runtime", tv::gem::SORBET_RUNTIME),
    managed!(Language::Ruby, "gem:steep", tv::gem::STEEP),
    managed!(Language::R, "cargo:async-trait", tv::cargo::ASYNC_TRAIT),
    managed!(Language::R, "cargo:extendr-api", tv::cargo::EXTENDR_API),
    managed!(Language::R, "cargo:serde", tv::cargo::SERDE),
    managed!(Language::R, "cargo:serde_json", tv::cargo::SERDE_JSON),
    managed!(Language::R, "cargo:tokio", tv::cargo::TOKIO),
    managed!(Language::R, "cargo:tracing", tv::cargo::TRACING),
    managed!(Language::R, "cran:rextendr", tv::cran::REXTENDR),
];

/// The registry entries belonging to `language`'s `dependency_versions` table.
pub fn entries_for(language: Language) -> impl Iterator<Item = &'static ManagedDependency> {
    MANAGED_DEPENDENCIES
        .iter()
        .filter(move |entry| entry.language == language)
}

/// The entry `key` names in `language`'s table, or `None` when the key is not a managed
/// version for that language.
pub fn lookup(language: Language, key: &str) -> Option<&'static ManagedDependency> {
    entries_for(language).find(|entry| entry.key == key)
}

/// Every key valid under `[crates.<language>.dependency_versions]`, sorted.
pub fn keys_for(language: Language) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = entries_for(language).map(|entry| entry.key).collect();
    keys.sort_unstable();
    keys
}

/// The literal alef emits for `key`, honouring `overrides` when it declares one.
///
/// Returns `None` when `key` is not registered for `language` — a caller passing a key it did
/// not take from this module has a bug, and silently falling back to an empty string would
/// emit a broken manifest.
pub fn resolve<'a>(
    language: Language,
    key: &str,
    overrides: &'a std::collections::BTreeMap<String, String>,
) -> Option<&'a str> {
    let entry = lookup(language, key)?;
    Some(overrides.get(key).map_or(entry.default, String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_unique_within_its_language() {
        for language in MANAGED_LANGUAGES {
            let keys = keys_for(language);
            let mut deduped = keys.clone();
            deduped.dedup();
            assert_eq!(
                keys, deduped,
                "duplicate key in {language}'s dependency_versions registry"
            );
        }
    }

    #[test]
    fn every_language_has_entries_and_manifests() {
        for language in MANAGED_LANGUAGES {
            assert!(
                entries_for(language).next().is_some(),
                "{language} is listed as managed but has no entries"
            );
            assert!(
                manifests_for(language).is_some(),
                "{language} is listed as managed but names no manifest"
            );
        }
    }

    #[test]
    fn every_entry_belongs_to_a_managed_language() {
        for entry in MANAGED_DEPENDENCIES {
            assert!(
                MANAGED_LANGUAGES.contains(&entry.language),
                "`{}` is registered for {}, which MANAGED_LANGUAGES does not list",
                entry.key,
                entry.language
            );
        }
    }

    #[test]
    fn no_default_is_empty() {
        for entry in MANAGED_DEPENDENCIES {
            assert!(
                !entry.default.is_empty(),
                "`{}` ({}) resolves to an empty literal",
                entry.key,
                entry.const_name
            );
        }
    }

    /// The same package name in two ecosystems is two entries, and the qualified keys keep
    /// them apart. Elixir emits both, so a single unqualified `rustler` key would be
    /// ambiguous exactly where it matters.
    #[test]
    fn ecosystem_qualified_keys_keep_same_named_packages_apart() {
        let cargo = lookup(Language::Elixir, "cargo:rustler").expect("cargo:rustler is registered");
        let hex = lookup(Language::Elixir, "hex:rustler").expect("hex:rustler is registered");
        assert_ne!(
            cargo.default, hex.default,
            "the two rustler entries must resolve to different literals"
        );
    }

    #[test]
    fn an_override_wins_over_the_default() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("cargo:pyo3".to_string(), "9.9.9-sentinel".to_string());
        assert_eq!(
            resolve(Language::Python, "cargo:pyo3", &overrides),
            Some("9.9.9-sentinel")
        );
        assert_eq!(
            resolve(Language::Python, "cargo:serde", &overrides),
            Some(tv::cargo::SERDE)
        );
    }

    /// Scoping is per language: a key registered for one language must not resolve under
    /// another, or an unknown-key error could be excused by an unrelated table.
    #[test]
    fn a_key_does_not_leak_across_languages() {
        assert!(lookup(Language::Python, "hex:credo").is_none());
        assert!(lookup(Language::Ruby, "maven:junit").is_none());
        assert!(lookup(Language::Java, "cargo:serde").is_none());
    }
}
