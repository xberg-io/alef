use super::*;

/// Regression test for sample_crate-dev/alef#1 / sample_crate-dev/sample_crate#310.
///
/// A type with both `has_default = true` AND `is_return_type = true` (e.g. `ParseOutput`)
/// must be re-exported in `__init__.py` from the native Rust module, NOT from `options.py`.
/// `options.py` must NOT emit a `@dataclass` shadow class for such types; the authoritative
/// definition lives in the native module as a `#[pyclass]` struct. The shadow class caused
/// static analysis tools (Pylance) to report a type mismatch because the two classes are
/// unrelated even though they share a name.
#[test]
fn test_return_type_exported_from_native_module_not_options() {
    let backend = Pyo3Backend;

    // ParseOutput: has_default=true (implements Default), is_return_type=true (returned by convert())
    // ParseOptions: has_default=true, is_return_type=false (input/config type)
    let conversion_result = TypeDef {
        name: "ParseOutput".to_string(),
        rust_path: "my_lib::ParseOutput".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("content", TypeRef::String, false),
            make_field("title", TypeRef::Optional(Box::new(TypeRef::String)), true),
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Result of a conversion operation.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let conversion_options = TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool), false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Options for conversion.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![conversion_result, conversion_options],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("ParseOutput".to_string()),
            is_async: false,
            error_type: None,
            doc: "Convert input to markdown.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_my_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/my_lib"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        sdist_include: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");
    let options_py = files
        .iter()
        .find(|f| f.path.ends_with("options.py"))
        .expect("options.py not generated");

    // ParseOutput (return type) must be imported from the native module.
    let native_import_line = init_py
        .content
        .lines()
        .find(|l| l.contains("from ._my_lib import"))
        .unwrap_or("");
    assert!(
        native_import_line.contains("ParseOutput"),
        "__init__.py must import ParseOutput from the native module, got:\n{}",
        init_py.content
    );

    // ParseOutput must NOT appear in the .options import.
    let options_import_line = init_py
        .content
        .lines()
        .find(|l| l.contains("from .options import"))
        .unwrap_or("");
    assert!(
        !options_import_line.contains("ParseOutput"),
        "__init__.py must not import ParseOutput from .options, got:\n{}",
        init_py.content
    );

    // ParseOptions (config/input type) must still be imported from .options.
    assert!(
        options_import_line.contains("ParseOptions"),
        "__init__.py must import ParseOptions from .options, got:\n{}",
        init_py.content
    );

    // Both names must appear in __all__.
    assert!(
        init_py.content.contains("\"ParseOutput\""),
        "__init__.py __all__ must include ParseOutput, got:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("\"ParseOptions\""),
        "__init__.py __all__ must include ParseOptions, got:\n{}",
        init_py.content
    );

    // options.py must NOT define a @dataclass shadow for ParseOutput.
    assert!(
        !options_py.content.contains("class ParseOutput"),
        "options.py must not define a ParseOutput shadow class, got:\n{}",
        options_py.content
    );

    // options.py MUST still define ParseOptions (the input/config type).
    assert!(
        options_py.content.contains("class ParseOptions"),
        "options.py must still define ParseOptions dataclass, got:\n{}",
        options_py.content
    );
}

#[test]
fn test_api_py_imports_config_dto_with_self_returning_method_from_options() {
    // Regression: alef#72. A has_default config DTO that exposes a builder method
    // returning `Self` (e.g. `PackConfig::from_toml_file -> PackConfig`) must still
    // be imported from `.options` in api.py, not from `._native`. The pre-fix code
    // walked method return types into `return_type_names`, which incorrectly pulled
    // self-builders out of the options classification.
    let backend = Pyo3Backend;

    // ParseOutput: return type of free function `convert` — stays on ._native.
    let conversion_result = TypeDef {
        name: "ParseOutput".to_string(),
        rust_path: "my_lib::ParseOutput".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("content", TypeRef::String, false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    // ParseOptions: input/config DTO with `Self`-returning builder methods.
    // This is the regression: before the fix, the method returns caused this type
    // to be excluded from options_type_names.
    let with_verbose = MethodDef {
        name: "with_verbose".to_string(),
        params: vec![make_param_def(
            "verbose",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
        )],
        return_type: TypeRef::Named("ParseOptions".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Owned),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let default_method = MethodDef {
        name: "default".to_string(),
        params: vec![],
        return_type: TypeRef::Named("ParseOptions".to_string()),
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let conversion_options = TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("verbose", TypeRef::Primitive(PrimitiveType::Bool), false)],
        methods: vec![with_verbose, default_method],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![conversion_result, conversion_options],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "my_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "input".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ParseOptions".to_string()),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                },
            ],
            return_type: TypeRef::Named("ParseOutput".to_string()),
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let mut config = make_config();
    config.python = Some(PythonConfig {
        module_name: Some("_my_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/my_lib"),
            emit_docstrings: false,
        }),
        features: None,
        serde_rename_all: None,
        capsule_types: Default::default(),
        release_gil: false,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        extra_dependencies: Default::default(),
        pip_dependencies: Vec::new(),
        sdist_include: Vec::new(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");

    let native_import_line = api_py
        .content
        .lines()
        .find(|l| l.contains("from ._my_lib import"))
        .unwrap_or("");
    let options_import_line = api_py
        .content
        .lines()
        .find(|l| l.contains("from .options import"))
        .unwrap_or("");

    // ParseOptions has Self-returning methods, so the pre-fix code put it in
    // return_type_names and excluded it from options_type_names. Verify the fix.
    assert!(
        options_import_line.contains("ParseOptions"),
        "api.py must import ParseOptions from .options, got native={:?} options={:?}\n\nFull api.py:\n{}",
        native_import_line,
        options_import_line,
        api_py.content
    );
    assert!(
        !native_import_line.contains("ParseOptions"),
        "api.py must NOT import ParseOptions from ._my_lib, got native={:?}\n\nFull api.py:\n{}",
        native_import_line,
        api_py.content
    );

    // Regression boundary: ParseOutput IS a free-function return type, so it
    // must continue to come from the native module.
    assert!(
        native_import_line.contains("ParseOutput"),
        "api.py must import ParseOutput from ._my_lib, got native={:?}\n\nFull api.py:\n{}",
        native_import_line,
        api_py.content
    );
    assert!(
        !options_import_line.contains("ParseOutput"),
        "api.py must NOT import ParseOutput from .options, got options={:?}\n\nFull api.py:\n{}",
        options_import_line,
        api_py.content
    );
}
