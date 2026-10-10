use std::path::PathBuf;

#[test]
fn published_alef_crate_allowlists_runtime_files() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read workspace manifest");
    let parsed: toml::Value = toml::from_str(&manifest).expect("parse workspace manifest");
    let includes = parsed["package"]["include"]
        .as_array()
        .expect("package.include must allowlist the published crate payload");
    let actual_includes: Vec<_> = includes.iter().filter_map(toml::Value::as_str).collect();

    assert_eq!(
        actual_includes,
        [
            "Cargo.lock",
            "LICENSE",
            "README.md",
            "assets/**",
            "build.rs",
            "src/**",
            "!src/**/tests.rs",
            "!src/**/tests/**",
            "!src/**/*_test.rs",
            "!src/**/*_tests.rs",
            "!src/test_support.rs",
            "!src/test_support/**",
            "src/backends/jni/gen_shims/backend_tests.rs",
            "src/backends/jni/gen_shims/target_gate_tests.rs",
            "src/backends/jni/gen_shims/tests.rs",
            "src/backends/jni/gen_shims/transparent_string_tests.rs",
            "src/backends/kotlin_android/gen_seed_test.rs",
            "src/backends/kotlin/gen_bindings/jni_emitter/tests.rs",
            "src/e2e/codegen/brew/run_tests.rs",
        ],
        "the published crate must contain only crate source, build, and runtime inputs"
    );
}
