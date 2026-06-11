use super::*;
use crate::core::config::{
    Language, NewAlefConfig, PrecommitConfig, PythonConfig, ResolvedCrateConfig, ScaffoldCargoTargets, ScaffoldConfig,
};
use crate::scaffold::languages::generate_pre_commit_config;
use std::path::{Path, PathBuf};

fn test_config() -> ResolvedCrateConfig {
    test_config_from_toml("")
}

fn test_config_from_toml(extra_crate_config: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python", "node"]

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
    cfg.resolve().expect("resolve ok").remove(0)
}

fn minimal_config_from_toml(extra_crate_config: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
{extra_crate_config}
"#,
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

fn test_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Filter out project-level scaffold files (like .pre-commit-config.yaml)
/// to isolate language-specific scaffold tests.
fn language_files(files: &[GeneratedFile]) -> Vec<&GeneratedFile> {
    files
        .iter()
        .filter(|f| {
            let p = f.path.to_string_lossy();
            !p.ends_with(".pre-commit-config.yaml")
                && !p.ends_with(".typos.toml")
                && !p.ends_with("rust-toolchain.toml")
                && !p.ends_with(".cargo/config.toml")
                && p != ".gitattributes"
                // LICENSE files are synced from the workspace root; the consolidated
                // single-crate layout runs tests from the repo root which has a LICENSE
                // file, causing scaffold_license_files() to emit per-package LICENSE
                // entries. Filter them out here so file-count assertions remain stable.
                && !p.ends_with("/LICENSE")
                && p != "LICENSE"
        })
        .collect()
}

#[test]
fn test_scaffold_python() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
    // scaffold_python: pyproject.toml + py.typed; scaffold_python_cargo: Cargo.toml
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].path, PathBuf::from("packages/python/pyproject.toml"));
    assert!(files[0].content.contains("maturin"));
    assert!(files[0].content.contains("my-lib"));
    assert_eq!(files[1].path, PathBuf::from("packages/python/my_lib/py.typed"));
    assert!(
        files[1].content.is_empty(),
        "py.typed must be empty (0 bytes) so end-of-file-fixer leaves it untouched on every regen; a lone trailing newline gets stripped back to empty; content: {:?}",
        files[1].content
    );
    assert_eq!(files[2].path, PathBuf::from("crates/my-lib-py/Cargo.toml"));
    assert!(files[2].content.contains("pyo3"));
}

#[test]
fn test_scaffold_python_central_pyproject_ignores_source_output() {
    let config = test_config_from_toml(
        r#"
[crates.output]
python = "crates/my-lib-py/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);

    assert_eq!(files[0].path, PathBuf::from("packages/python/pyproject.toml"));
    assert!(
        !files
            .iter()
            .any(|file| file.path == Path::new("crates/my-lib-py/src/pyproject.toml")),
        "Python scaffold must not emit the old source-tree pyproject"
    );
}

#[test]
fn test_scaffold_node() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    // scaffold_node: parent package.json + index.js + platform package manifests; scaffold_node_cargo: Cargo.toml.
    // The dead `packages/node/` scaffold (parallel unscoped npm package) was removed —
    // the real publish target is `crates/<crate>-node/` built by NAPI-RS.
    assert_eq!(files.len(), 11);
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-node/package.json"));
    assert!(files[0].content.contains("napi"));
    assert_eq!(files[1].path, PathBuf::from("crates/my-lib-node/index.js"));
    // Verify platform dispatch index contains expected platforms and binary name
    assert!(files[1].content.contains("const { platform, arch } = process"));
    assert!(files[1].content.contains("darwin"));
    assert!(files[1].content.contains("linux"));
    assert!(files[1].content.contains("win32"));
    assert!(files[1].content.contains("my-lib-node.darwin-arm64.node"));
    assert!(files[1].content.contains("tryLoadBinding"));
    let cargo = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/Cargo.toml"))
        .expect("node Cargo.toml must be emitted");
    assert!(cargo.content.contains("napi-derive"));
}

#[test]
fn test_scaffold_node_napi_package_name_matches_scoped_package() {
    // Regression: when `package_name` is a scoped npm name (e.g. `@scope/foo`),
    // napi.packageName must be set to that same scoped name so `napi create-npm-dirs`
    // emits platform sub-packages as `@scope/foo-linux-x64-gnu` etc. — not bare
    // `foo-node-linux-x64-gnu`. The index.js platform-dispatch table must require
    // those scoped optional-dep names accordingly.
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "@scope/foo"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("crate package.json must be emitted");
    let parsed: serde_json::Value =
        serde_json::from_str(&pkg_json.content).expect("emitted package.json must be valid JSON");
    let napi = parsed.get("napi").expect("napi block required");
    assert_eq!(
        napi.get("packageName").and_then(|v| v.as_str()),
        Some("@scope/foo"),
        "napi.packageName must mirror the scoped package_name so platform sub-packages inherit the scope"
    );
    let index_js = files
        .iter()
        .find(|f| f.path.ends_with("index.js"))
        .expect("crate index.js must be emitted");
    assert!(
        index_js.content.contains("\"@scope/foo-linux-x64-gnu\""),
        "index.js optional-dep names must use the scoped napi.packageName"
    );
    assert!(
        !index_js.content.contains("\"my-lib-node-linux-x64-gnu\""),
        "index.js must not fall back to the unscoped binaryName for optional-dep names"
    );
}

#[test]
fn test_scaffold_node_package_json_includes_repository_url() {
    // Regression: npm publish-with-provenance verifies the package's
    // `repository.url` against the provenance attestation and rejects the
    // upload with E422 if the field is missing or empty. The emitted
    // package.json must therefore carry a non-empty `repository.url` sourced
    // from the configured scaffold/registry repository URL.
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let pkg_json = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/package.json"))
        .expect("crate package.json must be emitted");

    let parsed: serde_json::Value =
        serde_json::from_str(&pkg_json.content).expect("emitted package.json must be valid JSON");
    let repository = parsed
        .get("repository")
        .expect("package.json must contain a `repository` field");
    let url = repository
        .get("url")
        .and_then(|v| v.as_str())
        .expect("`repository.url` must be a string");
    assert!(!url.is_empty(), "`repository.url` must not be empty, got: {url}");
    assert!(
        url.contains("github.com/test/my-lib"),
        "`repository.url` must reflect the configured scaffold.repository (https://github.com/test/my-lib), got: {url}"
    );
    assert_eq!(
        repository.get("type").and_then(|v| v.as_str()),
        Some("git"),
        "`repository.type` must be \"git\" for npm provenance verification"
    );
}

#[test]
fn test_scaffold_node_omits_repository_when_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let manifests: Vec<&GeneratedFile> = files
        .iter()
        .copied()
        .filter(|f| f.path.to_string_lossy().ends_with("package.json"))
        .collect();

    assert!(!manifests.is_empty(), "node package.json files must be emitted");
    for manifest in manifests {
        let parsed: serde_json::Value =
            serde_json::from_str(&manifest.content).expect("emitted package.json must be valid JSON");
        assert!(
            parsed.get("repository").is_none(),
            "unconfigured npm manifest must not invent repository metadata in {}:\n{}",
            manifest.path.display(),
            manifest.content
        );
    }
}

#[test]
fn test_scaffold_csharp_omits_repository_when_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Csharp]).unwrap();
    let files = language_files(&all_files);
    let csproj = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".csproj"))
        .expect("C# project file must be emitted");

    assert!(
        !csproj.content.contains("<RepositoryUrl>"),
        "unconfigured C# scaffold must not invent repository metadata:\n{}",
        csproj.content
    );
}

#[test]
fn test_scaffold_wasm_omits_repository_when_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let files = language_files(&all_files);
    let package_json = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-wasm/package.json"))
        .expect("WASM package.json must be emitted");
    let parsed: serde_json::Value =
        serde_json::from_str(&package_json.content).expect("emitted package.json must be valid JSON");

    assert!(
        parsed.get("repository").is_none(),
        "unconfigured WASM manifest must not invent repository metadata:\n{}",
        package_json.content
    );
}

