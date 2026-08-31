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

/// `ruby.scaffold_output` is set directly on the resolved config rather than through
/// `[crates.ruby] scaffold_output` TOML: path-safety validation now rejects an absolute
/// `scaffold_output` value at `resolve()` time (it would let a hostile config value write
/// generated files outside the project root), but these tests need `package_dir()` to resolve
/// to a real absolute tempdir with real gemspec fixture files on disk
/// (`ResolvedCrateConfig::package_dir_raw` reads `ruby.scaffold_output` directly for
/// `Language::Ruby`; it does not consult `output_paths`). ~keep
fn ruby_validate_config(package_dir: &Path, version_manifest: &Path) -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
version_from = {}
"#,
        toml_path(version_manifest),
    ))
    .unwrap();
    let mut config = cfg.resolve().unwrap().remove(0);
    config
        .ruby
        .get_or_insert_with(|| toml::from_str("").expect("an empty table deserializes to all-default RubyConfig"))
        .scaffold_output = Some(package_dir.to_path_buf());
    config
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

    // `validate_php_manifests` checks the split layout (a package-local composer.json
    // nested under `pkg_dir`, distinct from the root one), so the package directory is
    // forced to `packages/php` here via an explicit `[crates.output]` entry rather than
    // relying on the default -- which, since this crate targets php, resolves to the
    // co-located `crates/my-lib-php/src`. An explicit `[crates.output] php = "packages/php"`
    // with no `[crates.php.stubs] output` names the class directory verbatim (`packages/php`,
    // no appended `/src`), so `packages/php/` -- not `packages/php/src/` -- is what
    // `php_psr4_target` actually derives for it. ~keep
    let config = validate_config_for(root, "php", "[crates.output]\nphp = \"packages/php\"\n");
    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("PSR-4 path must be packages/php/")
                && !issue.contains("PSR-4 path must be packages/php/src/")),
        "root PSR-4 mismatch must be reported; got: {issues:?}"
    );
}

/// Regression: `validate_php_manifests` used to hardcode `"src/"` and `"packages/php/src/"` as
/// the only correct PSR-4 targets, so any project whose PHP class output directory was
/// configured somewhere else -- via `[crates.php.stubs] output`, which `php_class_output_dir`
/// prioritizes over `[crates.output] php` -- would fail validation even though its manifests
/// were exactly what `scaffold_php` would have written. This pins both the failure (stale
/// manifests still declaring the old default) and the success (manifests matching the
/// configured, non-default output) against the same authority `php_class_output_dir` /
/// `php_psr4_target` use, so the two can no longer disagree. ~keep
#[test]
fn validate_php_derives_psr4_targets_from_the_configured_class_output_dir() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/php")).unwrap();

    let extra = "[crates.output]\nphp = \"packages/php\"\n[crates.php.stubs]\noutput = \"packages/php/generated\"\n";
    let config = validate_config_for(root, "php", extra);

    let stale_composer = r#"{
  "name": "acme/my-lib",
  "autoload": {"psr-4": {"Acme\\MyLib\\": "src/"}}
}
"#;
    std::fs::write(root.join("packages/php/composer.json"), stale_composer).unwrap();
    std::fs::write(root.join("composer.json"), stale_composer).unwrap();

    let stale_issues = validate(&config, &[Language::Php]).unwrap();
    assert!(
        stale_issues
            .iter()
            .any(|issue| issue == "php: packages/php/composer.json PSR-4 path must be generated/"),
        "package-local manifest must be checked against the configured stubs output, not a \
         hardcoded src/; got: {stale_issues:?}"
    );
    assert!(
        stale_issues
            .iter()
            .any(|issue| issue == "php: root composer.json PSR-4 path must be packages/php/generated/"),
        "root manifest must be checked against the configured stubs output, not a hardcoded \
         packages/php/src/; got: {stale_issues:?}"
    );

    let correct_composer = r#"{
  "name": "acme/my-lib",
  "autoload": {"psr-4": {"Acme\\MyLib\\": "generated/"}}
}
"#;
    std::fs::write(root.join("packages/php/composer.json"), correct_composer).unwrap();
    let correct_root_composer = r#"{
  "name": "acme/my-lib",
  "autoload": {"psr-4": {"Acme\\MyLib\\": "packages/php/generated/"}}
}
"#;
    std::fs::write(root.join("composer.json"), correct_root_composer).unwrap();

    let clean_issues = validate(&config, &[Language::Php]).unwrap();
    assert!(
        clean_issues.iter().all(|issue| !issue.contains("PSR-4")),
        "manifests matching the configured non-default output must not be flagged; got: {clean_issues:?}"
    );
}

