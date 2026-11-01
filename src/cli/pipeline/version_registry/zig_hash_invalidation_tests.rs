use super::sync_registry_package_versions;

const CONTENT_ID: &str = "Jfgk_AR7FjNQG4Xrznb1QGb5Ok1RHe9S-qBaV1eYM_f4";

fn configuration(name: &str, fields: &str) -> String {
    format!(
        r#"[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.e2e.packages.zig]
# Keep this package selection.
name = "{name}"
path = "../../packages/zig"
{fields}
[crates.e2e.registry.packages.zig]
name = "{name}"
path = "../../packages/zig"
{fields}
"#
    )
}

fn synchronize(content: &str, version: &str) -> (bool, String) {
    let directory = tempfile::tempdir().expect("configuration fixture");
    let path = directory.path().join("alef.toml");
    std::fs::write(&path, content).expect("write real CLI configuration input");
    let changed = sync_registry_package_versions(&path, version).expect("synchronize package references");
    (
        changed,
        std::fs::read_to_string(path).expect("read actual configuration output"),
    )
}

fn zig_packages(content: &str) -> Vec<toml::Value> {
    let document: toml::Value = toml::from_str(content).expect("configuration remains valid TOML");
    let e2e = &document["crates"][0]["e2e"];
    vec![
        e2e["packages"]["zig"].clone(),
        e2e["registry"]["packages"]["zig"].clone(),
    ]
}

fn assert_platform_selection_is_retained_with_an_invalidated_identity(package: &toml::Value) {
    let platforms = package
        .get("platform_hashes")
        .and_then(toml::Value::as_table)
        .expect("platform selection must survive invalidation");
    assert_eq!(platforms.len(), 1);
    assert_eq!(
        platforms["aarch64-apple-darwin"].as_str(),
        Some("STALE_HASH_REGENERATE")
    );
}

#[test]
fn version_change_removes_content_hashes_from_both_package_tables() {
    let content = configuration(
        "my-client",
        &format!("version = \"2.4.4\"\nhash = \"my-client-2.4.4-{CONTENT_ID}\"\n"),
    );
    let (changed, output) = synchronize(&content, "2.4.5");
    assert!(changed);
    for package in zig_packages(&output) {
        assert_eq!(package["version"].as_str(), Some("2.4.5"));
        assert!(package.get("hash").is_none(), "stale identity remains: {package}");
        assert_eq!(package["name"].as_str(), Some("my-client"));
        assert_eq!(package["path"].as_str(), Some("../../packages/zig"));
    }
    assert!(output.contains("# Keep this package selection."));
}

#[test]
fn same_version_preserves_hashes_and_configuration_bytes() {
    let content = configuration(
        "my-client",
        &format!("version = \"2.4.4\"\nhash = \"my-client-2.4.4-{CONTENT_ID}\"\n"),
    );
    let (changed, output) = synchronize(&content, "2.4.4");
    assert!(!changed);
    assert_eq!(output, content);
}

#[test]
fn hash_only_version_change_removes_hash_without_inserting_version() {
    let content = configuration("liter_llm", &format!("hash = \"liter_llm-2.4.4-{CONTENT_ID}\"\n"));
    let (changed, output) = synchronize(&content, "2.4.5");
    assert!(changed);
    for package in zig_packages(&output) {
        assert!(package.get("hash").is_none(), "stale identity remains: {package}");
        assert!(package.get("version").is_none());
    }
}

#[test]
fn hash_only_current_versions_preserve_hyphenated_names_and_content_ids() {
    for name in ["liter_llm", "my-client", "client-1.2.3"] {
        for version in ["2.4.4", "2.4.4-rc.1", "2.4.4-rc-build.2"] {
            let content = configuration(name, &format!("hash = \"{name}-{version}-{CONTENT_ID}\"\n"));
            let (changed, output) = synchronize(&content, version);
            assert!(!changed, "current hash was changed for {name}/{version}: {output}");
            assert_eq!(output, content);
        }
    }
}

#[test]
fn hash_only_prerelease_bumps_invalidate_both_tables() {
    for name in ["liter_llm", "my-client"] {
        let content = configuration(name, &format!("hash = \"{name}-2.4.4-rc-build.2-{CONTENT_ID}\"\n"));
        let (changed, output) = synchronize(&content, "2.4.4");
        assert!(changed);
        for package in zig_packages(&output) {
            assert!(package.get("hash").is_none(), "stale identity remains: {package}");
            assert!(package.get("version").is_none());
            assert_eq!(package["name"].as_str(), Some(name));
        }
    }
}

