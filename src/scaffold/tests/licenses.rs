use super::*;

#[test]
fn test_scaffold_python_license_files_field() {
    // Verify that pyproject.toml includes license-files = ["LICENSE"] to ensure
    // maturin bundles the LICENSE file in the wheel. This fixes BLK-10 where PyPI
    // rejected wheels with License-File: LICENSE in METADATA but no actual LICENSE
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

    assert!(
        pyproject_content.contains("license-files = [ \"LICENSE\" ]"),
        "pyproject.toml should declare license-files = [ \"LICENSE\" ] (with inner spaces)"
    );

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

    let all_files = scaffold(&api, &config, &[Language::Dart]).unwrap();
    let license_files: Vec<_> = all_files.iter().filter(|f| f.path.ends_with("LICENSE")).collect();

    assert_eq!(license_files.len(), 1, "one language → one LICENSE, no duplicates");
    assert_eq!(
        license_files[0].path,
        PathBuf::from("packages/dart/LICENSE"),
        "Dart LICENSE must live in packages/dart/"
    );
}
