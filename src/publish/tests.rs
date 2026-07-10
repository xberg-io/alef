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

fn toml_basic_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn toml_path(path: &Path) -> String {
    toml_basic_string(path.to_string_lossy().as_ref())
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

fn ruby_validate_config(package_dir: &Path, version_manifest: &Path) -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
version_from = {}

[crates.ruby]
scaffold_output = {}
"#,
        toml_path(version_manifest),
        toml_path(package_dir),
    ))
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn validate_ruby_detects_nested_stale_gemspecs() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let package_dir = root.join("packages/ruby");
    std::fs::create_dir_all(package_dir.join("ext/my_lib_rb")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::write(package_dir.join("my_lib.gemspec"), "Gem::Specification.new\n").unwrap();
    std::fs::write(
        package_dir.join("ext/my_lib_rb/my_lib.gemspec"),
        "Gem::Specification.new\n",
    )
    .unwrap();

    let config = ruby_validate_config(&package_dir, &root.join("Cargo.toml"));
    let issues = validate(&config, &[Language::Ruby]).unwrap();

    assert!(
        issues.iter().any(|issue| issue.contains("stale nested gemspec")),
        "nested gemspec must be reported; got: {issues:?}"
    );
}

#[test]
fn validate_ruby_requires_root_gemspec() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let package_dir = root.join("packages/ruby");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();

    let config = ruby_validate_config(&package_dir, &root.join("Cargo.toml"));
    let issues = validate(&config, &[Language::Ruby]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("missing") && issue.contains("*.gemspec")),
        "missing root gemspec must be reported; got: {issues:?}"
    );
}

fn validate_config_for(root: &Path, language: &str, extra: &str) -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["{language}"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
version_from = {}

[crates.scaffold]
repository = "https://github.com/acme/my-lib"
description = "My library"
license = "MIT"

{extra}
"#,
        toml_path(&root.join("Cargo.toml")),
    ))
    .unwrap();
    let mut config = cfg.resolve().unwrap().remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config
}

#[test]
fn validate_go_reports_v2_layout_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/go")).unwrap();
    std::fs::write(
        root.join("packages/go/go.mod"),
        "module github.com/acme/my-lib/v2\n\ngo 1.26\n",
    )
    .unwrap();

    let config = validate_config_for(
        root,
        "go",
        r#"
[crates.go]
module = "github.com/acme/my-lib/v2"
"#,
    );
    let issues = validate(&config, &[Language::Go]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("requires package directory packages/go/v2")),
        "v2 module layout mismatch must be reported; got: {issues:?}"
    );
}

#[test]
fn validate_php_reports_root_psr4_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/php")).unwrap();
    let composer = r#"{
  "name": "acme/my-lib",
  "autoload": {"psr-4": {"Acme\\MyLib\\": "src/"}}
}
"#;
    std::fs::write(root.join("packages/php/composer.json"), composer).unwrap();
    std::fs::write(root.join("composer.json"), composer).unwrap();

    let config = validate_config_for(root, "php", "");
    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("PSR-4 path must be packages/php/src/")),
        "root PSR-4 mismatch must be reported; got: {issues:?}"
    );
}

#[test]
fn validate_csharp_reports_stale_root_project() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/csharp/MyLib")).unwrap();
    let project = crate::scaffold::render_csharp_csproj(&validate_config_for(root, "csharp", ""), "1.2.3");
    std::fs::write(root.join("packages/csharp/MyLib/MyLib.csproj"), &project).unwrap();
    std::fs::write(root.join("packages/csharp/MyLib.csproj"), &project).unwrap();

    let config = validate_config_for(root, "csharp", "");
    let issues = validate(&config, &[Language::Csharp]).unwrap();

    assert!(
        issues.iter().any(|issue| issue.contains("stale root project")),
        "stale root csproj must be reported; got: {issues:?}"
    );
}

#[test]
fn validate_dart_and_zig_check_central_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/dart")).unwrap();
    std::fs::write(
        root.join("packages/dart/pubspec.yaml"),
        "name: wrong\nversion: 1.2.3\ndescription: My library\nrepository: https://github.com/acme/my-lib\n",
    )
    .unwrap();
    let dart_config = validate_config_for(root, "dart", "");
    let dart_issues = validate(&dart_config, &[Language::Dart]).unwrap();
    assert!(
        dart_issues
            .iter()
            .any(|issue| issue.contains("pubspec.yaml name must be my_lib")),
        "Dart name mismatch must be reported; got: {dart_issues:?}"
    );

    std::fs::create_dir_all(root.join("packages/zig")).unwrap();
    std::fs::write(root.join("packages/zig/build.zig"), "").unwrap();
    std::fs::write(
        root.join("packages/zig/build.zig.zon"),
        ".{ .name = .wrong, .paths = .{} }\n",
    )
    .unwrap();
    let zig_config = validate_config_for(root, "zig", "");
    let zig_issues = validate(&zig_config, &[Language::Zig]).unwrap();
    assert!(
        zig_issues
            .iter()
            .any(|issue| issue.contains("build.zig.zon name must be my_lib")),
        "Zig name mismatch must be reported; got: {zig_issues:?}"
    );
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
