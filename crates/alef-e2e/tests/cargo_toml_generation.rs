use alef_e2e::codegen::rust::render_cargo_toml;

#[test]
fn test_cargo_toml_contains_empty_workspace_section_in_local_mode() {
    let result = render_cargo_toml(
        "my-lib",                                // crate_name
        "my_lib",                                // dep_name
        "../../crates/my-lib",                   // crate_path
        false,                                   // needs_serde_json
        false,                                   // needs_mock_server
        false,                                   // needs_tokio
        alef_e2e::config::DependencyMode::Local, // dep_mode
        None,                                    // version
        &[],                                     // features
    );

    // The generated Cargo.toml MUST contain an empty `[workspace]` table so the
    // e2e crate is its own workspace root and never gets pulled into a parent
    // crate's workspace (which would break `cargo fmt`/`cargo build`).
    assert!(
        result.contains("[workspace]"),
        "Local mode e2e Cargo.toml must contain an empty [workspace] section so it stands alone"
    );
}

#[test]
fn test_cargo_toml_contains_empty_workspace_section_in_registry_mode() {
    let result = render_cargo_toml(
        "my-lib",                                   // crate_name
        "my_lib",                                   // dep_name
        "../../crates/my-lib",                      // crate_path
        false,                                      // needs_serde_json
        false,                                      // needs_mock_server
        false,                                      // needs_tokio
        alef_e2e::config::DependencyMode::Registry, // dep_mode
        Some("0.1.0"),                              // version
        &[],                                        // features
    );

    // Registry mode e2e crates also need an empty `[workspace]` so they're
    // self-contained inside any consuming project that happens to be a
    // workspace.
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
        alef_e2e::config::DependencyMode::Local,
        None,
        &[],
    );

    assert!(
        result.contains("name = \"my_lib-e2e-rust\""),
        "Cargo.toml should contain the correct package name"
    );
}
