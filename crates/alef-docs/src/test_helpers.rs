use alef_core::ir::{ApiSurface, CoreWrapper, DefaultValue, FieldDef, FunctionDef, MethodDef, TypeRef};

pub(crate) const TEST_PREFIX: &str = "Htm";

pub(crate) fn make_param(name: &str, ty: TypeRef, optional: bool) -> alef_core::ir::ParamDef {
    alef_core::ir::ParamDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
    }
}

pub(crate) fn make_method(
    name: &str,
    params: Vec<alef_core::ir::ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    is_static: bool,
    error_type: Option<&str>,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static,
        error_type: error_type.map(str::to_string),
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }
}

pub(crate) fn make_function(
    name: &str,
    params: Vec<alef_core::ir::ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    error_type: Option<&str>,
) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async,
        error_type: error_type.map(str::to_string),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

pub(crate) fn make_field(name: &str, ty: TypeRef, optional: bool, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
    }
}

pub(crate) fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}

pub(crate) fn make_test_config() -> alef_core::config::AlefConfig {
    use alef_core::config::*;
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "mylib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![Language::Python],
        exclude: ExcludeConfig::default(),
        include: IncludeConfig::default(),
        output: OutputConfig::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        go: None,
        java: None,
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: CustomModulesConfig::default(),
        custom_registrations: CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),}
}

pub(crate) fn make_minimal_api(version: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "mylib".to_string(),
        version: version.to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}