#[test]
fn test_scaffold_java_requires_publish_metadata() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::Java]).expect_err("Java scaffold must require publish metadata");

    assert!(
        err.to_string()
            .contains("Java scaffold requires package metadata repository"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_kotlin_requires_publish_metadata() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::Kotlin]).expect_err("Kotlin scaffold must require publish metadata");

    assert!(
        err.to_string()
            .contains("Kotlin scaffold requires package metadata repository"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_r_requires_authors() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let err = scaffold(&api, &config, &[Language::R]).expect_err("R scaffold must require authors");

    assert!(
        err.to_string().contains("R scaffold requires package metadata authors"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_scaffold_multiple() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
    let files = language_files(&all_files);
    // Python: 3 files; Node: 11 files (parent manifest, loader, platform manifests, Cargo.toml).
    assert_eq!(files.len(), 14);
}

#[test]
fn test_scaffold_gitattributes_covers_all_generated_dirs() {
    // Test that .gitattributes is emitted and covers: language package dirs,
    // binding crate dirs (py/php/ffi/jni are separate from package_dir), and e2e/.
    let config = test_config();
    let api = test_api();

    // Python + Node: the default test_config languages.
    let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted by scaffold");

    assert!(
        !ga.generated_header,
        "generated_header must be false — create-once seed"
    );

    let content = &ga.content;
    // Package directories
    assert!(content.contains("packages/python/**"), "must cover Python package dir");
    assert!(content.contains("crates/my-lib-node/**"), "must cover Node crate dir");
    // Binding crate separate from package_dir
    assert!(content.contains("crates/my-lib-py/**"), "must cover PyO3 binding crate");
    // e2e is always included regardless of language selection
    assert!(content.contains("e2e/**"), "must cover e2e test output");
    // All entries carry the linguist attribute
    for line in content.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        assert!(
            line.ends_with("linguist-generated=true"),
            "every non-comment line must set linguist-generated=true, got: {line}"
        );
    }
}

#[test]
fn test_scaffold_gitattributes_ffi_and_jni_use_crate_dirs() {
    // FFI and JNI don't have packages/ dirs — their output is the binding crate itself.
    let config = test_config();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Ffi, Language::Jni]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    let content = &ga.content;
    assert!(content.contains("crates/my-lib-ffi/**"), "must cover FFI crate dir");
    assert!(content.contains("crates/my-lib-jni/**"), "must cover JNI crate dir");
    assert!(!content.contains("packages/ffi"), "must not emit bogus packages/ffi");
    assert!(!content.contains("packages/jni"), "must not emit bogus packages/jni");
}

#[test]
fn test_scaffold_gitattributes_kotlin_native_uses_kotlin_native_dir() {
    use crate::core::config::NewAlefConfig;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://github.com/test/my-lib"

[crates.kotlin]
target = "native"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-native/**"),
        "native target must use packages/kotlin-native, got:\n{}",
        ga.content
    );
    assert!(
        !ga.content.contains("packages/kotlin/**"),
        "native target must not emit JVM dir, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_gitattributes_kotlin_mpp_uses_kotlin_mpp_dir() {
    use crate::core::config::NewAlefConfig;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://github.com/test/my-lib"

[crates.kotlin]
mode = "kmp"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-mpp/**"),
        "kmp mode must use packages/kotlin-mpp, got:\n{}",
        ga.content
    );
    assert!(
        !ga.content.contains("packages/kotlin/**"),
        "kmp mode must not emit JVM dir, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_gitattributes_kotlin_multiplatform_target_uses_kotlin_mpp_dir() {
    // target = "multiplatform" (no mode) must also resolve to packages/kotlin-mpp/
    use crate::core::config::NewAlefConfig;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test"
license = "MIT"
repository = "https://github.com/test/my-lib"

[crates.kotlin]
target = "multiplatform"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-mpp/**"),
        "target=multiplatform must use packages/kotlin-mpp, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_gitattributes_kotlin_android_uses_kotlin_android_dir() {
    let config = test_config();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::KotlinAndroid]).unwrap();
    let ga = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".gitattributes"))
        .expect(".gitattributes must be emitted");

    assert!(
        ga.content.contains("packages/kotlin-android/**"),
        "KotlinAndroid must use packages/kotlin-android, got:\n{}",
        ga.content
    );
}

#[test]
fn test_scaffold_python_production_features() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let content = &files[0].content;
    assert!(content.contains("urls.repository"));
    assert!(content.contains("repository ="));
    // Linter config (ruff) is included in the generated pyproject.toml
    assert!(content.contains("[tool.ruff]"));
}

#[test]
fn test_scaffold_python_pyproject_canonical_format() {
    // Verify pyproject.toml is emitted in pyproject-fmt canonical form:
    // - build-backend before requires in [build-system]
    // - arrays with spaces: [ "a", "b" ]
    // - sorted keywords
    // - dot-syntax for nested tables: urls.repository instead of [project.urls]
    // - tool sections use dot-syntax: lint.* instead of [tool.ruff.lint]
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Bob"]
keywords = ["zebra", "apple", "banana"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let content = &files[0].content;

    // Check build-system section ordering: build-backend before requires
    let build_system_section = content
        .split("[project]")
        .next()
        .expect("should have [project] section");
    let backend_idx = build_system_section
        .find("build-backend")
        .expect("should have build-backend");
    let requires_idx = build_system_section.find("requires").expect("should have requires");
    assert!(
        backend_idx < requires_idx,
        "build-backend should come before requires in [build-system]"
    );

    // Short arrays stay inline with inner spaces (pyproject-fmt collapses any
    // array whose total inline width fits in column_width=80, so emitting
    // them inline natively keeps the file pyproject-fmt-clean across regens).
    assert!(
        content.contains("requires = [ \"maturin"),
        "single-element requires array should stay inline with inner spaces. got:\n{content}",
    );

    // Multi-item arrays still inline-with-spaces when the total length fits
    // within pyproject-fmt's 80-char column width (`keywords = [ "apple",
    // "banana", "zebra" ]` is well under that), matching what prek's
    // pyproject-fmt hook would otherwise rewrite on every regen.
    assert!(
        content.contains("keywords = [ \"apple\", \"banana\", \"zebra\" ]"),
        "short multi-item keywords array should stay inline, alphabetised. got:\n{content}",
    );

    // Check dot-syntax for URLs: urls.repository instead of [project.urls]
    assert!(
        !content.contains("[project.urls]"),
        "should use dot-syntax urls.repository, not [project.urls] section"
    );
    assert!(
        content.contains("urls.repository = "),
        "should have urls.repository in dot-syntax"
    );

    // Check tool.ruff uses dot-syntax for nested sections
    assert!(
        !content.contains("[tool.ruff.lint]"),
        "should use dot-syntax lint.*, not [tool.ruff.lint]"
    );
    assert!(
        content.contains("lint.select = "),
        "should have lint.select in dot-syntax"
    );
    assert!(
        content.contains("lint.mccabe.max-complexity"),
        "should have lint.mccabe.max-complexity in dot-syntax"
    );

    // lint.ignore is a long alphabetised list whose inline form blows past
    // pyproject-fmt's 80-char column width, so it must expand to one element
    // per line with a trailing comma after the last entry.
    assert!(
        content.contains("lint.ignore = [\n  \"ANN401\","),
        "long lint.ignore array should expand to multi-line. got:\n{content}"
    );

    // The mypy overrides table must be rendered as an array of inline tables
    // (`overrides = [ { module = ..., disable_error_code = [...] } ]`) rather
    // than the `[[tool.mypy.overrides]]` block form: pyproject-fmt rewrites
    // every block-form override into the inline-array form, so emitting the
    // block form ourselves triggers a rewrite on every regen.
    assert!(
        !content.contains("[[tool.mypy.overrides]]"),
        "tool.mypy.overrides must be rendered inline, not as [[tool.mypy.overrides]]. got:\n{content}"
    );
    assert!(
        content.contains("overrides = [\n") && content.contains("disable_error_code = ["),
        "tool.mypy.overrides must be rendered as inline-table array with disable_error_code. got:\n{content}"
    );
}

/// The generated `pyproject.toml` must already be in `pyproject-fmt` canonical form so the
/// `pyproject-fmt` pre-commit hook is a no-op on every regen. Running `pyproject-fmt` on our
/// output must produce zero changes — otherwise the hook rewrites the alef-hash-tracked file
/// and breaks `alef verify`. Skips when the `pyproject-fmt` binary is unavailable.
#[test]
fn test_scaffold_python_pyproject_is_pyproject_fmt_clean() {
    use std::process::Command;

    if Command::new("pyproject-fmt").arg("--version").output().is_err() {
        eprintln!("skipping: pyproject-fmt not installed");
        return;
    }

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Bob"]
keywords = ["zebra", "apple", "banana"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let content = &files[0].content;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pyproject.toml");
    std::fs::write(&path, content).unwrap();

    // Format in place, then compare bytes. A single subprocess avoids the flakiness of
    // spawning pyproject-fmt twice, and comparing content (rather than the `--check` exit
    // status) is robust to non-formatting exit codes.
    let spawn = Command::new("pyproject-fmt").arg(&path).output();
    let Ok(output) = spawn else {
        eprintln!("skipping: pyproject-fmt failed to spawn");
        return;
    };
    let formatted = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        &formatted,
        content,
        "generated pyproject.toml is not pyproject-fmt-clean.\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn test_scaffold_python_license_files_field() {
    // Verify that pyproject.toml includes license-files = ["LICENSE"] to ensure
    // maturin bundles the LICENSE file in the wheel. This fixes BLK-10 where PyPI
    // rejected wheels with License-File: LICENSE in METADATA but no actual LICENSE
    // in the wheel root.
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let pyproject_content = &files[0].content;

    // Verify license-files field is present and correctly formatted
    // (inline with inner spaces — pyproject-fmt canonical).
    assert!(
        pyproject_content.contains("license-files = [ \"LICENSE\" ]"),
        "pyproject.toml should declare license-files = [ \"LICENSE\" ] (with inner spaces)"
    );

    // Verify it appears in the [project] section after the license field
    let project_section = pyproject_content
        .split("[tool.maturin]")
        .next()
        .expect("should have [project] section before [tool.maturin]");
    assert!(
        project_section.contains("license = \"MIT\""),
        "should have license field"
    );
    let license_idx = project_section
        .find("license = \"MIT\"")
        .expect("should find license field");
    let license_files_idx = project_section
        .find("license-files = [ \"LICENSE\" ]")
        .expect("should find license-files field");
    assert!(
        license_idx < license_files_idx,
        "license-files should come after license in [project]"
    );
}

#[test]
fn test_scaffold_node_production_features() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    // files[0] is the crate-level package.json (the real NAPI-RS publish target).
    let content = &files[0].content;
    assert!(content.contains("\"scripts\""));
    assert!(content.contains("\"build\""));
    assert!(content.contains("\"files\""));
    assert!(content.contains("\"devDependencies\""));
    assert!(content.contains("@napi-rs/cli"));
    // Crate-level NAPI package.json uses `targets` (modern NAPI-RS field), not `triples`.
    assert!(content.contains("\"targets\""));
}

#[test]
fn test_scaffold_node_package_json_centralizes_platform_metadata() {
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let parent = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/package.json"))
        .expect("parent package.json must be emitted");
    let parsed: serde_json::Value = serde_json::from_str(&parent.content).expect("valid parent package.json");
    let optional_deps = parsed["optionalDependencies"]
        .as_object()
        .expect("optionalDependencies must be an object");
    assert!(optional_deps.contains_key("my-lib-win32-arm64-msvc"));
    assert!(optional_deps.contains_key("my-lib-linux-x64-musl"));
    assert_eq!(parsed["engines"]["node"], ">= 18");
    assert_eq!(parsed["publishConfig"]["access"], "public");
    assert_eq!(parsed["exports"]["."]["types"], "./index.d.ts");

    let targets = parsed["napi"]["targets"]
        .as_array()
        .expect("napi.targets must be an array");
    assert!(targets.iter().any(|target| target == "aarch64-pc-windows-msvc"));
    assert!(targets.iter().any(|target| target == "x86_64-unknown-linux-musl"));

    let platform = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/npm/linux-x64-musl/package.json"))
        .expect("musl platform package manifest must be emitted");
    let platform_json: serde_json::Value =
        serde_json::from_str(&platform.content).expect("valid platform package.json");
    assert_eq!(platform_json["name"], "my-lib-linux-x64-musl");
    assert_eq!(platform_json["libc"][0], "musl");
    assert_eq!(platform_json["main"], "my-lib-node.linux-x64-musl.node");
    assert_eq!(platform_json["publishConfig"]["access"], "public");
}

#[test]
fn test_scaffold_node_exclude_platforms_drops_musl() {
    let config = test_config_from_toml(
        r#"
[crates.node]
exclude_platforms = ["linux-x64-musl", "linux-arm64-musl"]
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();

    let parent = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/package.json"))
        .expect("parent package.json must be emitted");
    let parsed: serde_json::Value = serde_json::from_str(&parent.content).expect("valid parent package.json");

    let optional_deps = parsed["optionalDependencies"]
        .as_object()
        .expect("optionalDependencies must be an object");
    assert!(
        !optional_deps.contains_key("my-lib-linux-x64-musl"),
        "linux-x64-musl must be excluded from optionalDependencies"
    );
    assert!(
        !optional_deps.contains_key("my-lib-linux-arm64-musl"),
        "linux-arm64-musl must be excluded from optionalDependencies"
    );
    assert!(
        optional_deps.contains_key("my-lib-linux-x64-gnu"),
        "linux-x64-gnu must still be present"
    );
    assert!(
        optional_deps.contains_key("my-lib-darwin-arm64"),
        "darwin-arm64 must still be present"
    );

    let targets = parsed["napi"]["targets"]
        .as_array()
        .expect("napi.targets must be an array");
    assert!(
        !targets.iter().any(|t| t == "x86_64-unknown-linux-musl"),
        "x86_64-unknown-linux-musl must be excluded from napi.targets"
    );
    assert!(
        !targets.iter().any(|t| t == "aarch64-unknown-linux-musl"),
        "aarch64-unknown-linux-musl must be excluded from napi.targets"
    );
    assert!(
        targets.iter().any(|t| t == "x86_64-unknown-linux-gnu"),
        "x86_64-unknown-linux-gnu must still be present"
    );
    assert!(
        targets.iter().any(|t| t == "aarch64-pc-windows-msvc"),
        "aarch64-pc-windows-msvc must still be present"
    );

    assert!(
        !files
            .iter()
            .any(|f| f.path == Path::new("crates/my-lib-node/npm/linux-x64-musl/package.json")),
        "linux-x64-musl per-platform stub must not be emitted"
    );
    assert!(
        !files
            .iter()
            .any(|f| f.path == Path::new("crates/my-lib-node/npm/linux-arm64-musl/package.json")),
        "linux-arm64-musl per-platform stub must not be emitted"
    );
    assert!(
        files
            .iter()
            .any(|f| f.path == Path::new("crates/my-lib-node/npm/linux-x64-gnu/package.json")),
        "linux-x64-gnu per-platform stub must still be emitted"
    );

    let index_js = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/index.js"))
        .expect("index.js must be emitted");
    assert!(
        !index_js.content.contains("linux-x64-musl"),
        "index.js dispatch table must not reference linux-x64-musl"
    );
    assert!(
        !index_js.content.contains("linux-arm64-musl"),
        "index.js dispatch table must not reference linux-arm64-musl"
    );
    assert!(
        index_js.content.contains("linux-x64-gnu"),
        "index.js dispatch table must still reference linux-x64-gnu"
    );
}

#[test]
fn test_scaffold_ffi_with_core_import() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 2);
    let cargo_toml = &files[0].content;
    assert!(cargo_toml.contains("serde"));
    assert!(cargo_toml.contains("serde_json"));
    // Should have core_import as dependency
    assert!(cargo_toml.contains("my-lib ="));
    // Should generate cmake config
    let cmake = &files[1].content;
    assert!(cmake.contains("find_package"));
    assert!(cmake.contains("my-lib-ffi::my-lib-ffi"));
}

#[test]
fn test_scaffold_ffi_deps_are_pinned() {
    // Audit: FFI Cargo.toml ships sensible, current dependency pins.
    // Bumping cbindgen requires re-generating headers; treat this test as a
    // canary — if it fails, audit cbindgen's changelog before adjusting.
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;
    assert!(
        cargo_toml.contains("cbindgen = \"0.29\""),
        "cbindgen should be pinned to a specific minor for reproducible headers"
    );
    assert!(cargo_toml.contains("serde_json = \"1\""));
    assert!(cargo_toml.contains("tokio = "));
    assert!(cargo_toml.contains("[dev-dependencies]"));
    assert!(cargo_toml.contains("tempfile = \"3\""));
}

#[test]
fn test_scaffold_ffi_merges_extra_dependencies() {
    // Multi-crate workspaces (e.g. mylib's mylib-core/-http/-extra) emit FFI
    // bindings that reference qualified crate paths. The scaffold must merge
    // [crate.extra_dependencies] from alef.toml so the generated cdylib can
    // resolve those imports.
    let mut config = test_config();
    let mut deps: std::collections::HashMap<String, toml::Value> = Default::default();
    deps.insert(
        "my-lib-http".to_string(),
        toml::Value::try_from(toml::Table::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-http".to_string()),
        )]))
        .unwrap(),
    );
    deps.insert(
        "my-lib-graphql".to_string(),
        toml::Value::try_from(toml::Table::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-graphql".to_string()),
        )]))
        .unwrap(),
    );
    deps.insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies = deps;

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;
    assert!(
        cargo_toml.contains("my-lib-http = { path = \"../my-lib-http\" }"),
        "scaffold should emit my-lib-http path dep, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib-graphql = { path = \"../my-lib-graphql\" }"),
        "scaffold should emit my-lib-graphql path dep, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("anyhow = \"1.0\""),
        "scaffold should emit anyhow string dep, got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_ffi_injects_version_for_workspace_member_deps() {
    use std::fs;
    use tempfile::TempDir;

    // `cargo publish` rejects path-only deps: "all dependencies must have a
    // version requirement specified when publishing". Every internal workspace
    // dep the FFI/umbrella manifest pulls in (auto-detected from the public
    // surface via `[crate.extra_dependencies]`) must therefore carry the
    // resolved workspace version alongside its path, mirroring the core dep.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib-core", "crates/my-lib-http"]

[workspace.package]
version = "4.2.0"
"#,
    )
    .unwrap();
    for member in ["my-lib-core", "my-lib-http"] {
        fs::create_dir_all(root.join(format!("crates/{member}/src"))).unwrap();
        fs::write(root.join(format!("crates/{member}/src/lib.rs")), "pub fn f() {}").unwrap();
        fs::write(
            root.join(format!("crates/{member}/Cargo.toml")),
            format!("[package]\nname = \"{member}\"\nversion.workspace = true\n"),
        )
        .unwrap();
    }

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    let mut deps: std::collections::HashMap<String, toml::Value> = Default::default();
    // Path-only internal workspace member deps (as auto-detected and emitted
    // into [crate.extra_dependencies]).
    for member in ["my-lib-core", "my-lib-http"] {
        deps.insert(
            member.to_string(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "path".to_string(),
                toml::Value::String(format!("../{member}")),
            )])),
        );
    }
    // A genuinely external dep must stay untouched (no spurious version inject).
    deps.insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies = deps;

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    for member in ["my-lib-core", "my-lib-http"] {
        // Each internal member dep must carry the injected workspace version.
        assert!(
            cargo_toml.contains(&format!("{member} = {{ path = \"../{member}\", version = \"4.2.0\" }}")),
            "FFI manifest must version-inject internal workspace dep {member}; got:\n{cargo_toml}"
        );
    }
    // External dep unchanged.
    assert!(
        cargo_toml.contains("anyhow = \"1.0\""),
        "external dep must be emitted unchanged, got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_ffi_target_dep_overrides_emit_cfg_blocks() {
    // When FfiConfig.target_dep_overrides is configured, the core-crate
    // dependency moves out of the main [dependencies] table into per-cfg
    // [target.'cfg(...)'.dependencies] tables. This is the only shape that
    // satisfies targets whose feature set differs from the default, e.g.
    // x86_64-linux-android (no ONNX Runtime prebuilt) needs the
    // `android-target` feature instead of `full`.
    use crate::core::config::FfiTargetDepOverride;
    use crate::core::config::languages::FfiConfig;

    let mut config = test_config();
    config.features = vec!["full".to_string(), "ocr".to_string()];
    config.ffi = Some(FfiConfig {
        prefix: None,
        error_style: "last_error".to_string(),
        header_name: None,
        lib_name: None,
        visitor_callbacks: false,
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        rename_fields: Default::default(),
        plugin_error_constructor: None,
        target_dep_overrides: vec![FfiTargetDepOverride {
            cfg: "all(target_os = \"android\", target_arch = \"x86_64\")".to_string(),
            features: vec!["android-target".to_string()],
        }],
    });

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = &files[0].content;

    // The default branch is wrapped in cfg(not(<override-cfg>)).
    assert!(
        cargo_toml.contains("[target.'cfg(not(all(target_os = \"android\", target_arch = \"x86_64\")))'.dependencies]"),
        "expected default-branch target table with cfg(not(...)), got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib = { path = \"../my-lib\", version = \"0.1.0\", features = [\"full\", \"ocr\"] }"),
        "default branch should keep the full feature set, got:\n{cargo_toml}"
    );

    // The override branch keeps the explicit cfg and a reduced feature set.
    assert!(
        cargo_toml.contains("[target.'cfg(all(target_os = \"android\", target_arch = \"x86_64\"))'.dependencies]"),
        "expected override target table, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("my-lib = { path = \"../my-lib\", version = \"0.1.0\", features = [\"android-target\"] }"),
        "override branch should emit android-target feature, got:\n{cargo_toml}"
    );

    // The main [dependencies] table still exists for ahash/serde_json/tokio but
    // no longer contains the core-crate line.
    assert!(cargo_toml.contains("[dependencies]\nahash = \"0.8\""));
    assert!(
        !cargo_toml.contains("\n[dependencies]\nmy-lib ="),
        "core-crate dep should have moved out of [dependencies], got:\n{cargo_toml}"
    );
}

#[test]
fn test_scaffold_go_production_format() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    // go.mod + .golangci.yml + .lib/.gitkeep
    assert_eq!(files.len(), 3);
    let content = &files[0].content;
    assert!(content.contains("go 1.26"));
    assert!(!content.contains("require ("));
}

#[test]
fn test_scaffold_go_uses_inert_module_when_unconfigured() {
    let config = minimal_config_from_toml(
        r#"
[crates.go]
module_major = 5
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    let go_mod = files
        .iter()
        .find(|f| f.path == Path::new("packages/go/v5/go.mod"))
        .expect("go.mod must be emitted");

    assert!(
        go_mod.content.starts_with("module example.invalid/my-lib\n"),
        "unconfigured Go scaffold must use inert example.invalid fallback, got:\n{}",
        go_mod.content
    );
}

#[test]
fn test_scaffold_java_production_features() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    // pom.xml + checkstyle.xml + checkstyle.properties + checkstyle-suppressions.xml
    // + eclipse-formatter.xml + versions-rules.xml + pmd-ruleset.xml
    assert_eq!(files.len(), 7);
    let content = &files[0].content;
    assert!(content.contains("<properties>"));
    assert!(content.contains("<project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>"));
    assert!(content.contains("<dependencies>"));
    assert!(content.contains("<build>"));
    assert!(content.contains("maven-compiler-plugin"));
    assert!(content.contains("maven-surefire-plugin"));
    assert!(content.contains("--enable-native-access=ALL-UNNAMED"));
    assert!(content.contains("-Djava.library.path=${project.basedir}/../../target/release"));
}

#[test]
fn test_scaffold_java_scm_uses_configured_non_github_host() {
    let config = minimal_config_from_toml(
        r#"
[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://gitlab.example.com/acme/my-lib"
authors = ["Alice"]
keywords = ["test"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files
        .iter()
        .find(|f| f.path == Path::new("packages/java/pom.xml"))
        .expect("pom.xml must be emitted");

    assert!(pom.content.contains("scm:git:git://gitlab.example.com/acme/my-lib.git"));
    assert!(
        pom.content
            .contains("scm:git:ssh://git@gitlab.example.com/acme/my-lib.git")
    );
    assert!(!pom.content.contains("github.com/acme/my-lib"));
}

#[test]
fn test_scaffold_ruby_production_features() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    // scaffold_ruby: gemspec, rubocop, Rakefile, extconf.rb, Gemfile, Steepfile = 6 files.
    // The `lib/<gem>.rb` entry point is emitted by the magnus backend (gen_bindings),
    // not the scaffold — it requires `<gem>/native` and `<gem>/version`.
    // scaffold_ruby_cargo: Cargo.toml = 1 file
    assert_eq!(files.len(), 7);
    let content = &files[0].content;
    assert!(content.contains("spec.required_ruby_version"));
    assert!(content.contains("spec.extensions"));
    assert!(content.contains("README*"));
    assert!(content.contains("LICENSE*"));
    assert!(content.contains("lib/**/*"));
    assert!(content.contains("ext/**/*"));
    assert!(content.contains("sig/**/*"));
    assert!(content.contains("spec.metadata[\"keywords\"]"));
    assert!(content.contains("frozen_string_literal: true"));
    assert!(content.contains("spec.metadata[\"rubygems_mfa_required\"] = \"true\""));
    // Check for .rubocop.yml generation
    assert_eq!(files[1].path, PathBuf::from("packages/ruby/.rubocop.yml"));
    // Check for Rakefile generation
    assert_eq!(files[2].path, PathBuf::from("packages/ruby/Rakefile"));
    assert!(files[2].content.contains("RbSys::ExtensionTask"));
    assert!(files[2].content.contains("my_lib_rb"));
    assert!(files[2].content.contains("require \"rb_sys/extensiontask\""));
    assert!(files[2].content.contains("MANIFEST_PATH"));
    assert!(files[2].content.contains("--manifest-path"));
    assert!(files[2].content.contains("task compile: \"compile:ruby\""));
    // Check for extconf.rb generation
    assert_eq!(
        files[3].path,
        PathBuf::from("packages/ruby/ext/my_lib_rb/native/extconf.rb")
    );
    assert!(files[3].content.contains("create_rust_makefile"));
    assert!(files[3].content.contains("rb_sys/mkmf"));
    assert!(
        files[3].content.contains("config.ext_dir = \".\""),
        "extconf.rb must set ext_dir = \".\" so rb_sys finds the sibling Cargo.toml"
    );
    // files[4] is Gemfile; files[5] is Steepfile; files[6] is the Cargo.toml from scaffold_ruby_cargo
    assert_eq!(files[4].path, PathBuf::from("packages/ruby/Gemfile"));
    assert_eq!(files[5].path, PathBuf::from("packages/ruby/Steepfile"));
    // Check for Cargo.toml generation
    assert_eq!(
        files[6].path,
        PathBuf::from("packages/ruby/ext/my_lib_rb/native/Cargo.toml")
    );
    assert!(files[6].content.contains("magnus"));
    assert!(
        files[6].content.contains("path = \"../src/lib.rs\""),
        "Ruby Cargo.toml [lib] must set path to the binding source crate"
    );
}

/// Regression: the generated gemspec must declare `sorbet-runtime` as a runtime
/// dependency so projects running `bundle install --without development` can load
/// the `native.rb` wrapper, which unconditionally `require 'sorbet-runtime'`.
/// Missing the dep caused `LoadError: cannot load such file -- sorbet-runtime`
/// in CI E2E runs.
#[test]
fn test_scaffold_ruby_gemspec_includes_sorbet_runtime_dependency() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    // files[0] is the gemspec
    let gemspec = &files[0].content;
    assert!(
        gemspec.contains("sorbet-runtime"),
        "gemspec must add sorbet-runtime as a runtime dependency; got:\n{gemspec}"
    );
    assert!(
        gemspec.contains("spec.add_dependency \"sorbet-runtime\""),
        "gemspec must use spec.add_dependency (not add_development_dependency) for sorbet-runtime; got:\n{gemspec}"
    );
    assert!(
        gemspec.contains("~> 0.5"),
        "sorbet-runtime dependency must carry a ~> 0.5 version constraint; got:\n{gemspec}"
    );
}

