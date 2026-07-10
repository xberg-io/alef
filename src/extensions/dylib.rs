//! Dynamic library loader for alef extensions (feature `dylib-loader`).
//!
//! # Status
//!
//! The public surface (config block, feature flag) is stable.
//! The concrete loader is deferred; see the Unreleased section in `CHANGELOG.md`.

use crate::core::extension::Extension;
use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

/// One `[[extensions.dylib]]` block in `alef.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct DylibBlock {
    pub path: PathBuf,
}

/// Load dynamic extension plugins declared in `[[extensions.dylib]]`.
///
/// Currently a stub — returns an empty list.
pub fn load_dylib_extensions(_blocks: &[DylibBlock]) -> Result<Vec<Box<dyn Extension>>> {
    // ~keep TODO: implement libloading-based loader.
    Ok(vec![])
}
