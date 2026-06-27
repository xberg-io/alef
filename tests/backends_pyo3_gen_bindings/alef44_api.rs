use super::*;

// ---------------------------------------------------------------------------
// Tests for alef#44 fixes
// ---------------------------------------------------------------------------

/// Item 1 — `#[serde(skip)]` must be emitted for sanitized fields.
///
/// A field like `cancel_token: String` (sanitized from `CancellationToken`) must carry
/// `#[serde(skip)]` in the generated Rust binding struct so that JSON round-trips do not
/// include the field and cause "unknown field 'cancel_token'" errors at runtime.
#[test]
fn test_sanitized_field_gets_serde_skip() {
    let backend = Pyo3Backend;

    let mut cancel_field = make_field("cancel_token", TypeRef::String, true);
    cancel_field.sanitized = true;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false),
                cancel_field,
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_file = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    let content = &lib_file.content;

    // The sanitized field must be tagged with #[serde(skip)] so JSON round-trips skip it.
    assert!(
        content.contains("serde(skip)"),
        "sanitized cancel_token field must get #[serde(skip)];\ncontent:\n{}",
        content
    );
}

/// Item 2 — Non-`Option` enum fields must not fall back to `String::default()` (`""`).
///
/// When a struct field's type is sanitized to `String` (e.g. `result_format: OutputFormat`
/// where `OutputFormat` was unknown to the extractor), the generated binding stores it as
/// `result_format: String`. Serde deserialization of `{"result_format": ""}` (the
/// `String::default()`) would fail with "unknown variant ''". The `#[serde(skip)]` fix
/// ensures the field is excluded from JSON, so its Rust `Default::default()` (`""`) is
/// used silently — avoiding the failure. This test verifies that a sanitized `String`
/// field in a has_default struct gets `#[serde(skip)]`.
#[test]
fn test_sanitized_enum_like_field_gets_serde_skip() {
    let backend = Pyo3Backend;

    // Simulate OutputFormat sanitized to String (extractor could not resolve the enum type)
    let mut format_field = make_field("result_format", TypeRef::String, false);
    format_field.sanitized = true;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false),
                format_field,
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_file = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib_file.content.contains("serde(skip)"),
        "sanitized result_format field must get #[serde(skip)] to avoid unknown-variant errors;\ncontent:\n{}",
        lib_file.content
    );
}

/// Item 3 — `api.py` wrapper must forward arguments by keyword, not positional.
///
/// The pyo3 signature order (required first, optional second) may differ from the
/// Python wrapper function signature. Forwarding by keyword ensures slot alignment
/// regardless of declaration order.
#[test]
fn test_api_py_uses_keyword_arguments() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
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
                    name: "config".to_string(),
                    ty: TypeRef::String,
                    optional: true,
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
                    name: "mime_type".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
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
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Extract file.".to_string(),
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
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
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
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();

    // The call to _rust.extract_file must use keyword arguments.
    assert!(
        api_py.content.contains("path=path"),
        "api.py must forward path by keyword;\ncontent:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("mime_type=mime_type"),
        "api.py must forward mime_type by keyword;\ncontent:\n{}",
        api_py.content
    );
    // Must NOT use raw positional call like `_rust.extract_file(path, mime_type, config)`
    assert!(
        !api_py.content.contains("_rust.extract_file(path, "),
        "api.py must not use positional arguments for extract_file;\ncontent:\n{}",
        api_py.content
    );
}

/// Item 4 — Async pyo3 functions must produce `async def` + `await` wrappers in `api.py`.
///
/// Pyo3 async functions return coroutines. The Python wrapper must be `async def` and
/// must `await` the native call so callers can use `await extract_file(...)`.
#[test]
fn test_async_function_emits_async_def_and_await() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_bytes".to_string(),
            rust_path: "test_lib::extract_bytes".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "data".to_string(),
                ty: TypeRef::Bytes,
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
            return_type: TypeRef::String,
            is_async: true,
            error_type: None,
            doc: "Extract bytes.".to_string(),
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
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
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
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();

    assert!(
        api_py.content.contains("async def extract_bytes"),
        "api.py async function must use 'async def';\ncontent:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("await _rust.extract_bytes"),
        "api.py async function must await the native call;\ncontent:\n{}",
        api_py.content
    );
    // Must NOT be a plain sync def
    assert!(
        !api_py.content.contains("\ndef extract_bytes"),
        "api.py async function must NOT use plain 'def';\ncontent:\n{}",
        api_py.content
    );
}

