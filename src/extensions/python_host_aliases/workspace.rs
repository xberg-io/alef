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
    let Some(raw) = crate::core::extension::read_extension_config(path, "python_host_aliases")? else {
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
}
