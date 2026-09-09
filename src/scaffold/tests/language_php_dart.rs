use super::*;

/// Every composer.json the PHP scaffold emits, as (path, content).
///
/// ~keep Declares `php` in the workspace languages deliberately. `resolve_output_paths` inserts an
/// entry for every ENABLED language — falling back to the output template when the crate sets no
/// explicit path — so `output_paths` only carries a `php` key when php is enabled. A fixture that
/// omits it can never exercise the co-located branch, and would silently assert the split layout
/// while claiming to test co-location.
fn php_manifests(workspace_languages: &str, extra_crate_config: &str) -> Vec<(String, String)> {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = [{workspace_languages}]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Alice"]
keywords = ["test"]
{extra_crate_config}
"#,
    ))
    .expect("valid toml");
    let config = cfg.resolve().expect("resolve ok").remove(0);

    scaffold(&test_api(), &config, &[Language::Php])
        .unwrap()
        .into_iter()
        .filter(|f| f.path.to_string_lossy().ends_with("composer.json"))
        .map(|f| (f.path.to_string_lossy().into_owned(), f.content))
        .collect()
}

/// ~keep The co-located layout must emit exactly ONE manifest. Both manifests render the same
/// composer `name`, so a second one beside the classes publishes a duplicate of the package
/// identity into the consumer's repository — and nothing can resolve it, because Packagist reads
/// the repository root and every consumer reference targets the class directory, not the manifest.
/// This shipped into six consumer repositories before anyone noticed.
#[test]
fn co_located_layout_emits_exactly_one_manifest() {
    let manifests = php_manifests("\"php\"", "[crates.output]\nphp = \"crates/my-lib-php/src/\"\n");

    let paths: Vec<&str> = manifests.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["composer.json"],
        "co-located layout must emit only the root manifest"
    );
    assert!(
        manifests[0].1.contains("crates/my-lib-php/src/"),
        "the root manifest must autoload the co-located class directory; got:\n{}",
        manifests[0].1
    );
}

/// ~keep The co-located branch does not actually require `[crates.output] php`. Enabling php is
/// enough, because `resolve_output_paths` inserts a template-resolved entry for every enabled
/// language, so `output_paths.contains_key("php")` is true either way. This pins that, because it
/// means the split branch below is unreachable for any real consumer and the flag's name promises
/// a narrower condition than it tests.
#[test]
fn enabling_php_alone_selects_the_co_located_layout() {
    let manifests = php_manifests("\"php\"", "");

    let paths: Vec<&str> = manifests.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["composer.json"],
        "an enabled php language resolves an output path, so the layout is co-located"
    );
}

/// ~keep The split layout keeps both: there the classes live under `packages/php/src/`, so the
/// package-dir manifest is the installable package and the root one exists so the repository itself
/// resolves. Dropping the package manifest unconditionally would break those consumers.
///
/// Reaching this branch requires scaffolding php while php is NOT an enabled workspace language —
/// see `enabling_php_alone_selects_the_co_located_layout` for why. The branch is therefore
/// preserved for safety rather than because a current consumer exercises it.
#[test]
fn split_layout_still_emits_root_and_package_manifests() {
    let manifests = php_manifests("\"python\", \"node\"", "");

    let mut paths: Vec<&str> = manifests.iter().map(|(path, _)| path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec!["composer.json", "packages/php/composer.json"],
        "split layout must keep both manifests"
    );

    let package = manifests
        .iter()
        .find(|(path, _)| path == "packages/php/composer.json")
        .expect("package manifest");
    assert!(
        package.1.contains("\"src/\""),
        "the package manifest autoloads its nested src/; got:\n{}",
        package.1
    );
    let root = manifests
        .iter()
        .find(|(path, _)| path == "composer.json")
        .expect("root manifest");
    assert!(
        root.1.contains("packages/php/src/"),
        "the root manifest reaches through to the package sources; got:\n{}",
        root.1
    );
}