/// Item 5 — Trait-bridge `register_*` helpers must appear in `api.py` and `__init__.py` `__all__`.
///
/// `register_embedding_backend` and `register_text_backend` are emitted as `#[pyfunction]`
/// by trait_bridge codegen and added to the pyo3 module, but they are not in `api.functions`.
/// They must be re-exported through `api.py` and listed in `__all__` so callers can use
/// `sample_crate.register_text_backend(...)` instead of `sample_crate._sample_crate.register_text_backend(...)`.
#[test]
fn test_trait_bridge_register_fns_in_api_py_and_all() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "path".to_string(),
                ty: TypeRef::Path,
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
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Extract.".to_string(),
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
        module_name: Some("_test_lib".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: Some(StubsConfig {
            output: std::path::PathBuf::from("packages/python/src/"),
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
    // Configure two trait bridges with register_fn
    config.trait_bridges = vec![
        TraitBridgeConfig {
            trait_name: "TextBackend".to_string(),
            super_trait: None,
            registry_getter: Some("test_lib::get_ocr_registry".to_string()),
            register_fn: Some("register_text_backend".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        },
        TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: None,
            registry_getter: Some("test_lib::get_embedding_registry".to_string()),
            register_fn: Some("register_embedding_backend".to_string()),

            unregister_fn: None,

            clear_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            bind_via: alef::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        },
    ];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();
    let init_py = files.iter().find(|f| f.path.ends_with("__init__.py")).unwrap();

    // api.py must contain pass-through wrappers for both register_* functions
    assert!(
        api_py.content.contains("def register_text_backend"),
        "api.py must contain register_text_backend wrapper;\ncontent:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("def register_embedding_backend"),
        "api.py must contain register_embedding_backend wrapper;\ncontent:\n{}",
        api_py.content
    );

    // __init__.py must re-export them from .api
    assert!(
        init_py.content.contains("register_text_backend"),
        "__init__.py must import register_text_backend from .api;\ncontent:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("register_embedding_backend"),
        "__init__.py must import register_embedding_backend from .api;\ncontent:\n{}",
        init_py.content
    );

    // Both must appear in __all__
    assert!(
        init_py.content.contains("\"register_text_backend\""),
        "__init__.py __all__ must include register_text_backend;\ncontent:\n{}",
        init_py.content
    );
    assert!(
        init_py.content.contains("\"register_embedding_backend\""),
        "__init__.py __all__ must include register_embedding_backend;\ncontent:\n{}",
        init_py.content
    );
}

#[test]
fn test_options_py_imports_data_enums_as_native_classes() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StructureItem".to_string(),
            rust_path: "test_lib::StructureItem".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("kind", TypeRef::Named("StructureKind".to_string()), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A structural item.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "StructureKind".to_string(),
            rust_path: "test_lib::StructureKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Function".to_string(),
                fields: vec![make_field("name", TypeRef::String, false)],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: "The kind of structural item.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend.generate_public_api(&api, &config).expect("generate public API");
    let options_py = files.iter().find(|f| f.path.ends_with("options.py")).unwrap();

    // Data enums are imported from the native module as their class (the same class users
    // construct), referenced by name in field annotations — not redefined as a flattened union
    // alias that would shadow the public class and reject the documented usage in a type checker.
    assert!(
        options_py
            .content
            .contains("from ._test_lib import (\n    StructureKind,"),
        "data enum class must be imported from the native module;\ncontent:\n{}",
        options_py.content
    );
    assert!(
        !options_py.content.contains("StructureKind = "),
        "the flattened data-enum union alias must no longer be emitted;\ncontent:\n{}",
        options_py.content
    );
    // `StructureKind` here has only a payload-carrying variant (no unit/tag-only variant), so the
    // field is typed as the class alone — no `| str` widening.
    assert!(
        options_py.content.contains("kind: StructureKind | None"),
        "config field must be typed as the data-enum class;\ncontent:\n{}",
        options_py.content
    );
}
