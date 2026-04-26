use alef_e2e::codegen::rust::render_cargo_toml;

#[test]
fn test_cargo_toml_does_not_contain_workspace_section_in_local_mode() {
    let result = render_cargo_toml(
        "my-lib",         // crate_name
        "my_lib",         // dep_name
        "../../crates/my-lib",  // crate_path
        false,            // needs_serde_json
        false,            // needs_mock_server
        false,            // needs_tokio
        alef_e2e::config::DependencyMode::Local,  // dep_mode
        None,             // version
        &[],              // features
    );

    // The generated Cargo.toml should NOT contain a [workspace] section
    // because local e2e crates should inherit the parent workspace configuration
    assert!(
        !result.contains("[workspace]"),
        "Local mode e2e Cargo.toml should not contain [workspace] section"
    );
}

#[test]
fn test_cargo_toml_does_not_contain_workspace_section_in_registry_mode() {
    let result = render_cargo_toml(
        "my-lib",         // crate_name
        "my_lib",         // dep_name
        "../../crates/my-lib",  // crate_path
        false,            // needs_serde_json
        false,            // needs_mock_server
        false,            // needs_tokio
        alef_e2e::config::DependencyMode::Registry,  // dep_mode
        Some("0.1.0"),    // version
        &[],              // features
    );

    // The generated Cargo.toml should NOT contain a [workspace] section.
    // Registry mode e2e crates are downloaded from a registry; they don't need
    // their own [workspace] header because the consuming project manages that.
    assert!(
        !result.contains("[workspace]"),
        "Registry mode e2e Cargo.toml should not contain [workspace] section"
    );
}

#[test]
fn test_cargo_toml_contains_package_name() {
    let result = render_cargo_toml(
        "my-lib",
        "my_lib",
        "../../crates/my-lib",
        false,
        false,
        false,
        alef_e2e::config::DependencyMode::Local,
        None,
        &[],
    );

    assert!(
        result.contains("name = \"my_lib-e2e-rust\""),
        "Cargo.toml should contain the correct package name"
    );
}