#[test]
fn test_scaffold_php_omits_phpstan_and_cs_fixer_configs() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let paths: Vec<String> = all_files
        .iter()
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect();
    assert!(
        !paths.iter().any(|p| p.ends_with(".php-cs-fixer.dist.php")),
        "must not emit .php-cs-fixer.dist.php; got {paths:?}"
    );
    assert!(
        !paths
            .iter()
            .any(|p| p.ends_with("phpstan.neon") || p.ends_with("phpstan-baseline.neon")),
        "must not emit phpstan config; got {paths:?}"
    );
    let composers: Vec<&GeneratedFile> = all_files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with("composer.json"))
        .collect();
    assert_eq!(
        composers.len(),
        2,
        "expected root + package composer.json; got {:?}",
        composers
            .iter()
            .map(|f| f.path.display().to_string())
            .collect::<Vec<_>>()
    );
    for composer in &composers {
        assert!(
            !composer.content.contains("phpstan") && !composer.content.contains("php-cs-fixer"),
            "{} must not reference phpstan/php-cs-fixer; content:\n{}",
            composer.path.display(),
            composer.content
        );
        assert!(
            composer.content.contains("\"lint\": \"poly lint\""),
            "{} lint script must call poly; content:\n{}",
            composer.path.display(),
            composer.content
        );
    }
}

#[test]
fn test_scaffold_php_emits_root_composer_json_mirroring_package() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);

    let pkg_composer = files
        .iter()
        .find(|f| f.path.to_string_lossy() == "packages/php/composer.json")
        .expect("packages/php/composer.json must be emitted");
    let root_composer = files
        .iter()
        .find(|f| f.path.to_string_lossy() == "composer.json")
        .expect("root composer.json must be emitted at repo root for Packagist/PIE");

    let pkg: serde_json::Value =
        serde_json::from_str(&pkg_composer.content).expect("packages/php/composer.json must be valid JSON");
    let root: serde_json::Value =
        serde_json::from_str(&root_composer.content).expect("root composer.json must be valid JSON");

    assert_eq!(pkg["name"], root["name"], "package and root should have the same name");
    assert_eq!(
        pkg["php-ext"], root["php-ext"],
        "package and root should have the same php-ext block"
    );
    assert_eq!(pkg["autoload"]["psr-4"], serde_json::json!({"My\\Lib\\": "src/"}));
    assert_eq!(
        root["autoload"]["psr-4"],
        serde_json::json!({"My\\Lib\\": "packages/php/src/"})
    );

    for (label, json) in &[("packages/php/composer.json", pkg), ("composer.json", root)] {
        assert!(
            json.get("extra").is_some(),
            "{} must have an extra block; content:\n{}",
            label,
            if label == &"packages/php/composer.json" {
                &pkg_composer.content
            } else {
                &root_composer.content
            }
        );
        assert!(
            json["extra"]["pie"]["binary"]["url-template"].is_string(),
            "{} must contain PIE url-template block",
            label,
        );

        let pie_url = json["extra"]["pie"]["binary"]["url-template"]
            .as_str()
            .expect("url-template must be a string");
        assert!(
            !pie_url.contains("-nodebug-"),
            "{} url-template must not include -nodebug- token (PIE 1.4.5 compat); got: {pie_url}",
            label
        );
        assert!(
            pie_url.contains("/releases/download/{Version}/"),
            "{} url-template must use {{Version}} in release path (PIE 1.4+ supplies a `v`-prefixed version); got: {pie_url}",
            label
        );
    }
}

#[test]
fn test_scaffold_php_uses_inert_composer_vendor_when_repository_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let root_composer = files
        .iter()
        .find(|f| f.path.to_string_lossy() == "composer.json")
        .expect("root composer.json must be emitted");

    let parsed: serde_json::Value =
        serde_json::from_str(&root_composer.content).expect("composer.json must be valid JSON");
    assert_eq!(parsed["name"], "unconfigured/my-lib");
}

