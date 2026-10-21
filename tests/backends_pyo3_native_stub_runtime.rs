use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ReceiverKind, TypeDef, TypeRef,
};
use alef::e2e::codegen::{E2eCodegen, python::PythonE2eCodegen};
use alef::e2e::config::E2eConfig;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn should_keep_native_methods_in_stubs_without_adding_them_to_public_dataclasses() {
    let fixture = tempfile::tempdir().unwrap();
    let (api, config) = surface();
    write_python(fixture.path(), &api, &config);
    let checked = Command::new("pyrefly")
        .args(["check", "consumer.py"])
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert_success(&checked);
    write(
        fixture.path(),
        "public_missing.py",
        include_str!("pyo3_native_stub/public_missing.py"),
    );
    let rejected = Command::new("pyrefly")
        .args(["check", "public_missing.py"])
        .current_dir(fixture.path())
        .output()
        .unwrap();
    assert!(
        !rejected.status.success(),
        "public dataclass acquired native-only methods: {} {}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr),
    );
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(
        diagnostics.contains("default") && diagnostics.contains("validate"),
        "{diagnostics}"
    );
    assert_eq!(
        diagnostics.lines().filter(|line| line.starts_with("ERROR ")).count(),
        2,
        "{diagnostics}"
    );
    run_native(fixture.path(), &api, &config);
}

fn surface() -> (ApiSurface, ResolvedCrateConfig) {
    let config = toml::from_str::<NewAlefConfig>(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n[crates.python]\nmodule_name='_sample'\n[crates.python.stubs]\noutput='sample'\n",
    ).unwrap().resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "sample-core".into(),
        types: vec![TypeDef {
            name: "ContentConfig".into(),
            rust_path: "sample_core::ContentConfig".into(),
            has_default: true,
            is_clone: true,
            fields: vec![FieldDef {
                name: "label".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            methods: vec![
                MethodDef {
                    name: "default".into(),
                    is_static: true,
                    return_type: TypeRef::Named("ContentConfig".into()),
                    ..Default::default()
                },
                MethodDef {
                    name: "validate".into(),
                    receiver: Some(ReceiverKind::Ref),
                    return_type: TypeRef::Unit,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        errors: vec![ErrorDef {
            name: "CoreError".into(),
            rust_path: "sample_core::CoreError".into(),
            variants: vec![ErrorVariant {
                name: "Rejected".into(),
                fields: vec![FieldDef {
                    name: "vendor".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            exception_base: None,
            original_rust_path: String::new(),
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "fail".into(),
            rust_path: "sample_core::fail".into(),
            return_type: TypeRef::Unit,
            error_type: Some("sample_core::CoreError".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    (api, config)
}

fn write_python(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) {
    let files = Pyo3Backend.generate_type_stubs(api, config).unwrap();
    assert_eq!(files.len(), 1);
    write(root, "sample/_sample.pyi", &files[0].content);
    let public = Pyo3Backend.generate_public_api(api, config).unwrap();
    let options = public.iter().find(|file| file.path.ends_with("options.py")).unwrap();
    write(root, "sample/options.py", &options.content);
    write(root, "sample/__init__.py", "");
    write(root, "pyrefly.toml", "project-includes=['*.py']\nsearch-path=['.']\n");
    write(root, "consumer.py", include_str!("pyo3_native_stub/consumer.py"));
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.module = "sample".into();
    let e2e = PythonE2eCodegen
        .generate(&[], &e2e_config, config, &[], &[], &[], &[])
        .unwrap();
    let conformance = e2e
        .iter()
        .find(|file| file.path.ends_with("test_stub_conformance.py"))
        .unwrap();
    write(root, "sample/conformance.py", &conformance.content);
}

fn run_native(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) {
    write(
        root,
        "sample-core/Cargo.toml",
        "[package]\nname='sample-core'\nversion='0.0.0'\nedition='2024'\n",
    );
    write(root, "sample-core/src/lib.rs", include_str!("pyo3_native_stub/core.rs"));
    write(
        root,
        "sample-py/Cargo.toml",
        "[package]\nname='native-stub-probe'\nversion='0.0.0'\nedition='2024'\n[dependencies]\nsample-core={path='../sample-core'}\npyo3={version='0.30',features=['auto-initialize']}\nserde={version='1',features=['derive']}\nserde_json='1'\n",
    );
    let generated = Pyo3Backend.generate_bindings(api, config).unwrap();
    assert_eq!(generated.len(), 1);
    let source = format!(
        "{}\n{}",
        generated[0].content,
        include_str!("pyo3_native_stub/probe.rs")
    );
    write(root, "sample-py/src/lib.rs", &source);
    write(
        root,
        "sample-py/src/conformance_probe.py",
        include_str!("pyo3_native_stub/test_conformance_probe.py"),
    );
    write(
        root,
        "sample-py/Taskfile.yml",
        "version: '3'\ntasks:\n  test:\n    cmds:\n      - cargo clippy --offline --quiet --lib -- -D warnings\n      - cargo test --offline --quiet --lib\n",
    );
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pyo3-native-stub-probe");
    let tested = Command::new("task")
        .arg("--dir")
        .arg(root.join("sample-py"))
        .arg("test")
        .env("CARGO_TARGET_DIR", target)
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    assert_success(&tested);
    assert!(String::from_utf8_lossy(&tested.stdout).contains("1 passed"));
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
