use alef::core::config::PythonConfig;
use alef::core::config::python_build::{
    PYO3_EXTENSION_MODULE_FEATURE, PYO3_EXTENSION_MODULE_PYO3_FEATURES, extension_module_feature_line,
};

#[test]
fn original_exhaustive_python_config_literal_and_public_abi_helpers_compile_unchanged() {
    let config = PythonConfig {
        dependency_versions: Default::default(),
        send_sync_types: vec![],
        module_name: Some("_sample".to_string()),
        async_runtime: None,
        stubs: None,
        pip_name: None,
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        pip_dependencies: vec![],
        sdist_include: vec![],
        scaffold_output: None,
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: vec![],
        extra_init_imports: Default::default(),
        reexported_types: vec![],
        target_dep_overrides: vec![],
    };
    assert_eq!(config.module_name.as_deref(), Some("_sample"));
    assert_eq!(PYO3_EXTENSION_MODULE_FEATURE, "extension-module");
    assert_eq!(
        PYO3_EXTENSION_MODULE_PYO3_FEATURES,
        &["pyo3/extension-module", "pyo3/abi3-py311"]
    );
    assert_eq!(
        extension_module_feature_line(),
        "extension-module = [\"pyo3/extension-module\", \"pyo3/abi3-py311\"]"
    );
}