#[test]
fn unrecognized_hash_only_values_are_preserved_without_claiming_a_version_change() {
    for hash in ["STALE_HASH_REGENERATE", "1220abcdef", "unrecognized"] {
        let content = configuration("demo", &format!("hash = \"{hash}\"\n"));
        let (changed, output) = synchronize(&content, "2.4.5");
        assert!(!changed);
        assert_eq!(output, content);
    }
}

#[test]
fn invalidation_is_idempotent() {
    let content = configuration(
        "demo",
        &format!("version = \"2.4.4\"\nhash = \"demo-2.4.4-{CONTENT_ID}\"\n"),
    );
    let (_, output) = synchronize(&content, "2.4.5");
    assert!(
        zig_packages(&output)
            .iter()
            .all(|package| package.get("hash").is_none())
    );
    let (changed_again, output_again) = synchronize(&output, "2.4.5");
    assert!(!changed_again);
    assert_eq!(output_again, output);
}

#[test]
fn version_change_invalidates_platform_overrides_and_generic_hashes() {
    let fields = format!(
        "version = \"2.4.4\"\nhash = \"demo-2.4.4-{CONTENT_ID}\"\n\
         platform_hashes = {{ aarch64-apple-darwin = \"demo-2.4.4-{CONTENT_ID}\" }}\n"
    );
    let content = configuration("demo", &fields);
    let (unchanged, same_output) = synchronize(&content, "2.4.4");
    assert!(!unchanged);
    assert_eq!(same_output, content);
    let (changed, output) = synchronize(&content, "2.4.5");
    assert!(changed);
    for package in zig_packages(&output) {
        assert_platform_selection_is_retained_with_an_invalidated_identity(&package);
        assert!(package.get("hash").is_none());
        assert_eq!(package["version"].as_str(), Some("2.4.5"));
        assert_eq!(package["path"].as_str(), Some("../../packages/zig"));
    }
}

#[test]
fn platform_hash_only_tables_infer_version_without_fabricating_metadata() {
    let content = configuration(
        "my-client",
        &format!("platform_hashes = {{ aarch64-apple-darwin = \"my-client-2.4.4-rc.1-{CONTENT_ID}\" }}\n"),
    );
    let (unchanged, same_output) = synchronize(&content, "2.4.4-rc.1");
    assert!(!unchanged);
    assert_eq!(same_output, content);
    let (changed, output) = synchronize(&content, "2.4.5");
    assert!(changed, "stale platform identities were not invalidated");
    for package in zig_packages(&output) {
        assert_platform_selection_is_retained_with_an_invalidated_identity(&package);
        assert!(package.get("hash").is_none());
        assert!(package.get("version").is_none());
    }
}

#[test]
fn hash_only_tables_without_a_package_name_infer_prerelease_versions() {
    let content = configuration("my-client", &format!("hash = \"my-client-2.4.4-rc.1-{CONTENT_ID}\"\n"))
        .replace("name = \"my-client\"\n", "");
    let (changed, output) = synchronize(&content, "2.4.5");
    assert!(changed);
    for package in zig_packages(&output) {
        assert!(package.get("hash").is_none(), "stale identity remains: {package}");
        assert!(package.get("version").is_none());
    }
}

#[test]
fn ambiguous_hash_only_versions_are_preserved_without_an_explicit_package_identity() {
    let content = configuration("client-1.2.3", &format!("hash = \"client-1.2.3-2.4.4-{CONTENT_ID}\"\n"))
        .replace("name = \"client-1.2.3\"\n", "");
    for version in ["2.4.4", "2.4.5"] {
        let (changed, output) = synchronize(&content, version);
        assert!(
            !changed,
            "an ambiguous identity must not invent a prior version: {output}"
        );
        assert_eq!(output, content);
    }
}

#[test]
fn non_zig_content_hashes_are_not_removed_or_rewritten() {
    let content = configuration("demo", "version = \"2.4.4\"\n")
        + "\n[crates.e2e.registry.packages.rust]\nversion = \"2.4.4\"\nhash = \"independent-identity\"\n";
    let (changed, output) = synchronize(&content, "2.4.5");
    assert!(changed);
    let document: toml::Value = toml::from_str(&output).expect("valid output TOML");
    let rust = &document["crates"][0]["e2e"]["registry"]["packages"]["rust"];
    assert_eq!(rust["version"].as_str(), Some("2.4.5"));
    assert_eq!(rust["hash"].as_str(), Some("independent-identity"));
}
