use super::*;

/// Adapters (streaming method wrappers):
/// - api.py emits module-level wrapper functions for each adapter
/// - __init__.py imports and re-exports them in __all__
#[test]
fn test_adapter_wrapper_functions() {
    use alef::core::config::{AdapterParam, AdapterPattern};

    let backend = Pyo3Backend;

    // Create a minimal API with a handle type and a function that returns an iterator.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Handle".to_string(),
                rust_path: "test_lib::Handle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Handle type".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "StreamEvent".to_string(),
                rust_path: "test_lib::StreamEvent".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Stream event type".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let mut config = make_config();
    // Add one adapter
    config.adapters = vec![alef::core::config::AdapterConfig {
        name: "test_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "test_stream".to_string(),
        owner_type: Some("Handle".to_string()),
        item_type: Some("StreamEvent".to_string()),
        error_type: None,
        returns: None,
        request_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        params: vec![AdapterParam {
            name: "url".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        skip_languages: vec![],
    }];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    // Check api.py contains the wrapper function
    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");

    // Rust `String` adapter param type must be mapped to Python `str`.
    assert!(
        api_py
            .content
            .contains("async def test_stream(engine: Handle, url: str) -> AsyncIterator[StreamEvent]:"),
        "api.py must map String param to str in streaming wrapper signature; content:\n{}",
        api_py.content
    );

    assert!(
        api_py.content.contains("async for item in engine.test_stream(url):"),
        "api.py must contain async for loop delegating to engine method; content:\n{}",
        api_py.content
    );

    assert!(
        api_py.content.contains("yield item"),
        "api.py must contain yield statement in adapter wrapper; content:\n{}",
        api_py.content
    );

    // Check __init__.py imports and exports the adapter
    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");

    assert!(
        init_py.content.contains("test_stream"),
        "__init__.py must import and export the adapter wrapper; content:\n{}",
        init_py.content
    );

    assert!(
        init_py.content.contains("\"test_stream\"") || init_py.content.contains("'test_stream'"),
        "__init__.py must list test_stream in __all__; content:\n{}",
        init_py.content
    );
}

/// Adapter async_method wrappers:
/// - emit `return await engine.foo(...)` (not `async for item in engine.foo(): yield item`)
/// - return the type from `adapter.returns`
/// - map Rust `String` param type to Python `str`
/// - do NOT add AsyncIterator to the typing imports
#[test]
fn test_async_method_adapter_wrapper() {
    use alef::core::config::{AdapterParam, AdapterPattern};

    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handle".to_string(),
            rust_path: "test_lib::Handle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Handle type".to_string(),
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
        ..Default::default()
    };

    let mut config = make_config();
    config.adapters = vec![alef::core::config::AdapterConfig {
        name: "fetch_data".to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: "fetch_data".to_string(),
        owner_type: Some("Handle".to_string()),
        item_type: None,
        returns: Some("DataResult".to_string()),
        error_type: None,
        request_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        params: vec![AdapterParam {
            name: "key".to_string(),
            ty: "String".to_string(),
            optional: false,
        }],
        skip_languages: vec![],
    }];

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");

    // Must use return-await form, not async-for-yield.
    assert!(
        api_py
            .content
            .contains("async def fetch_data(engine: Handle, key: str) -> DataResult:"),
        "api.py must emit return-await signature for async_method adapter; content:\n{}",
        api_py.content
    );
    assert!(
        api_py.content.contains("return await engine.fetch_data(key)"),
        "api.py must emit `return await engine.fetch_data(key)` for async_method adapter; content:\n{}",
        api_py.content
    );
    assert!(
        !api_py.content.contains("async for item in engine.fetch_data"),
        "api.py must NOT emit async-for loop for async_method adapter; content:\n{}",
        api_py.content
    );
    assert!(
        !api_py.content.contains("AsyncIterator"),
        "api.py must NOT import AsyncIterator when there are no streaming adapters; content:\n{}",
        api_py.content
    );
}

#[test]
fn test_serde_rename_in_constructor_and_properties() {
    let backend = Pyo3Backend;

    // Create a struct with a field that has serde_rename
    let mut field_with_rename = make_field("max_characters", TypeRef::Primitive(PrimitiveType::Usize), true);
    field_with_rename.serde_rename = Some("max_chars".to_string());
    field_with_rename.typed_default = Some(alef::core::ir::DefaultValue::IntLiteral(1000));

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ChunkingConfig".to_string(),
            rust_path: "test_lib::ChunkingConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![field_with_rename, {
                let mut f = make_field("overlap", TypeRef::Primitive(PrimitiveType::Usize), true);
                f.typed_default = Some(alef::core::ir::DefaultValue::IntLiteral(200));
                f
            }],
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
            doc: "Chunking configuration with serde renames".to_string(),
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
        ..Default::default()
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    // Find the generated lib.rs
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // The PyO3 signature should use max_chars (the serde_rename name)
    assert!(
        lib_rs.content.contains("max_chars=None"),
        "PyO3 signature should use serde_rename 'max_chars=None'; content:\n{}",
        lib_rs.content
    );

    // The constructor parameter should be max_chars
    assert!(
        lib_rs.content.contains("pub fn new(max_chars:"),
        "Constructor parameter should use serde_rename 'max_chars'; content:\n{}",
        lib_rs.content
    );

    // The struct literal should use max_characters (bare Rust field name)
    assert!(
        lib_rs.content.contains("Self { max_characters: max_chars"),
        "Struct literal should use bare field name 'max_characters'; content:\n{}",
        lib_rs.content
    );

    // The serde rename attribute should be present on the field
    assert!(
        lib_rs.content.contains("#[serde(rename = \"max_chars\")]"),
        "Field should have serde(rename = \"max_chars\"); content:\n{}",
        lib_rs.content
    );
}

