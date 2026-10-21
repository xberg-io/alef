use alef::backends::pyo3::Pyo3Backend;
use alef::codegen::error_gen::pyo3_error_attribute_fields;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use std::path::Path;
use std::process::Command;

fn surface(root: &Path) -> (ApiSurface, ResolvedCrateConfig) {
    let source = root.join("lib.rs");
    std::fs::write(&source, include_str!("pyo3_cfg_error_payload/core.rs")).unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample-core", "0.0.0", None).unwrap();
    let config = toml::from_str::<NewAlefConfig>(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n[crates.python]\nmodule_name='_sample'\n[crates.python.stubs]\noutput='sample'\n",
    )
    .unwrap()
    .resolve()
    .unwrap()
    .remove(0);
    (api, config)
}

#[test]
fn should_omit_cfg_gated_payload_from_native_projection_and_stubs() {
    let root = tempfile::tempdir().unwrap();
    let (api, config) = surface(root.path());
    assert_eq!(api.errors.len(), 1);
    let variant = &api.errors[0].variants[0];
    assert_eq!(variant.fields.len(), 2);
    assert!(
        variant
            .fields
            .iter()
            .find(|field| field.name == "status")
            .unwrap()
            .cfg
            .is_some()
    );
    let fields = pyo3_error_attribute_fields(variant);
    assert_eq!(
        fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["reason"]
    );
    let files = Pyo3Backend.generate_type_stubs(&api, &config).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].content.contains("reason: str"), "{}", files[0].content);
    assert!(!files[0].content.contains("status:"), "{}", files[0].content);
}

#[test]
fn should_compile_and_preserve_python_payload_with_core_feature_off_and_on() {
    let root = tempfile::tempdir().unwrap();
    let (api, config) = surface(root.path());
    write_native_fixture(root.path(), &api, &config);
    for features in ["", "--features http"] {
        let target = std::env::var("ALEF_PYTHON_CFG_NATIVE_TARGET").unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/pyo3-cfg-error-payload")
                .display()
                .to_string()
        });
        let task = format!(
            "version: '3'\ntasks:\n  test:\n    cmds:\n      - cargo test --offline --quiet --lib --no-default-features {features}\n"
        );
        write(root.path(), "sample-py/Taskfile.yml", &task);
        let result = Command::new("task")
            .args(["--dir", root.path().join("sample-py").to_str().unwrap(), "test"])
            .env("CARGO_TARGET_DIR", target)
            .env("RUSTC_WRAPPER", "")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "feature flags {features:?}: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed; 0 failed"));
    }
}

fn write_native_fixture(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) {
    write(
        root,
        "sample-core/src/lib.rs",
        include_str!("pyo3_cfg_error_payload/core.rs"),
    );
    write(
        root,
        "sample-core/Cargo.toml",
        "[package]\nname='sample-core'\nversion='0.0.0'\nedition='2024'\n[features]\nhttp=[]\n[dependencies]\nthiserror='2'\n",
    );
    let generated = Pyo3Backend.generate_bindings(api, config).unwrap();
    assert_eq!(generated.len(), 1);
    write(
        root,
        "sample-py/src/lib.rs",
        &format!(
            "{}\n{}",
            generated[0].content,
            include_str!("pyo3_cfg_error_payload/probe.rs")
        ),
    );
    write_native_manifest(root);
}

fn write_native_manifest(root: &Path) {
    write(
        root,
        "sample-py/Cargo.toml",
        "[package]\nname='pyo3-cfg-error-payload'\nversion='0.0.0'\nedition='2024'\n[features]\nhttp=['sample-core/http']\n[dependencies]\nsample-core={path='../sample-core'}\npyo3={version='0.30',features=['auto-initialize']}\nserde={version='1',features=['derive']}\nserde_json='1'\n",
    );
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
