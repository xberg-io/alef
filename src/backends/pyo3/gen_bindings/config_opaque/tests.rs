use super::super::Pyo3Backend;
use crate::core::backend::Backend;
use crate::core::config::{CapsuleTypeConfig, NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use std::path::Path;
use std::process::{Command, Output};

mod fixture;

fn config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.python]
module_name = "_test_lib"
"#,
    )
    .unwrap();
    config.resolve().unwrap().remove(0)
}

fn api() -> ApiSurface {
    let types = ["SharedHandle", "MutableHandle", "LocalHandle"]
        .into_iter()
        .map(|name| {
            let mut methods = vec![MethodDef {
                name: "value".into(),
                receiver: Some(ReceiverKind::Ref),
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }];
            if name != "LocalHandle" {
                methods.push(MethodDef {
                    name: "increment".into(),
                    receiver: Some(if name == "MutableHandle" {
                        ReceiverKind::RefMut
                    } else {
                        ReceiverKind::Ref
                    }),
                    return_type: TypeRef::Unit,
                    ..Default::default()
                });
            }
            TypeDef {
                name: name.into(),
                rust_path: format!("test_lib::{name}"),
                is_opaque: true,
                has_default: true,
                methods,
                ..Default::default()
            }
        })
        .collect();
    ApiSurface {
        crate_name: "test-lib".into(),
        version: "0.1.0".into(),
        types,
        ..Default::default()
    }
}

#[test]
fn send_sync_opt_in_defaults_to_empty_and_round_trips_config() {
    let mut config = config();
    let python = config.python.as_mut().unwrap();
    assert!(python.send_sync_types.is_empty());
    python.send_sync_types = vec!["SharedHandle".into()];
    let encoded = serde_json::to_value(&python).unwrap();
    let decoded: crate::core::config::PythonConfig = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.send_sync_types, ["SharedHandle"]);
}

#[test]
fn send_sync_opt_in_rejects_unavailable_or_non_opaque_classes() {
    for case in [
        "missing",
        "dto",
        "trait",
        "excluded",
        "binding_excluded",
        "capsule",
        "external",
    ] {
        let mut api = api();
        let mut config = config();
        let python = config.python.as_mut().unwrap();
        python.send_sync_types = vec!["SharedHandle".into()];
        match case {
            "missing" => {
                api.types.remove(0);
            }
            "dto" => api.types[0].is_opaque = false,
            "trait" => api.types[0].is_trait = true,
            "excluded" => python.exclude_types.push("SharedHandle".into()),
            "binding_excluded" => api.types[0].binding_excluded = true,
            "capsule" => {
                python.capsule_types.insert(
                    "SharedHandle".into(),
                    CapsuleTypeConfig::Capsule("sample.Handle".into()),
                );
            }
            "external" => {
                config
                    .opaque_types
                    .insert("SharedHandle".into(), "external::Handle".into());
                api.types.remove(0);
            }
            _ => unreachable!(),
        };
        let error = Pyo3Backend.generate_bindings(&api, &config).expect_err(case);
        assert!(error.to_string().contains("python.send_sync_types"), "{case}: {error}");
        assert!(error.to_string().contains("SharedHandle"), "{case}: {error}");
    }
}

fn write_fixture(root: &Path) -> std::path::PathBuf {
    let core = root.join("test-lib");
    let binding = root.join("test-lib-py");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(core.join("Cargo.toml"), fixture::CORE_MANIFEST).unwrap();
    std::fs::write(core.join("src/lib.rs"), fixture::CORE_SOURCE).unwrap();
    std::fs::write(binding.join("Cargo.toml"), fixture::BINDING_MANIFEST).unwrap();
    binding
}

fn generate_fixture(binding: &Path, send_sync_types: &[&str]) -> String {
    let mut config = config();
    config.python.as_mut().unwrap().send_sync_types = send_sync_types.iter().map(|name| (*name).into()).collect();
    config.output_paths.insert("python".into(), binding.join("src"));
    let files = Pyo3Backend.generate_bindings(&api(), &config).unwrap();
    let mut source = files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content
        .clone();
    source.push_str(fixture::RUNTIME_TEST);
    std::fs::write(binding.join("src/lib.rs"), &source).unwrap();
    source
}

fn cargo_command(binding: &Path, command: &str) -> Command {
    let mut cargo = crate::test_support::spawn_from_stable_dir("cargo");
    cargo
        .args([command, "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", binding.parent().unwrap().join("target"))
        .current_dir(binding);
    cargo
}

fn run_cargo(binding: &Path, command: &str) -> Output {
    cargo_command(binding, command).output().unwrap()
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn cargo_command_should_use_fixture_as_working_directory() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let binding = write_fixture(temporary_directory.path());
    let command = cargo_command(&binding, "check");
    assert_eq!(command.get_current_dir(), Some(binding.as_path()));
}

#[test]
fn send_sync_generated_handles_cross_threads_without_weakening_rust_bounds() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let binding = write_fixture(temporary_directory.path());

    generate_fixture(&binding, &[]);
    let default_check = run_cargo(&binding, "check");
    assert!(default_check.status.success(), "{}", diagnostics(&default_check));
    let default_run = run_cargo(&binding, "test");
    assert!(
        !default_run.status.success(),
        "default handles must remain thread-affine"
    );
    assert!(
        diagnostics(&default_run).contains("unsendable") && diagnostics(&default_run).contains("1 failed"),
        "{}",
        diagnostics(&default_run)
    );

    let source = generate_fixture(&binding, &["SharedHandle", "MutableHandle"]);
    let shared_run = run_cargo(&binding, "test");
    assert!(shared_run.status.success(), "{}\n{source}", diagnostics(&shared_run));
    assert!(
        diagnostics(&shared_run).contains("1 passed"),
        "{}",
        diagnostics(&shared_run)
    );

    generate_fixture(&binding, &["LocalHandle"]);
    let invalid_check = run_cargo(&binding, "check");
    let errors = diagnostics(&invalid_check);
    assert!(
        !invalid_check.status.success(),
        "Rc-backed handles must not satisfy the opt-in"
    );
    assert!(
        errors.contains("Rc<u32>") && errors.contains("between threads safely"),
        "{errors}"
    );
}
