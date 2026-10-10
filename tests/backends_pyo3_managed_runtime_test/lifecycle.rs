use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};
use std::path::Path;
use std::process::Command;

#[path = "stream_fixture.rs"]
mod stream_fixture;

pub fn run() {
    let dir = tempfile::tempdir().unwrap();
    let core = dir.path().join("sample-core");
    std::fs::create_dir_all(core.join("src")).unwrap();
    std::fs::write(core.join("Cargo.toml"), "[package]\nname='sample-core'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ntokio={version='1',features=['time']}\nfutures='0.3'\n").unwrap();
    std::fs::write(core.join("src/lib.rs"), stream_fixture::CORE).unwrap();
    let target = std::env::var_os("ALEF_LIFECYCLE_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.path().join("target"));
    for mode in ["managed", "default"] {
        let binding = dir.path().join(mode);
        std::fs::create_dir_all(binding.join("src")).unwrap();
        std::fs::write(binding.join("Cargo.toml"), "[package]\nname='sample-py'\nversion='0.1.0'\nedition='2024'\n[lib]\nname='_sample'\ncrate-type=['cdylib']\n[dependencies]\nsample-core={path='../sample-core'}\nserde={version='1',features=['derive']}\nserde_json='1'\npyo3={version='0.29',features=['extension-module','abi3-py310']}\npyo3-async-runtimes={version='0.29',features=['tokio-runtime']}\ntokio={version='1',features=['full']}\nfutures='0.3'\n[build-dependencies]\npyo3-build-config='0.29'\n").unwrap();
        std::fs::write(
            binding.join("build.rs"),
            "fn main() { pyo3_build_config::add_extension_module_link_args(); }\n",
        )
        .unwrap();
        let option = if mode == "managed" {
            "async_runtime='managed'\n"
        } else {
            ""
        };
        let config: NewAlefConfig = toml::from_str(&format!("[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n[crates.python]\nmodule_name='_sample'\n{option}{}", stream_fixture::ADAPTER)).unwrap();
        let api = stream_fixture::with_stream(ApiSurface {
            functions: vec![FunctionDef {
                name: "fetch".into(),
                rust_path: "sample_core::fetch".into(),
                is_async: true,
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut source = Pyo3Backend
            .generate_bindings(&api, &config.resolve().unwrap().remove(0))
            .unwrap()
            .into_iter()
            .find(|f| f.path.ends_with("lib.rs"))
            .unwrap()
            .content;
        source = source.replace("    m.add_function(wrap_pyfunction!(init_async_runtime, m)?)?;", "    m.add_function(wrap_pyfunction!(init_async_runtime, m)?)?;\n    m.add_function(wrap_pyfunction!(unread_stream, m)?)?;");
        source.push_str(stream_fixture::WRAPPER);
        std::fs::write(binding.join("src/lib.rs"), source).unwrap();
        let output = Command::new("cargo")
            .args(["build", "--quiet", "--manifest-path"])
            .arg(binding.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", &target)
            .env("RUSTC_WRAPPER", "")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let library = if cfg!(target_os = "macos") {
            "lib_sample.dylib"
        } else {
            "lib_sample.so"
        };
        std::fs::copy(target.join("debug").join(library), binding.join("_sample.abi3.so")).unwrap();
        std::fs::write(binding.join("lifecycle.py"), include_str!("lifecycle.py")).unwrap();
        if let Some(artifacts) = std::env::var_os("ALEF_LIFECYCLE_ARTIFACTS") {
            let destination = Path::new(&artifacts).join(mode);
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::copy(binding.join("_sample.abi3.so"), destination.join("_sample.abi3.so")).unwrap();
            std::fs::copy(binding.join("lifecycle.py"), destination.join("lifecycle.py")).unwrap();
        }
        verify(&binding, mode);
    }
}

fn verify(binding: &Path, mode: &str) {
    let mut failures = Vec::new();
    for scenario in ["exit", "unread", "manual", "fork-cold", "fork-warm"] {
        let output = Command::new("python3")
            .args(["-W", "ignore:This process:DeprecationWarning"])
            .arg(binding.join("lifecycle.py"))
            .arg(scenario)
            .arg(mode)
            .env("PYTHONPATH", binding)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success()
            || !stdout.contains("LIFECYCLE_OK")
            || stdout.contains("LIFECYCLE_FAILED")
            || !output.stderr.is_empty()
        {
            failures.push(format!(
                "{mode}/{scenario}: {stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
