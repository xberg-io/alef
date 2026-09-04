//! Dynamic library loader for alef extensions (feature `dylib-loader`).
//!
//! # Status
//!
//! The public surface (config block, feature flag, and factory symbol) is stable.

use crate::core::extension::Extension;
use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use std::path::Path;
use std::path::PathBuf;

/// One `[[extensions.dylib]]` block in `alef.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct DylibBlock {
    pub path: PathBuf,
}

#[allow(improper_ctypes_definitions)]
type ExtensionFactory = unsafe extern "C" fn() -> Box<dyn Extension>;

impl DylibBlock {
    fn with_base_dir(self, base_dir: &Path) -> Self {
        if self.path.is_absolute() {
            return self;
        }

        Self {
            path: base_dir.join(self.path),
        }
    }
}

/// Read `[[extensions.dylib]]` blocks from an `alef.toml` file.
pub fn read_dylib_blocks(config_path: &Path) -> Result<Vec<DylibBlock>> {
    // An absent config file declares no dylib extensions, exactly as an `alef.toml` carrying no
    // `[extensions.dylib]` section does -- the two are the same "nothing declared" answer and
    // must not diverge. This runs from `run_with_extensions` BEFORE command dispatch, so
    // propagating a missing file here aborts every command with `ExitCode::FAILURE` whenever the
    // working directory has no `alef.toml` -- including commands that neither need one nor ever
    // read it. Only a file that exists but cannot be read or parsed is a real error. ~keep
    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let Some(raw) = crate::core::extension::read_extension_config(config_path, "dylib")? else {
        return Ok(Vec::new());
    };

    let blocks = match raw {
        toml::Value::Array(_) => raw
            .try_into::<Vec<DylibBlock>>()
            .context("failed to parse [[extensions.dylib]] blocks")?,
        toml::Value::Table(_) => vec![
            raw.try_into::<DylibBlock>()
                .context("failed to parse [extensions.dylib] block")?,
        ],
        other => bail!(
            "extensions.dylib must be a table or array of tables, got {}",
            other.type_str()
        ),
    };

    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(blocks.into_iter().map(|block| block.with_base_dir(base_dir)).collect())
}

/// Load dynamic extension plugins declared in `[[extensions.dylib]]`.
pub fn load_dylib_extensions(blocks: &[DylibBlock]) -> Result<Vec<Box<dyn Extension>>> {
    let mut extensions = Vec::with_capacity(blocks.len());

    for block in blocks {
        // SAFETY: loading arbitrary dylibs is inherently unsafe. The caller opts
        let library = unsafe { libloading::Library::new(&block.path) }
            .with_context(|| format!("failed to load extension dylib {}", block.path.display()))?;

        let extension = {
            // SAFETY: dynamic extensions must export the documented
            let factory: libloading::Symbol<'_, ExtensionFactory> = unsafe { library.get(b"alef_extension_factory") }
                .with_context(|| {
                format!(
                    "failed to load symbol alef_extension_factory from {}",
                    block.path.display()
                )
            })?;

            // SAFETY: the symbol type above is the documented factory ABI.
            unsafe { factory() }
        };

        Box::leak(Box::new(library));
        extensions.push(extension);
    }

    Ok(extensions)
}

/// Load all dynamic extensions declared in an `alef.toml` file.
pub fn load_dylib_extensions_from_config(config_path: &Path) -> Result<Vec<Box<dyn Extension>>> {
    let blocks = read_dylib_blocks(config_path)?;
    load_dylib_extensions(&blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_dylib_blocks_resolves_relative_paths() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let config_path = dir.path().join("alef.toml");
        std::fs::write(
            &config_path,
            r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[extensions.dylib]]
path = "target/debug/libsample_extension.dylib"
"#,
        )
        .expect("write config");

        let blocks = read_dylib_blocks(&config_path).expect("read dylib blocks");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].path,
            dir.path().join("target/debug/libsample_extension.dylib")
        );
    }

    /// `run_with_extensions` loads dylib blocks before dispatching any command, and returns
    /// `ExitCode::FAILURE` on error. Propagating a missing `alef.toml` from here therefore aborted
    /// EVERY command under `--features dylib-loader` whenever the working directory had no config
    /// -- observed as four `cli_snippets_missing_directories` tests failing under `--all-features`
    /// while passing under the default feature set, with the real cause hidden behind
    /// `failed to read alef.toml for extension config`. An absent file declares no extensions.
    #[test]
    fn read_dylib_blocks_treats_an_absent_config_file_as_no_extensions() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let config_path = dir.path().join("alef.toml");
        assert!(!config_path.exists(), "the fixture must not create the config file");

        let blocks = read_dylib_blocks(&config_path).expect("an absent alef.toml must not be an error");

        assert!(blocks.is_empty(), "an absent config file declares no dylib extensions");
    }

    #[test]
    fn read_dylib_blocks_still_errors_on_a_config_file_that_exists_but_is_unparseable() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let config_path = dir.path().join("alef.toml");
        std::fs::write(&config_path, "this is not = = valid toml").expect("write config");

        let error = read_dylib_blocks(&config_path).expect_err("a malformed config must still fail");

        assert!(
            format!("{error:#}").contains("failed to parse alef.toml for extension config"),
            "a present-but-broken file stays a hard error, unlike an absent one: {error:#}"
        );
    }
}
