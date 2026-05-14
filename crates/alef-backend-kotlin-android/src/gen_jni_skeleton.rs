//! `.gitkeep` placeholder files inside `src/main/jniLibs/<abi>/` so the
//! AAR build picks up the directory layout even before any `.so` is copied
//! in. The release pipeline writes the real `lib<crate>.so` files here.

use std::path::Path;

use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;

use crate::naming::abis;

/// Emit one `.gitkeep` per ABI directory.
pub fn emit(config: &ResolvedCrateConfig, aar_root: &Path) -> Vec<GeneratedFile> {
    abis(config)
        .into_iter()
        .map(|abi| GeneratedFile {
            path: aar_root.join("src/main/jniLibs").join(abi).join(".gitkeep"),
            content: String::new(),
            generated_header: false,
        })
        .collect()
}
