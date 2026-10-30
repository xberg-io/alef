use anyhow::{Context, Result};

use crate::core::backend::GeneratedFile;
use crate::core::config::Language;

pub(super) fn inputs(languages: &[Language], files: &[GeneratedFile]) -> Result<Vec<u8>> {
    let mut selected: Vec<_> = languages.iter().map(ToString::to_string).collect();
    selected.sort_unstable();
    selected.dedup();
    let mut outputs: Vec<_> = files.iter().map(|file| (&file.path, &file.content)).collect();
    outputs.sort_unstable();
    serde_json::to_vec(&(selected, outputs)).context("Failed to encode README stage cache inputs")
}
