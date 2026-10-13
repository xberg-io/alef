use super::*;

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn sync_updates_root_and_workspace_members_but_preserves_registry_entries() {
    let root = tempfile::tempdir().unwrap();
    let _cwd = crate::test_support::CwdGuard::enter(root.path());
    write(
        root.path(),
        "pyproject.toml",
        "[project]\nname = 'sample-lib'\nversion = '1.2.3'\n",
    );
    write(
        root.path(),
        "member/pyproject.toml",
        "[project]\nname = 'sample-member'\nversion = '2.3.4'\n",
    );
    write(
        root.path(),
        "uv.lock",
        "version = 1\n# preserve this annotation\n[[package]]\nname = 'sample_lib'\nversion = '1.2.2'\nsource = { editable = '.' }\n[[package]]\nname = 'sample-member'\nversion = '2.3.3'\nsource = { virtual = 'member' }\n[[package]]\nname = 'dependency'\nversion = '9.8.7'\nsource = { registry = 'https://example.com/index' }\n",
    );
    let filter = IgnoreFilter::for_current_dir();
    let mut updated = Vec::new();
    sync(&filter, &mut updated).unwrap();
    assert_eq!(updated.len(), 1);
    let text = std::fs::read_to_string("uv.lock").unwrap();
    let lock: toml::Value = toml::from_str(&text).unwrap();
    let versions: Vec<_> = lock["package"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["version"].as_str().unwrap())
        .collect();
    assert_eq!(versions, ["1.2.3", "2.3.4", "9.8.7"]);
    assert!(text.contains("# preserve this annotation"));
    updated.clear();
    sync(&filter, &mut updated).unwrap();
    assert!(updated.is_empty());
    assert_eq!(std::fs::read_to_string("uv.lock").unwrap(), text);
}

#[test]
fn maturin_dynamic_version_uses_the_configured_rust_manifest_and_pep440() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "native/Cargo.toml",
        "[package]\nname = 'sample-lib'\nversion = '1.2.3-rc.1'\n",
    );
    let manifest = toml::from_str("[project]\nname = 'sample-lib'\ndynamic = ['version']\n[build-system]\nbuild-backend = 'maturin'\n[tool.maturin]\nmanifest-path = 'native/Cargo.toml'\n").unwrap();
    assert_eq!(project_version(&manifest, root.path()).as_deref(), Some("1.2.3rc1"));
}

#[test]
fn version_sync_updates_root_python_manifest_and_its_real_uv_lock() {
    let root = tempfile::tempdir().unwrap();
    let _cwd = crate::test_support::CwdGuard::enter(root.path());
    write(
        root.path(),
        "Cargo.toml",
        "[package]\nname = 'sample-lib'\nversion = '1.2.3'\n",
    );
    write(
        root.path(),
        "pyproject.toml",
        "[project]\nname = 'sample-lib'\nversion = '1.2.2'\nrequires-python = '>=3.10'\n",
    );
    write(
        root.path(),
        "uv.lock",
        "version = 1\n[[package]]\nname = 'sample-lib'\nversion = '1.2.2'\nsource = { virtual = '.' }\n",
    );
    let config = crate::core::config::ResolvedCrateConfig {
        name: "sample-lib".into(),
        version_from: "Cargo.toml".into(),
        ..Default::default()
    };
    super::super::super::version::sync_versions(&config, Path::new("alef.toml"), None, true, true, None).unwrap();
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string("pyproject.toml").unwrap()).unwrap();
    let lock: toml::Value = toml::from_str(&std::fs::read_to_string("uv.lock").unwrap()).unwrap();
    assert_eq!(manifest["project"]["version"].as_str(), Some("1.2.3"));
    assert_eq!(lock["package"][0]["version"].as_str(), Some("1.2.3"));
    assert!(super::super::super::lock_freshness::uv::stale_uv_lock_findings(root.path()).is_empty());
}

#[test]
fn local_package_version_preserves_workspace_containment_and_package_identity() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(
        workspace.path(),
        "pyproject.toml",
        "[project]\nname = 'sample-lib'\nversion = '1.2.3'\n",
    );
    write(
        outside.path(),
        "pyproject.toml",
        "[project]\nname = 'sample-lib'\nversion = '8.9.0'\n",
    );
    let root = workspace.path().canonicalize().unwrap();
    for (directory, name, expected) in [
        (workspace.path(), "sample_lib", Some("1.2.3")),
        (workspace.path(), "other-lib", None),
        (outside.path(), "sample-lib", None),
    ] {
        let document: toml_edit::DocumentMut =
            format!("[[package]]\nname = '{name}'\nversion = '1.0.0'\nsource = {{ editable = '.' }}\n")
                .parse()
                .unwrap();
        let package = document["package"].as_array_of_tables().unwrap().get(0).unwrap();
        assert_eq!(local_package_version(package, directory, &root).as_deref(), expected);
    }
}
