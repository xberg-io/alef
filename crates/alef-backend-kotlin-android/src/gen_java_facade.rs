//! Re-emit the Java facade into the AAR's `src/main/java/` so the AAR is
//! self-contained (no runtime dependency on `packages/java/`).
//!
//! Strategy: invoke [`alef_backend_java::JavaBackend::generate_bindings`]
//! unmodified, then rewrite every emitted path so it lives under
//! `<aar_root>/src/main/java/...` regardless of where the Java backend
//! originally targeted. The two trees are otherwise identical — same
//! package hierarchy, same files, same content.

use std::path::{Path, PathBuf};

use alef_core::backend::{Backend, GeneratedFile};
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::ApiSurface;

use crate::naming::java_package_path;

/// Returns every Java source file the AAR needs to ship, with paths rewritten
/// to live under `<aar_root>/src/main/java/<java_pkg_path>/`.
pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig, aar_root: &Path) -> anyhow::Result<Vec<GeneratedFile>> {
    let backend = alef_backend_java::JavaBackend;
    let files = backend.generate_bindings(api, config)?;

    let java_pkg_path = java_package_path(config);
    let java_pkg_segment = PathBuf::from(&java_pkg_path);
    let target_root = aar_root.join("src/main/java").join(&java_pkg_segment);

    // The Java backend emits files at <java_output>/<package_path>/<file>.java
    // We need to strip everything up to and including <package_path>, then
    // re-anchor under <aar_root>/src/main/java/<package_path>/.
    let mut rewritten = Vec::with_capacity(files.len());
    for file in files {
        let new_path = rewrite_path(&file.path, &java_pkg_segment, &target_root)?;
        rewritten.push(GeneratedFile {
            path: new_path,
            content: file.content,
            generated_header: file.generated_header,
        });
    }
    Ok(rewritten)
}

fn rewrite_path(original: &Path, java_pkg_segment: &Path, target_root: &Path) -> anyhow::Result<PathBuf> {
    // Walk components and find the position of the java package segment.
    // Once found, everything from that point is preserved.
    let original_components: Vec<_> = original.components().collect();
    let pkg_components: Vec<_> = java_pkg_segment.components().collect();
    if pkg_components.is_empty() {
        anyhow::bail!("java package path is empty");
    }

    // Find the start index where original matches pkg_components contiguously.
    let mut match_start: Option<usize> = None;
    for start in 0..original_components.len() {
        if start + pkg_components.len() > original_components.len() {
            break;
        }
        let slice = &original_components[start..start + pkg_components.len()];
        if slice.iter().zip(pkg_components.iter()).all(|(a, b)| a == b) {
            match_start = Some(start);
            break;
        }
    }

    // Suffix is everything after the matched package segment.
    let suffix_start = match match_start {
        Some(start) => start + pkg_components.len(),
        None => {
            // Fall back to just the file name if the package path isn't on the
            // emitted path (defensive — should not happen for the Java backend).
            return Ok(target_root.join(original.file_name().unwrap_or_default()));
        }
    };

    let mut rewritten = target_root.to_path_buf();
    for comp in &original_components[suffix_start..] {
        rewritten.push(comp);
    }
    Ok(rewritten)
}