#[test]
fn test_pre_commit_config_python_node() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Python, Language::Node]);
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    // Common hooks always present
    assert!(content.contains("cargo-fmt"));
    assert!(content.contains("cargo-clippy"));
    assert!(content.contains("trailing-whitespace"));
    assert!(content.contains("cargo-deny"));
    // Python-specific TOML formatting
    assert!(content.contains("pyproject-fmt"));
    // Alef hooks are opt-in — not present in default config
    assert!(!content.contains("alef-readme"));
    assert!(!content.contains("alef-verify"));
    // No per-language hooks
    assert!(!content.contains("ruff-pre-commit"));
    assert!(!content.contains("oxlint"));
    assert!(!content.contains("php-lint"));
    assert!(!content.contains("golangci-lint"));
    assert!(!content.contains("mix-credo"));
}

#[test]
fn test_pre_commit_config_ffi_only() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Ffi]);
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    // Common + Rust hooks
    assert!(content.contains("cargo-fmt"));
    assert!(content.contains("cargo-clippy"));
    // Alef hooks are opt-in — not present in default config
    assert!(!content.contains("alef-verify"));
    assert!(!content.contains("alef-readme"));
    // No per-language hooks
    assert!(!content.contains("clang-format"));
    assert!(!content.contains("ruff"));
    assert!(!content.contains(concat!("bio", "me")));
}

#[test]
fn test_pre_commit_config_clippy_excludes() {
    let config = test_config();
    let files = generate_pre_commit_config(
        &config,
        &[Language::Python, Language::Node, Language::Php, Language::Wasm],
    );
    let content = &files[0].content;
    assert!(content.contains("--exclude=my-lib-py"));
    assert!(content.contains("--exclude=my-lib-node"));
    assert!(content.contains("--exclude=my-lib-php"));
    // Wasm is NOT excluded — rust-toolchain.toml provides the target
    assert!(!content.contains("--exclude=my-lib-wasm"));
    // Ruby not in languages, should not be excluded
    assert!(!content.contains("--exclude=my-lib-rb"));
}

#[test]
fn test_pre_commit_config_all_languages() {
    let config = test_config();
    let files = generate_pre_commit_config(
        &config,
        &[
            Language::Python,
            Language::Node,
            Language::Ruby,
            Language::Php,
            Language::Ffi,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
            Language::R,
        ],
    );
    let content = &files[0].content;
    // Common hooks always present
    assert!(content.contains("cargo-fmt"));
    assert!(content.contains("cargo-clippy"));
    assert!(content.contains("trailing-whitespace"));
    assert!(content.contains("typos"));
    // Python-specific TOML formatting
    assert!(content.contains("pyproject-fmt"));
    // Alef hooks are opt-in — not present in default config
    assert!(!content.contains("alef-readme"));
    assert!(!content.contains("alef-verify"));
    // Clippy excludes for all binding crates
    assert!(content.contains("--exclude=my-lib-py"));
    assert!(content.contains("--exclude=my-lib-node"));
    assert!(content.contains("--exclude=my-lib-rb"));
    assert!(content.contains("--exclude=my-lib-php"));
    assert!(content.contains("--exclude=my-lib-r"));
    // No per-language hooks
    assert!(!content.contains("ruff"));
    assert!(!content.contains("oxlint"));
    assert!(!content.contains("clang-format"));
    assert!(!content.contains("golangci-lint"));
    assert!(!content.contains("cpd"));
    assert!(!content.contains("dotnet-format"));
    assert!(!content.contains("mix-credo"));
    assert!(!content.contains("rubocop"));
    assert!(!content.contains("php-lint"));
    assert!(!content.contains("r-lintr"));
}

#[test]
fn test_pre_commit_config_uses_default_shared_repo() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Python]);
    let content = &files[0].content;
    // All cargo/rumdl/typos/pyproject-fmt/file-safety hooks live under the
    // single shared repo as of the configured hooks bundle v2.0.0.
    assert!(content.contains("https://github.com/kreuzberg-dev/pre-commit-hooks"));
    // The dropped upstream sources must NOT reappear in the scaffold output.
    assert!(!content.contains("https://github.com/pre-commit/pre-commit-hooks"));
    assert!(!content.contains("AndrejOrsula/pre-commit-cargo"));
    assert!(!content.contains("DevinR528/cargo-sort"));
    assert!(!content.contains("bnjbvr/cargo-machete"));
    assert!(!content.contains("EmbarkStudios/cargo-deny"));
    assert!(!content.contains("rvben/rumdl-pre-commit"));
    assert!(!content.contains("tox-dev/pyproject-fmt"));
    assert!(!content.contains("crate-ci/typos"));
}

#[test]
fn test_pre_commit_config_includes_rust_max_lines() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Ffi]);
    let content = &files[0].content;
    // rust-max-lines lives under the shared block; default --max=1000.
    assert!(content.contains("rust-max-lines"));
    assert!(content.contains("--max=1000"));
}

#[test]
fn test_pre_commit_config_includes_new_file_safety_hooks() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Ffi]);
    let content = &files[0].content;
    // Added in the shared hooks bundle v2.0.0.
    assert!(content.contains("check-executables-have-shebangs"));
    assert!(content.contains("check-shebang-scripts-are-executable"));
    assert!(content.contains("mixed-line-ending"));
}

#[test]
fn test_pre_commit_config_drops_cargo_check_hook() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Ffi]);
    let content = &files[0].content;
    // cargo-check was removed in v2.0.0 — cargo-clippy with -D warnings
    // already runs the same compile pipeline plus the clippy lints.
    assert!(!content.contains("id: cargo-check"));
}

// --- Oxc toolchain tests ---

#[test]
fn test_node_scaffold_uses_oxc_tooling() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    for f in &files {
        assert!(
            !f.content.contains(concat!("bio", "me")),
            "File {} should not reference the legacy Node formatter: found in content",
            f.path.display()
        );
        assert!(
            !f.path.to_string_lossy().contains(concat!("bio", "me")),
            "File path should not contain the legacy Node formatter: {}",
            f.path.display()
        );
    }
}

