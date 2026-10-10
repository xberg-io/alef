use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

const TEST_PREFIX: &str = r#"
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_core_errors_work_without_from_pyerr() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_sample").unwrap();
            _sample(&module).unwrap();
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("native", module).unwrap();
            let code = std::ffi::CString::new("class Worker:\n def process(self): return 'done'\nclass Failed(Worker):\n def process(self): raise ValueError('callback failed')\n").unwrap();
            py.run(&code, Some(&globals), None).unwrap();
            for (name, expected) in [("Worker", "done"), ("Failed", "callback failed")] {
                let worker = globals.get_item(name).unwrap().unwrap().call0().unwrap().unbind();
                let bridge = PyWorkerBridge::new(worker).unwrap();
                let result = py.detach(move || {
"#;

const TEST_SUFFIX: &str = r#"
                        .block_on(sample_core::Worker::process(&bridge))
                });
                if name == "Worker" {
                    assert_eq!(result.unwrap(), expected);
                } else {
                    assert!(result.unwrap_err().to_string().contains(expected));
                }
            }
"#;

const TEST_END: &str = r#"
        });
    }
}
"#;

pub fn run() {
    let temp = tempfile::tempdir().unwrap();
    let core = temp.path().join("sample-core");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(
        core.join("Cargo.toml"),
        "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nasync-trait='0.1'\n",
    )
    .unwrap();
    std::fs::write(core.join("src/lib.rs"), include_str!("trait_core.rs")).unwrap();
    for mode in ["managed", "default"] {
        verify_mode(temp.path(), mode);
    }
}

fn verify_mode(directory: &std::path::Path, mode: &str) {
    let binding = directory.join(mode);
    std::fs::create_dir_all(binding.join("src")).unwrap();
    std::fs::write(binding.join("Cargo.toml"), "[package]\nname='sample-py'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nsample-core={path='../sample-core'}\nasync-trait='0.1'\nserde={version='1',features=['derive']}\nserde_json='1'\npyo3={version='0.29',features=['auto-initialize']}\npyo3-async-runtimes={version='0.29',features=['tokio-runtime']}\ntokio={version='1',features=['full']}\n").unwrap();
    let option = if mode == "managed" {
        "async_runtime='managed'\n"
    } else {
        ""
    };
    let config: NewAlefConfig = toml::from_str(&format!("[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\ncore_import='sample_core'\nerror_type='CoreError'\nerror_constructor='sample_core::CoreError::new({{msg}})'\n[crates.python]\nmodule_name='_sample'\n{option}[[crates.trait_bridges]]\ntrait_name='Worker'\ntype_alias='Worker'\nparam_name='worker'\n")).unwrap();
    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Worker".into(),
            rust_path: "sample_core::Worker".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "process".into(),
                receiver: Some(ReceiverKind::Ref),
                is_async: true,
                return_type: TypeRef::String,
                error_type: Some("CoreError".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut source = Pyo3Backend
        .generate_bindings(&api, &config.resolve().unwrap().remove(0))
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    source.push_str(TEST_PREFIX);
    if mode == "managed" {
        source.push_str("                    crate::alef_async_runtime::get_runtime_checked().unwrap()\n");
    } else {
        source.push_str("                    pyo3_async_runtimes::tokio::get_runtime()\n");
    }
    source.push_str(TEST_SUFFIX);
    if mode == "managed" {
        source.push_str("            crate::alef_async_runtime::shutdown(py).unwrap();\n");
    }
    source.push_str(TEST_END);
    std::fs::write(binding.join("src/lib.rs"), source).unwrap();
    let target = std::env::var_os("ALEF_LIFECYCLE_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.join("target"));
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
        "{diagnostics}"
    );
}
