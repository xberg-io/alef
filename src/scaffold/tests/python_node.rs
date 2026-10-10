// Test module: debug output to stderr is expected here. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr)]

use super::*;

#[test]
fn test_scaffold_python() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
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
fn python_package_metadata_splits_author_names_and_emails_and_keeps_maintainers() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>", "Grace Hopper <grace@example.com>"]
"#,
    );
    let all_files = scaffold(&test_api(), &config, &[Language::Python]).unwrap();
    let pyproject = language_files(&all_files)
        .iter()
        .find(|file| file.path == Path::new("packages/python/pyproject.toml"))
        .expect("Python pyproject must be emitted")
        .content
        .clone();
    let manifest: toml::Value = toml::from_str(&pyproject).expect("generated pyproject must be valid TOML");
    let project = &manifest["project"];
    assert_eq!(project["authors"][0]["name"].as_str(), Some("Ada Lovelace"));
    assert_eq!(project["authors"][0]["email"].as_str(), Some("ada@example.com"));
    assert_eq!(project["maintainers"], project["authors"]);
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
    assert_eq!(files.len(), 12);
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-node/package.json"));
    assert!(files[0].content.contains("napi"));
    assert_eq!(files[1].path, PathBuf::from("crates/my-lib-node/index.js"));
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

/// `napi build --dts index.native.d.ts` writes napi-rs's auto-derived declarations beside
/// alef's own `index.d.ts` on every build. That redirect target is never published (it is
/// absent from `files`), nothing imports it, and no `types`/`exports` entry names it -- so
/// leaving it untracked means each build dirties the consumer's tree with a file no one
/// consumes. Asserted by path rather than index so adding a scaffold file cannot silently
/// move what this checks. ~keep
#[test]
fn node_scaffold_gitignores_the_napi_generated_declarations() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);

    let gitignore = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/.gitignore"))
        .expect("node scaffold must emit a .gitignore");
    assert!(
        gitignore
            .content
            .lines()
            .any(|line| line.trim() == crate::core::template_versions::npm::NAPI_AUTO_DTS_FILENAME),
        "the napi --dts redirect target must be ignored, got:\n{}",
        gitignore.content
    );
    assert!(
        !gitignore.content.lines().any(|line| line.trim() == "index.d.ts"),
        "alef's own index.d.ts is committed output and must NOT be ignored, got:\n{}",
        gitignore.content
    );
    assert!(
        !gitignore.generated_header,
        ".gitignore is scaffolded once and user-owned after; it must not carry a DO NOT EDIT header"
    );
}

#[test]
fn test_scaffold_node_napi_package_name_matches_scoped_package() {
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
    assert!(content.contains("[tool.pyrefly]"));
    assert!(!content.contains("[tool.ruff]"), "ruff config moved to poly.toml");
}

#[test]
fn test_scaffold_python_pyproject_canonical_format() {
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

    assert!(
        content.contains("requires = [ \"maturin"),
        "single-element requires array should stay inline with inner spaces. got:\n{content}",
    );

    assert!(
        content.contains("keywords = [ \"apple\", \"banana\", \"zebra\" ]"),
        "short multi-item keywords array should stay inline, alphabetised. got:\n{content}",
    );

    assert!(
        !content.contains("[project.urls]"),
        "should use dot-syntax urls.repository, not [project.urls] section"
    );
    assert!(
        content.contains("urls.repository = "),
        "should have urls.repository in dot-syntax"
    );

    assert!(
        !content.contains("[tool.ruff]"),
        "ruff config must not be emitted into pyproject.toml — it lives in poly.toml. got:\n{content}"
    );

    assert!(
        !content.contains("[tool.mypy]"),
        "mypy config must not be emitted — pyrefly replaces it. got:\n{content}"
    );
    assert!(
        content.contains("[tool.pyrefly]") && content.contains("preset = \"strict\""),
        "should emit [tool.pyrefly] with strict preset. got:\n{content}"
    );
    assert!(
        content.contains("[[tool.pyrefly.sub-config]]") && content.contains("matches = \"**/api.py\""),
        "should emit a pyrefly sub-config matching the api.py wrapper. got:\n{content}"
    );
    assert!(
        content.contains("[tool.pyrefly.sub-config.errors]") && content.contains("bad-argument-count = false"),
        "should suppress the api.py wrapper's still-unaddressed errors via sub-config.errors. got:\n{content}"
    );
    // `bad-return` and `bad-argument-type` were the public-dataclass-vs-native-pyclass boundary
    // mismatch alef-310 fixed in codegen; they must stay off this list, or a codegen regression
    // on that boundary would go undetected by pyrefly again. ~keep
    assert!(
        !content.contains("bad-return = false") && !content.contains("bad-argument-type = false"),
        "the fixed boundary-mismatch codes must not be suppressed again. got:\n{content}"
    );
}

