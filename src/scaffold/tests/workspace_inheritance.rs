//! Regression coverage for the workspace-package-inheritance membership check.
//!
//! A generated crate can only emit `<field>.workspace = true` when it can actually
//! *reach* a `[workspace.package]` that defines the field: either it is a member of the
//! workspace rooted at `config.workspace_root` (i.e. not named in that root's
//! `[workspace] exclude`), or it self-hosts its own `[workspace.package]`. Blindly
//! trusting the root regardless of exclusion produces a manifest `cargo metadata` can
//! never resolve for any crate excluded from the root workspace -- exactly the shape of
//! the Elixir NIF and Ruby native-extension crates, which are excluded so their own
//! toolchain, not the root workspace's resolver, builds them.

use super::*;
use std::fs;

fn write_root_cargo_toml(root: &std::path::Path, body: &str) {
    fs::write(root.join("Cargo.toml"), body).expect("write root Cargo.toml");
}

/// Acceptance test: a crate directory named in the root workspace's `[workspace]
/// exclude` list, whose own manifest does not exist yet (so it cannot self-host a
/// `[workspace.package]` either), must keep every `[package]` field as a literal --
/// never `<field>.workspace = true` -- since it can reach no `[workspace.package]` at
/// all.
#[test]
fn excluded_crate_with_no_reachable_workspace_package_keeps_literal_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_root_cargo_toml(
        &root,
        "[workspace]\n\
         members = [\"crates/*\"]\n\
         exclude = [\"packages/elixir/native/my_lib_nif\"]\n\n\
         [workspace.package]\n\
         version = \"9.9.9\"\n\
         license = \"Apache-2.0\"\n\
         license-file = \"LICENSE\"\n\
         authors = [\"Ignored\"]\n\
         keywords = [\"ignored\"]\n",
    );

    let mut config = test_config_from_toml("");
    config.workspace_root = Some(root.clone());
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let cargo_toml = language_files(&all_files)
        .into_iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Elixir NIF Cargo.toml must be generated");

    assert_eq!(
        cargo_toml.path,
        std::path::Path::new("packages/elixir/native/my_lib_nif/Cargo.toml"),
        "sanity: must be the excluded NIF crate's own manifest"
    );

    for literal in ["version = \"0.1.0\"", "license = \"MIT\""] {
        assert!(
            cargo_toml.content.contains(literal),
            "excluded crate with no reachable [workspace.package] must emit the literal `{literal}`, got:\n{}",
            cargo_toml.content
        );
    }
    for inherited in [
        "version.workspace = true",
        "license.workspace = true",
        "license-file.workspace = true",
        "authors.workspace = true",
        "keywords.workspace = true",
    ] {
        assert!(
            !cargo_toml.content.contains(inherited),
            "excluded crate must never emit `{inherited}` -- it cannot reach the root's \
             [workspace.package] and never got the chance to be checked, got:\n{}",
            cargo_toml.content
        );
    }
}

