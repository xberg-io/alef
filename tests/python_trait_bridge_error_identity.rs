use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::template_versions::cargo::{PYO3, PYO3_ASYNC_RUNTIMES};
use std::path::Path;
use std::process::Command;

#[path = "support/python_native.rs"]
mod python_native;

const PROBE: &str = "trait_bridge_runtime::should_rethrow_the_original_callback_exception";

#[test]
fn should_preserve_registered_callback_exceptions_across_rust_error_wrappers() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    write_fixture(root);
    let mut command = Command::new("cargo");
    python_native::configure_python(&mut command);
    let output = command
        .args(["test", "--lib", "--", "--exact", PROBE])
        .current_dir(root.join("binding"))
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(&format!("test {PROBE} ... ok")),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_fixture(root: &Path) {
    write(
        root,
        "sample/Cargo.toml",
        "[package]\nname='sample'\nversion='0.1.0'\nedition='2024'\n\
         [dependencies]\nasync-trait='0.1'\nparking_lot='0.12'\nthiserror='2'\n",
    );
    write(
        root,
        "sample/src/lib.rs",
        include_str!("fixtures/python_trait_bridge_errors/core.rs"),
    );
    write(
        root,
        "binding/Cargo.toml",
        &format!(
            "[package]\nname='python-trait-bridge-error-probe'\nversion='0.1.0'\nedition='2024'\n\
             [dependencies]\nsample={{path='../sample'}}\nasync-trait='0.1'\n\
             pyo3={{version='{PYO3}',features=['auto-initialize']}}\n\
             pyo3-async-runtimes={{version='{PYO3_ASYNC_RUNTIMES}',features=['tokio-runtime']}}\n\
             serde={{version='1',features=['derive']}}\nserde_json='1'\ntokio={{version='1',features=['full']}}\n",
        ),
    );

    let source = root.join("sample/src/lib.rs");
    let mut api = alef::extract::extractor::extract(&[&source], "sample", "0.1.0", None).unwrap();
    api.types.retain(|ty| ty.name == "Worker");
    api.functions.retain(|function| {
        matches!(
            function.name.as_str(),
            "invoke_worker" | "invoke_worker_async" | "reject_natively"
        )
    });
    assert_eq!(api.functions.len(), 3);
    let config = toml::from_str::<NewAlefConfig>(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample'\nsources=[]\n\
         core_import='sample'\nerror_type='WorkerError'\n\
         error_constructor='sample::WorkerError::Callback({msg}, {source})'\n\
         [crates.python]\nmodule_name='_sample'\n\
         [[crates.trait_bridges]]\ntrait_name='Worker'\n\
         register_fn='register_worker'\nregistry_getter='sample::workers'\n\
         unregister_fn='unregister_worker'\nclear_fn='clear_workers'\n",
    )
    .unwrap()
    .resolve()
    .unwrap()
    .remove(0);

    let bindings = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let lib = bindings.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    write(
        root,
        "binding/src/lib.rs",
        &format!(
            "{}\n{}",
            lib.content,
            include_str!("fixtures/python_trait_bridge_errors/probe.rs")
        ),
    );
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
