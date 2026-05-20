//! `.gitkeep` placeholder files inside `src/main/jniLibs/<abi>/` so the
//! AAR build picks up the directory layout even before any `.so` is copied
//! in. The release pipeline writes the real `lib<crate>.so` files here.

use std::path::Path;

use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;

use crate::naming::abis;

/// Emit one `.gitkeep` per ABI directory.
///
/// The file content is a single newline so end-of-file-fixer hooks treat
/// the file as already-terminated and leave it untouched (an empty file
/// would be rewritten to add a trailing newline on every commit).
pub fn emit(config: &ResolvedCrateConfig, aar_root: &Path) -> Vec<GeneratedFile> {
    abis(config)
        .into_iter()
        .map(|abi| GeneratedFile {
            path: aar_root.join("src/main/jniLibs").join(abi).join(".gitkeep"),
            content: "\n".to_owned(),
            generated_header: false,
        })
        .collect()
}