// The dead `packages/node/` scaffold previously emitted `.oxfmtrc.json`,
// `.oxlintrc.json`, and a top-level `package.json` with `oxfmt`/`oxlint`
// dev-deps. With that scaffold removed, the only `package.json` we emit is
// the crate-level NAPI-RS manifest at `crates/<crate>-node/`, which doesn't
// run formatting/linting (those are managed at the workspace root). The
// previous tests asserting on those files are intentionally removed.

#[test]
fn test_precommit_uses_unified_hooks_with_node() {
    let config = test_config();
    let files = generate_pre_commit_config(&config, &[Language::Node]);
    let content = &files[0].content;
    assert!(!content.contains(concat!("bio", "me", "-format")));
    assert!(!content.contains(concat!("bio", "me", "-lint")));
    assert!(!content.contains(concat!("bio", "me", "js")));
    assert!(!content.contains("alef-readme"));
    assert!(!content.contains("alef-verify"));
    assert!(!content.contains("oxlint"));
}

#[test]
fn test_precommit_includes_alef_hooks_when_explicitly_enabled() {
    let mut config = test_config();
    config.scaffold.as_mut().unwrap().precommit = Some(PrecommitConfig {
        include_shared_hooks: None,
        shared_hooks_repo: None,
        shared_hooks_rev: None,
        include_alef_hooks: Some(true),
        alef_hooks_repo: None,
        alef_hooks_rev: None,
    });

    let files = generate_pre_commit_config(&config, &[Language::Node]);
    let content = &files[0].content;

    assert!(content.contains("- repo: local"));
    assert!(content.contains("alef-readme"));
    assert!(content.contains("alef-verify"));
    assert!(content.contains("alef-sync-versions"));
}

#[test]
fn test_precommit_uses_configured_hook_repositories() {
    let mut config = test_config();
    config.scaffold.as_mut().unwrap().precommit = Some(PrecommitConfig {
        include_shared_hooks: Some(true),
        shared_hooks_repo: Some("https://github.com/acme/hooks".to_string()),
        shared_hooks_rev: Some("v9.8.7".to_string()),
        include_alef_hooks: Some(false),
        alef_hooks_repo: None,
        alef_hooks_rev: None,
    });

    let files = generate_pre_commit_config(&config, &[Language::Node]);
    let content = &files[0].content;

    assert!(content.contains("https://github.com/acme/hooks"));
    assert!(content.contains("rev: v9.8.7"));
    assert!(!content.contains("https://github.com/sample_crate-dev/alef"));
    assert!(!content.contains("alef-readme"));
}

#[test]
fn test_precommit_defaults_do_not_invent_alef_remote_or_bot_identity() {
    let config = minimal_config_from_toml("");
    let files = generate_pre_commit_config(&config, &[Language::Node]);
    let content = &files[0].content;
    let project_org = format!("{}-{}", "project", "dev");

    assert!(
        !content.contains(&project_org) && !content.contains("project-bot") && !content.contains("bot@"),
        "unconfigured project precommit scaffold must not copy Alef organization, repo, or bot metadata:\n{content}"
    );
    assert!(
        !content.contains("alef-readme") && !content.contains("alef-verify"),
        "unconfigured project precommit scaffold must leave Alef hooks opt-in:\n{content}"
    );
}

// --- Java checkstyle tests ---

#[test]
fn test_java_checkstyle_no_cosmetic_checks() {
    let mut config = test_config();
    config.languages = vec![Language::Java];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let checkstyle = files.iter().find(|f| f.path.ends_with("checkstyle.xml")).unwrap();
    // Should NOT have cosmetic whitespace checks (Spotless handles formatting)
    assert!(!checkstyle.content.contains("WhitespaceAfter"));
    assert!(!checkstyle.content.contains("WhitespaceAround"));
    assert!(!checkstyle.content.contains("GenericWhitespace"));
    assert!(!checkstyle.content.contains("EmptyBlock"));
    assert!(!checkstyle.content.contains("NeedBraces"));
    assert!(!checkstyle.content.contains("MagicNumber"));
    assert!(!checkstyle.content.contains("JavadocPackage"));
    // Should still have correctness checks
    assert!(checkstyle.content.contains("EqualsHashCode"));
    assert!(checkstyle.content.contains("UnusedImports"));
    assert!(checkstyle.content.contains("MethodLength"));
    assert!(checkstyle.content.contains("LineLength"));
    // LineLength max is 200 to accommodate the alef-emitted DefaultClient FFM
    // call shims (single-line chains of arena allocation + marshalling that
    // don't reflow cleanly within shorter limits).
    assert!(checkstyle.content.contains("\"200\""));
}

// --- Go golangci v2 format tests ---

#[test]
fn test_go_golangci_v2_format() {
    let mut config = test_config();
    config.languages = vec![Language::Go];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
    let files = language_files(&all_files);
    let golangci = files.iter().find(|f| f.path.ends_with(".golangci.yml")).unwrap();
    assert!(golangci.content.contains("version: \"2\""));
    assert!(golangci.content.contains("default: none"));
    assert!(golangci.content.contains("settings:"));
    // Should NOT use old v1 format
    assert!(!golangci.content.contains("linters-settings:"));
    // Should have detailed config
    assert!(golangci.content.contains("errcheck"));
    assert!(golangci.content.contains("govet"));
    assert!(golangci.content.contains("misspell"));
    assert!(golangci.content.contains("locale: US"));
    assert!(golangci.content.contains("exclusions:"));
}

#[test]
fn test_scaffold_csharp_csproj_at_package_root() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Csharp]).unwrap();
    let files = language_files(&all_files);
    // csproj at package root + .editorconfig + Directory.Build.props
    let csproj = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".csproj"))
        .expect("C# scaffold must produce a .csproj file");
    // Must be at packages/csharp/<Namespace>.csproj (package root), NOT inside the source subdir
    assert_eq!(
        csproj.path,
        PathBuf::from("packages/csharp/MyLib/MyLib.csproj"),
        "csproj must be in the namespace subdirectory so runtimes/** glob aligns with FFI staging"
    );
    assert!(
        csproj.content.contains("Microsoft.NET.Sdk"),
        "csproj must use Microsoft.NET.Sdk"
    );
    assert!(
        csproj.content.contains("net10.0"),
        "csproj must target net10.0 by default"
    );
    assert!(
        csproj.content.contains("<RootNamespace>MyLib</RootNamespace>"),
        "csproj must set RootNamespace to the PascalCase project name"
    );
    assert!(
        csproj.content.contains("<Nullable>enable</Nullable>"),
        "csproj must enable nullable reference types"
    );
    assert!(
        !csproj.generated_header,
        "csproj must be scaffold-once (generated_header = false)"
    );
}

#[test]
fn test_render_csharp_csproj_runtimes_glob_is_relative() {
    // Regression: the runtimes glob must NOT have a "../" prefix.
    // The csproj lives at packages/csharp/<Namespace>/<Namespace>.csproj, so
    // `runtimes/**` resolves to packages/csharp/<Namespace>/runtimes/ — the exact
    // directory where alef-publish stages the FFI shared libraries.
    let config = test_config();
    let content = render_csharp_csproj(&config, "1.2.3");
    assert!(
        content.contains(r#"Include="runtimes/**""#),
        "runtimes glob must be relative (no ../ prefix): {content}"
    );
    assert!(
        !content.contains(r#"Include="../runtimes"#),
        "runtimes glob must NOT have ../: {content}"
    );
    // The csproj lives at packages/csharp/<Namespace>/<Namespace>.csproj (3 levels deep),
    // so ../../../LICENSE correctly reaches the workspace root.
    assert!(
        content.contains(r#"Include="../../../LICENSE""#),
        "LICENSE path must be ../../../LICENSE to reach workspace root: {content}"
    );
    assert!(
        content.contains("<Version>1.2.3</Version>"),
        "version must be substituted: {content}"
    );
}

fn config_with_extra_deps() -> ResolvedCrateConfig {
    let mut config = test_config();
    config
        .extra_dependencies
        .insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies.insert(
        "tracing".to_string(),
        toml::Value::Table({
            let mut t = toml::map::Map::new();
            t.insert("version".to_string(), toml::Value::String("0.1".to_string()));
            t.insert(
                "features".to_string(),
                toml::Value::Array(vec![toml::Value::String("log".to_string())]),
            );
            t
        }),
    );
    config
}

#[test]
fn test_render_extra_deps_empty() {
    let config = test_config();
    assert_eq!(render_extra_deps(&config, Language::Python), "");
}

#[test]
fn test_render_extra_deps_string_version() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    assert!(rendered.contains("anyhow = \"1.0\""), "got: {rendered}");
}

#[test]
fn test_render_extra_deps_table_value() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    assert!(rendered.contains("tracing = "), "got: {rendered}");
    assert!(rendered.contains("\"log\""), "got: {rendered}");
}

#[test]
fn test_render_extra_deps_sorted() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    let anyhow_pos = rendered.find("anyhow").expect("anyhow missing");
    let tracing_pos = rendered.find("tracing").expect("tracing missing");
    assert!(anyhow_pos < tracing_pos, "deps should be sorted alphabetically");
}

#[test]
fn test_scaffold_python_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("tracing"),
        "content: {}",
        cargo_toml.content
    );
    // Extra deps should appear inside the [dependencies] section (which follows [features]).
    let deps_pos = cargo_toml.content.find("[dependencies]").unwrap();
    let anyhow_pos = cargo_toml.content.find("anyhow").unwrap();
    assert!(anyhow_pos > deps_pos, "anyhow should appear inside [dependencies]");
}

