use super::{
    MATERIALIZED_TRIO_LEN, emit_swift_bridge_files_from_output, find_swift_bridge_out_dir_at_target,
    materialize_from_build_output, swift_bridge_target_root, umbrella_header, write_materialized_files,
};
use crate::core::backend::CompilePolicy;
use anyhow::Context as _;
use std::path::{Path, PathBuf};

/// Source-only generation preserves populated headers without hiding incomplete fresh output
/// (alef #776). ~keep
pub(crate) fn materialize_from_build_output_for_generation(
    binding_crate_name: &str,
    package_root: &Path,
    project_root: &Path,
    compile: CompilePolicy,
) -> anyhow::Result<()> {
    if compile != CompilePolicy::Skipped {
        return materialize_from_build_output(binding_crate_name, package_root, project_root);
    }
    let target =
        swift_bridge_target_root().unwrap_or_else(|| crate::core::cargo_target_dir::for_workspace_root(project_root));
    if let Some(out) = find_swift_bridge_out_dir_at_target(binding_crate_name, &target) {
        return materialize_known_output(binding_crate_name, package_root, project_root, &out);
    }
    if let Some(out) = find_existing_swift_bridge_out_dir(binding_crate_name, &target)? {
        anyhow::bail!(
            "no usable swift-bridge build output for '{binding_crate_name}': existing output path {} \
             has no discoverable SwiftBridgeCore.swift. The committed files under {} were left unchanged",
            out.display(),
            package_root.display()
        );
    }
    if has_populated_header(package_root)? {
        tracing::info!(
            "Preserved committed swift-bridge files for '{binding_crate_name}': compilation was skipped \
             and no build output exists"
        );
        return Ok(());
    }
    materialize_from_build_output(binding_crate_name, package_root, project_root)
}

fn materialize_known_output(
    binding_crate_name: &str,
    package_root: &Path,
    project_root: &Path,
    out: &Path,
) -> anyhow::Result<()> {
    let files = emit_swift_bridge_files_from_output("", binding_crate_name, package_root, Some(out.to_path_buf()))?
        .with_context(|| {
            format!(
                "no usable swift-bridge build output for '{binding_crate_name}': {} lacks the complete four-file set",
                out.display()
            )
        })?;
    let written = files.len();
    anyhow::ensure!(
        written == MATERIALIZED_TRIO_LEN,
        "expected {MATERIALIZED_TRIO_LEN} swift-bridge files for '{binding_crate_name}', got {written}"
    );
    write_materialized_files(files, project_root)
        .with_context(|| format!("failed to write swift-bridge files for '{binding_crate_name}'"))?;
    tracing::info!("Re-materialized {written} swift-bridge files for '{binding_crate_name}' from fresh build output");
    Ok(())
}

fn has_populated_header(package_root: &Path) -> anyhow::Result<bool> {
    let header_path = package_root.join("Sources/RustBridgeC/RustBridgeC.h");
    match std::fs::read_to_string(&header_path) {
        Ok(header) => Ok(umbrella_header::is_populated(&header)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect preserved Swift bridge header {}",
                header_path.display()
            )
        }),
    }
}

fn find_existing_swift_bridge_out_dir(binding_crate_name: &str, target: &Path) -> anyhow::Result<Option<PathBuf>> {
    let crate_prefix = format!("{binding_crate_name}-");
    for profile in ["release", "debug"] {
        let build_dir = target.join(profile).join("build");
        let entries = match std::fs::read_dir(&build_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect Swift bridge build directory {}", build_dir.display())
                });
            }
        };
        for entry in entries {
            let entry = entry
                .with_context(|| format!("failed to read Swift bridge build directory {}", build_dir.display()))?;
            if !entry.file_name().to_string_lossy().starts_with(&crate_prefix) {
                continue;
            }
            let out = entry.path().join("out");
            if out
                .try_exists()
                .with_context(|| format!("failed to inspect Swift bridge output path {}", out.display()))?
            {
                return Ok(Some(out));
            }
        }
    }
    Ok(None)
}
