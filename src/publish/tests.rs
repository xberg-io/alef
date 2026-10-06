use super::*;
use crate::core::config::output::StringOrVec;
#[cfg(not(target_os = "windows"))]
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn make_temp_marker_file() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let marker = temp_dir.path().join("marker.txt");
    (temp_dir, marker)
}

pub(super) fn toml_basic_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub(super) fn toml_path(path: &Path) -> String {
    toml_basic_string(path.to_string_lossy().as_ref())
}

/// Regression test for alef#368: `alef publish`'s Node build command has the same
/// napi-rs package-name resolution hazard as the default `alef build` command
/// (`cli::pipeline::commands::build::build_command::build_command_for`) -- napi reads
/// `package.json` from the process cwd unless told otherwise, and this command always runs
/// from the repo root, not the binding crate directory. `--package-json-path` must name the
/// binding crate's own manifest explicitly. ~keep
#[test]
fn node_publish_command_points_napi_at_the_crate_local_package_json() {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);

    let command = build_command_for_lang(Language::Node, &config, None, false);

    assert!(
        command.contains("--package-json-path 'crates/sample-lib-node/package.json'"),
        "napi build must be told explicitly which package.json names the binding crate, \
         rather than letting it default to the repo root's: {command}"
    );
}

#[test]
fn generated_publish_command_quotes_derived_crate_paths() {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.output]
node = "crates/evil; touch ALEF_PUBLISH_PWNED; #/src"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let command = build_command_for_lang(Language::Node, &config, None, false);

    assert!(
        command.contains("'crates/evil; touch ALEF_PUBLISH_PWNED; #/Cargo.toml'"),
        "got: {command}"
    );
}

#[test]
#[cfg(not(target_os = "windows"))]
fn test_run_publish_hooks_runs_before_only() {
    let (_temp_dir, marker) = make_temp_marker_file();
    let marker_str = marker.to_str().unwrap();

    let config = PublishLanguageConfig {
        before: Some(StringOrVec::Single(format!("echo 'before' > {marker_str}"))),
        ..Default::default()
    };

    let result = run_publish_hooks(Language::Python, &config);
    assert!(result.is_ok());
    assert!(marker.exists(), "before hook should have created marker file");
}

#[test]
fn test_run_publish_hooks_precondition_failure_skips() {
    let (_temp_dir, marker) = make_temp_marker_file();
    let marker_str = marker.to_str().unwrap();

    let config = PublishLanguageConfig {
        precondition: Some("false".to_string()),
        before: Some(StringOrVec::Single(format!("echo 'before' > {marker_str}"))),
        ..Default::default()
    };

    let result = run_publish_hooks(Language::Python, &config);
    assert!(result.is_ok());
    assert!(!marker.exists(), "before hook should not run when precondition fails");
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_run_publish_after_hooks_runs_after_only() {
    let (_temp_dir, marker) = make_temp_marker_file();
    let marker_str = marker.to_str().unwrap();

    let config = PublishLanguageConfig {
        after: Some(StringOrVec::Single(format!("echo 'after' > {marker_str}"))),
        ..Default::default()
    };

    let result = run_publish_after_hooks(Language::Python, &config);
    assert!(result.is_ok());
    assert!(marker.exists(), "after hook should have created marker file");

    let content = fs::read_to_string(&marker).unwrap();
    assert!(content.contains("after"));
}

#[test]
fn default_vendor_mode_source_build_langs_use_registry() {
    assert_eq!(default_vendor_mode(Language::Python), VendorMode::Registry);
    assert_eq!(default_vendor_mode(Language::Ruby), VendorMode::Registry);
    assert_eq!(default_vendor_mode(Language::Elixir), VendorMode::Registry);
    assert_eq!(default_vendor_mode(Language::Php), VendorMode::Registry);
    assert_eq!(default_vendor_mode(Language::Swift), VendorMode::Registry);
    assert_eq!(default_vendor_mode(Language::R), VendorMode::Full);
    assert_eq!(default_vendor_mode(Language::Zig), VendorMode::None);
}

#[test]
fn test_run_publish_after_hooks_no_after_is_noop() {
    let config = PublishLanguageConfig::default();
    let result = run_publish_after_hooks(Language::Python, &config);
    assert!(result.is_ok(), "after hooks should succeed when not specified");
}
#[cfg(not(target_os = "windows"))]
#[test]
fn test_run_publish_after_hooks_multiple_commands() {
    let temp_dir = TempDir::new().unwrap();
    let marker1 = temp_dir.path().join("marker1.txt");
    let marker2 = temp_dir.path().join("marker2.txt");

    let marker1_str = marker1.to_str().unwrap();
    let marker2_str = marker2.to_str().unwrap();

    let config = PublishLanguageConfig {
        after: Some(StringOrVec::Multiple(vec![
            format!("echo 'after1' > {marker1_str}"),
            format!("echo 'after2' > {marker2_str}"),
        ])),
        ..Default::default()
    };

    let result = run_publish_after_hooks(Language::Python, &config);
    assert!(result.is_ok());
    assert!(marker1.exists(), "first after command should execute");
    assert!(marker2.exists(), "second after command should execute");
}

#[test]
fn test_run_publish_after_hooks_failure_propagates_error() {
    let config = PublishLanguageConfig {
        after: Some(StringOrVec::Single("false".to_string())),
        ..Default::default()
    };

    let result = run_publish_after_hooks(Language::Python, &config);
    assert!(result.is_err(), "after hook failure should propagate error");
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_publish_hooks_full_lifecycle_success() {
    let temp_dir = TempDir::new().unwrap();
    let before_marker = temp_dir.path().join("before.txt");
    let after_marker = temp_dir.path().join("after.txt");

    let before_str = before_marker.to_str().unwrap();
    let after_str = after_marker.to_str().unwrap();

    let config = PublishLanguageConfig {
        before: Some(StringOrVec::Single(format!("echo 'before' > {before_str}"))),
        after: Some(StringOrVec::Single(format!("echo 'after' > {after_str}"))),
        ..Default::default()
    };

    let before_result = run_publish_hooks(Language::Python, &config);
    assert!(before_result.is_ok());
    assert!(before_marker.exists(), "before hook should run");

    let after_result = run_publish_after_hooks(Language::Python, &config);
    assert!(after_result.is_ok());
    assert!(after_marker.exists(), "after hook should run on success");
}

/// Build a temp workspace with a core crate `my-lib` and a Python binding
/// crate `my-lib-py` whose manifest carries a workspace-member path dep.
/// Returns (TempDir, resolved config wired to the temp root).
fn setup_registry_workspace() -> (TempDir, ResolvedCrateConfig) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib", "crates/my-lib-py"]

[workspace.package]
version = "3.1.4"
"#,
    )
    .unwrap();

    std::fs::create_dir_all(root.join("crates/my-lib/src")).unwrap();
    std::fs::write(root.join("crates/my-lib/src/lib.rs"), "pub fn hi() {}").unwrap();
    std::fs::write(
        root.join("crates/my-lib/Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"3.1.4\"\nedition = \"2021\"\n",
    )
    .unwrap();

    std::fs::create_dir_all(root.join("crates/my-lib-py/src")).unwrap();
    std::fs::write(root.join("crates/my-lib-py/src/lib.rs"), "pub fn hi() {}").unwrap();
    std::fs::write(
        root.join("crates/my-lib-py/Cargo.toml"),
        r#"
[package]
name = "my-lib-py"
version = "3.1.4"
edition = "2021"

[dependencies]
my-lib = { path = "../my-lib", features = ["x"] }
anyhow = "1"
"#,
    )
    .unwrap();

    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "my-lib"
sources = ["crates/my-lib/src/lib.rs"]
"#,
    )
    .unwrap();
    let mut config = cfg.resolve().unwrap().remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config.version_from = root.join("Cargo.toml").to_string_lossy().to_string();

    (tmp, config)
}

