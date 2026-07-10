use alef::e2e::codegen::rust::render_cargo_toml;

#[test]
fn test_cargo_toml_contains_empty_workspace_section_in_local_mode() {
    let result = render_cargo_toml(
        "my-lib",
        "my_lib",
        "../../crates/my-lib",
        false,
        false,
        false,
        false,
        false,
        false,
        alef::e2e::config::DependencyMode::Local,
        None,
        &[],
    );

    assert!(
        result.contains("[workspace]"),
        "Local mode e2e Cargo.toml must contain an empty [workspace] section so it stands alone"
    );
}

#[test]
fn test_cargo_toml_contains_empty_workspace_section_in_registry_mode() {
    let result = render_cargo_toml(
        "my-lib",
        "my_lib",
        "../../crates/my-lib",
        false,
        false,
        false,
        false,
        false,
        false,
        alef::e2e::config::DependencyMode::Registry,
        Some("0.1.0"),
        &[],
    );

    assert!(
        result.contains("[workspace]"),
        "Registry mode e2e Cargo.toml must contain an empty [workspace] section so it stands alone"
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
        false,
        false,
        false,
        alef::e2e::config::DependencyMode::Local,
        None,
        &[],
    );

    assert!(
        result.contains("name = \"my_lib-e2e-rust\""),
        "Cargo.toml should contain the correct package name"
    );
}