/// Regression: the co-located layout (the default whenever no split-layout `[crates.output]
/// php` is configured) never has a package-local `composer.json` -- the root manifest already
/// autoloads the class directory directly. `validate_php_manifests` must keep silently skipping
/// the package-local PSR-4 check in that shape rather than newly report a missing file, since
/// `validate()`'s generic `expected_files` check already owns reporting an actually-missing
/// manifest. ~keep
#[test]
fn validate_php_co_located_layout_has_no_package_local_manifest_to_check() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("crates/my-lib-php/src")).unwrap();

    let root_composer = r#"{
  "name": "acme/my-lib",
  "autoload": {"psr-4": {"Acme\\MyLib\\": "crates/my-lib-php/src/"}}
}
"#;
    std::fs::write(root.join("composer.json"), root_composer).unwrap();

    let config = validate_config_for(root, "php", "");
    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        issues.iter().all(|issue| !issue.contains("PSR-4")),
        "co-located layout has no package-local manifest and a correct root manifest, so no \
         PSR-4 issue should be reported; got: {issues:?}"
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

/// Regression: `validate_csharp_project` kept requiring the `runtimes/**` item after
/// `render_csharp_csproj` deliberately dropped it (thin meta-package, HTTP 413 fix), so every
/// freshly generated csproj failed validation. The validator must accept the generator's own
/// output verbatim.
#[test]
fn validate_csharp_accepts_the_generated_thin_meta_package() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/csharp/MyLib")).unwrap();
    let config = validate_config_for(root, "csharp", "");
    let project = crate::scaffold::render_csharp_csproj(&config, "1.2.3");
    std::fs::write(root.join("packages/csharp/MyLib/MyLib.csproj"), &project).unwrap();

    let issues = validate(&config, &[Language::Csharp]).unwrap();

    assert_eq!(
        issues,
        Vec::<String>::new(),
        "the csproj rendered by render_csharp_csproj must validate cleanly; got: {issues:?}"
    );
}

/// The inverse guard: a csproj that *does* pack `runtimes/**` is the HTTP 413 regression and
/// must be reported.
#[test]
fn validate_csharp_rejects_a_csproj_that_packs_the_native_payload() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/csharp/MyLib")).unwrap();
    let config = validate_config_for(root, "csharp", "");
    let fat = crate::scaffold::render_csharp_csproj(&config, "1.2.3").replace(
        r#"<None Include="runtime.json" Pack="true" PackagePath="/" Condition="Exists('runtime.json')" />"#,
        r#"<None Include="runtimes/**" Pack="true" PackagePath="runtimes/" CopyToOutputDirectory="PreserveNewest" />"#,
    );
    std::fs::write(root.join("packages/csharp/MyLib/MyLib.csproj"), &fat).unwrap();

    let issues = validate(&config, &[Language::Csharp]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("must not pack the runtimes/** native payload")),
        "a csproj packing runtimes/** must be reported as the 413 regression; got: {issues:?}"
    );
}

/// Regression: `validate_elixir_manifest` compared against a single-line `targets: ~w(...)`
/// sigil, but the Elixir scaffold renders a multi-line list of quoted strings. Every generated
/// `mix.exs` therefore failed validation even when its targets matched `nif_targets` exactly.
#[test]
fn validate_elixir_accepts_the_generated_multiline_nif_target_list() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/elixir")).unwrap();
    // Verbatim shape rendered by `scaffold::languages::elixir`. ~keep
    std::fs::write(
        root.join("packages/elixir/mix.exs"),
        r#"defmodule MyLib.MixProject do
  use Mix.Project

  def project do
    [
      app: :my_lib,
      version: "1.2.3",
      rustler_crates: [
        my_lib_nif: [
          mode: :release,
          targets: [
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu"
          ]
        ]
      ]
    ]
  end
end
"#,
    )
    .unwrap();

    let config = validate_config_for(
        root,
        "elixir",
        "[crates.elixir]\nnif_targets = [\"aarch64-apple-darwin\", \"x86_64-unknown-linux-gnu\"]\n",
    );
    let issues = validate(&config, &[Language::Elixir]).unwrap();

    assert!(
        issues.iter().all(|issue| !issue.contains("nif_targets")),
        "a multi-line targets list matching nif_targets must validate cleanly; got: {issues:?}"
    );
}

/// The inverse guard: a genuine target mismatch must still be reported.
#[test]
fn validate_elixir_reports_a_genuine_nif_target_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/elixir")).unwrap();
    std::fs::write(
        root.join("packages/elixir/mix.exs"),
        r#"defmodule MyLib.MixProject do
  def project do
    [
      rustler_crates: [
        my_lib_nif: [
          mode: :release,
          targets: [
            "aarch64-apple-darwin"
          ]
        ]
      ]
    ]
  end
end
"#,
    )
    .unwrap();

    let config = validate_config_for(
        root,
        "elixir",
        "[crates.elixir]\nnif_targets = [\"aarch64-apple-darwin\", \"x86_64-unknown-linux-gnu\"]\n",
    );
    let issues = validate(&config, &[Language::Elixir]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("nif_targets: aarch64-apple-darwin x86_64-unknown-linux-gnu")),
        "a targets list missing a configured NIF target must be reported; got: {issues:?}"
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
