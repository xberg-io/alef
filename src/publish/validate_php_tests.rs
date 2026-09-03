//! `alef publish validate` against the PHP layout the generator actually writes.
//!
//! ~keep This lives beside `src/publish/tests.rs` rather than inside it because every assertion
//! here is a *drift* assertion between two subsystems -- `scaffold::languages::php` and
//! `publish::validate` -- and the fixture it needs (a temp repository whose `composer.json` files
//! are the ones the scaffolder emitted, not hand-written literals) is not shared with any other
//! test in that file.
//!
//! The drift these pin: `c159e2dc0` made `scaffold_php` emit exactly one `composer.json` per
//! layout, and the layout every real consumer resolves is the co-located one -- `co_located` keys
//! off `output_paths.contains_key("php")`, and `resolve_output_paths` inserts an entry for every
//! ENABLED language, so it is true whenever php is enabled. The validator was not updated: it kept
//! requiring `{pkg_dir}/composer.json`, and because that file is never there, its own
//! `read_json(&package_manifest)` early-return also silently skipped every root-manifest check
//! below it.

use super::validate::validate;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::path::Path;
use tempfile::TempDir;

/// The crate name every fixture here uses; the co-located PHP class directory derives from it.
const CRATE_NAME: &str = "my-lib";

/// The class directory `[crates.output] php` resolves to for [`CRATE_NAME`] with no override --
/// the same shape a real downstream crate configures explicitly (`crates/<crate>-php/src/`).
const CO_LOCATED_CLASS_DIR: &str = "crates/my-lib-php/src";

fn toml_path(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// A php-enabled config rooted at `root`, with a `Cargo.toml` written so `resolved_version()`
/// succeeds and cannot contribute an unrelated issue to the assertions below.
fn php_config(root: &Path, extra: &str) -> ResolvedCrateConfig {
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"my-lib\"\nversion = \"1.2.3\"\n",
    )
    .expect("write Cargo.toml");

    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "{CRATE_NAME}"
sources = ["src/lib.rs"]
version_from = {version_from}

[crates.scaffold]
repository = "https://github.com/acme/my-lib"
description = "My library"
license = "MIT"
authors = ["Alice"]

{extra}
"#,
        version_from = toml_path(&root.join("Cargo.toml")),
    ))
    .expect("valid toml");
    let mut config = cfg.resolve().expect("resolve ok").remove(0);
    config.workspace_root = Some(root.to_path_buf());
    config
}

/// Write every `composer.json` `scaffold_php` emits for `config` into `root`, and return their
/// repository-relative paths.
///
/// Reading the manifests off the scaffolder instead of writing literals is the point: a literal
/// fixture can agree with a validator that both disagree with the generator, which is exactly how
/// the requirement for `{pkg_dir}/composer.json` outlived the file.
fn write_scaffolded_composer_files(root: &Path, config: &ResolvedCrateConfig) -> Vec<String> {
    let files =
        crate::scaffold::scaffold(&ApiSurface::default(), config, &[Language::Php]).expect("php scaffold must succeed");
    let mut written = Vec::new();
    for file in files
        .iter()
        .filter(|f| f.path.file_name().is_some_and(|name| name == "composer.json"))
    {
        let target = root.join(&file.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("create manifest parent");
        }
        std::fs::write(&target, &file.content).expect("write manifest");
        written.push(file.path.to_string_lossy().into_owned());
    }
    written.sort();
    written
}

/// ~keep The generator's own output must pass the validator. `scaffold_php` emits ONE manifest for
/// the co-located layout -- the repository-root one -- so requiring `{pkg_dir}/composer.json`
/// reported a file no stage writes. A real downstream crate (`php = "crates/<crate>-php/src/"`)
/// fails `alef publish validate` on exactly this today; three other real downstream crates using
/// this same php class-dir shape pass only because a pre-`c159e2dc0` leftover manifest still sits
/// in their class directory, so they would start failing the moment that leftover is removed.
#[test]
fn scaffolded_co_located_layout_passes_validation() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let config = php_config(root, "[crates.output]\nphp = \"crates/my-lib-php/src/\"\n");
    std::fs::create_dir_all(root.join(CO_LOCATED_CLASS_DIR)).unwrap();

    let manifests = write_scaffolded_composer_files(root, &config);
    assert_eq!(
        manifests,
        vec!["composer.json".to_string()],
        "the co-located scaffold emits only the root manifest"
    );

    let issues = validate(&config, &[Language::Php]).unwrap();

    assert_eq!(
        issues,
        Vec::<String>::new(),
        "everything the php scaffolder wrote must satisfy the validator; got: {issues:?}"
    );
}