#[test]
fn test_scaffold_node_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_ruby_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_php_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_lib_name_no_path() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    // [lib] must NOT have a path pointing to a non-existent -elixir crate.
    // Cargo defaults to src/lib.rs, which is where the generated lib.rs lives.
    assert!(
        !cargo_toml.content.contains("-elixir/src/lib.rs"),
        "Elixir Cargo.toml [lib] must NOT point to a non-existent -elixir crate; content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("name = \"my_lib_nif\""),
        "Elixir Cargo.toml [lib] must set name to {{app_name}}_nif; content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_lib_path_for_external_output() {
    let config = test_config_from_toml(
        r#"
[crates.output]
elixir = "crates/my-lib-elixir/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo_toml
            .content
            .contains(r#"path = "../../../../crates/my-lib-elixir/src/lib.rs""#),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_elixirc_paths_normalizes_leading_slash() {
    let config = test_config_from_toml(
        r#"
[crates.output]
elixir = "/crates/my-lib-elixir/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs
            .content
            .contains(r#"elixirc_paths: ["lib", Path.expand("../../crates/my-lib-elixir/src", __DIR__)],"#),
        "content: {}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("../..//crates"),
        "content: {}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_mix_exs_files_list_omits_nonexistent_lib_and_checksum() {
    // Default config has no explicit elixir output and no trait bridges, so the
    // generated tree contains no `lib/` directory and no `checksum-*.exs` files.
    // Hex publish refuses to package a non-existent path, so the emitted
    // `files:` list must not advertise them.
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs
            .content
            .contains("files:\n        ~w(.formatter.exs mix.exs README* checksum-*.exs native/my_lib_nif/Cargo.toml native/my_lib_nif/Cargo.lock)"),
        "content: {}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_mix_exs_uses_configured_nif_targets() {
    let config = test_config_from_toml(
        r#"
[crates.elixir]
nif_targets = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs.content.contains("rustler_crates: [\n")
            && mix_exs.content.contains("my_lib_nif: [")
            && mix_exs.content.contains("\"aarch64-apple-darwin\",")
            && mix_exs.content.contains("\"x86_64-unknown-linux-gnu\""),
        "mix.exs must wire configured nif_targets into rustler_crates as a multi-line list; content:\n{}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_mix_exs_files_list_includes_external_source_dir() {
    // When the Elixir output lives outside packages/elixir/ (the NIF crate's
    // `[lib] path` points there), the NIF Rust `lib.rs` AND the generated `*.ex`
    // modules both live in that external dir — NOT in `native/<nif>/src`. The
    // `files:` list must therefore (a) NOT advertise the nonexistent
    // `native/<nif>/src` (else `mix hex.build` fails with "Missing files") and
    // (b) list the external source dir itself so the Rust NIF source ships in
    // the tarball for RustlerPrecompiled's source-compile fallback.
    let config = test_config_from_toml(
        r#"
[crates.output]
elixir = "crates/my-lib-elixir/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs.content.contains(
            "files:\n        ~w(.formatter.exs mix.exs README* checksum-*.exs native/my_lib_nif/Cargo.toml native/my_lib_nif/Cargo.lock ../../crates/my-lib-elixir/src)"
        ),
        "content: {}",
        mix_exs.content
    );
    // The nonexistent native src dir must NOT be listed (it would hard-fail mix hex.build).
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list the nonexistent native/<nif>/src dir; content: {}",
        mix_exs.content
    );
    // The Rust NIF source dir must ship (not just a *.ex glob), so the
    // source-compile fallback can build standalone from the tarball.
    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must ship the whole source dir, not just *.ex; content: {}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_language_level_extra_deps_override_crate_level() {
    let mut config = test_config();
    // Crate-level dep with version "1.0"
    config
        .extra_dependencies
        .insert("shared-dep".to_string(), toml::Value::String("1.0".to_string()));
    // Python-level override with a different version; inject via extra_deps_for_language
    // by inserting directly into a Python extra_dependencies map.
    let mut python_extra: std::collections::HashMap<String, toml::Value> = std::collections::HashMap::new();
    python_extra.insert("shared-dep".to_string(), toml::Value::String("2.0".to_string()));
    config.python = Some(PythonConfig {
        module_name: None,
        async_runtime: None,
        stubs: None,
        pip_name: None,
        features: None,
        serde_rename_all: None,
        capsule_types: std::collections::HashMap::new(),
        release_gil: false,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: python_extra,
        pip_dependencies: Vec::new(),
        scaffold_output: None,
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });
    let rendered = render_extra_deps(&config, Language::Python);
    // Python-level "2.0" should win over crate-level "1.0"
    assert!(rendered.contains("shared-dep = \"2.0\""), "got: {rendered}");
    assert!(
        !rendered.contains("1.0"),
        "crate-level version should be overridden, got: {rendered}"
    );
}

#[test]
fn test_scaffold_elixir_cargo_no_tokio_when_sync_only() {
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let api = test_api(); // all sync — no async functions or methods
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("tokio"),
        "sync-only API must not include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        !cargo_toml.content.contains("async-trait"),
        "sync-only API without trait bridges must not include async-trait; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_ruby_cargo_no_tokio_when_sync_only() {
    let mut config = test_config();
    config.languages = vec![Language::Ruby];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("tokio"),
        "sync-only Ruby API must not include tokio; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_java_checkstyle_suppressions_use_config_location() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let xml = files.iter().find(|f| f.path.ends_with("checkstyle.xml")).unwrap();
    assert!(
        xml.content.contains(r#"value="checkstyle-suppressions.xml""#),
        "checkstyle suppressions path must be relative to project basedir; content:\n{}",
        xml.content
    );
    let properties = files
        .iter()
        .find(|f| f.path.ends_with("checkstyle.properties"))
        .unwrap();
    assert!(
        properties.content.is_empty(),
        "checkstyle properties must be empty (0 bytes) so end-of-file-fixer leaves it untouched on every regen; a lone trailing newline gets stripped back to empty; content:\n{}",
        properties.content
    );
}

#[test]
fn test_scaffold_php_cs_fixer_handles_missing_tests_dir() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let fixer = files
        .iter()
        .find(|f| f.path.ends_with(".php-cs-fixer.dist.php"))
        .unwrap();
    assert!(
        fixer.content.contains("declare(strict_types=1);"),
        "php-cs-fixer config should be fixer-clean; content:\n{}",
        fixer.content
    );
    assert!(
        fixer.content.contains("is_dir(__DIR__ . '/tests')"),
        "php-cs-fixer config must not require a tests directory; content:\n{}",
        fixer.content
    );
    assert!(
        fixer.content.contains("setUnsupportedPhpVersionAllowed(true)"),
        "php-cs-fixer config must suppress unsupported-runtime advisory in config; content:\n{}",
        fixer.content
    );
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
    // pubspec.yaml + analysis_options.yaml + .gitignore + test + .editorconfig + README.md + example + CHANGELOG.md
    assert_eq!(files.len(), 8, "Expected 8 files for Dart scaffold");
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

    let test_file = &files[3];
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

    assert_eq!(files[4].path, PathBuf::from("packages/dart/.editorconfig"));
    assert!(files[4].content.contains("*.dart"));

    assert_eq!(files[5].path, PathBuf::from("packages/dart/README.md"));
    assert!(files[5].content.contains("dart pub get"));
    assert!(files[5].content.contains("flutter_rust_bridge_codegen generate"));

    assert_eq!(
        files[6].path,
        PathBuf::from("packages/dart/example/my_lib_example.dart")
    );
    assert!(files[6].content.contains("void main"));

    let changelog = &files[7];
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

#[test]
fn test_scaffold_elixir_cargo_tokio_when_async_function() {
    use crate::core::ir::{FunctionDef, TypeRef};
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let mut api = test_api();
    api.functions.push(FunctionDef {
        name: "do_work".to_string(),
        rust_path: "my_lib::do_work".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: true,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("tokio"),
        "async function API must include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("rt-multi-thread"),
        "tokio dep must include rt-multi-thread feature; content:\n{}",
        cargo_toml.content
    );
}

/// Trait bridge module names must use PascalCase for hyphenated crate names.
///
/// When the source crate name contains hyphens (e.g., `demo-markup`), the
/// Elixir trait bridge module name must be `DemoMarkupHtmlVisitorBridge`, not
/// `Demo_markupHtmlVisitorBridge` (which is what `capitalize_first` produces).
#[test]
fn test_scaffold_elixir_trait_bridge_module_name_is_pascal_case_for_hyphenated_crate() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "demo-markup".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(crate::core::config::ElixirConfig {
        app_name: Some("demo_markup".to_string()),
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
        nif_targets: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("html_visitor_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file.content.contains("defmodule DemoMarkupHtmlVisitorBridge do"),
        "trait bridge module name must be PascalCase for hyphenated crate names; got:\n{}",
        bridge_file.content
    );
    assert!(
        !bridge_file.content.contains("Demo_markup"),
        "trait bridge module name must not contain capitalize_first artifact 'Demo_markup'; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_trait_bridge_registers_genserver_pid_and_plugin_name() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "demo-markup".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(crate::core::config::ElixirConfig {
        app_name: Some("demo_markup".to_string()),
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
        nif_targets: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("demo_markup::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("ocr_backend_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file.content.contains("plugin_name = impl_module.name()")
            && bridge_file
                .content
                .contains("DemoMarkup.Native.register_ocr_backend(pid, plugin_name)"),
        "register/1 must require Plugin.name/0 and register the started GenServer pid; got:\n{}",
        bridge_file.content
    );
    assert!(
        !bridge_file
            .content
            .contains("register_ocr_backend(self(), Atom.to_string(impl_module))"),
        "register/1 must not register the caller pid or fallback module string name; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_trait_bridge_module_name_is_pascal_case_for_multi_word_crate() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "sample-language-pack".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(crate::core::config::ElixirConfig {
        app_name: Some("sample_language_pack".to_string()),
        features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
        nif_targets: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Parser".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("parser_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file
            .content
            .contains("defmodule SampleLanguagePackParserBridge do"),
        "trait bridge module name must be full PascalCase; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_tokio_when_async_method() {
    use crate::core::ir::{MethodDef, TypeDef, TypeRef};
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let mut api = test_api();
    api.types.push(TypeDef {
        name: "Worker".to_string(),
        rust_path: "my_lib::Worker".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "run".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("tokio"),
        "async method API must include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("rt-multi-thread"),
        "tokio dep must include rt-multi-thread feature; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_swift() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Swift]).unwrap();
    let files = language_files(&all_files);
    // Original 6 + root Package.swift + .editorconfig + .swiftformat + README.md + Examples/Demo/main.swift = 11
    assert_eq!(
        files.len(),
        11,
        "Expected 11 files for Swift scaffold (original 6 + root Package.swift + 4 extras)"
    );

    let package_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Package.swift"))
        .unwrap();
    // Module name derives to PascalCase of "my-lib" → "MyLib"
    assert!(
        package_swift.content.contains("name: \"MyLib\""),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains(".macOS(.v13)"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains(".iOS(.v16)"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("swift-tools-version: 6.0"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("Sources/MyLib"),
        "got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("Tests/MyLibTests"),
        "got: {}",
        package_swift.content
    );
    // Must declare RustBridge and RustBridgeC targets
    assert!(
        package_swift.content.contains("\"RustBridge\""),
        "Package.swift must declare RustBridge target; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("\"RustBridgeC\""),
        "Package.swift must declare RustBridgeC target; got: {}",
        package_swift.content
    );
    // RustBridge target must exist with unsafeFlags for in-tree development
    assert!(
        package_swift.content.contains("name: \"RustBridge\""),
        "Package.swift must declare RustBridge target; got: {}",
        package_swift.content
    );
    assert!(
        package_swift.content.contains("unsafeFlags"),
        "In-tree Package.swift must include unsafeFlags for local development; got: {}",
        package_swift.content
    );
    assert!(
        package_swift
            .content
            .contains("Run `cargo build -p my-lib-swift` and then rerun `alef generate`"),
        "Package.swift must document the Alef materialization step; got: {}",
        package_swift.content
    );

    let gitignore = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.gitignore"))
        .unwrap();
    assert_eq!(gitignore.path, PathBuf::from("packages/swift/.gitignore"));
    assert!(gitignore.content.contains(".build/"), "got: {}", gitignore.content);
    assert!(gitignore.content.contains(".swiftpm/"), "got: {}", gitignore.content);

    // RustBridgeC placeholder header (pure C target)
    let header = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridgeC/RustBridgeC.h"))
        .unwrap();
    assert!(
        header.content.contains("#ifndef RUST_BRIDGE_C_H"),
        "got: {}",
        header.content
    );

    // module.modulemap in RustBridge (kept as documentation comment)
    let modulemap = files.iter().find(|f| f.path.ends_with("module.modulemap")).unwrap();
    assert!(!modulemap.content.is_empty(), "module.modulemap must not be empty");

    // RustBridge placeholder Swift source
    let rust_bridge_swift = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Sources/RustBridge/RustBridge.swift"))
        .unwrap();
    assert!(
        !rust_bridge_swift.content.is_empty(),
        "RustBridge.swift must not be empty"
    );

    // Check for new production files
    let readme = files.iter().find(|f| f.path == Path::new("packages/swift/README.md"));
    assert!(readme.is_some(), "README.md should be generated");
    assert!(
        readme.unwrap().content.contains("swift build"),
        "README.md must document build process"
    );
    let readme_content = &readme.unwrap().content;
    assert!(
        readme_content.contains("alef generate --lang swift"),
        "README.md must tell users to rerun Alef instead of manually copying swift-bridge output: {readme_content}"
    );
    assert!(
        !readme_content.contains("cat \"$OUT/SwiftBridgeCore.h\""),
        "README.md must not imply manual copied bridge output is the generated-package contract: {readme_content}"
    );
    // .editorconfig and .swiftformat must both declare 2-space indent to match
    // `swift-format` defaults, so editors and the formatter stay in sync.
    let editorconfig = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.editorconfig"))
        .expect(".editorconfig should be generated");
    assert!(
        editorconfig.content.contains("indent_size = 2"),
        ".editorconfig must use 2-space indent; got: {}",
        editorconfig.content
    );
    let swiftformat = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/.swiftformat"))
        .expect(".swiftformat should be generated");
    assert!(
        swiftformat.content.contains("indent = 2"),
        ".swiftformat must use 2-space indent; got: {}",
        swiftformat.content
    );

    // Package.swift must use 2-space indentation — `swift-format` rewrites 4-space to 2.
    assert!(
        package_swift.content.contains("\n  name:"),
        "Package.swift must use 2-space indentation; got: {}",
        package_swift.content
    );
    // Single-element products array must not have a trailing comma (swift-format removes it).
    assert!(
        !package_swift
            .content
            .contains(".library(name: \"MyLib\", targets: [\"MyLib\"]),"),
        "Package.swift single-element products array must not have trailing comma; got: {}",
        package_swift.content
    );

    // Test stub must emit a blank line between import groups (swift-format requirement).
    let test_stub = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Tests") && f.path.extension().is_some_and(|e| e == "swift"))
        .expect("test stub .swift should be generated");
    assert!(
        test_stub.content.contains("import XCTest\n\n@testable"),
        "test stub must have blank line between import groups; got: {}",
        test_stub.content
    );

    // Demo must use 2-space indentation.
    let demo = files
        .iter()
        .find(|f| f.path == Path::new("packages/swift/Examples/Demo/main.swift"))
        .expect("Demo example should be generated");
    assert!(
        demo.content.contains("\n  static func main()"),
        "Demo must use 2-space indentation; got: {}",
        demo.content
    );

    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Swift scaffold must not emit GitHub workflows"
    );
}

#[test]
fn test_scaffold_kotlin() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    // build.gradle.kts, settings.gradle.kts, .gitignore, .editorconfig, gradle.properties, README.md, Sample.kt
    assert_eq!(files.len(), 7, "Expected 7 files for Kotlin scaffold");
    assert_eq!(files[0].path, PathBuf::from("packages/kotlin/build.gradle.kts"));
    assert!(files[0].content.contains("kotlin(\"jvm\")"));
    assert!(files[0].content.contains("org.jlleitschuh.gradle.ktlint"));
    // jspecify is required by the alef-emitted Java facade.
    assert!(
        files[0].content.contains("org.jspecify:jspecify:"),
        "build.gradle.kts must declare jspecify; got:\n{}",
        files[0].content
    );
    // ktlint must skip the Java facade and build/generated dirs.
    assert!(
        files[0].content.contains("filter {")
            && files[0].content.contains("/packages/java/")
            && files[0].content.contains("**/build/**")
            && files[0].content.contains("**/generated/**"),
        "ktlint filter block missing or incomplete; got:\n{}",
        files[0].content
    );
    // ktlint must skip the alef-emitted binding-class file (pascal-cased crate name).
    // The `my-lib` test crate becomes `MyLib.kt`.
    assert!(
        files[0].content.contains(r#"endsWith("/MyLib.kt")"#),
        "ktlint filter must exclude alef-emitted binding-class file; got:\n{}",
        files[0].content
    );
    // Maven artifactId override disambiguates Kotlin module from sibling Java module.
    assert!(
        files[0].content.contains("artifactId = \"my-lib-kotlin\""),
        "publication artifactId override missing; got:\n{}",
        files[0].content
    );
    // Kotlin/JVM targets JDK 21 (KOTLIN_JVM_TARGET); JDK 25 is reserved for
    // the Java/Panama backend via JAVA_JVM_TARGET.
    assert!(
        files[0].content.contains("JavaVersion.VERSION_21") && files[0].content.contains("JvmTarget.JVM_21"),
        "build.gradle.kts must target JDK 21; got:\n{}",
        files[0].content
    );
    assert_eq!(files[1].path, PathBuf::from("packages/kotlin/settings.gradle.kts"));
    assert_eq!(files[2].path, PathBuf::from("packages/kotlin/.gitignore"));
    assert_eq!(files[3].path, PathBuf::from("packages/kotlin/.editorconfig"));
    assert!(files[3].content.contains("*.kt"));
    assert_eq!(files[4].path, PathBuf::from("packages/kotlin/gradle.properties"));
    assert!(files[4].content.contains("org.gradle.parallel=true"));
    assert_eq!(files[5].path, PathBuf::from("packages/kotlin/README.md"));
    assert!(files[5].content.contains("my_lib"));
    assert!(files[5].content.contains(":my-lib-kotlin:0.1.0"));
    assert!(files[5].content.contains("gradle build"));
    assert_eq!(
        files[6].path,
        PathBuf::from("packages/kotlin/src/main/kotlin/com/github/test/sample/Sample.kt")
    );
    assert!(files[6].content.contains("object"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Kotlin scaffold must not emit GitHub workflows"
    );
    assert!(
        files[0].content.contains("native.lib.path") && !files[0].content.contains("kb.lib.path"),
        "Kotlin scaffold must use generic native.lib.path override; got:\n{}",
        files[0].content
    );
}

#[test]
fn test_scaffold_kotlin_scm_uses_configured_non_github_host() {
    let config = minimal_config_from_toml(
        r#"
[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://gitlab.example.com/acme/my-lib"
authors = ["Alice"]
keywords = ["test"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    let build_gradle = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin/build.gradle.kts"))
        .expect("build.gradle.kts must be emitted");

    assert!(
        build_gradle
            .content
            .contains("scm:git:git://gitlab.example.com/acme/my-lib.git")
    );
    assert!(
        build_gradle
            .content
            .contains("scm:git:ssh://git@gitlab.example.com/acme/my-lib.git")
    );
    assert!(!build_gradle.content.contains("github.com/acme/my-lib"));
}

#[test]
fn test_scaffold_kotlin_android_mode_returns_helpful_error() {
    // `mode = "android"` was removed in alef 0.16. Scaffolding must surface
    // a clear migration message rather than silently fall back.
    let config = test_config_from_toml(
        r#"
[crates.kotlin]
mode = "android"
"#,
    );
    let api = test_api();
    let err =
        scaffold(&api, &config, &[Language::Kotlin]).expect_err("scaffold must reject deprecated kotlin android mode");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("kotlin_android"),
        "error must point at the new Language::KotlinAndroid slug; got: {msg}"
    );
}

#[test]
fn test_scaffold_kotlin_native_target() {
    let config = test_config_from_toml(
        r#"
[crates.kotlin]
target = "native"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 5, "Expected 5 files for Kotlin Native scaffold");
    let build_gradle = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin-native/build.gradle.kts"))
        .unwrap();
    assert!(build_gradle.content.contains(r#"kotlin("multiplatform")"#));
    assert!(build_gradle.content.contains("linuxX64"));
    let def_file = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin-native/my-lib.def"))
        .unwrap();
    assert!(def_file.content.contains("headers = my_lib.h"));
    assert!(
        def_file
            .content
            .contains("linkerOpts = -L../../../target/release -lmy_lib")
    );
}

#[test]
fn test_scaffold_kotlin_multiplatform_mode() {
    let config = test_config_from_toml(
        r#"
[crates.kotlin]
mode = "kmp"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Kotlin]).unwrap();
    let files = language_files(&all_files);
    assert_eq!(files.len(), 5, "Expected 5 files for Kotlin Multiplatform scaffold");
    let build_gradle = files
        .iter()
        .find(|f| f.path == Path::new("packages/kotlin-mpp/build.gradle.kts"))
        .unwrap();
    assert!(build_gradle.content.contains(r#"kotlin("multiplatform")"#));
    assert!(build_gradle.content.contains("jvm()"));
    assert!(build_gradle.content.contains("linuxX64"));
    assert!(build_gradle.content.contains("macosArm64"));
    assert!(
        files
            .iter()
            .any(|f| f.path == Path::new("packages/kotlin-mpp/my-lib.def")),
        "KMP scaffold must include cinterop .def file"
    );
}

#[test]
fn test_scaffold_gleam() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Gleam]).unwrap();
    let files = language_files(&all_files);
    // gleam.toml + manifest.toml + .gitignore + test + .editorconfig + README.md + example
    assert_eq!(files.len(), 7, "Expected 7 files for Gleam scaffold");

    let gleam_toml = &files[0];
    assert_eq!(gleam_toml.path, PathBuf::from("packages/gleam/gleam.toml"));
    assert!(
        gleam_toml.content.contains("description"),
        "gleam.toml should include description"
    );
    assert!(
        gleam_toml.content.contains("licences = [\"MIT\"]"),
        "gleam.toml should include licences"
    );

    let manifest = &files[1];
    assert_eq!(manifest.path, PathBuf::from("packages/gleam/manifest.toml"));

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/gleam/.gitignore"));
    assert!(gitignore.content.contains("build/"));

    assert!(files[3].path.to_string_lossy().ends_with("_test.gleam"));

    let editorconfig = &files[4];
    assert_eq!(editorconfig.path, PathBuf::from("packages/gleam/.editorconfig"));
    assert!(editorconfig.content.contains("*.gleam"));

    let readme = &files[5];
    assert_eq!(readme.path, PathBuf::from("packages/gleam/README.md"));
    assert!(readme.content.contains("gleam build"));

    assert!(files[6].path.to_string_lossy().ends_with("_example.gleam"));
    assert!(files[6].content.contains("Nil"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Gleam scaffold must not emit GitHub workflows"
    );
}

#[test]
fn test_scaffold_gleam_uses_configured_license_and_no_fake_github_dependency() {
    let config = minimal_config_from_toml(
        r#"
[crates.scaffold]
description = "Test library"
license = "Apache-2.0"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Gleam]).unwrap();
    let files = language_files(&all_files);
    let gleam_toml = files
        .iter()
        .find(|f| f.path == Path::new("packages/gleam/gleam.toml"))
        .expect("gleam.toml must be emitted");
    let readme = files
        .iter()
        .find(|f| f.path == Path::new("packages/gleam/README.md"))
        .expect("README.md must be emitted");

    assert!(gleam_toml.content.contains("licences = [\"Apache-2.0\"]"));
    assert!(
        !readme.content.contains("github = \"example/"),
        "Gleam README must not invent GitHub dependency metadata:\n{}",
        readme.content
    );
    assert!(readme.content.contains("{path = \"../packages/gleam\"}"));
}

#[test]
fn test_scaffold_zig() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Zig]).unwrap();
    let files = language_files(&all_files);
    // build.zig + build.zig.zon + .gitignore + .editorconfig + README.md + example.zig + main.zig (re-export stub)
    assert_eq!(files.len(), 7, "Expected 7 files for Zig scaffold");

    let build_zig = &files[0];
    assert_eq!(build_zig.path, PathBuf::from("packages/zig/build.zig"));
    assert!(build_zig.content.contains("addModule"));

    let build_zig_zon = &files[1];
    assert_eq!(build_zig_zon.path, PathBuf::from("packages/zig/build.zig.zon"));
    assert!(build_zig_zon.content.contains(".fingerprint"));

    let gitignore = &files[2];
    assert_eq!(gitignore.path, PathBuf::from("packages/zig/.gitignore"));
    assert!(gitignore.content.contains("zig-cache/"));

    let editorconfig = &files[3];
    assert_eq!(editorconfig.path, PathBuf::from("packages/zig/.editorconfig"));
    assert!(editorconfig.content.contains("*.zig"));

    let readme = &files[4];
    assert_eq!(readme.path, PathBuf::from("packages/zig/README.md"));
    assert!(readme.content.contains("zig build"));

    let example = &files[5];
    assert_eq!(example.path, PathBuf::from("packages/zig/examples/example.zig"));
    assert!(example.content.contains("pub fn main"));

    let main = &files[6];
    assert_eq!(main.path, PathBuf::from("packages/zig/src/main.zig"));
    assert!(main.content.contains("pub const api"));
    assert!(main.content.contains(".zig"));
    assert!(
        files.iter().all(|f| !f.path.starts_with(".github/workflows")),
        "Zig scaffold must not emit GitHub workflows"
    );
}

// ---------------------------------------------------------------------------
// `[scaffold.cargo]` workspace `.cargo/config.toml` rendering tests.
// ---------------------------------------------------------------------------

fn cargo_only_config(cargo: ScaffoldCargo) -> ResolvedCrateConfig {
    let mut cfg = test_config();
    cfg.scaffold = Some(ScaffoldConfig {
        description: Some("Test library".to_string()),
        license: Some("MIT".to_string()),
        repository: Some("https://github.com/test/my-lib".to_string()),
        homepage: None,
        authors: vec!["Alice".to_string()],
        keywords: vec!["test".to_string()],
        generated_header: None,
        precommit: None,
        cargo: Some(cargo),
    });
    cfg
}

#[test]
fn cargo_config_default_renders_canonical_six_target_template() {
    let rendered = render_cargo_config(&ScaffoldCargo::default());

    // Header marker so finalize_hashes will stamp it.
    assert!(rendered.starts_with("# This file is auto-generated by alef. DO NOT EDIT.\n"));
    assert!(rendered.contains("# Re-generate with: alef scaffold\n"));

    // Canonical sections, in fixed order.
    assert!(rendered.contains("[build]\nincremental = true"));
    assert!(rendered.contains("[net]\ngit-fetch-with-cli = true"));
    assert!(rendered.contains("[registries.crates-io]\nprotocol = \"sparse\""));

    // All six target families present by default.
    assert!(rendered.contains("[target.'cfg(target_os = \"macos\")']"));
    assert!(rendered.contains("link-arg=-Wl,-undefined,dynamic_lookup"));
    assert!(rendered.contains("[target.x86_64-pc-windows-msvc]"));
    assert!(rendered.contains("[target.i686-pc-windows-msvc]"));
    assert!(rendered.contains("[target.aarch64-unknown-linux-gnu]"));
    assert!(rendered.contains("[target.x86_64-unknown-linux-musl]"));
    assert!(rendered.contains("[target.wasm32-unknown-unknown]"));
    assert!(rendered.contains("getrandom_backend=\\\"wasm_js\\\""));

    // No [env] block when none declared.
    assert!(!rendered.contains("\n[env]\n"));
}

#[test]
fn cargo_config_re_render_is_byte_identical() {
    let cargo = ScaffoldCargo::default();
    let first = render_cargo_config(&cargo);
    let second = render_cargo_config(&cargo);
    assert_eq!(first, second);
}

#[test]
fn cargo_config_disabling_individual_targets_omits_their_blocks() {
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets {
            i686_pc_windows_msvc: false,
            x86_64_unknown_linux_musl: false,
            ..ScaffoldCargoTargets::default()
        },
        build_jobs: 4,
        env: Default::default(),
    };
    let rendered = render_cargo_config(&cargo);
    assert!(!rendered.contains("[target.i686-pc-windows-msvc]"));
    assert!(!rendered.contains("[target.x86_64-unknown-linux-musl]"));
    // Other targets remain.
    assert!(rendered.contains("[target.x86_64-pc-windows-msvc]"));
    assert!(rendered.contains("[target.aarch64-unknown-linux-gnu]"));
    assert!(rendered.contains("[target.'cfg(target_os = \"macos\")']"));
}

#[test]
fn cargo_config_disabling_macos_omits_dynamic_lookup() {
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets {
            macos_dynamic_lookup: false,
            ..ScaffoldCargoTargets::default()
        },
        build_jobs: 4,
        env: Default::default(),
    };
    let rendered = render_cargo_config(&cargo);
    assert!(!rendered.contains("dynamic_lookup"));
    assert!(!rendered.contains("cfg(target_os = \"macos\")"));
}

#[test]
fn cargo_config_env_plain_string_renders_into_env_block() {
    let mut env = std::collections::HashMap::new();
    env.insert("MY_VAR".to_string(), ScaffoldCargoEnvValue::Plain("hello".to_string()));
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 4,
        env,
    };
    let rendered = render_cargo_config(&cargo);
    assert!(rendered.contains("\n[env]\n"));
    assert!(rendered.contains("MY_VAR = \"hello\"\n"));
}

#[test]
fn cargo_config_env_structured_value_renders_with_relative() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "RUBY".to_string(),
        ScaffoldCargoEnvValue::Structured {
            value: "scripts/preferred-ruby.sh".to_string(),
            relative: true,
        },
    );
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 4,
        env,
    };
    let rendered = render_cargo_config(&cargo);
    assert!(rendered.contains("[env]\n"));
    assert!(rendered.contains("RUBY = { value = \"scripts/preferred-ruby.sh\", relative = true }\n"));
}

#[test]
fn cargo_config_env_keys_are_sorted_for_determinism() {
    let mut env = std::collections::HashMap::new();
    env.insert("ZED".to_string(), ScaffoldCargoEnvValue::Plain("z".to_string()));
    env.insert("ALPHA".to_string(), ScaffoldCargoEnvValue::Plain("a".to_string()));
    env.insert("MID".to_string(), ScaffoldCargoEnvValue::Plain("m".to_string()));
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 4,
        env,
    };
    let rendered = render_cargo_config(&cargo);
    let env_section = rendered.split("[env]\n").nth(1).expect("env section present");
    let alpha_pos = env_section.find("ALPHA").expect("ALPHA present");
    let mid_pos = env_section.find("MID").expect("MID present");
    let zed_pos = env_section.find("ZED").expect("ZED present");
    assert!(alpha_pos < mid_pos);
    assert!(mid_pos < zed_pos);
}

#[test]
fn cargo_config_env_string_with_quotes_is_escaped() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "QUOTED".to_string(),
        ScaffoldCargoEnvValue::Plain(r#"a"b\c"#.to_string()),
    );
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 4,
        env,
    };
    let rendered = render_cargo_config(&cargo);
    // Backslashes doubled, quotes escaped.
    assert!(rendered.contains("QUOTED = \"a\\\"b\\\\c\"\n"));
}

#[test]
fn cargo_config_default_includes_build_jobs_limit() {
    let rendered = render_cargo_config(&ScaffoldCargo::default());
    // Default is 4 jobs to prevent OOM on 16 GB dev machines.
    assert!(
        rendered.contains("[build]\nincremental = true\njobs = 4\n"),
        "build_jobs default (4) must be in [build] section; got:\n{rendered}"
    );
}

#[test]
fn cargo_config_build_jobs_zero_disables_limit() {
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 0,
        env: Default::default(),
    };
    let rendered = render_cargo_config(&cargo);
    assert!(
        !rendered.contains("jobs = "),
        "build_jobs = 0 must not emit jobs limit; got:\n{rendered}"
    );
}

#[test]
fn cargo_config_build_jobs_custom_value_renders() {
    let cargo = ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 2,
        env: Default::default(),
    };
    let rendered = render_cargo_config(&cargo);
    assert!(
        rendered.contains("jobs = 2\n"),
        "custom build_jobs value must render; got:\n{rendered}"
    );
}

