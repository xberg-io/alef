//! `alef publish validate` checks for the non-PHP languages.

use super::tests::toml_path;
use super::*;
use std::path::Path;
use tempfile::TempDir;

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

/// Go permits the `/vN` module-path suffix to be omitted from the package directory
/// (https://go.dev/ref/mod#module-path), and `go_tag::run` already tags both layouts, so a
/// `/v2` module living in a bare `packages/go` directory is a valid layout, not a mismatch.
#[test]
fn validate_go_accepts_v2_module_with_suffix_omitted_from_directory() {
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
        issues.iter().all(|issue| !issue.starts_with("go:")),
        "a /v2 module in a bare packages/go directory is a valid Go layout; got: {issues:?}"
    );
}

#[test]
fn validate_go_accepts_v2_module_in_v2_directory() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/go/v2")).unwrap();
    std::fs::write(
        root.join("packages/go/v2/go.mod"),
        "module github.com/acme/my-lib/v2\n\ngo 1.26\n",
    )
    .unwrap();

    let config = validate_config_for(
        root,
        "go",
        r#"
[crates.go]
module = "github.com/acme/my-lib/v2"
module_major = 2
"#,
    );
    let issues = validate(&config, &[Language::Go]).unwrap();

    assert!(
        issues.iter().all(|issue| !issue.starts_with("go:")),
        "a /v2 module in a packages/go/v2 directory is a valid Go layout; got: {issues:?}"
    );
}

#[test]
fn validate_go_reports_mismatched_directory_suffix() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("packages/go/v3")).unwrap();
    std::fs::write(
        root.join("packages/go/v3/go.mod"),
        "module github.com/acme/my-lib/v2\n\ngo 1.26\n",
    )
    .unwrap();

    let config = validate_config_for(
        root,
        "go",
        r#"
[crates.go]
module = "github.com/acme/my-lib/v2"
module_major = 3
"#,
    );
    let issues = validate(&config, &[Language::Go]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("packages/go/v3") && issue.contains("packages/go/v2")),
        "a mismatched vM directory suffix must be reported, naming both the found and expected \
         directories; got: {issues:?}"
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

fn write_node_crate_with_platforms(root: &Path, platforms: &[&str]) {
    let pkg_dir = root.join("crates/my-lib-node");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("package.json"), "{}").unwrap();
    for platform in platforms {
        std::fs::create_dir_all(pkg_dir.join("npm").join(platform)).unwrap();
    }
}

/// Regression test for alef#358: a root `pnpm-workspace.yaml` whose `packages:` list does not
/// cover `crates/<crate>-node/npm/*` lets a frozen pnpm install try to resolve the not-yet-
/// published platform package from the registry instead of linking it locally.
#[test]
fn validate_node_reports_pnpm_workspace_missing_native_platform_glob() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    write_node_crate_with_platforms(root, &["linux-x64-gnu", "darwin-arm64"]);
    std::fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - 'packages/*'\n").unwrap();

    let config = validate_config_for(root, "node", "");
    let issues = validate(&config, &[Language::Node]).unwrap();

    assert!(
        issues.iter().any(|issue| issue.contains("crates/my-lib-node/npm/*")),
        "a pnpm-workspace.yaml missing the native platform glob must be reported; got: {issues:?}"
    );
}

/// A `pnpm-workspace.yaml` that already covers `crates/<crate>-node/npm/*` should report no
/// finding.
#[test]
fn validate_node_accepts_pnpm_workspace_with_native_platform_glob() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    write_node_crate_with_platforms(root, &["linux-x64-gnu", "darwin-arm64"]);
    std::fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'packages/*'\n  - 'crates/my-lib-node/npm/*'\n",
    )
    .unwrap();

    let config = validate_config_for(root, "node", "");
    let issues = validate(&config, &[Language::Node]).unwrap();

    assert!(
        !issues.iter().any(|issue| issue.contains("npm")),
        "a pnpm-workspace.yaml already covering the native platform glob must be clean; got: {issues:?}"
    );
}

/// A broader workspace glob that still reaches every platform directory is just as good as the
/// exact one; the check is about coverage, not spelling.
#[test]
fn validate_node_accepts_pnpm_workspace_with_a_broader_glob() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    write_node_crate_with_platforms(root, &["linux-x64-gnu", "darwin-arm64"]);
    std::fs::write(root.join("pnpm-workspace.yaml"), "packages:\n  - 'crates/*/npm/*'\n").unwrap();

    let config = validate_config_for(root, "node", "");
    let issues = validate(&config, &[Language::Node]).unwrap();

    assert!(
        !issues.iter().any(|issue| issue.contains("npm")),
        "a `crates/*/npm/*` entry covers every platform directory and must be clean; got: {issues:?}"
    );
}

/// No `pnpm-workspace.yaml` at all means pnpm workspaces are not in play, so there is nothing to
/// validate.
#[test]
fn validate_node_is_clean_without_a_pnpm_workspace_file() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .unwrap();
    write_node_crate_with_platforms(root, &["linux-x64-gnu", "darwin-arm64"]);

    let config = validate_config_for(root, "node", "");
    let issues = validate(&config, &[Language::Node]).unwrap();

    assert!(
        !issues.iter().any(|issue| issue.contains("npm")),
        "no pnpm-workspace.yaml must produce no node workspace finding; got: {issues:?}"
    );
}
