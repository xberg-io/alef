//! `.gitkeep` placeholder files inside `src/main/jniLibs/<abi>/` and
//! `src/test/resources/host-jni/<platform>/` so the AAR build and JVM test
//! harness pick up the directory layout even before any `.so`/`.dylib`/`.dll`
//! is copied in. The release pipeline writes Android-ABI `.so` files here;
//! the build.gradle.kts `copyHostJni` task populates host-platform binaries.

use std::path::Path;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;

use crate::backends::kotlin_android::naming::{HOST_PLATFORMS, abis};

/// Emit one `.gitkeep` per ABI directory for Android AAR libraries, plus
/// one per host platform for JVM unit test resources.
///
/// The file content is empty. The canonical pre-commit `end-of-file-fixer`
/// hook truncates whitespace-only files (including a lone `"\n"`) to zero
/// bytes and leaves empty files alone, so emitting `String::new()` is the
/// stable resolution. Emitting `"\n"` triggers an infinite ping-pong
/// between alef regen (writes `"\n"`) and prek autofix (truncates to `""`).
pub fn emit(config: &ResolvedCrateConfig, aar_root: &Path) -> Vec<GeneratedFile> {
    let mut files = abis(config)
        .into_iter()
        .map(|abi| GeneratedFile {
            path: aar_root.join("src/main/jniLibs").join(abi).join(".gitkeep"),
            content: String::new(),
            generated_header: false,
        })
        .collect::<Vec<_>>();

    for platform in HOST_PLATFORMS {
        files.push(GeneratedFile {
            path: aar_root
                .join("src/test/resources/host-jni")
                .join(platform)
                .join(".gitkeep"),
            content: String::new(),
            generated_header: false,
        });
    }

    files
}
