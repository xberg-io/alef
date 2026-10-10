use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, TypeRef};

#[cfg(unix)]
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

#[test]
fn default_async_runtime_closes_before_python_finalization() {
    let config = toml::from_str::<NewAlefConfig>(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n",
    )
    .unwrap()
    .resolve()
    .unwrap()
    .remove(0);
    let api = ApiSurface {
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
    let native = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    assert!(
        native.content.contains("register_exit_hook(m.py())?"),
        "default async bindings must own and close their executor before finalization"
    );
    assert!(
        !native.content.contains("wrap_pyfunction!(shutdown_async_runtime, m)"),
        "manual shutdown remains opt-in"
    );
}

#[cfg(unix)]
#[path = "backends_pyo3_managed_runtime_test/lifecycle.rs"]
mod lifecycle;

#[test]
#[cfg(unix)]
fn generated_async_runtime_closes_at_exit_and_rejects_inherited_executors() {
    lifecycle::run();
}

#[test]
fn sanitized_async_functions_do_not_register_an_absent_executor() {
    let config =
        toml::from_str::<NewAlefConfig>("[workspace]\nlanguages=['python']\n[[crates]]\nname='sample'\nsources=[]\n")
            .unwrap()
            .resolve()
            .unwrap()
            .remove(0);
    let api = ApiSurface {
        functions: vec![FunctionDef {
            name: "skip".into(),
            is_async: true,
            sanitized: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let source = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;
    assert!(!source.contains("alef_async_runtime"), "{source}");
}

#[test]
fn async_service_entrypoints_emit_the_owned_executor() {
    use alef::core::ir::{EntrypointDef, EntrypointKind, ServiceDef};
    let config =
        toml::from_str::<NewAlefConfig>("[workspace]\nlanguages=['python']\n[[crates]]\nname='sample'\nsources=[]\n")
            .unwrap()
            .resolve()
            .unwrap()
            .remove(0);
    let api = ApiSurface {
        services: vec![ServiceDef {
            name: "App".into(),
            rust_path: "sample::App".into(),
            constructor: Default::default(),
            configurators: vec![],
            registrations: vec![],
            doc: String::new(),
            cfg: None,
            entrypoints: vec![EntrypointDef {
                method: "run".into(),
                kind: EntrypointKind::Run,
                is_async: true,
                params: vec![],
                return_type: TypeRef::Unit,
                error_type: Some("Error".into()),
                doc: String::new(),
            }],
        }],
        ..Default::default()
    };
    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let source = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;
    assert!(
        source.contains("mod alef_async_runtime"),
        "async services require owned runtime support"
    );
}

#[path = "backends_pyo3_managed_runtime_test/trait_fixture.rs"]
mod trait_fixture;

#[test]
fn generated_async_trait_callbacks_keep_configured_core_error_conversions() {
    trait_fixture::run();
}

#[path = "backends_pyo3_managed_runtime_test/service_fixture.rs"]
mod service_fixture;

#[test]
fn generated_async_services_preserve_core_error_boundaries() {
    service_fixture::run();
}
