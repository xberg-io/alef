use super::*;

#[test]
fn test_setup_ts_no_duplicate_imports() {
    // Test that setup.ts with file_setup does not emit duplicate imports.
    // Both createRequire and fileURLToPath should appear exactly once.
    let setup = render_setup("fixtures", true, "@sample_core/wasm", &std::collections::HashMap::new());

    let create_require_count = setup.matches("import { createRequire }").count();
    let file_url_to_path_count = setup.matches("import { fileURLToPath }").count();
    let read_file_sync_count = setup.matches("readFileSync").count();
    let path_imports_count = setup.matches("import { dirname, join }").count();

    assert_eq!(create_require_count, 1, "createRequire should be imported exactly once");
    assert_eq!(
        file_url_to_path_count, 1,
        "fileURLToPath should be imported exactly once"
    );
    assert!(read_file_sync_count >= 2, "readFileSync should be imported and used"); // one import, multiple uses
    assert_eq!(path_imports_count, 1, "dirname, join should be imported exactly once");

    // Verify consolidated imports are at the top.
    let first_import = setup.find("import {").expect("should have imports");
    let first_patch_comment = setup.find("// Patch CommonJS").expect("should have patch comment");
    assert!(
        first_import < first_patch_comment,
        "all imports should come before the patch comment"
    );
}

#[test]
fn test_package_json_includes_node_options_memory_cap() {
    // NODE_OPTIONS=--max-old-space-size=4096 caps V8 heap to 4GB to prevent OOM
    // on WASM e2e tests when compiling large WASM modules.
    let pkg_json = render_package_json(
        "@test/wasm",
        "pkg",
        false,
        "0.1.0",
        crate::e2e::config::DependencyMode::Local,
        None,
    );

    assert!(
        pkg_json.contains("NODE_OPTIONS=--max-old-space-size=4096"),
        "package.json test script must include NODE_OPTIONS memory cap; got:\n{pkg_json}"
    );
    assert!(
        pkg_json.contains("\"test\": \"NODE_OPTIONS=--max-old-space-size=4096 vitest run\""),
        "NODE_OPTIONS must be part of the test script; got:\n{pkg_json}"
    );
    assert!(
        pkg_json.contains("\"node\": \">= 22\""),
        "package.json must require Node 22 or newer; got:\n{pkg_json}"
    );
}

#[test]
fn test_package_json_registry_release_uses_caret() {
    let pkg_json = render_package_json(
        "@test/wasm",
        "pkg",
        false,
        "1.2.3",
        crate::e2e::config::DependencyMode::Registry,
        None,
    );
    assert!(
        pkg_json.contains("\"^1.2.3\""),
        "registry release pin must use caret; got:\n{pkg_json}"
    );
}

#[test]
fn test_package_json_registry_prerelease_uses_caret_semver() {
    let pkg_json = render_package_json(
        "@test/wasm",
        "pkg",
        false,
        "3.6.0-rc.1",
        crate::e2e::config::DependencyMode::Registry,
        None,
    );
    assert!(
        pkg_json.contains("\"^3.6.0-rc.1\""),
        "registry pre-release pin must use caret with raw semver; got:\n{pkg_json}"
    );
}

#[test]
fn test_package_json_registry_already_prefixed_passes_through() {
    // When alef.toml's [crates.e2e.registry.packages.wasm] version field already
    // includes a semver range operator (`^3.6.0-rc.1`), the codegen must use it
    // verbatim — prepending another `^` produces a double-prefix bug.
    let pkg_json = render_package_json(
        "@test/wasm",
        "pkg",
        false,
        "^3.6.0-rc.1",
        crate::e2e::config::DependencyMode::Registry,
        None,
    );
    assert!(
        pkg_json.contains("\"^3.6.0-rc.1\""),
        "already-prefixed input must pass through verbatim; got:\n{pkg_json}"
    );
    assert!(
        !pkg_json.contains("^^"),
        "must not double the `^` prefix; got:\n{pkg_json}"
    );
}

#[test]
fn render_setup_emits_e2e_env_assignments_alphabetically() {
    // Test that env vars are emitted as `process.env.KEY ??= "VALUE";`
    // assignments, sorted alphabetically by key.

    // Test with non-empty env map
    let mut env = std::collections::HashMap::new();
    env.insert("ZEBRA".to_string(), "last_alphabetically".to_string());
    env.insert("APPLE".to_string(), "first_alphabetically".to_string());
    env.insert("SAMPLE_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());

    let setup = render_setup("fixtures", true, "@sample_core/wasm", &env);

    // Verify all env keys are present
    assert!(
        setup.contains("process.env.APPLE ??= \"first_alphabetically\";"),
        "APPLE env var should be emitted, got:\n{setup}"
    );
    assert!(
        setup.contains("process.env.SAMPLE_ALLOW_PRIVATE_NETWORK ??= \"true\";"),
        "SAMPLE_ALLOW_PRIVATE_NETWORK env var should be emitted, got:\n{setup}"
    );
    assert!(
        setup.contains("process.env.ZEBRA ??= \"last_alphabetically\";"),
        "ZEBRA env var should be emitted, got:\n{setup}"
    );

    // Verify alphabetical order: APPLE < SAMPLE < ZEBRA
    let apple_idx = setup.find("APPLE").expect("APPLE should be present");
    let sample_idx = setup.find("SAMPLE").expect("SAMPLE should be present");
    let zebra_idx = setup.find("ZEBRA").expect("ZEBRA should be present");
    assert!(
        apple_idx < sample_idx && sample_idx < zebra_idx,
        "env vars must be sorted alphabetically; got positions: APPLE={}, SAMPLE={}, ZEBRA={}",
        apple_idx,
        sample_idx,
        zebra_idx
    );

    // Verify env block appears after imports but before wasm init
    let imports_end = setup
        .find("import { dirname, join }")
        .expect("imports should be present");
    let env_block = setup.find("process.env.APPLE").expect("env block should be present");
    let wasm_init = setup
        .find("// Pre-initialize the wasm")
        .expect("wasm init should be present");
    assert!(
        imports_end < env_block && env_block < wasm_init,
        "env block must come after imports and before wasm init"
    );

    // Test with empty env map — should emit no env block
    let empty_setup = render_setup("fixtures", true, "@sample_core/wasm", &std::collections::HashMap::new());
    assert!(
        !empty_setup.contains("process.env."),
        "empty env map should not emit any env assignments, got:\n{empty_setup}"
    );
}