#[test]
fn test_scaffold_python_emits_configured_pyrefly_sub_configs() {
    let config = test_config_from_toml(
        r#"
[workspace.poly.pyrefly-sub-configs]
"**/app.py" = ["bad-argument-type", "implicit-any-empty-container"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
    let content = &files[0].content;

    assert!(
        content.contains("matches = \"**/api.py\""),
        "built-in api.py sub-config must remain. got:\n{content}"
    );
    assert!(
        content.contains("matches = \"**/app.py\"") && content.contains("implicit-any-empty-container = false"),
        "configured pyrefly sub-config must be emitted. got:\n{content}"
    );
}

/// The generated `pyproject.toml` must already be in `pyproject-fmt` canonical form so the
/// `pyproject-fmt` pre-commit hook is a no-op on every regen. Running `pyproject-fmt` on our
/// output must produce zero changes — otherwise the hook rewrites the alef-hash-tracked file
/// and breaks `alef verify`. Skips when the `pyproject-fmt` binary is unavailable.
#[test]
fn test_scaffold_python_pyproject_is_pyproject_fmt_clean() {
    if crate::test_support::spawn_from_stable_dir("pyproject-fmt")
        .arg("--version")
        .output()
        .is_err()
    {
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

    let spawn = std::process::Command::new("pyproject-fmt")
        .arg(&path)
        .current_dir(dir.path())
        .output();
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
    let content = &files[0].content;
    assert!(content.contains("\"scripts\""));
    assert!(content.contains("\"build\""));
    assert!(content.contains("\"files\""));
    assert!(content.contains("\"devDependencies\""));
    assert!(content.contains("@napi-rs/cli"));
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
fn test_scaffold_node_package_json_includes_publish_metadata() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
homepage = "https://example.com/my-lib"
issues = "https://example.com/my-lib/issues"
authors = ["Alice", "Bob"]
keywords = ["llm", "bindings"]
"#,
    );
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/package.json"))
        .expect("crate package.json must be emitted");
    let parsed: serde_json::Value =
        serde_json::from_str(&pkg_json.content).expect("emitted package.json must be valid JSON");

    assert_eq!(parsed["homepage"], "https://example.com/my-lib");
    assert_eq!(parsed["bugs"]["url"], "https://example.com/my-lib/issues");
    assert_eq!(parsed["author"], "Alice");
    assert_eq!(parsed["contributors"][0], "Bob");
    let keywords = parsed["keywords"].as_array().expect("keywords must be an array");
    assert!(keywords.contains(&serde_json::Value::String("llm".to_string())));
    assert!(keywords.contains(&serde_json::Value::String("bindings".to_string())));

    let platform = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/npm/linux-x64-musl/package.json"))
        .expect("musl platform package manifest must be emitted");
    let platform_json: serde_json::Value =
        serde_json::from_str(&platform.content).expect("valid platform package.json");
    assert_eq!(
        platform_json["homepage"], "https://example.com/my-lib",
        "platform manifests must also carry publish metadata, got: {platform_json}"
    );
    assert_eq!(platform_json["author"], "Alice");
}

#[test]
fn test_scaffold_node_package_json_omits_publish_metadata_when_unconfigured() {
    let config = minimal_config_from_toml("");
    let api = test_api();
    let files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let pkg_json = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/package.json"))
        .expect("crate package.json must be emitted");
    let parsed: serde_json::Value =
        serde_json::from_str(&pkg_json.content).expect("emitted package.json must be valid JSON");

    assert!(parsed.get("homepage").is_none(), "got: {parsed}");
    assert!(parsed.get("bugs").is_none(), "got: {parsed}");
    assert!(parsed.get("author").is_none(), "got: {parsed}");
    assert!(parsed.get("contributors").is_none(), "got: {parsed}");
    assert!(parsed.get("keywords").is_none(), "got: {parsed}");
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
fn test_scaffold_node_targets_toggle_drops_mac_intel() {
    // The systematic `[targets]` opt-out drops a target across the whole binding, ~keep
    // just like the node-specific `exclude_platforms`. ~keep
    let config = test_config_from_toml(
        r#"
[crates.targets]
mac_intel = false
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
        !optional_deps.contains_key("my-lib-darwin-x64"),
        "darwin-x64 must be excluded via mac_intel = false"
    );
    assert!(
        optional_deps.contains_key("my-lib-darwin-arm64"),
        "darwin-arm64 must still be present"
    );

    let targets = parsed["napi"]["targets"]
        .as_array()
        .expect("napi.targets must be an array");
    assert!(
        !targets.iter().any(|t| t == "x86_64-apple-darwin"),
        "x86_64-apple-darwin must be excluded from napi.targets"
    );
    assert!(
        targets.iter().any(|t| t == "aarch64-apple-darwin"),
        "aarch64-apple-darwin must still be present"
    );

    assert!(
        !files
            .iter()
            .any(|f| f.path == Path::new("crates/my-lib-node/npm/darwin-x64/package.json")),
        "darwin-x64 per-platform stub must not be emitted"
    );

    let index_js = files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/index.js"))
        .expect("index.js must be emitted");
    assert!(
        !index_js.content.contains("darwin-x64"),
        "index.js dispatch table must not reference darwin-x64"
    );
}

#[test]
fn test_scaffold_targets_toggle_rejects_unknown_key() {
    // An unknown `[targets]` key must fail resolution, not silently no-op. ~keep
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.targets]
mac_intle = false
"#,
    )
    .expect("valid toml");
    let err = cfg.resolve().expect_err("unknown target key must error");
    let msg = err.to_string();
    assert!(msg.contains("mac_intle"), "error must name the bad key: {msg}");
    assert!(msg.contains("valid keys"), "error must list valid keys: {msg}");
}

#[test]
fn test_scaffold_node_index_js_re_exports_service_api() {
    let config = test_config();

    let mut api = test_api();
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
            cfg: None,
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
    let config = test_config();
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let index_js = files
        .iter()
        .find(|f| f.path.ends_with("index.js"))
        .expect("crate index.js must be emitted");

    assert!(
        !index_js.content.contains("_service"),
        "index.js must not reference service.cjs when no services are defined, got:\n{}",
        index_js.content
    );

    assert!(
        index_js
            .content
            .lines()
            .any(|line| line.trim() == "module.exports = nativeBinding;"),
        "index.js must have bare nativeBinding export when no services are defined, got:\n{}",
        index_js.content
    );
}

/// Regression for alef-task #320: `scaffold_node_cargo` unconditionally forwarded every
/// `collect_cfg_features` name into the wrapper's own `default = [...]` array and every
/// `[crates.node].features` name into the core dependency's own explicit `features = [...]`
/// line, re-enabling a feature a `target_dep_overrides` entry excluded for one cfg target -- the
/// same defect `RubyConfig::excluded_default_features` fixed for the Magnus crate, generalized
/// here. Asserts both directions on both surfaces: the excluded name is never defaulted or
/// forwarded, and a name nobody excluded still is.
#[test]
fn test_scaffold_node_excludes_named_feature_from_default_but_keeps_others() {
    let config = minimal_config_from_toml(
        r#"
[crates.node]
features = ["native-http", "wasm-http"]
excluded_default_features = ["native-http"]
[[crates.node.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["wasm-http"]
default_features = false
"#,
    );
    let api = ApiSurface {
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
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let cargo = &files
        .iter()
        .find(|f| f.path == Path::new("crates/my-lib-node/Cargo.toml"))
        .expect("node Cargo.toml must be emitted")
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
