use super::*;

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
    assert_eq!(parsed["engines"]["node"], ">= 22");
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
fn test_scaffold_node_index_js_re_exports_service_api() {
    // When services are defined, `index.js` must re-export the service wrapper
    // from `service.cjs` so that `require("<pkg>")` returns both the raw native
    // binding and the wrapper class (with method shortcuts like registerRoute).
    // Without the re-export, consumers calling wrapper methods hit `TypeError`.
    let config = test_config();

    let mut api = test_api();
    // Add a service definition so api.services is non-empty.
    // The exact structure doesn't matter for the scaffold test — we just need
    // api.services to be non-empty, which triggers has_service_api = true.
    api.services = vec![crate::core::ir::ServiceDef {
        name: "App".to_string(),
        rust_path: "my_lib::App".to_string(),
        constructor: crate::core::ir::MethodDef {
            name: "new".to_string(),
            params: vec![],
            return_type: crate::core::ir::TypeRef::Named("App".to_string()),
            is_async: false,
            is_static: true,
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
        },
        configurators: vec![],
        registrations: vec![],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    }];

    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let index_js = files
        .iter()
        .find(|f| f.path.ends_with("index.js"))
        .expect("crate index.js must be emitted");

    // Verify the index.js contains the re-export pattern
    assert!(
        index_js
            .content
            .contains(r#"const _service = require("./service.cjs");"#),
        "index.js must require service.cjs when services are defined, got:\n{}",
        index_js.content
    );
    assert!(
        index_js
            .content
            .contains("module.exports = { ...nativeBinding, ..._service };"),
        "index.js must spread both nativeBinding and _service so wrapper methods override native, got:\n{}",
        index_js.content
    );

    // Ensure the old single export line is not present
    assert!(
        !index_js
            .content
            .lines()
            .any(|line| line.trim() == "module.exports = nativeBinding;"),
        "index.js must not have bare nativeBinding export when services are defined, got:\n{}",
        index_js.content
    );
}

#[test]
fn test_scaffold_node_index_js_single_export_without_services() {
    // When no services are defined, `index.js` should export only nativeBinding.
    let config = test_config();
    let api = test_api(); // api.services is empty

    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let index_js = files
        .iter()
        .find(|f| f.path.ends_with("index.js"))
        .expect("crate index.js must be emitted");

    // Verify the index.js does NOT contain the re-export pattern
    assert!(
        !index_js.content.contains("_service"),
        "index.js must not reference service.cjs when no services are defined, got:\n{}",
        index_js.content
    );

    // Verify the simple export is present
    assert!(
        index_js
            .content
            .lines()
            .any(|line| line.trim() == "module.exports = nativeBinding;"),
        "index.js must have bare nativeBinding export when no services are defined, got:\n{}",
        index_js.content
    );
}