#[test]
fn scaffold_emits_cargo_config_when_scaffold_cargo_is_set() {
    let config = cargo_only_config(ScaffoldCargo::default());
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let cargo_file = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".cargo/config.toml"))
        .expect(".cargo/config.toml should be emitted when [scaffold.cargo] is set");
    assert!(
        cargo_file.generated_header,
        "generated_header must be true so verify walks it"
    );
    assert!(cargo_file.content.contains("auto-generated by alef"));
    assert!(cargo_file.content.contains("dynamic_lookup"));
    assert!(cargo_file.content.contains("[target.x86_64-pc-windows-msvc]"));
}

#[test]
fn scaffold_skips_cargo_config_in_legacy_mode_when_file_exists() {
    // No `[scaffold.cargo]` opt-in. Existing-file check is filesystem-bound, so
    // we only assert that scaffold() does not panic and produces no `.cargo/config.toml`
    // entry when the legacy create-if-missing branch detects the file already exists.
    // (The existing tests filter `.cargo/config.toml` out via `language_files()`,
    // implicitly relying on this branch never producing a hash-headered file.)
    let config = test_config(); // scaffold.cargo is None
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let cargo_files: Vec<_> = all_files
        .iter()
        .filter(|f| f.path == std::path::Path::new(".cargo/config.toml"))
        .collect();
    // Legacy branch is gated on Wasm + !exists. We're not requesting Wasm here,
    // so no .cargo/config.toml should appear regardless of filesystem state.
    assert!(
        cargo_files.is_empty(),
        "legacy branch should not emit .cargo/config.toml without Wasm",
    );
}