/// Negative control: without this, a fix that never inherits (always literal) would
/// pass the acceptance test above while silently breaking every legitimate excluded
/// crate that deliberately self-hosts its own `[workspace.package]`. A crate directory
/// excluded from the root workspace, but whose own pre-existing manifest declares
/// `[workspace]` with a `[workspace.package]` defining `version`, must inherit that
/// field -- while a field the self-hosted `[workspace.package]` does NOT define
/// (`license`) still falls back to a literal, proving the check is per-field, not
/// per-crate.
#[test]
fn self_hosting_excluded_crate_inherits_only_the_fields_its_own_workspace_package_defines() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_root_cargo_toml(
        &root,
        "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"packages/elixir/native/my_lib_nif\"]\n",
    );

    let native_dir = root.join("packages/elixir/native/my_lib_nif");
    fs::create_dir_all(&native_dir).expect("create native crate dir");
    fs::write(
        native_dir.join("Cargo.toml"),
        "[package]\nname = \"my_lib_nif\"\nversion.workspace = true\n\n\
         [workspace]\n\n[workspace.package]\nversion = \"0.1.0\"\nauthors = [\"Maintainer\"]\n",
    )
    .expect("write self-hosting native Cargo.toml");

    let mut config = test_config_from_toml("");
    config.workspace_root = Some(root.clone());
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let cargo_toml = language_files(&all_files)
        .into_iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Elixir NIF Cargo.toml must be generated");

    assert!(
        cargo_toml.content.contains("version.workspace = true"),
        "a crate excluded from the root workspace but self-hosting its own \
         [workspace.package].version must still inherit it, got:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("license = \"MIT\""),
        "the self-hosted [workspace.package] defines no `license`, so this field must still \
         fall back to a literal, got:\n{}",
        cargo_toml.content
    );
    assert!(
        !cargo_toml.content.contains("license.workspace = true"),
        "must not emit license.workspace = true when neither the root nor the self-hosted \
         workspace defines it, got:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("authors.workspace = true"),
        "the self-hosted workspace defines authors, so the excluded crate must inherit them, got:\n{}",
        cargo_toml.content
    );
    assert!(
        !cargo_toml.content.contains("license-file.workspace = true"),
        "the self-hosted workspace does not define license-file, so the excluded crate must not inherit it, got:\n{}",
        cargo_toml.content
    );
}

/// A real workspace member (not named in the root's `[workspace] exclude`) must
/// continue to inherit from the root's `[workspace.package]` exactly as before this
/// membership check existed.
#[test]
fn real_workspace_member_still_inherits_from_the_root_workspace_package() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_root_cargo_toml(
        &root,
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"4.2.0\"\nlicense = \"MIT\"\nlicense-file = \"LICENSE\"\nauthors = [\"Maintainer\"]\n",
    );

    let mut config = test_config_from_toml("");
    config.workspace_root = Some(root.clone());
    config.name = "my-lib".to_string();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let cargo_toml = language_files(&all_files)
        .into_iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("FFI Cargo.toml must be generated");

    assert_eq!(
        cargo_toml.path,
        std::path::Path::new("crates/my-lib-ffi/Cargo.toml"),
        "sanity: must be the member FFI crate's own manifest"
    );
    for inherited in [
        "version.workspace = true",
        "license.workspace = true",
        "authors.workspace = true",
    ] {
        assert!(
            cargo_toml.content.contains(inherited),
            "a real workspace member must inherit `{inherited}` from the root's \
             [workspace.package], got:\n{}",
            cargo_toml.content
        );
    }
    assert!(
        !cargo_toml.content.contains("license-file.workspace = true"),
        "a workspace declaring both `license` and `license-file` must emit only the SPDX \
         `license` field -- Cargo warns that only one of the two is necessary, got:\n{}",
        cargo_toml.content
    );
}

/// When a workspace declares `license-file` but no SPDX `license`, the crate must
/// inherit `license-file.workspace = true` (the sole field available) rather than
/// emitting nothing.
#[test]
fn workspace_with_only_license_file_inherits_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    write_root_cargo_toml(
        &root,
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"4.2.0\"\nlicense-file = \"LICENSE\"\nauthors = [\"Maintainer\"]\n",
    );

    let mut config = test_config_from_toml("");
    config.workspace_root = Some(root.clone());
    config.name = "my-lib".to_string();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let cargo_toml = language_files(&all_files)
        .into_iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("FFI Cargo.toml must be generated");

    assert!(
        cargo_toml.content.contains("license-file.workspace = true"),
        "a workspace defining only `license-file` must still inherit it, got:\n{}",
        cargo_toml.content
    );
    assert!(
        !cargo_toml.content.contains("license.workspace = true"),
        "no SPDX `license` is defined, so `license.workspace = true` must not be emitted, got:\n{}",
        cargo_toml.content
    );
}
