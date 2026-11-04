//! Docs-stage dispatch for `alef all`, split out of `all_commands.rs` so
//! wiring `--skip-snippet-validation` through does not push that file over
//! the 1,000-line file-modularization cap.

use std::path::Path;

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

pub(crate) fn generate(
    skip_snippet_validation: bool,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    workspace_root: &Path,
    config_path: &Path,
) -> (Vec<GeneratedFile>, anyhow::Result<()>) {
    crate::docs::generate_docs_stage_for_manifest(
        api,
        config,
        languages,
        None,
        workspace_root,
        config_path,
        !skip_snippet_validation,
    )
}