#[test]
fn wasm_package_name_strips_node_suffix_from_scoped_package() {
    // @scope/foo-node  →  @scope/foo-wasm  (not @scope/foo-node-wasm)
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "@scope/foo-node"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    assert!(
        pkg_json.content.contains("\"@scope/foo-wasm\""),
        "expected @scope/foo-wasm, got:\n{}",
        pkg_json.content
    );
    assert!(
        !pkg_json.content.contains("foo-node-wasm"),
        "must not emit foo-node-wasm, got:\n{}",
        pkg_json.content
    );
}

#[test]
fn wasm_package_name_strips_node_suffix_from_unscoped_package() {
    // foo-node  →  foo-wasm  (not foo-node-wasm)
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "foo-node"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    assert!(
        pkg_json.content.contains("\"foo-wasm\""),
        "expected foo-wasm, got:\n{}",
        pkg_json.content
    );
}

#[test]
fn wasm_package_name_fallback_when_no_node_suffix() {
    // foo  →  foo-wasm  (no -node suffix present, no stripping)
    let config = test_config();
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    // Default node_package_name for crate "my-lib" is "my-lib" (no -node suffix).
    // Stripping "-node" is a no-op → wasm name is "my-lib-wasm".
    assert!(
        pkg_json.content.contains("\"my-lib-wasm\""),
        "expected my-lib-wasm, got:\n{}",
        pkg_json.content
    );
}

#[test]
fn wasm_package_name_uses_explicit_wasm_config() {
    let config = test_config_from_toml(
        r#"
[crates.node]
package_name = "@scope/foo-node"

[crates.wasm]
package_name = "@scope/foo-web"
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Wasm]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path.ends_with("package.json"))
        .expect("wasm scaffold must emit package.json");
    let parsed: serde_json::Value = serde_json::from_str(&pkg_json.content).expect("valid wasm package.json");
    assert_eq!(parsed["name"], "@scope/foo-web");
    assert_eq!(parsed["publishConfig"]["access"], "public");
    assert_eq!(parsed["main"], "pkg/nodejs/my_lib_wasm.js");
    assert_eq!(parsed["types"], "pkg/nodejs/my_lib_wasm.d.ts");
}

#[test]
fn scaffold_emits_cargo_config_with_env_block_for_sample_markup_style_ruby_path() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "RUBY".to_string(),
        ScaffoldCargoEnvValue::Structured {
            value: "scripts/preferred-ruby.sh".to_string(),
            relative: true,
        },
    );
    let config = cargo_only_config(ScaffoldCargo {
        targets: ScaffoldCargoTargets::default(),
        build_jobs: 4,
        env,
    });
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let cargo_file = all_files
        .iter()
        .find(|f| f.path == std::path::Path::new(".cargo/config.toml"))
        .expect(".cargo/config.toml should be emitted");
    assert!(cargo_file.content.contains("[env]\n"));
    assert!(
        cargo_file
            .content
            .contains("RUBY = { value = \"scripts/preferred-ruby.sh\", relative = true }")
    );
}

/// Helper: extract the [dependencies] key order from a Cargo.toml string.
///
/// Returns the dependency keys in the order they appear, so tests can assert
/// that the emitted file is already cargo-sort canonical (alphabetical order).
fn dep_keys_in_order(cargo_toml: &str) -> Vec<&str> {
    let mut in_deps = false;
    let mut keys = Vec::new();
    for line in cargo_toml.lines() {
        if line.trim_start().starts_with('[') {
            in_deps = line.trim() == "[dependencies]";
            continue;
        }
        if in_deps {
            if let Some(key) = line.split('=').next() {
                let key = key.trim();
                if !key.is_empty() && !key.starts_with('#') {
                    keys.push(key);
                }
            }
        }
    }
    keys
}

#[test]
fn test_scaffold_elixir_cargo_deps_are_alphabetically_sorted() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    // With a trait bridge, async-trait and tokio must be present.
    assert!(
        keys.contains(&"async-trait"),
        "async-trait must appear when trait bridges are configured; keys: {keys:?}"
    );
    assert!(
        keys.contains(&"tokio"),
        "tokio must appear when trait bridges are configured; keys: {keys:?}"
    );
    // All keys must be in sorted order.
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "elixir Cargo.toml [dependencies] must be alphabetically sorted; got: {keys:?}"
    );
}

#[test]
fn test_scaffold_ruby_cargo_deps_are_alphabetically_sorted() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.languages = vec![Language::Ruby];
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    // With a trait bridge, async-trait and tokio must be present.
    assert!(
        keys.contains(&"async-trait"),
        "async-trait must appear when trait bridges are configured; keys: {keys:?}"
    );
    assert!(
        keys.contains(&"tokio"),
        "tokio must appear when trait bridges are configured; keys: {keys:?}"
    );
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "ruby Cargo.toml [dependencies] must be alphabetically sorted; got: {keys:?}"
    );
}

#[test]
fn test_scaffold_r_cargo_deps_are_alphabetically_sorted() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.languages = vec![Language::R];
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    // With a trait bridge, async-trait must be present.
    assert!(
        keys.contains(&"async-trait"),
        "async-trait must appear when trait bridges are configured; keys: {keys:?}"
    );
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "r Cargo.toml [dependencies] must be alphabetically sorted; got: {keys:?}"
    );
}

#[test]
fn test_scaffold_elixir_cargo_deps_sorted_no_trait_bridges() {
    // Even without trait bridges, the basic deps must be in sorted order.
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "elixir Cargo.toml [dependencies] must be alphabetically sorted (sync-only); got: {keys:?}"
    );
}

#[test]
fn test_scaffold_r_cargo_deps_sorted_no_trait_bridges() {
    // Without trait bridges, the basic R deps must still be in sorted order.
    let mut config = test_config();
    config.languages = vec![Language::R];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "r Cargo.toml [dependencies] must be alphabetically sorted (no trait bridges); got: {keys:?}"
    );
}

/// Helper: extract TOML section headers in the order they appear, skipping
/// inline sub-tables (lines that don't start with `[`).
fn section_headers_in_order(cargo_toml: &str) -> Vec<&str> {
    cargo_toml
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.starts_with('[') && !t.starts_with("[[") {
                Some(t)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn test_scaffold_elixir_cargo_section_order_is_cargo_sort_canonical() {
    // cargo-sort canonical order for a NIF crate: [package] → [workspace] → [lib] → [dependencies]
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let headers = section_headers_in_order(&cargo_toml.content);
    // [workspace] must appear before [lib], which must appear before [dependencies].
    let workspace_pos = headers.iter().position(|h| *h == "[workspace]");
    let lib_pos = headers.iter().position(|h| *h == "[lib]");
    let deps_pos = headers.iter().position(|h| *h == "[dependencies]");

    assert!(
        workspace_pos.is_some(),
        "Elixir NIF Cargo.toml must contain a [workspace] section; headers: {headers:?}"
    );
    assert!(
        lib_pos.is_some(),
        "Elixir NIF Cargo.toml must contain a [lib] section; headers: {headers:?}"
    );
    assert!(
        deps_pos.is_some(),
        "Elixir NIF Cargo.toml must contain a [dependencies] section; headers: {headers:?}"
    );

    assert!(
        workspace_pos < lib_pos,
        "[workspace] must come before [lib] (cargo-sort canonical); headers: {headers:?}"
    );
    assert!(
        lib_pos < deps_pos,
        "[lib] must come before [dependencies] (cargo-sort canonical); headers: {headers:?}"
    );
}

// ---- LICENSE sync tests -----------------------------------------------

/// When a LICENSE file exists at the workspace root, alef must copy it into
/// every per-language package directory so ecosystems like pub.dev that require
/// a LICENSE can publish successfully.
#[test]
fn test_scaffold_license_files_emitted_when_license_exists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = dir.path().to_path_buf();
    std::fs::write(workspace_root.join("LICENSE"), "MIT License\n").expect("write LICENSE");

    let mut config = test_config();
    config.workspace_root = Some(workspace_root);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Python, Language::Dart]).unwrap();
    let license_files: Vec<_> = all_files.iter().filter(|f| f.path.ends_with("LICENSE")).collect();

    // One LICENSE per unique package dir (packages/python and packages/dart)
    assert_eq!(license_files.len(), 2, "should emit one LICENSE per unique package dir");

    let paths: Vec<_> = license_files.iter().map(|f| f.path.as_path()).collect();
    assert!(
        paths.iter().any(|p| *p == Path::new("packages/python/LICENSE")),
        "should emit packages/python/LICENSE; got: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| *p == Path::new("packages/dart/LICENSE")),
        "should emit packages/dart/LICENSE; got: {paths:?}"
    );

    // Content must match the workspace-root LICENSE verbatim.
    for f in &license_files {
        assert_eq!(
            f.content, "MIT License\n",
            "LICENSE content must match workspace-root file; got: {:?}",
            f.content
        );
    }
}

/// When no LICENSE file exists at the workspace root, scaffold must succeed
/// without error — just skip the LICENSE sync.
#[test]
fn test_scaffold_license_files_skips_gracefully_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = dir.path().to_path_buf();
    // Intentionally do NOT write a LICENSE file.

    let mut config = test_config();
    config.workspace_root = Some(workspace_root);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let license_files: Vec<_> = all_files.iter().filter(|f| f.path.ends_with("LICENSE")).collect();

    assert!(
        license_files.is_empty(),
        "no LICENSE file must be emitted when workspace root has no LICENSE; got: {:?}",
        license_files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

/// FFI, JNI, Rust, and C languages must not get a LICENSE copy — they do not
/// produce a standalone publishable package directory.
#[test]
fn test_scaffold_license_files_skips_internal_languages() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = dir.path().to_path_buf();
    std::fs::write(workspace_root.join("LICENSE"), "Apache-2.0\n").expect("write LICENSE");

    let mut config = test_config();
    config.workspace_root = Some(workspace_root);
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
    let license_files: Vec<_> = all_files.iter().filter(|f| f.path.ends_with("LICENSE")).collect();

    assert!(
        license_files.is_empty(),
        "FFI language must not produce a LICENSE copy; got: {license_files:?}"
    );
}

/// When multiple languages share the same package directory, only one LICENSE
/// must be emitted (no duplicates).
#[test]
fn test_scaffold_license_files_deduplicates_same_package_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = dir.path().to_path_buf();
    std::fs::write(workspace_root.join("LICENSE"), "MIT\n").expect("write LICENSE");

    let mut config = test_config();
    config.workspace_root = Some(workspace_root);
    let api = test_api();

    // Dart uses packages/dart — single language, single dir.
    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let license_files: Vec<_> = all_files.iter().filter(|f| f.path.ends_with("LICENSE")).collect();

    assert_eq!(license_files.len(), 1, "one language → one LICENSE, no duplicates");
    assert_eq!(
        license_files[0].path,
        PathBuf::from("packages/dart/LICENSE"),
        "Dart LICENSE must live in packages/dart/"
    );
}

