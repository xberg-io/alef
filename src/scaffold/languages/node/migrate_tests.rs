use super::*;

fn pre_fix_service_package_json() -> String {
    format!(
        "{{\n  \"name\": \"@scope/example\",\n  \"version\": \"1.0.0\",\n  \"main\": \"index-wrapper.cjs\",\n  \"types\": \"index.d.ts\",\n  {STALE_SERVICE_EXPORTS_MAP}\n  \"files\": [\"index.js\", \"index-wrapper.cjs\", \"index.d.ts\", \"service.cjs\", \"*.node\"],\n  \"scripts\": {{\n    \"build\": \"napi build\"\n  }}\n}}\n"
    )
}

#[test]
fn should_add_service_subpath_export_when_missing_from_service_api_crate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-node");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-node");
    std::fs::write(pkg_dir.join("package.json"), pre_fix_service_package_json()).expect("write pre-fix package.json");

    let relative_path = Path::new("crates/example-node/package.json");
    let changed =
        migrate_node_package_json_service_export(dir.path(), relative_path).expect("migration must not error");
    assert!(
        changed,
        "a service-API package.json missing ./service export must be reported as changed"
    );

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read migrated file");
    let parsed: serde_json::Value = serde_json::from_str(&on_disk).expect("migrated file must be valid JSON");
    assert_eq!(parsed["exports"]["./service"]["require"], "./service.cjs");
    assert_eq!(
        parsed["exports"]["."]["require"], "./index-wrapper.cjs",
        "the root export must be preserved"
    );
    assert_eq!(
        parsed["scripts"]["build"], "napi build",
        "fields outside exports must survive untouched"
    );

    let changed_again =
        migrate_node_package_json_service_export(dir.path(), relative_path).expect("second pass must not error");
    assert!(
        !changed_again,
        "second pass over an already-migrated file must be a no-op"
    );
}

#[test]
fn should_not_touch_a_non_service_package_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-node");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-node");
    let non_service = "{\n  \"name\": \"@scope/example\",\n  \"main\": \"index.js\",\n  \"exports\": {\n    \".\": {\n      \"types\": \"./index.d.ts\",\n      \"require\": \"./index.js\",\n      \"default\": \"./index.js\"\n    }\n  },\n  \"files\": [\"index.js\", \"index.d.ts\", \"*.node\"]\n}\n";
    std::fs::write(pkg_dir.join("package.json"), non_service).expect("write non-service package.json");

    let relative_path = Path::new("crates/example-node/package.json");
    let changed =
        migrate_node_package_json_service_export(dir.path(), relative_path).expect("migration must not error");
    assert!(
        !changed,
        "a package.json with no service.cjs in files must never be touched"
    );

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(
        on_disk, non_service,
        "a non-service package.json must survive byte-for-byte"
    );
}

#[test]
fn should_not_touch_a_package_json_that_already_has_the_service_export() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg_dir = dir.path().join("crates/example-node");
    std::fs::create_dir_all(&pkg_dir).expect("create crates/example-node");
    let already_fixed = format!(
        "{{\n  \"name\": \"@scope/example\",\n  {FIXED_SERVICE_EXPORTS_MAP}\n  \"files\": [\"index.js\", \"index-wrapper.cjs\", \"index.d.ts\", \"service.cjs\", \"*.node\"]\n}}\n"
    );
    std::fs::write(pkg_dir.join("package.json"), &already_fixed).expect("write already-fixed package.json");

    let relative_path = Path::new("crates/example-node/package.json");
    let changed =
        migrate_node_package_json_service_export(dir.path(), relative_path).expect("migration must not error");
    assert!(
        !changed,
        "a package.json that already has the ./service export must never be touched"
    );

    let on_disk = std::fs::read_to_string(pkg_dir.join("package.json")).expect("read file");
    assert_eq!(
        on_disk, already_fixed,
        "an already-fixed package.json must survive byte-for-byte"
    );
}

#[test]
fn migrate_node_service_export_is_a_no_op_when_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let relative_path = Path::new("crates/example-node/package.json");
    let changed = migrate_node_package_json_service_export(dir.path(), relative_path).expect("must not error");
    assert!(!changed);
    assert!(!dir.path().join(relative_path).exists());
}
