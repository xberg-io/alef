use super::*;

#[test]
fn test_scaffold_php_omits_phpstan_and_cs_fixer_configs() {
    // PHP lint+format is poly-native via mago: no phpstan/php-cs-fixer config
    // files, and composer.json carries neither dep nor their scripts.
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
    // Both emitted manifests — the repo-root composer.json (Packagist/PIE) and
    // the package-dir packages/php/composer.json (dev manifest) — are rendered
    // from one builder; assert neither carries a retired PHP tool dep or script.
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
    // Packagist indexes the repo-root composer.json. The scaffold must emit a
    // root composer.json that mirrors the package manifest byte-for-byte except
    // that the PSR-4 autoload src path is repointed from `src/` to
    // `packages/php/src/`, so the same classes resolve when consumers install
    // the package via Composer/PIE from the repo root.
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

    // Parse both as JSON to compare structure independently of formatting
    let pkg: serde_json::Value =
        serde_json::from_str(&pkg_composer.content).expect("packages/php/composer.json must be valid JSON");
    let root: serde_json::Value =
        serde_json::from_str(&root_composer.content).expect("root composer.json must be valid JSON");

    // Root should have the same structure as package except for autoload src and the pie block
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

    // Both composer.json files must have the extra.pie.binary.url-template block
    // (both the dev manifest and Packagist/PIE manifest need it)
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

#[test]
fn test_scaffold_dart() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let files = language_files(&all_files);
    // pubspec.yaml + analysis_options.yaml + .gitignore + .pubignore + test + .editorconfig + README.md + example + CHANGELOG.md
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
    assert!(
        pubspec.content.contains("sdk: '>=3.11.0 <4.0.0'"),
        "got: {}",
        pubspec.content
    );
    assert!(
        pubspec.content.contains("freezed_annotation: '^3.1.0'"),
        "got: {}",
        pubspec.content
    );
    assert!(
        pubspec.content.contains("build_runner: '^2.15.0'"),
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
    // Dart 3.x removed these lints — they must not appear in the rules list.
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
    // analyzer.exclude block silences flutter_rust_bridge-generated paths.
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
    assert!(
        pubignore.content.contains("lib/src/native/"),
        "got: {}",
        pubignore.content
    );
    assert!(pubignore.content.contains("rust/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("example/"), "got: {}", pubignore.content);
    assert!(pubignore.content.contains("test/"), "got: {}", pubignore.content);

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
    assert!(
        test_file.content.contains("expect(1 + 1, equals(2))"),
        "got: {}",
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
    // freezed_annotation/json_annotation/freezed/build_runner/json_serializable are now
    // emitted in both FFI and FRB scaffolds because product-type DTOs are generated via
    // @freezed regardless of the bridge mode (STY-10).
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
}