/// Regression for alef-task #320: `scaffold_php_cargo` unconditionally forwarded every
/// `collect_cfg_features` name (minus function-gated names) into the wrapper's own
/// `default = [...]` array and every `[crates.php].features` name into the core dependency's own
/// explicit `features = [...]` line, re-enabling a feature a `target_dep_overrides` entry
/// excluded for one cfg target -- the same defect `RubyConfig::excluded_default_features` fixed
/// for the Magnus crate, generalized here. Asserts both directions on both surfaces: the excluded
/// name is never defaulted or forwarded, and a name nobody excluded still is.
#[test]
fn test_scaffold_php_excludes_named_feature_from_default_but_keeps_others() {
    let config = minimal_config_from_toml(
        r#"
[crates.php]
features = ["native-http", "wasm-http"]
excluded_default_features = ["native-http"]
[[crates.php.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["wasm-http"]
default_features = false
"#,
    );
    let api = crate::core::ir::ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            crate::core::ir::TypeDef {
                name: "NativeOnly".to_string(),
                rust_path: "my_lib::NativeOnly".to_string(),
                cfg: Some(r#"feature = "native-http""#.to_string()),
                ..Default::default()
            },
            crate::core::ir::TypeDef {
                name: "WasmOnly".to_string(),
                rust_path: "my_lib::WasmOnly".to_string(),
                cfg: Some(r#"feature = "wasm-http""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let cargo = &files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-php/Cargo.toml"))
        .expect("php Cargo.toml must be emitted")
        .content;

    let default_line = cargo
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert!(
        !default_line.contains("native-http"),
        "excluded_default_features must drop the name from the wrapper's own default array:\n{default_line}"
    );
    assert!(
        default_line.contains("wasm-http"),
        "a feature nobody excluded must still be forwarded into default:\n{default_line}"
    );
    assert!(
        cargo.contains(r#"native-http = ["my-lib/native-http"]"#),
        "the excluded feature stays declared (so `cargo build --features native-http` still \
         works), just not defaulted:\n{cargo}"
    );

    let default_target_block = cargo
        .split("[target.'cfg(not(target_os")
        .nth(1)
        .expect("default target block present");
    let default_block_dep_line = default_target_block
        .lines()
        .find(|line| line.trim_start().starts_with("my-lib ="))
        .expect("core dependency line present in default target block");
    assert!(
        !default_block_dep_line.contains("native-http"),
        "excluded_default_features must also drop the name from the core dependency's own \
         explicit features = [...] line, not just the wrapper's default array:\n{default_block_dep_line}"
    );
}

#[test]
fn test_scaffold_dart() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 9, "Expected 9 files for Dart scaffold");
    assert!(
        files.iter().all(|f| !f.path.ends_with("BUILDING.md")),
        "Dart scaffold must not emit BUILDING.md"
    );

    let pubspec = &files[0];
    assert_eq!(pubspec.path, PathBuf::from("packages/dart/pubspec.yaml"));
    assert!(pubspec.content.contains("name: my_lib"), "got: {}", pubspec.content);
    assert!(pubspec.content.contains("version: 0.1.0"), "got: {}", pubspec.content);
    assert!(
        pubspec.content.contains("flutter_rust_bridge:"),
        "got: {}",
        pubspec.content
    );
    // flutter_rust_bridge checks Dart-runtime == codegen version for equality at
    // `RustLib.init()`, and the sibling Rust crate pins the same constant with `=`. A caret
    // here lets `dart pub get` satisfy a stale lockfile with an older minor, which surfaces as
    // a "Bad state: codegen version should be the same as runtime version" on first call
    // rather than as a resolution failure. Assert the exact pin, and assert the caret is gone
    // so a "relax the constraint" edit has to argue with this test.
    assert!(
        pubspec.content.contains(&format!(
            "flutter_rust_bridge: {}",
            crate::core::template_versions::cargo::FLUTTER_RUST_BRIDGE
        )),
        "flutter_rust_bridge must be pinned exactly, got: {}",
        pubspec.content
    );
    assert!(
        !pubspec.content.contains("flutter_rust_bridge: ^"),
        "flutter_rust_bridge must not carry a caret range, got: {}",
        pubspec.content
    );
    assert!(
        pubspec.content.contains("sdk: '>=3.13.0 <4.0.0'"),
        "got: {}",
        pubspec.content
    );
    assert!(
        pubspec.content.contains("freezed_annotation: '^3.1.0'"),
        "got: {}",
        pubspec.content
    );
    assert!(
        pubspec.content.contains("build_runner: '^2.16.0'"),
        "got: {}",
        pubspec.content
    );
    assert!(pubspec.content.contains("test:"), "got: {}", pubspec.content);
    assert!(pubspec.content.contains("lints:"), "got: {}", pubspec.content);
    assert!(
        pubspec.content.contains("repository:"),
        "pubspec.yaml must include a repository field for pub.dev; got: {}",
        pubspec.content
    );
    assert!(
        pubspec.content.contains("github.com/test/my-lib"),
        "pubspec.yaml repository must contain the configured URL; got: {}",
        pubspec.content
    );

    let analysis_options = &files[1];
    assert_eq!(
        analysis_options.path,
        PathBuf::from("packages/dart/analysis_options.yaml")
    );
    assert!(
        analysis_options.content.contains("package:lints/recommended.yaml"),
        "got: {}",
        analysis_options.content
    );
    assert!(
        analysis_options.content.contains("linter:"),
        "analysis_options.yaml should include linter rules; got: {}",
        analysis_options.content
    );
    for removed_lint in [
        "avoid_returning_null",
        "avoid_returning_null_for_future",
        "invariant_booleans",
        "iterable_contains_unrelated_type",
        "list_remove_unrelated_type",
    ] {
        assert!(
            !analysis_options.content.contains(removed_lint),
            "analysis_options.yaml references lint removed in Dart 3.x: {removed_lint}"
        );
    }
    assert!(
        analysis_options.content.contains("analyzer:")
            && analysis_options.content.contains("exclude:")
            && analysis_options.content.contains("lib/src/frb/**"),
        "analysis_options.yaml must include analyzer.exclude block; got:\n{}",
        analysis_options.content
    );
    assert!(
        analysis_options.content.contains("lib/src/my_lib_bridge_generated/**"),
        "analysis_options.yaml must use crate-derived generated paths; got:\n{}",
        analysis_options.content
    );

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/dart/.gitignore"));
    assert!(gitignore.content.contains(".dart_tool/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains("build/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains("pubspec.lock"), "got: {}", gitignore.content);

    let pubignore = &files[3];
    assert_eq!(pubignore.path, PathBuf::from("packages/dart/.pubignore"));
    assert!(pubignore.content.contains("android/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("ios/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("blobs/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("rust/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("example/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("test/"), "got: {}", pubignore.content);
    // Native FFI libraries MUST ship in the published pub.dev tarball, so .pubignore
    // must NOT exclude the native dir or the shared-library globs.
    assert!(
        !pubignore.content.contains("lib/src/native/"),
        "got: {}",
        pubignore.content
    );
    assert!(!pubignore.content.contains("*.so"), "got: {}", pubignore.content);
    assert!(!pubignore.content.contains("*.dylib"), "got: {}", pubignore.content);
    assert!(!pubignore.content.contains("*.dll"), "got: {}", pubignore.content);

    let test_file = &files[4];
    assert_eq!(test_file.path, PathBuf::from("packages/dart/test/my_lib_test.dart"));
    assert!(
        test_file.content.contains("import 'package:test/test.dart'"),
        "got: {}",
        test_file.content
    );
    assert!(
        test_file.content.contains("test('placeholder'"),
        "got: {}",
        test_file.content
    );
    // A placeholder that never imports the package under test stays green through a total
    // API break — it must link the generated package, not just package:test.
    assert!(
        test_file.content.contains("import 'package:my_lib/my_lib.dart'"),
        "placeholder test must import the package under test; got: {}",
        test_file.content
    );

    assert_eq!(files[5].path, PathBuf::from("packages/dart/.editorconfig"));
    assert!(files[5].content.contains("*.dart"));

    assert_eq!(files[6].path, PathBuf::from("packages/dart/README.md"));
    assert!(files[6].content.contains("dart pub get"));
    assert!(files[6].content.contains("flutter_rust_bridge_codegen generate"));

    assert_eq!(
        files[7].path,
        PathBuf::from("packages/dart/example/my_lib_example.dart")
    );
    assert!(files[7].content.contains("void main"));
    // `package:my_lib' (no path segment) is an invalid `package:` URI and fails `dart analyze`.
    assert!(
        files[7].content.contains("import 'package:my_lib/my_lib.dart'"),
        "example must use a valid package: URI with a path segment; got: {}",
        files[7].content
    );

    let changelog = &files[8];
    assert_eq!(changelog.path, PathBuf::from("packages/dart/CHANGELOG.md"));
    assert!(
        changelog.content.contains("## 0.1.0"),
        "CHANGELOG.md must contain the current version; got: {}",
        changelog.content
    );

    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Dart scaffold must not emit GitHub workflows"
    );
}

// Regression for #555: once `[crates.readme.languages.dart]` is configured, the
// README module (`crate::readme`) owns `packages/dart/README.md` end-to-end, and
// scaffold must not emit a second, independent copy at the same path.
#[test]
fn should_not_emit_placeholder_readme_when_readme_module_configures_dart() {
    let config = test_config_from_toml(
        r#"
[crates.readme.languages.dart]
template = "language_package.md"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let files = language_files(&all_files);
    assert!(
        files.iter().all(|f| f.path != Path::new("packages/dart/README.md")),
        "scaffold must not emit packages/dart/README.md once the README module is \
         configured for dart (#555)"
    );
}

#[test]
fn test_scaffold_dart_ffi_style() {
    let config = test_config_from_toml(
        r#"
[crates.dart]
style = "ffi"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let files = language_files(&all_files);
    let pubspec = &files[0];
    assert!(pubspec.content.contains("ffi: '^2.2.0'"), "got: {}", pubspec.content);
    {
        let frb_only_dep = "flutter_rust_bridge:";
        assert!(
            !pubspec.content.contains(frb_only_dep),
            "FFI Dart scaffold must not include FRB-only dependency {frb_only_dep}; got:\n{}",
            pubspec.content
        );
    }
    for product_dto_dep in [
        "freezed_annotation:",
        "json_annotation:",
        "freezed:",
        "build_runner:",
        "json_serializable:",
    ] {
        assert!(
            pubspec.content.contains(product_dto_dep),
            "FFI Dart scaffold must include product-type DTO dependency {product_dto_dep} (STY-10); got:\n{}",
            pubspec.content
        );
    }
    let readme = files
        .iter()
        .find(|f| f.path == Path::new("packages/dart/README.md"))
        .unwrap();
    assert!(readme.content.contains("cargo build --release -p my-lib-ffi"));
    assert!(!readme.content.contains("flutter_rust_bridge_codegen generate"));

    // FFI style has no top-level barrel file (see gen_ffi::emit) — the public re-export
    // wrapper lives at lib/src/{module}.dart, not lib/{module}.dart like FRB's default
    // barrel, so the example/test package: URIs must point at the `src/` path instead.
    let example = files
        .iter()
        .find(|f| f.path == Path::new("packages/dart/example/my_lib_example.dart"))
        .unwrap();
    assert!(
        example.content.contains("import 'package:my_lib/src/my_lib.dart'"),
        "FFI style example must import the src/ wrapper, not a nonexistent barrel; got: {}",
        example.content
    );

    let test_file = files
        .iter()
        .find(|f| f.path == Path::new("packages/dart/test/my_lib_test.dart"))
        .unwrap();
    assert!(
        test_file.content.contains("import 'package:my_lib/src/my_lib.dart'"),
        "FFI style placeholder test must import the src/ wrapper, not a nonexistent barrel; got: {}",
        test_file.content
    );
}