fn read_py_manifest(root: &Path) -> toml_edit::DocumentMut {
    let manifest = root.join("crates/my-lib-py/Cargo.toml");
    std::fs::read_to_string(manifest).unwrap().parse().unwrap()
}

#[test]
fn resolve_binding_manifest_python_path() {
    let (_tmp, config) = setup_registry_workspace();
    let path = resolve_binding_manifest(&config, Language::Python).unwrap();
    assert_eq!(path, Path::new("crates").join("my-lib-py").join("Cargo.toml"));
}

#[test]
fn resolve_binding_manifest_zig_is_none() {
    let (_tmp, config) = setup_registry_workspace();
    assert!(resolve_binding_manifest(&config, Language::Zig).is_none());
}

#[test]
fn prepare_registry_rewrites_member_path_deps() {
    let (tmp, config) = setup_registry_workspace();
    let root = tmp.path();

    prepare(&config, &[Language::Python], None, false, false).unwrap();

    let doc = read_py_manifest(root);
    let deps = doc["dependencies"].as_table().unwrap();
    let my_lib = deps["my-lib"].as_inline_table().unwrap();
    assert_eq!(my_lib.get("version").and_then(|v| v.as_str()), Some("3.1.4"));
    assert!(my_lib.get("path").is_none(), "path must be stripped");
    assert!(my_lib.get("features").is_some(), "features preserved");
    assert_eq!(deps["anyhow"].as_str(), Some("1"));
}

#[test]
fn prepare_registry_dry_run_mutates_nothing() {
    let (tmp, config) = setup_registry_workspace();
    let root = tmp.path();

    let before = std::fs::read_to_string(root.join("crates/my-lib-py/Cargo.toml")).unwrap();
    prepare(&config, &[Language::Python], None, true, false).unwrap();
    let after = std::fs::read_to_string(root.join("crates/my-lib-py/Cargo.toml")).unwrap();

    assert_eq!(before, after, "dry-run must not modify the manifest");
    let doc: toml_edit::DocumentMut = after.parse().unwrap();
    let my_lib = doc["dependencies"]["my-lib"].as_inline_table().unwrap();
    assert!(my_lib.get("path").is_some(), "dry-run leaves path intact");
}

#[test]
fn assert_no_member_path_deps_detects_skipped_prepare() {
    let (_tmp, config) = setup_registry_workspace();
    let ws_root = config.workspace_root.clone().unwrap();
    let manifest = ws_root.join(resolve_binding_manifest(&config, Language::Python).unwrap());
    let members = workspace::workspace_member_crates(&ws_root).unwrap();

    let err = assert_no_member_path_deps(&manifest, &members, Language::Python).unwrap_err();
    assert!(err.to_string().contains("still has a `path`"), "got: {err}");

    vendor::rewrite_path_deps_to_registry(&manifest, &members, "3.1.4").unwrap();
    assert_no_member_path_deps(&manifest, &members, Language::Python).unwrap();
}

#[test]
fn ffi_static_libs_command_asks_rustc_for_the_native_libs_of_the_ffi_crate() {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let target = platform::RustTarget::parse("aarch64-apple-darwin").unwrap();
    assert_eq!(
        ffi_static_libs_command(&config, Some(&target), false),
        "cargo rustc --release --lib -p 'sample-lib-ffi' --target aarch64-apple-darwin -- --print native-static-libs"
    );
    assert!(
        ffi_static_libs_command(&config, None, true).starts_with("cross rustc --release --lib -p 'sample-lib-ffi' -- ")
    );
}
