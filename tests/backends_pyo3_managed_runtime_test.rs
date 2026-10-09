use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

#[path = "backends_pyo3_managed_runtime_test/fixture.rs"]
mod managed_runtime_fixture;

#[test]
fn managed_python_runtime_can_stop_and_restart_after_async_work() {
    let source = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-core"
sources = []
[crates.python]
module_name = "_sample"
async_runtime = "managed"
[crates.python.stubs]
output = "python/sample"
"#;
    let config = toml::from_str::<NewAlefConfig>(source)
        .expect("config")
        .resolve()
        .expect("resolved")
        .remove(0);
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
    let files = Pyo3Backend.generate_bindings(&api, &config).expect("bindings");
    let native = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("native source");
    assert!(
        native.content.contains("mod alef_async_runtime"),
        "managed executor is emitted"
    );
    assert!(
        native.content.contains("wrap_pyfunction!(shutdown_async_runtime"),
        "shutdown is registered"
    );
    assert!(
        native.content.contains("alef_async_runtime::future_into_py"),
        "async calls use managed executor"
    );
    assert!(
        !native.content.contains("pyo3_async_runtimes::tokio::get_runtime()"),
        "global executor is never started"
    );
    let public = Pyo3Backend.generate_public_api(&api, &config).expect("public API");
    let init = public.iter().find(|f| f.path.ends_with("__init__.py")).expect("init");
    assert!(init.content.contains("from ._sample import shutdown_async_runtime"));
}

#[test]
#[cfg(unix)]
fn generated_managed_runtime_reclaims_resources_and_preserves_cancellation() {
    managed_runtime_fixture::run();
}