#[test]
fn test_cfg_gated_fields_excluded_from_constructor() {
    let backend = Pyo3Backend;

    // Create fields: one cfg-gated by a predicate `cfg_present_for_pyo3` cannot prove
    // (a non-feature, non-wasm gate — here a contrived `any(unix, windows)` form),
    // and one ungated. Feature gates (`feature = "pdf"`) are now treated as present
    // because the pyo3 Cargo.toml controls which features compile in; only predicates
    // that may genuinely strip the field at build time are excluded.
    let mut cfg_field = make_field("pdf_options", TypeRef::String, true);
    cfg_field.cfg = Some("any(unix, windows)".to_string());
    cfg_field.typed_default = Some(alef::core::ir::DefaultValue::None);

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                {
                    let mut f = make_field("use_cache", TypeRef::Primitive(PrimitiveType::Bool), false);
                    f.typed_default = Some(alef::core::ir::DefaultValue::BoolLiteral(true));
                    f
                },
                cfg_field,
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: true,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: "Config with cfg-gated field".to_string(),
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
        ..Default::default()
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    // Find the generated lib.rs
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // The constructor should NOT have pdf_options as a parameter (it's cfg-gated)
    assert!(
        !lib_rs.content.contains("pub fn new(pdf_options:"),
        "Constructor should NOT have cfg-gated parameter 'pdf_options'; content:\n{}",
        lib_rs.content
    );

    // The constructor should have use_cache as a parameter (not cfg-gated)
    assert!(
        lib_rs.content.contains("#[new]\n    pub fn new(use_cache:"),
        "Constructor should have non-cfg parameter 'use_cache'; content:\n{}",
        lib_rs.content
    );

    // The struct literal should include use_cache (shorthand) and pdf_options (explicitly set to None)
    assert!(
        lib_rs.content.contains("Self { use_cache, pdf_options: None }"),
        "Struct literal should use shorthand for non-cfg field and explicit None for cfg-gated optional field; content:\n{}",
        lib_rs.content
    );

    // The pdf_options field should still be in the struct definition
    // (cfg attributes are typically not preserved by PyO3 codegen, but the field itself should be there)
    assert!(
        lib_rs.content.contains("pub pdf_options:"),
        "Field should still exist in struct definition; content:\n{}",
        lib_rs.content
    );
}

/// Regression test: a struct field with `serde(rename = "type")` must generate compilable Rust.
/// Before this fix alef emitted `pub fn new(type: String, ...)` and `Self { item_type: type }` —
/// both invalid because `type` is a Rust keyword.  The fix escapes all Rust keywords in
/// constructor parameters and struct-literal RHS values using raw-identifier syntax (`r#type`).
/// PyO3 strips the `r#` prefix so the Python-facing kwarg name stays `type`.
#[test]
fn test_serde_rename_rust_keyword_emitted_as_raw_ident() {
    let backend = Pyo3Backend;

    // `item_type` field carries serde(rename = "type") — the wire name is a Rust keyword.
    let mut item_type_field = make_field("item_type", TypeRef::String, false);
    item_type_field.serde_rename = Some("type".to_string());

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ResponseOutputItem".to_string(),
            rust_path: "test_lib::ResponseOutputItem".to_string(),
            original_rust_path: String::new(),
            fields: vec![item_type_field, make_field("content", TypeRef::String, false)],
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
            doc: "A response output item".to_string(),
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
        ..Default::default()
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");

    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs not generated");

    // The constructor parameter must use raw-identifier syntax, not the bare keyword `type`.
    assert!(
        lib_rs.content.contains("pub fn new(r#type:"),
        "Constructor parameter for serde-renamed 'type' field must be 'r#type'; content:\n{}",
        lib_rs.content
    );

    // The PyO3 signature attribute must also use the raw identifier (PyO3 strips `r#` → Python sees `type`).
    assert!(
        lib_rs.content.contains("r#type") && !lib_rs.content.contains("(type,") && !lib_rs.content.contains("(type)"),
        "pyo3 signature must not contain bare 'type' token; content:\n{}",
        lib_rs.content
    );

    // The struct literal must assign via raw identifier: `item_type: r#type`.
    assert!(
        lib_rs.content.contains("item_type: r#type"),
        "Struct literal must use 'item_type: r#type' for the renamed field; content:\n{}",
        lib_rs.content
    );

    // Sanity: the field definition should still carry the serde rename attribute.
    assert!(
        lib_rs.content.contains("#[serde(rename = \"type\")]"),
        "Field must retain #[serde(rename = \"type\")] attribute; content:\n{}",
        lib_rs.content
    );
}
