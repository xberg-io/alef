use super::*;

/// End-to-end: `sync_versions` must rewrite both `package.json` (root) and
/// every `crates/*-node/package.json` file alongside the existing manifests.
/// Regression test for the sample_core publish.yaml dry-run failure where the
/// root manifest stayed at 4.9.5 while Cargo.toml jumped to 5.0.0-rc.1.
#[test]
fn sync_versions_writes_root_and_node_crate_package_json() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    // Minimal workspace: Cargo.toml at canonical "1.0.0", root package.json
    // and crates/mylib-node/package.json both stale at "0.9.0".
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(
        root.join("package.json"),
        "{\n  \"name\": \"mylib-root\",\n  \"version\": \"0.9.0\",\n  \"private\": true\n}\n",
    )
    .expect("write root package.json");
    std::fs::create_dir_all(root.join("crates/mylib-node")).expect("mkdir crates/mylib-node");
    std::fs::write(
        root.join("crates/mylib-node/package.json"),
        "{\n  \"name\": \"mylib\",\n  \"version\": \"0.9.0\"\n}\n",
    )
    .expect("write crates/mylib-node/package.json");

    // Drop a minimal alef.toml so we can resolve a config.
    // Normalize backslashes to / so the path is a valid TOML basic string on Windows.
    let alef_toml = format!(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    // Switch into the tempdir for the duration of the call — sync_versions
    // resolves relative paths against CWD.
    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    // Always restore the CWD before unwrapping, so a panic doesn't leave
    // the test runner in a broken directory.
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let root_pkg = std::fs::read_to_string(root.join("package.json")).expect("read root package.json");
    assert!(
        root_pkg.contains(r#""version": "1.0.0""#),
        "root package.json must be bumped to canonical version, got:\n{root_pkg}"
    );
    assert!(
        !root_pkg.contains("0.9.0"),
        "old version must be gone from root package.json, got:\n{root_pkg}"
    );

    let node_pkg = std::fs::read_to_string(root.join("crates/mylib-node/package.json"))
        .expect("read crates/mylib-node/package.json");
    assert!(
        node_pkg.contains(r#""version": "1.0.0""#),
        "crates/*-node/package.json must be bumped to canonical version, got:\n{node_pkg}"
    );
}

/// `sync_versions` must rewrite `optionalDependencies` pins to sibling NAPI
/// platform packages and the pre-staged platform manifests under
/// `crates/*-node/npm/<platform>/package.json`. Leaving these stale makes
/// `pnpm install --frozen-lockfile` fail with `ERR_PNPM_OUTDATED_LOCKFILE`
/// because the lockfile and manifest disagree on the platform-package
/// version. Regression test for the sample_crawler rc.34 Build Node bindings
/// failure where the top-level version was rc.34 but the `optionalDependencies`
/// and platform manifests stayed at rc.33.
#[test]
fn sync_versions_bumps_napi_platform_pins_and_manifests() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    // crate-level manifest with optionalDependencies at the OLD version
    std::fs::create_dir_all(root.join("crates/mylib-node")).expect("mkdir crates/mylib-node");
    std::fs::write(
            root.join("crates/mylib-node/package.json"),
            "{\n  \"name\": \"@scope/mylib\",\n  \"version\": \"0.9.0\",\n  \"optionalDependencies\": {\n    \"@scope/mylib-linux-x64-gnu\": \"0.9.0\",\n    \"@scope/mylib-darwin-arm64\": \"0.9.0\",\n    \"@scope/mylib-win32-x64-msvc\": \"0.9.0\"\n  }\n}\n",
        )
        .expect("write crates/mylib-node/package.json");

    // Pre-staged platform manifests at the OLD version
    for platform in &["linux-x64-gnu", "darwin-arm64", "win32-x64-msvc"] {
        let dir = root.join(format!("crates/mylib-node/npm/{platform}"));
        std::fs::create_dir_all(&dir).expect("mkdir platform dir");
        std::fs::write(
            dir.join("package.json"),
            format!("{{\n  \"name\": \"@scope/mylib-{platform}\",\n  \"version\": \"0.9.0\"\n}}\n"),
        )
        .expect("write platform package.json");
    }

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let crate_pkg = std::fs::read_to_string(root.join("crates/mylib-node/package.json"))
        .expect("read crates/mylib-node/package.json");
    assert!(
        !crate_pkg.contains("0.9.0"),
        "old version must be gone from crates/mylib-node/package.json (including optionalDependencies), got:\n{crate_pkg}"
    );
    assert!(
        crate_pkg.contains(r#""@scope/mylib-linux-x64-gnu": "1.0.0""#),
        "optionalDependencies pin to linux-x64-gnu must be bumped, got:\n{crate_pkg}"
    );
    assert!(
        crate_pkg.contains(r#""@scope/mylib-darwin-arm64": "1.0.0""#),
        "optionalDependencies pin to darwin-arm64 must be bumped, got:\n{crate_pkg}"
    );
    assert!(
        crate_pkg.contains(r#""@scope/mylib-win32-x64-msvc": "1.0.0""#),
        "optionalDependencies pin to win32-x64-msvc must be bumped, got:\n{crate_pkg}"
    );

    for platform in &["linux-x64-gnu", "darwin-arm64", "win32-x64-msvc"] {
        let manifest = std::fs::read_to_string(root.join(format!("crates/mylib-node/npm/{platform}/package.json")))
            .expect("read platform package.json");
        assert!(
            manifest.contains(r#""version": "1.0.0""#),
            "platform manifest {platform} must be bumped, got:\n{manifest}"
        );
        assert!(
            !manifest.contains("0.9.0"),
            "old version must be gone from platform manifest {platform}, got:\n{manifest}"
        );
    }
}

/// `sync_versions` must bump BOTH the consumer pyproject
/// (`packages/python/pyproject.toml`) and the source-template pyproject that
/// lives alongside the PyO3 crate (`crates/{lib}-py/src/pyproject.toml`,
/// selected via `[crates.output] python`) to the PEP 440 normalised
/// prerelease form. Regression test for the source-template version drift
/// that made `alef validate versions` fail on a tagged prerelease.
#[test]
fn sync_versions_bumps_both_python_pyprojects_to_pep440_prerelease() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.15.6-rc.2\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    // Consumer publish manifest, stale.
    std::fs::create_dir_all(root.join("packages/python")).expect("mkdir packages/python");
    std::fs::write(
        root.join("packages/python/pyproject.toml"),
        "[project]\nname = \"mylib\"\nversion = \"0.15.5\"\n",
    )
    .expect("write packages/python/pyproject.toml");

    // Source-template manifest alongside the PyO3 crate, stale.
    std::fs::create_dir_all(root.join("crates/mylib-py/src")).expect("mkdir crates/mylib-py/src");
    std::fs::write(
        root.join("crates/mylib-py/src/pyproject.toml"),
        "[project]\nname = \"mylib\"\nversion = \"0.15.5\"\n",
    )
    .expect("write crates/mylib-py/src/pyproject.toml");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"python\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n[crates.output]\npython = \"crates/mylib-py/src/\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let consumer =
        std::fs::read_to_string(root.join("packages/python/pyproject.toml")).expect("read consumer pyproject");
    assert!(
        consumer.contains(r#"version = "0.15.6rc2""#),
        "consumer pyproject must be PEP 440 normalised, got:\n{consumer}"
    );

    let source = std::fs::read_to_string(root.join("crates/mylib-py/src/pyproject.toml"))
        .expect("read source-template pyproject");
    assert!(
        source.contains(r#"version = "0.15.6rc2""#),
        "source-template pyproject must be PEP 440 normalised, got:\n{source}"
    );
    assert!(
        !source.contains("0.15.5") && !source.contains("0.15.6-rc.2"),
        "source-template must hold only the normalised version, got:\n{source}"
    );
}

// -----------------------------------------------------------------------
// patch_workspace_dep_versions unit tests
// -----------------------------------------------------------------------

/// patch_workspace_dep_versions updates [dependencies], [dev-dependencies],
/// [build-dependencies], [target.*.dependencies], and [workspace.dependencies]
/// but leaves external crate pins intact.
#[test]
fn patch_workspace_dep_versions_all_dep_table_shapes() {
    use std::collections::HashSet;

    let dir = tempfile::tempdir().expect("tempdir");

    let cargo_toml = r#"[package]
name = "crate-a"
version = "5.0.0-rc.1"

[dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1", optional = true }
serde = "1.0"

[dev-dependencies]
crate-c = { path = "../crate-c", version = "5.0.0-rc.1" }
tempfile = "3"

[build-dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1" }

[target.'cfg(unix)'.dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1", optional = true }
libc = "0.2"

[workspace.dependencies]
crate-c = { path = "../crate-c", version = "5.0.0-rc.1", default-features = false }
tokio = { version = "1.0", features = ["full"] }
"#;

    let path = dir.path().join("Cargo.toml");
    std::fs::write(&path, cargo_toml).expect("write");

    let members: HashSet<String> = ["crate-b", "crate-c"].iter().map(|s| s.to_string()).collect();

    let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "5.0.0-rc.2", &members).expect("patch ok");

    assert!(changed, "at least one version pin must have been updated");

    let result = std::fs::read_to_string(&path).expect("read");

    // [package] version is NOT touched by patch_workspace_dep_versions — only dep tables.
    // All workspace member dep-table pins must be bumped to rc.2.
    // crate-b appears in [dependencies], [build-dependencies], and [target.*.dependencies].
    let crate_b_lines: Vec<&str> = result
        .lines()
        .filter(|l| l.contains("crate-b") && l.contains("version"))
        .collect();
    assert!(
        !crate_b_lines.is_empty(),
        "expected crate-b dep lines with version=:\n{result}"
    );
    for line in &crate_b_lines {
        assert!(
            line.contains("5.0.0-rc.2"),
            "crate-b pin not bumped:\n  {line}\nfull:\n{result}"
        );
    }
    // crate-c appears in [dev-dependencies] and [workspace.dependencies].
    let crate_c_lines: Vec<&str> = result
        .lines()
        .filter(|l| l.contains("crate-c") && l.contains("version"))
        .collect();
    assert!(
        !crate_c_lines.is_empty(),
        "expected crate-c dep lines with version=:\n{result}"
    );
    for line in &crate_c_lines {
        assert!(
            line.contains("5.0.0-rc.2"),
            "crate-c pin not bumped:\n  {line}\nfull:\n{result}"
        );
    }

    // External crates must be untouched.
    assert!(
        result.contains(r#"serde = "1.0""#),
        "serde must not be touched:\n{result}"
    );
    assert!(
        result.contains(r#"tempfile = "3""#),
        "tempfile must not be touched:\n{result}"
    );
    assert!(
        result.contains(r#"libc = "0.2""#),
        "libc must not be touched:\n{result}"
    );
    assert!(
        result.contains(r#"tokio = { version = "1.0", features = ["full"] }"#),
        "tokio must not be touched:\n{result}"
    );
}

/// patch_workspace_dep_versions is idempotent: calling it twice with the
/// same target version returns false and does not rewrite the file.
#[test]
fn patch_workspace_dep_versions_is_idempotent() {
    use std::collections::HashSet;

    let dir = tempfile::tempdir().expect("tempdir");

    let cargo_toml = "[package]\nname = \"crate-a\"\nversion = \"5.0.0-rc.2\"\n\n[dependencies]\ncrate-b = { path = \"../crate-b\", version = \"5.0.0-rc.2\" }\n";

    let path = dir.path().join("Cargo.toml");
    std::fs::write(&path, cargo_toml).expect("write");

    let members: HashSet<String> = std::iter::once("crate-b".to_string()).collect();

    let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "5.0.0-rc.2", &members).expect("patch ok");
    assert!(!changed, "no change expected when already at target version");
}

/// patch_workspace_dep_versions does not touch path-only deps (no version= key).
#[test]
fn patch_workspace_dep_versions_skips_path_only_deps() {
    use std::collections::HashSet;

    let dir = tempfile::tempdir().expect("tempdir");

    let cargo_toml =
        "[package]\nname = \"crate-a\"\nversion = \"1.0.0\"\n\n[dependencies]\ncrate-b = { path = \"../crate-b\" }\n";

    let path = dir.path().join("Cargo.toml");
    std::fs::write(&path, cargo_toml).expect("write");

    let members: HashSet<String> = std::iter::once("crate-b".to_string()).collect();

    let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "2.0.0", &members).expect("patch ok");
    assert!(!changed, "path-only deps without version= must not be touched");
}

// -----------------------------------------------------------------------
// sync_versions dep-table end-to-end test
// -----------------------------------------------------------------------

/// Full workspace e2e: after sync_versions the version bump propagates from
/// [workspace.package] into [workspace.dependencies] and all dep-table shapes
/// in member crates. External pins must be untouched.
#[test]
fn sync_versions_patches_dep_tables_on_version_change() {
    use crate::core::config::NewAlefConfig;

    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    fn write_file(dir: &std::path::Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }

    // Root Cargo.toml: canonical version already at rc.2 (simulates task version:set).
    write_file(
        root,
        "Cargo.toml",
        "[workspace.package]\nversion = \"5.0.0-rc.2\"\n\n[workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n\n[workspace.dependencies]\nalpha = { path = \"crates/alpha\", version = \"5.0.0-rc.1\", default-features = false }\nserde = \"1.0\"\n",
    );

    // crates/alpha: upstream crate, no intra-workspace deps.
    write_file(
        root,
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\nserde = \"1.0\"\n",
    );

    // crates/beta: all four dep-table shapes referencing alpha.
    write_file(
        root,
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\", optional = true }\nserde = \"1.0\"\n\n[dev-dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\" }\ntempfile = \"3\"\n\n[build-dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\" }\n\n[target.'cfg(unix)'.dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\", features = [\"unix\"] }\nlibc = \"0.2\"\n",
    );

    // Normalize backslashes to / so the path is a valid TOML basic string on Windows.
    let alef_toml_content = format!(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"alpha\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    write_file(root, "alef.toml", &alef_toml_content);
    let alef_toml_path = root.join("alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml_content).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    // Root [workspace.dependencies] alpha pin must be bumped to rc.2.
    let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).expect("read root");
    assert!(
        root_cargo.contains(r#"alpha = { path = "crates/alpha", version = "5.0.0-rc.2""#),
        "root [workspace.dependencies] alpha must be bumped to rc.2:\n{root_cargo}"
    );
    assert!(
        root_cargo.contains(r#"serde = "1.0""#),
        "root serde must be untouched:\n{root_cargo}"
    );

    // crates/alpha [package] version must be rc.2.
    let alpha_cargo = std::fs::read_to_string(root.join("crates/alpha/Cargo.toml")).expect("read alpha");
    assert!(
        alpha_cargo.contains("version = \"5.0.0-rc.2\""),
        "alpha [package] must be bumped:\n{alpha_cargo}"
    );

    // crates/beta: all four dep-table shapes must reference rc.2.
    let beta_cargo = std::fs::read_to_string(root.join("crates/beta/Cargo.toml")).expect("read beta");
    let alpha_version_lines: Vec<&str> = beta_cargo
        .lines()
        .filter(|l| l.contains("alpha") && l.contains("version"))
        .collect();
    assert!(
        !alpha_version_lines.is_empty(),
        "expected alpha dep lines with version= in beta:\n{beta_cargo}"
    );
    for line in &alpha_version_lines {
        assert!(
            line.contains("5.0.0-rc.2"),
            "alpha pin not bumped to rc.2 in beta:\n  {line}\nfull:\n{beta_cargo}"
        );
    }
    assert!(
        !beta_cargo.contains("5.0.0-rc.1"),
        "old rc.1 must be gone from beta:\n{beta_cargo}"
    );

    // External deps in beta must be untouched.
    assert!(
        beta_cargo.contains(r#"serde = "1.0""#),
        "serde must not be touched:\n{beta_cargo}"
    );
    assert!(
        beta_cargo.contains(r#"tempfile = "3""#),
        "tempfile must not be touched:\n{beta_cargo}"
    );
    assert!(
        beta_cargo.contains(r#"libc = "0.2""#),
        "libc must not be touched:\n{beta_cargo}"
    );
}

#[test]
fn run_optional_logs_but_does_not_fail_on_missing_binary() {
    // Verify that run_optional gracefully handles a binary that doesn't exist.
    // This test just invokes the function and verifies it doesn't panic.
    // The actual command execution would fail, but run_optional logs and returns.
    crate::cli::pipeline::helpers::run_optional("nonexistent_binary_12345", &["arg1", "arg2"]);
    // If we reach here without panicking, the test passes.
}

#[test]
fn run_optional_succeeds_for_simple_command() {
    // Verify that run_optional can run a simple builtin command (echo) successfully.
    crate::cli::pipeline::helpers::run_optional("echo", &["test"]);
    // If we reach here without panicking, the test passes.
}

// --- Kotlin Gradle project version ------------------------------------

/// `sync_versions` must stamp the `--release-date` override into the
/// `date-released:` line of CITATION.cff verbatim, taking precedence over
/// any `[workspace.citation].date-released` configured in alef.toml.
///
/// Regression test for bug #327: release engineers had to hand-edit
/// alef.toml before every release. With the override, CI can pass
/// `--release-date $(date +%F)` and the stamped date wins.
#[test]
fn sync_versions_release_date_override_wins_over_configured_date() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"2.0.0\"\nlicense = \"MIT\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    // alef.toml with a [workspace.citation] block AND an explicit
    // date-released. Without the override we'd expect "2020-01-01" stamped.
    let alef_toml = format!(
        "[workspace]\nlanguages = [\"node\"]\n\n[workspace.citation]\ntitle = \"Tiny\"\nabstract = \"x.\"\nrepository-code = \"https://example.com/tiny\"\ndate-released = \"2020-01-01\"\n[[workspace.citation.authors]]\nname = \"Acme, Inc.\"\n\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, Some("2099-12-31"));
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let citation = std::fs::read_to_string(root.join("CITATION.cff")).expect("CITATION.cff written");
    assert!(
        citation.contains("date-released: 2099-12-31\n"),
        "override date must win over configured date, got:\n{citation}"
    );
    assert!(
        !citation.contains("date-released: 2020-01-01"),
        "configured date must be suppressed by override, got:\n{citation}"
    );
}

/// When no `--release-date` flag is supplied, `sync_versions` must preserve
/// the pre-flag behaviour: configured `[workspace.citation].date-released`
/// wins, falling back to the system date when unset. This guards against
/// accidental regression of the override plumbing.
#[test]
fn sync_versions_without_release_date_override_preserves_configured_date() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"2.0.0\"\nlicense = \"MIT\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write Cargo.toml");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"node\"]\n\n[workspace.citation]\ntitle = \"Tiny\"\nabstract = \"x.\"\nrepository-code = \"https://example.com/tiny\"\ndate-released = \"2020-01-01\"\n[[workspace.citation.authors]]\nname = \"Acme, Inc.\"\n\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let citation = std::fs::read_to_string(root.join("CITATION.cff")).expect("CITATION.cff written");
    assert!(
        citation.contains("date-released: 2020-01-01\n"),
        "configured date must be preserved when no override is supplied, got:\n{citation}"
    );
}

// -----------------------------------------------------------------------
// [patch.crates-io] version sync tests
// -----------------------------------------------------------------------

/// `sync_versions` must update the `version =` pin inside a `[patch.crates-io]`
/// entry when the entry's key matches the configured crate name.
///
/// Regression test for a release where the workspace version moved forward but
/// the patch block stayed on an older prerelease, breaking binding CI.
#[test]
fn sync_versions_patches_crates_io_patch_block_version() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    // Workspace Cargo.toml at the new version with a stale [patch.crates-io] pin.
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"3.6.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n\n[patch.crates-io]\nmy-lib-rs = { path = \"crates/my-lib\", version = \"3.6.0-rc.14\" }\n",
    )
    .expect("write Cargo.toml");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"my-lib-rs\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(
        cargo_toml.contains(r#"my-lib-rs = { path = "crates/my-lib", version = "3.6.3" }"#),
        "[patch.crates-io] version must be bumped to workspace version, got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("3.6.0-rc.14"),
        "stale patch version must be gone, got:\n{cargo_toml}"
    );
}

/// `sync_versions` must leave path-only `[patch.crates-io]` entries untouched —
/// entries without a `version =` key have no pin to drift.
#[test]
fn sync_versions_skips_path_only_crates_io_patch_entries() {
    use crate::core::config::NewAlefConfig;
    let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().expect("cwd");

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    let cargo_toml_content = "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n\n[patch.crates-io]\npath-only-lib = { path = \"crates/path-only\" }\n";
    std::fs::write(root.join("Cargo.toml"), cargo_toml_content).expect("write Cargo.toml");

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"path-only-lib\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = root.join("alef.toml");
    std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let resolved_cfg = resolved.remove(0);

    std::env::set_current_dir(root).expect("set_current_dir");
    let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true, None);
    let _ = std::env::set_current_dir(&original_cwd);
    sync_result.expect("sync_versions ok");

    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    // Entry must survive intact — no version key inserted.
    assert!(
        cargo_toml.contains(r#"path-only-lib = { path = "crates/path-only" }"#),
        "path-only patch entry must be untouched, got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains(r#"version = "2.0.0""#)
            || cargo_toml
                .lines()
                .all(|l| !l.contains("path-only-lib") || !l.contains("version")),
        "no version key must be inserted into path-only entry, got:\n{cargo_toml}"
    );
}

/// `patch_cargo_crates_io_version` unit test: returns false when no
/// `[patch.crates-io]` block exists.
#[test]
fn patch_cargo_crates_io_version_noop_when_no_patch_block() {
    use crate::cli::pipeline::version_core::patch_cargo_crates_io_version;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");
    std::fs::write(
        &path,
        "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
    )
    .expect("write");

    let changed = patch_cargo_crates_io_version(path.to_str().unwrap(), "my-lib", "1.0.0").expect("no error");
    assert!(!changed, "must return false when [patch.crates-io] is absent");
}

/// `patch_cargo_crates_io_version` unit test: returns false when the named
/// crate is not present in the patch block.
#[test]
fn patch_cargo_crates_io_version_noop_when_crate_absent() {
    use crate::cli::pipeline::version_core::patch_cargo_crates_io_version;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");
    std::fs::write(
        &path,
        "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n\n[patch.crates-io]\nother-crate = { path = \"crates/other\", version = \"0.9.0\" }\n",
    )
    .expect("write");

    let changed = patch_cargo_crates_io_version(path.to_str().unwrap(), "my-lib", "1.0.0").expect("no error");
    assert!(!changed, "must return false when crate is absent from patch block");

    // other-crate must be untouched.
    let content = std::fs::read_to_string(&path).expect("read");
    assert!(
        content.contains(r#"other-crate = { path = "crates/other", version = "0.9.0" }"#),
        "unrelated patch entry must be untouched:\n{content}"
    );
}

/// `patch_cargo_crates_io_version` unit test: is idempotent.
#[test]
fn patch_cargo_crates_io_version_is_idempotent() {
    use crate::cli::pipeline::version_core::patch_cargo_crates_io_version;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Cargo.toml");
    std::fs::write(
        &path,
        "[workspace.package]\nversion = \"3.6.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n\n[patch.crates-io]\nmy-lib-rs = { path = \"crates/my-lib\", version = \"3.6.3\" }\n",
    )
    .expect("write");

    let changed = patch_cargo_crates_io_version(path.to_str().unwrap(), "my-lib-rs", "3.6.3").expect("no error");
    assert!(!changed, "must return false when version already matches");
}