/// ~keep The same repository shape, spelled the way a consumer that sets no `[crates.output] php`
/// resolves it. `co_located` is true for an enabled language regardless of an explicit output
/// entry, so this must behave identically to the explicit form above.
#[test]
fn default_co_located_layout_does_not_require_a_package_local_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let config = php_config(root, "");
    std::fs::create_dir_all(root.join(CO_LOCATED_CLASS_DIR)).unwrap();

    write_scaffolded_composer_files(root, &config);

    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        !issues
            .iter()
            .any(|issue| issue.contains("missing") && issue.contains("composer.json")),
        "no composer.json may be reported missing when the scaffolder wrote them all; got: {issues:?}"
    );
    assert_eq!(
        issues,
        Vec::<String>::new(),
        "default co-located layout must validate clean; got: {issues:?}"
    );
}

/// ~keep The root manifest is the published package -- Packagist reads the repository root -- so
/// its absence is the one php failure that must never be swallowed. It was: the missing-root
/// report sat *below* an early `return` taken whenever `{pkg_dir}/composer.json` could not be read,
/// and in the co-located layout that file is never there, so a repository with NO `composer.json`
/// at all validated clean.
#[test]
fn co_located_layout_reports_a_missing_root_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let config = php_config(root, "");
    std::fs::create_dir_all(root.join(CO_LOCATED_CLASS_DIR)).unwrap();

    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        issues.iter().any(|issue| issue == "php: missing root composer.json"),
        "a repository with no composer.json anywhere must be reported; got: {issues:?}"
    );
}

/// ~keep The PSR-4 check that is real in the co-located layout is the ROOT one: the classes live in
/// `pkg_dir` itself, so `php_package_psr4_target` returns `None` by design and there is no
/// package-local manifest to check. That root check was dead for the same early-return reason, so a
/// root manifest autoloading a directory no stage writes -- the precise defect
/// `backends::php::layout` exists to prevent -- was accepted.
#[test]
fn co_located_layout_reports_a_root_psr4_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let config = php_config(root, "");
    std::fs::create_dir_all(root.join(CO_LOCATED_CLASS_DIR)).unwrap();

    // ~keep The stale split-layout target: what a repository scaffolded before the co-located
    // default carried, and which resolves to a directory `alef generate` never writes.
    std::fs::write(
        root.join("composer.json"),
        "{\n  \"name\": \"acme/my-lib\",\n  \"autoload\": {\"psr-4\": {\"Acme\\\\MyLib\\\\\": \"packages/php/src/\"}}\n}\n",
    )
    .unwrap();

    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue == "php: root composer.json PSR-4 path must be crates/my-lib-php/src/"),
        "the root manifest must be checked against the resolved class directory; got: {issues:?}"
    );
}

/// ~keep A package-local manifest is still validated when one is genuinely present and the class
/// directory is nested inside `pkg_dir` -- that block is conditional, not deleted. Pins that
/// relaxing the requirement did not also stop checking the file when it exists.
#[test]
fn nested_class_directory_still_validates_a_present_package_manifest() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();
    let config = php_config(
        root,
        "[crates.output]\nphp = \"packages/php\"\n[crates.php.stubs]\noutput = \"packages/php/generated\"\n",
    );
    std::fs::create_dir_all(root.join("packages/php/generated")).unwrap();

    let root_manifest = "{\n  \"name\": \"acme/my-lib\",\n  \"autoload\": {\"psr-4\": {\"Acme\\\\MyLib\\\\\": \"packages/php/generated/\"}}\n}\n";
    // ~keep Stale: the package manifest autoloads `src/`, but the classes go to `generated/`.
    let package_manifest =
        "{\n  \"name\": \"acme/my-lib\",\n  \"autoload\": {\"psr-4\": {\"Acme\\\\MyLib\\\\\": \"src/\"}}\n}\n";
    std::fs::write(root.join("composer.json"), root_manifest).unwrap();
    std::fs::write(root.join("packages/php/composer.json"), package_manifest).unwrap();

    let issues = validate(&config, &[Language::Php]).unwrap();

    assert!(
        issues
            .iter()
            .any(|issue| issue == "php: packages/php/composer.json PSR-4 path must be generated/"),
        "a present package-local manifest must still be checked; got: {issues:?}"
    );
}
