use super::PythonHostAliases;
use crate::core::config::NewAlefConfig;
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceAliases {
    #[serde(default)]
    crates: BTreeMap<String, PythonHostAliases>,
}

pub(super) fn parse(raw: &toml::Value) -> Result<WorkspaceAliases> {
    raw.clone()
        .try_into()
        .context("invalid crate-scoped extensions.python_host_aliases configuration")
}

pub(crate) fn read_for_crate(path: &Path, name: &str) -> Result<Option<PythonHostAliases>> {
    let raw = match crate::core::extension::read_extension_config(path, "python_host_aliases") {
        Ok(raw) => raw,
        // Resolved-config callers can generate without a manifest; only this optional lookup
        // permits its absence. CLI config loading and the generic extension reader stay strict. ~keep
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|source| source.kind() == std::io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    let aliases = parse(&raw)?;
    let source = std::fs::read_to_string(path).context("read workspace for Python host alias crate validation")?;
    let config: NewAlefConfig =
        toml::from_str(&source).context("parse workspace for Python host alias crate validation")?;
    let names: BTreeSet<_> = config.crates.iter().map(|krate| krate.name.as_str()).collect();
    for key in aliases.crates.keys() {
        ensure!(
            names.contains(key.as_str()),
            "Python host aliases: unknown crate `{key}`"
        );
    }
    Ok(aliases.crates.get(name).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};

    fn fixture(alias_crate: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let config = format!(
            "[workspace]\nlanguages=['python']\n\
             [[crates]]\nname='a'\nsources=[]\n\
             [[crates]]\nname='b'\nsources=[]\n\
             [extensions.python_host_aliases.crates.{alias_crate}.traits.Counter]\nchecked_count='count'\n"
        );
        std::fs::write(root.path().join("alef.toml"), config).unwrap();
        root
    }

    #[test]
    fn selects_only_the_current_source_crate() {
        let root = fixture("a");
        let path = root.path().join("alef.toml");
        assert!(read_for_crate(&path, "a").unwrap().is_some());
        assert!(read_for_crate(&path, "b").unwrap().is_none());
    }

    #[test]
    fn rejects_unknown_crates_even_when_reading_an_unrelated_crate() {
        let root = fixture("typo");
        let error = read_for_crate(&root.path().join("alef.toml"), "b").unwrap_err();
        assert!(error.to_string().contains("unknown crate `typo`"));
    }

    #[test]
    fn rejects_unscoped_trait_tables() {
        let raw = toml::from_str("[traits.Counter]\nchecked_count='count'").unwrap();
        let error = parse(&raw).unwrap_err();
        assert!(format!("{error:#}").contains("unknown field"));
    }

    fn generated_python(root: &Path, config_path: &Path) -> Vec<(std::path::PathBuf, String)> {
        let source = root.join("lib.rs");
        std::fs::write(
            &source,
            include_str!("../../../tests/fixtures/python_host_aliases/core.rs"),
        )
        .unwrap();
        let api = crate::extract::extractor::extract(&[&source], "a", "0.1.0", None).unwrap();
        let config = ResolvedCrateConfig {
            name: "a".into(),
            core_import: Some("a".into()),
            error_type: Some("Error".into()),
            error_constructor: Some("Error::from({msg})".into()),
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "Counter".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut output =
            crate::cli::pipeline::generate(&api, &config, &[Language::Python], true, config_path, false).unwrap();
        assert_eq!(output.len(), 1, "must inspect one actual Python generation result");
        let (language, files) = output.pop().unwrap();
        assert_eq!(language, Language::Python);
        assert!(!files.is_empty(), "Python generation must emit binding files");
        files.into_iter().map(|file| (file.path, file.content)).collect()
    }

    #[test]
    fn missing_optional_manifest_preserves_resolved_python_generation() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing-alef.toml");
        assert!(!missing.exists());
        let actual = generated_python(root.path(), &missing);
        let empty = root.path().join("alef.toml");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(actual, generated_python(root.path(), &empty));
        let code: String = actual.into_iter().map(|(_, content)| content).collect();
        assert!(code.contains("pub struct PyCounterBridge"));
        assert!(code.contains("struct PyCounterBridgeDefaultCheckedCount"));
        assert!(!code.contains("match obj.getattr(\"checked_count\")"));
        assert!(!missing.exists(), "in-memory generation must not create a manifest");
    }

    #[test]
    fn configured_manifest_still_emits_alias_in_resolved_generation() {
        let root = fixture("a");
        let code: String = generated_python(root.path(), &root.path().join("alef.toml"))
            .into_iter()
            .map(|(_, content)| content)
            .collect();
        assert!(code.contains("pub struct PyCounterBridge"));
        assert!(code.contains("match obj.getattr(\"checked_count\")"));
        assert!(code.contains("obj.getattr(\"count\")?"));
        assert!(!code.contains("struct PyCounterBridgeDefaultCheckedCount"));
    }

    #[test]
    fn malformed_existing_manifest_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("alef.toml");
        std::fs::write(&path, "[extensions.").unwrap();
        let error = read_for_crate(&path, "a").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to parse alef.toml for extension config")
        );
        assert!(error.downcast_ref::<toml::de::Error>().is_some());
    }

    #[test]
    fn non_missing_manifest_io_error_is_propagated() {
        let root = tempfile::tempdir().unwrap();
        let expected = std::fs::read_to_string(root.path()).unwrap_err().kind();
        assert_ne!(expected, std::io::ErrorKind::NotFound);
        let error = read_for_crate(root.path(), "a").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to read alef.toml for extension config")
        );
        assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), expected);
    }

    #[test]
    fn primary_cli_and_generic_extension_readers_still_require_a_manifest() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing-alef.toml");
        let errors = [
            crate::bin_cli::helpers::load_config(&path).unwrap_err(),
            crate::core::extension::read_extension_config(&path, "python_host_aliases").unwrap_err(),
        ];
        for error in errors {
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::NotFound
            );
        }
    }
}
