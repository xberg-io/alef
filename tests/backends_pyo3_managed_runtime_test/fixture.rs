use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

const RUNTIME_TEST: &str = r#"
#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[pyfunction]
    fn context_echo<'py>(py: Python<'py>, callback: Py<PyAny>) -> PyResult<Bound<'py, PyAny>> {
        crate::alef_async_runtime::future_into_py(py, async move {
            let future = Python::attach(|py| {
                crate::alef_async_runtime::into_future(callback.call0(py)?.into_bound(py))
            })?;
            future.await
        })
    }
    #[test]
    fn runtime_stops_restarts_and_rejects_active_shutdown() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_sample").unwrap();
            _sample(&module).unwrap();
            module.add_function(wrap_pyfunction!(context_echo, &module).unwrap()).unwrap();
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("native", module).unwrap();
            let script = std::ffi::CString::new(include_str!("runtime.py")).unwrap();
            py.run(&script, Some(&globals), None).unwrap();
            let close = std::ffi::CString::new("import atexit; atexit._run_exitfuncs()").unwrap();
            py.run(&close, Some(&globals), None).unwrap();
            py.detach(|| {
                let outer = tokio::runtime::Builder::new_current_thread().build().unwrap();
                outer.block_on(async {
                    let handle = crate::alef_async_runtime::get_runtime();
                    assert!(handle.spawn(async {}).await.unwrap_err().is_cancelled());
                });
            });
        });
    }
}
"#;

pub fn run() {
    let temp = tempfile::tempdir().expect("fixture directory");
    let core = temp.path().join("sample-core");
    let binding = temp.path().join("sample-py");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(core.join("Cargo.toml"), "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ntokio={version='1',features=['time']}\n").unwrap();
    std::fs::write(core.join("src/lib.rs"), "pub async fn fetch() -> String { tokio::time::sleep(std::time::Duration::from_millis(200)).await; \"done\".into() }\n").unwrap();
    std::fs::write(binding.join("Cargo.toml"), "[package]\nname='sample-py'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nsample-core={path='../sample-core'}\nserde={version='1',features=['derive']}\nserde_json='1'\npyo3={version='0.29',features=['auto-initialize']}\npyo3-async-runtimes={version='0.29',features=['tokio-runtime']}\ntokio={version='1',features=['full']}\n").unwrap();
    let parsed: NewAlefConfig = toml::from_str("[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n[crates.python]\nmodule_name='_sample'\nasync_runtime='managed'\n").unwrap();
    let config = parsed.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "sample-core".into(),
        functions: vec![FunctionDef {
            name: "fetch".into(),
            rust_path: "sample_core::fetch".into(),
            is_async: true,
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let mut source = files
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    source = source.replace("    fn get_runtime()", "    pub(crate) fn get_runtime()");
    source.push_str(RUNTIME_TEST);
    std::fs::write(binding.join("src/lib.rs"), source).unwrap();
    std::fs::write(binding.join("src/runtime.py"), include_str!("runtime.py")).unwrap();
    let output = std::process::Command::new("cargo")
        .args(["test", "--quiet", "--manifest-path"])
        .arg(binding.join("Cargo.toml"))
        .current_dir(&binding)
        .env("CARGO_TARGET_DIR", temp.path().join("target"))
        .env("RUSTC_WRAPPER", "")
        .output()
        .unwrap();
    let diagnostics = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{diagnostics}");
    assert!(diagnostics.contains("1 passed"), "{diagnostics}");
}