/// When explicit_output.elixir points at an external source directory (where the
/// NIF crate's `[lib] path` resolves), the generated mix.exs `files:` list must
/// list that directory as a self-contained dir entry — shipping the Rust NIF
/// `lib.rs` and any `*.rs`/`*.ex` together — instead of a bare `/*.ex` glob that
/// leaves the Rust source out of the tarball.
#[test]
fn test_scaffold_elixir_mix_exs_external_dir_is_listed_as_whole_dir() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let rs_dir = tmp.path();

    // Create a Rust NIF directory with only .rs and .toml files (no .ex files).
    std::fs::write(rs_dir.join("lib.rs"), "// Rust NIF source\n").expect("write lib.rs");
    std::fs::write(rs_dir.join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

    // Build config pointing explicit_output.elixir at the .rs-only directory.
    // Use a TOML literal string (single quotes) so Windows backslash paths like
    // `C:\Users\RUNNER~1\…` aren't interpreted as `\U` unicode escapes by the
    // TOML basic-string parser.
    let explicit_path = rs_dir.to_string_lossy().to_string();
    let config = test_config_from_toml(&format!(
        r#"
[crates.output]
elixir = '{explicit_path}'
"#
    ));
    let api = test_api();

    // Scaffold Elixir and extract the generated mix.exs.
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files
        .iter()
        .find(|f| f.path.ends_with("mix.exs"))
        .expect("mix.exs must be generated");

    // No bare `/*.ex` glob — the whole dir is shipped so the Rust source travels too.
    // (Note: checksum-*.exs contains the substring *.ex, so check the path-glob form.)
    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must list the whole source dir, not a /*.ex glob; content:\n{}",
        mix_exs.content
    );
    // The nonexistent native src dir must not be advertised (else mix hex.build fails).
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list native/<nif>/src; content:\n{}",
        mix_exs.content
    );
    // Verify that standard entries are still present.
    assert!(
        mix_exs.content.contains(".formatter.exs"),
        "mix.exs should contain .formatter.exs"
    );
    assert!(
        mix_exs.content.contains("native/my_lib_nif/Cargo.toml"),
        "mix.exs should still ship the NIF Cargo.toml"
    );
}

/// Even when the external Elixir output directory contains `.ex`/`.exs` modules,
/// it is still listed as a single self-contained directory entry (covering both
/// the Elixir modules and the co-located Rust NIF source), not a `/*.ex` glob.
#[test]
fn test_scaffold_elixir_mix_exs_external_dir_with_ex_sources_listed_as_dir() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let ex_dir = tmp.path();

    // Create a directory with both Elixir source files and the Rust NIF source.
    std::fs::write(ex_dir.join("module.ex"), "defmodule Test do\nend\n").expect("write module.ex");
    std::fs::write(ex_dir.join("helper.exs"), "# helper\n").expect("write helper.exs");
    std::fs::write(ex_dir.join("lib.rs"), "// Rust NIF source\n").expect("write lib.rs");

    // Build config pointing explicit_output.elixir at the .ex-containing directory.
    // Single-quoted TOML literal string keeps Windows `\U`/`\R`/`\T` path segments
    // intact (basic strings would parse them as unicode escapes and panic).
    let explicit_path = ex_dir.to_string_lossy().to_string();
    let config = test_config_from_toml(&format!(
        r#"
[crates.output]
elixir = '{explicit_path}'
"#
    ));
    let api = test_api();

    // Scaffold Elixir and extract the generated mix.exs.
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files
        .iter()
        .find(|f| f.path.ends_with("mix.exs"))
        .expect("mix.exs must be generated");

    // Listed as a whole directory, not a `/*.ex` glob (which would drop lib.rs).
    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must list the whole source dir, not a /*.ex glob; content:\n{}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list native/<nif>/src; content:\n{}",
        mix_exs.content
    );
}

// ---------------------------------------------------------------------------
// Dual-form core-facade dependency (`{ version = "X.Y.Z", path = "..." }`).
//
// The scaffolded binding-crate Cargo.toml must emit its workspace-member
// core-facade dependency in dual form so in-repo dev path builds keep working
// AND cargo-package flows (maturin sdist, `cargo package`) can strip the path
// to a registry version-dependency. The version equals the workspace version
// (here `api.version` == "0.1.0"), the path is preserved unchanged, features
// are preserved, and external (non-member) deps are emitted untouched.
// ---------------------------------------------------------------------------

/// Locate the binding-crate `Cargo.toml` generated for `lang` and return its
/// content. Filters out the Ruby `[lib]` Cargo (which lives under `native/`)
/// by matching the dependency-bearing manifest containing `[dependencies]`.
fn core_cargo_toml_for(lang: Language) -> String {
    let mut config = test_config();
    config.features = vec!["full".to_string(), "ocr".to_string()];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[lang]).unwrap();
    let files = language_files(&all_files);
    files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml") && f.content.contains("my-lib = {"))
        .map(|f| f.content.clone())
        .unwrap_or_else(|| panic!("no core Cargo.toml with `my-lib` dep emitted for {lang:?}"))
}

#[test]
fn render_core_dep_emits_dual_form_with_version_first() {
    let line = render_core_dep("my-lib", "../my-lib", "", "1.2.3");
    assert_eq!(line, r#"my-lib = { version = "1.2.3", path = "../my-lib" }"#);
}

#[test]
fn render_core_dep_preserves_features_suffix() {
    let line = render_core_dep("my-lib", "../my-lib", ", features = [\"full\", \"ocr\"]", "1.2.3");
    assert_eq!(
        line,
        r#"my-lib = { version = "1.2.3", path = "../my-lib", features = ["full", "ocr"] }"#
    );
}

#[test]
fn render_core_dep_falls_back_to_path_only_when_version_empty() {
    // Some unit fixtures (e.g. JNI with `ApiSurface::default()`) have no
    // resolvable workspace version; emit path-only rather than `version = ""`.
    let line = render_core_dep("my-lib", "../my-lib", "", "");
    assert_eq!(line, r#"my-lib = { path = "../my-lib" }"#);
}

#[test]
fn test_scaffold_python_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Python);
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../my-lib", features = ["full", "ocr"] }"#),
        "python core dep must be dual form with version + path + features; content:\n{content}"
    );
    // External deps unchanged.
    assert!(
        content.contains(r#"serde_json = "1""#),
        "external serde_json unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_node_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Node);
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../my-lib", features = ["full", "ocr"] }"#),
        "node core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains(r#"serde = { version = "1", features = ["derive"] }"#),
        "external serde unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_ruby_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Ruby);
    assert!(
        content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../../crates/my-lib", features = ["full", "ocr"] }"#
        ),
        "ruby core dep must be dual form with the deep crates path preserved; content:\n{content}"
    );
    assert!(
        content.contains("magnus = "),
        "external magnus unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_php_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Php);
    assert!(
        content.contains(r#"my-lib = { version = "0.1.0", path = "../my-lib", features = ["full", "ocr"] }"#),
        "php core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains("ext-php-rs = "),
        "external ext-php-rs unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_elixir_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::Elixir);
    assert!(
        content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../crates/my-lib", features = ["full", "ocr"] }"#
        ),
        "elixir core dep must be dual form with the deep crates path preserved; content:\n{content}"
    );
    assert!(
        content.contains("rustler = "),
        "external rustler unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_r_core_dep_is_dual_form() {
    let content = core_cargo_toml_for(Language::R);
    assert!(
        content.contains(
            r#"my-lib = { version = "0.1.0", path = "../../../../crates/my-lib", features = ["full", "ocr"] }"#
        ),
        "r core dep must be dual form; content:\n{content}"
    );
    assert!(
        content.contains("extendr-api = "),
        "external extendr-api unchanged; content:\n{content}"
    );
}

#[test]
fn test_scaffold_r_authors_r_parses_name_and_email() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let description = files.iter().find(|f| f.path.ends_with("DESCRIPTION")).unwrap();

    assert!(
        description
            .content
            .contains(r#"Authors@R: person("Ada", "Lovelace", email = "ada@example.com", role = c("aut", "cre"))"#),
        "DESCRIPTION must split Authors@R name/email; content:\n{}",
        description.content
    );
}

#[test]
fn test_scaffold_swift_core_dep_is_dual_form() {
    // Swift's binding-crate Cargo.toml is emitted by the swift backend's
    // `gen_rust_crate::emit` (not the generic scaffold step), so assert the
    // dual form there. The path must be preserved for dev builds and the
    // workspace version injected for cargo-package flows.
    let config = test_config();
    let api = test_api(); // version "0.1.0", crate "my-lib"
    let files = crate::backends::swift::gen_rust_crate::emit(&api, &config).unwrap();
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("swift Cargo.toml must be emitted");
    // core_dep_key is the Rust-ident form (`my_lib`); since it differs from the
    // cargo package name (`my-lib`) a `package = "..."` rename is appended after
    // the version + path. `core_path` is `../../..` for the same-as-workspace case.
    assert!(
        cargo
            .content
            .contains(r#"my_lib = { version = "0.1.0", path = "../../..", package = "my-lib" }"#),
        "swift core dep must be dual form (version + path) with package rename; content:\n{}",
        cargo.content
    );
    // External deps unchanged.
    assert!(
        cargo.content.contains(r#"serde_json = "1""#),
        "external serde_json unchanged; content:\n{}",
        cargo.content
    );
}

#[test]
fn test_scaffold_dev_path_build_form_preserved() {
    // The whole point of dual form: the `path` is still present (so in-repo dev
    // builds resolve the local member crate) AND a `version` is added (so
    // cargo-package can strip the path to a registry dep).
    for lang in [
        Language::Python,
        Language::Node,
        Language::Ruby,
        Language::Php,
        Language::Elixir,
        Language::R,
    ] {
        let content = core_cargo_toml_for(lang);
        let dep_line = content
            .lines()
            .find(|l| l.trim_start().starts_with("my-lib = {"))
            .unwrap_or_else(|| panic!("no my-lib dep line for {lang:?}"));
        assert!(
            dep_line.contains("path = "),
            "{lang:?}: dev-path-build path must be preserved: {dep_line}"
        );
        assert!(
            dep_line.contains(r#"version = "0.1.0""#),
            "{lang:?}: version must be injected: {dep_line}"
        );
    }
}

#[test]
fn test_render_extra_deps_injects_version_for_workspace_member() {
    use std::fs;
    use tempfile::TempDir;

    // Build a real temp workspace whose root Cargo.toml declares a member
    // `my-lib-http` at version 2.5.0 (via workspace inheritance).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib-http"]

[workspace.package]
version = "2.5.0"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/my-lib-http/src")).unwrap();
    fs::write(root.join("crates/my-lib-http/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/my-lib-http/Cargo.toml"),
        "[package]\nname = \"my-lib-http\"\nversion.workspace = true\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // A path-only workspace-member dep + a non-member external dep.
    config.extra_dependencies.insert(
        "my-lib-http".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-http".to_string()),
        )])),
    );
    config
        .extra_dependencies
        .insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));

    let rendered = render_extra_deps(&config, Language::Python);
    // Member: version injected (toml inline tables sort keys: path before version).
    assert!(
        rendered.contains(r#"my-lib-http = { path = "../my-lib-http", version = "2.5.0" }"#),
        "workspace member should get the resolved workspace version injected; got:\n{rendered}"
    );
    // Non-member external dep unchanged.
    assert!(
        rendered.contains(r#"anyhow = "1.0""#),
        "non-member external dep must be emitted unchanged; got:\n{rendered}"
    );
}

#[test]
fn test_render_extra_deps_leaves_non_member_path_dep_unchanged() {
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = []\n[workspace.package]\nversion = \"2.5.0\"\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // `vendored-thing` is NOT a workspace member, so its path-only table must
    // stay path-only (no version injection).
    config.extra_dependencies.insert(
        "vendored-thing".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_string(),
            toml::Value::String("../../vendor/thing".to_string()),
        )])),
    );

    let rendered = render_extra_deps(&config, Language::Python);
    assert!(
        rendered.contains(r#"vendored-thing = { path = "../../vendor/thing" }"#),
        "non-member path dep must remain path-only; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("version ="),
        "no version should be injected for a non-member; got:\n{rendered}"
    );
}

#[test]
fn test_render_extra_deps_does_not_double_inject_version() {
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/my-lib-http\"]\n[workspace.package]\nversion = \"2.5.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/my-lib-http/src")).unwrap();
    fs::write(root.join("crates/my-lib-http/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/my-lib-http/Cargo.toml"),
        "[package]\nname = \"my-lib-http\"\nversion = \"9.9.9\"\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // Already carries an explicit version — must be left untouched.
    config.extra_dependencies.insert(
        "my-lib-http".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([
            ("path".to_string(), toml::Value::String("../my-lib-http".to_string())),
            ("version".to_string(), toml::Value::String("1.0".to_string())),
        ])),
    );

    let rendered = render_extra_deps(&config, Language::Python);
    assert!(
        rendered.contains(r#"version = "1.0""#),
        "pre-existing version must be preserved; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("9.9.9") && !rendered.contains("2.5.0"),
        "must not overwrite or append a second version; got:\n{rendered}"
    );
}

#[test]
fn test_render_extra_deps_swift_injects_version_for_workspace_member() {
    use std::fs;
    use tempfile::TempDir;

    // Build a real temp workspace whose root Cargo.toml declares a member
    // `my-lib-http` at version 3.1.0 (via workspace inheritance).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib-http"]

[workspace.package]
version = "3.1.0"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/my-lib-http/src")).unwrap();
    fs::write(root.join("crates/my-lib-http/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/my-lib-http/Cargo.toml"),
        "[package]\nname = \"my-lib-http\"\nversion.workspace = true\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // Path-only workspace-member dep configured under swift extra_dependencies.
    config.extra_dependencies.insert(
        "my-lib-http".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-http".to_string()),
        )])),
    );

    // Calling with Language::Swift exercises the same code path as the swift
    // gen_rust_crate backend, which now delegates to this shared function.
    let rendered = render_extra_deps(&config, Language::Swift);
    assert!(
        rendered.contains(r#"version = "3.1.0""#),
        "swift backend: workspace member must get version injected; got:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"path = "../my-lib-http""#),
        "swift backend: path must be preserved alongside injected version; got:\n{rendered}"
    );
}

#[test]
fn test_ruby_cargo_machete_rb_sys_only() {
    // Regression test: v0.22.25 fixed the mingw sysroot bug via a cargo-dep pin on rb-sys.
    // The NIF code now directly uses `tokio` and `async-trait` (not just transitively through
    // the core crate), so they must NOT be in the cargo-machete ignored list. Only `rb-sys`
    // should be ignored (it's pinned but used transitively through Magnus).
    use crate::core::ir::*;

    let config = test_config_from_toml(
        r#"
[crates.ruby]
gem_name = "test_lib"
"#,
    );

    // Minimal ApiSurface with no async/trait bridges to verify the baseline cargo-machete section
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let result = super::languages::scaffold_ruby_cargo(&api, &config);
    assert!(result.is_ok(), "scaffold_ruby_cargo should succeed");

    let files = result.unwrap();
    let cargo_toml_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("Should generate Cargo.toml");

    let content = &cargo_toml_file.content;

    // Verify the cargo-machete section exists and contains only rb-sys
    assert!(
        content.contains("[package.metadata.cargo-machete]"),
        "Should contain [package.metadata.cargo-machete] section; got:\n{}",
        content
    );

    assert!(
        content.contains("ignored = [\"rb-sys\"]"),
        "Should ignore only rb-sys (pinned for mingw sysroot bug but used transitively through Magnus); got:\n{}",
        content
    );

    // Verify that the ignored list does NOT contain the conditional deps
    // (tokio, async-trait, futures, ahash are now directly used by NIF code and should not be ignored)
    let ignored_section = content
        .split("[package.metadata.cargo-machete]")
        .nth(1)
        .and_then(|s| s.split("[lib]").next())
        .unwrap_or("");

    assert!(
        !ignored_section.contains("\"tokio\""),
        "tokio should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
    assert!(
        !ignored_section.contains("\"async-trait\""),
        "async-trait should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
    assert!(
        !ignored_section.contains("\"futures\""),
        "futures should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
    assert!(
        !ignored_section.contains("\"ahash\""),
        "ahash should not be in ignored list (now directly used by NIF code); got:\n{}",
        ignored_section
    );
}
