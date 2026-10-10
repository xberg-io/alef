use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, EntrypointDef, EntrypointKind, MethodDef, ServiceDef, TypeRef};

const CORE: &str = r#"
#[derive(Debug)]
pub struct CoreError;
impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("service failed") }
}
impl std::error::Error for CoreError {}
pub struct App;
impl App {
    pub fn new() -> Self { Self }
    pub async fn run(&self) -> Result<(), CoreError> { Err(CoreError) }
    pub async fn finish(&self) {}
}
"#;

const TEST_PREFIX: &str = r#"
#[cfg(test)]
mod service_tests {
    use super::*;
    #[test]
    fn service_maps_core_error_without_requiring_from_pyerr() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_sample").unwrap();
            _sample(&module).unwrap();
            let error = module.getattr("app_run").unwrap().call1((pyo3::types::PyList::empty(py),)).unwrap_err();
            assert!(error.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py));
            assert!(error.to_string().contains("service failed"));
            module.getattr("app_finish").unwrap().call1((pyo3::types::PyList::empty(py),)).unwrap();
"#;

const TEST_SUFFIX: &str = r#"
        });
    }
}
"#;

pub fn run() {
    let dir = tempfile::tempdir().unwrap();
    let core = dir.path().join("sample-core");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(
        core.join("Cargo.toml"),
        "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(core.join("src/lib.rs"), CORE).unwrap();
    let target = std::env::var_os("ALEF_LIFECYCLE_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.path().join("target"));
    for mode in ["managed", "default"] {
        verify_mode(dir.path(), &target, mode);
    }
}

fn verify_mode(dir: &std::path::Path, target: &std::path::Path, mode: &str) {
    let binding = dir.join(mode);
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(binding.join("Cargo.toml"), "[package]\nname='service-py'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nsample-core={path='../sample-core'}\nserde={version='1',features=['derive']}\nserde_json='1'\naxum='0.8'\npyo3={version='0.29',features=['auto-initialize']}\npyo3-async-runtimes={version='0.29',features=['tokio-runtime']}\ntokio={version='1',features=['full']}\n").unwrap();
    let option = if mode == "managed" {
        "async_runtime='managed'\n"
    } else {
        ""
    };
    let config: NewAlefConfig = toml::from_str(&format!("[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n[crates.python]\nmodule_name='_sample'\n{option}")).unwrap();
    let config = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        services: vec![ServiceDef {
            name: "App".into(),
            rust_path: "sample_core::App".into(),
            constructor: MethodDef {
                name: "new".into(),
                ..Default::default()
            },
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![
                EntrypointDef {
                    method: "run".into(),
                    kind: EntrypointKind::Run,
                    is_async: true,
                    params: vec![],
                    return_type: TypeRef::Unit,
                    error_type: Some("CoreError".into()),
                    doc: String::new(),
                },
                EntrypointDef {
                    method: "finish".into(),
                    kind: EntrypointKind::Run,
                    is_async: true,
                    params: vec![],
                    return_type: TypeRef::Unit,
                    error_type: None,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
            cfg: None,
        }],
        ..Default::default()
    };
    let mut source = Pyo3Backend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    source.push_str(TEST_PREFIX);
    if mode == "managed" {
        source.push_str("            crate::alef_async_runtime::shutdown(py).unwrap();\n");
    }
    source.push_str(TEST_SUFFIX);
    std::fs::write(binding.join("src/lib.rs"), source).unwrap();
    let service = Pyo3Backend
        .generate_service_api(&api, &config)
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("service.rs"))
        .unwrap()
        .content;
    std::fs::write(binding.join("src/service.rs"), service).unwrap();
    let output = std::process::Command::new("cargo")
        .args(["test", "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", target)
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    let diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() && diagnostics.contains("1 passed"),
        "{mode}: {diagnostics}"
    );
}
