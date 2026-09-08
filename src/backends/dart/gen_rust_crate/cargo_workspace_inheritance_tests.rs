//! `version` inheritance for the Dart bridge crate must follow the crate's actual workspace
//! membership, not merely whether a workspace exists.
//!
//! Consumers routinely name `packages/dart/rust` in the root `[workspace] exclude` so
//! flutter_rust_bridge builds it with its own resolver rather than the workspace's. An emitter
//! that asks only "is there a `[workspace.package] version`?" then writes
//! `version.workspace = true` into a manifest that can never resolve it: cargo fails to parse
//! the manifest outright ("error inheriting `version` from workspace root manifest") and
//! `flutter_rust_bridge_codegen` dies with it, taking the whole dart post-build stage down.
//! ~keep

use super::cargo::emit_cargo_toml;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;

fn config_rooted_at(root: &std::path::Path) -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        workspace_root: Some(root.to_path_buf()),
        ..Default::default()
    }
}

fn write_root_manifest(root: &std::path::Path, exclude_dart: bool) {
    let exclude = if exclude_dart {
        "exclude = [\"packages/dart/rust\"]\n"
    } else {
        ""
    };
    std::fs::write(
        root.join("Cargo.toml"),
        format!("[workspace]\nmembers = [\"crates/core\"]\n{exclude}\n[workspace.package]\nversion = \"1.2.3\"\n"),
    )
    .expect("write root manifest");
}

/// The regression: excluded from the workspace, so the literal must survive.
#[test]
fn an_excluded_dart_crate_gets_a_literal_version_not_workspace_inheritance() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_root_manifest(dir.path(), true);
    let config = config_rooted_at(dir.path());

    let file = emit_cargo_toml("packages/dart/rust", &ApiSurface::default(), &config, "sample_lib");

    assert!(
        !file.content.contains("version.workspace = true"),
        "an excluded crate cannot inherit from [workspace.package]; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains("version = \""),
        "expected a literal version for the excluded crate; got:\n{}",
        file.content
    );
}

/// The control: a genuine member still inherits, so the fix does not regress the case the
/// inheritance was introduced for (a literal silently falling behind a workspace-wide bump).
#[test]
fn a_member_dart_crate_still_inherits_the_workspace_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_root_manifest(dir.path(), false);
    let config = config_rooted_at(dir.path());

    let file = emit_cargo_toml("packages/dart/rust", &ApiSurface::default(), &config, "sample_lib");

    assert!(
        file.content.contains("version.workspace = true"),
        "a workspace member must still inherit its version; got:\n{}",
        file.content
    );
}
