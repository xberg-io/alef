use super::*;

/// capsule_types wires up PyCapsule pass-through end-to-end:
/// - The Language type does NOT get a #[pyclass] wrapper.
/// - get_language returns via PyCapsule_New (capsule round-trip).
/// - get_parser constructs via py.import("sample_language").getattr("Parser").call1.
#[test]
fn test_capsule_types_end_to_end() {
    use alef::core::config::CapsuleTypeConfig;

    let backend = Pyo3Backend;

    // IR: two opaque types that are listed as capsule types + two functions.
    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            // Language — capsule round-trip type
            TypeDef {
                name: "Language".to_string(),
                rust_path: "sample_pack::Language".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "A sample_language Language handle.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Parser — ConstructFrom type (no into_raw; built via sample_language.Parser(language))
            TypeDef {
                name: "Parser".to_string(),
                rust_path: "sample_pack::Parser".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "A sample_language Parser.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![
            // get_language(name: &str) -> Result<Language, Error>
            FunctionDef {
                name: "get_language".to_string(),
                rust_path: "sample_pack::get_language".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "name".to_string(),
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
                return_type: TypeRef::Named("Language".to_string()),
                is_async: false,
                error_type: Some("sample_pack::Error".to_string()),
                doc: "Look up a language by name.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            // get_parser(name: &str) -> Result<Parser, Error>
            FunctionDef {
                name: "get_parser".to_string(),
                rust_path: "sample_pack::get_parser".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "name".to_string(),
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
                return_type: TypeRef::Named("Parser".to_string()),
                is_async: false,
                error_type: Some("sample_pack::Error".to_string()),
                doc: "Get a parser for a language by name.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "sample_pack::Error".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".to_string(),
                message_template: Some("language not found: {0}".to_string()),
                fields: vec![make_field("msg", TypeRef::String, false)],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let mut config = make_config();
    let mut capsule_map: HashMap<String, CapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        CapsuleTypeConfig::Capsule("sample_language.Language".to_string()),
    );
    capsule_map.insert(
        "Parser".to_string(),
        CapsuleTypeConfig::ConstructFrom {
            python_type: "sample_language.Parser".to_string(),
            construct_from: "Language".to_string(),
        },
    );
    config.python = Some(PythonConfig {
        module_name: Some("_sample_pack".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: None,
        features: None,
        serde_rename_all: None,
        capsule_types: capsule_map,
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
        .generate_bindings(&api, &config)
        .expect("generate_bindings with capsule_types should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Language and Parser must NOT appear as #[pyclass] opaque wrappers.
    assert!(
        !content.contains("struct Language"),
        "Language must not be emitted as a #[pyclass] struct; content:\n{content}"
    );
    assert!(
        !content.contains("struct Parser"),
        "Parser must not be emitted as a #[pyclass] struct; content:\n{content}"
    );

    // get_language must use PyCapsule_New for the capsule round-trip return.
    assert!(
        content.contains("PyCapsule_New"),
        "get_language must call PyCapsule_New; content:\n{content}"
    );

    // get_parser must import sample_language and call Parser via getattr + call1.
    assert!(
        content.contains("py.import(\"sample_language\")"),
        "get_parser must import the sample_language module; content:\n{content}"
    );
    assert!(
        content.contains("getattr(\"Parser\")"),
        "get_parser must call getattr(\"Parser\"); content:\n{content}"
    );
    assert!(
        content.contains("call1("),
        "get_parser must call call1 to construct the Parser; content:\n{content}"
    );

    // The preamble must suppress unsafe_code so downstreams with
    // workspace-level `unsafe_code = "deny"` compile without overrides.
    assert!(
        content.contains("allow(unsafe_code)"),
        "preamble must include #![allow(unsafe_code)]; content:\n{content}"
    );

    // Bug 1 — error_converter_name must emit function-ref, not redundant closure.
    // With Error in the IR, error_to_py_err is a known converter; it must appear as
    // `.map_err(error_to_py_err)`, NOT `.map_err(|e| error_to_py_err(e))`.
    assert!(
        content.contains(".map_err(error_to_py_err)"),
        "lib.rs must use .map_err(error_to_py_err) (function ref, not closure); content:\n{content}"
    );
    assert!(
        !content.contains(".map_err(|e| error_to_py_err(e))"),
        "lib.rs must NOT contain redundant closure .map_err(|e| error_to_py_err(e)); content:\n{content}"
    );

    // Bugs 2 and 3 — api.py import order and capsule type imports.
    let pub_files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api with capsule_types should succeed");
    let api_py = pub_files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not generated");
    let api_py_content = &api_py.content;

    // Bug 2: stdlib `from typing import` must appear BEFORE any `from .` local imports.
    let typing_pos = api_py_content
        .find("from typing import")
        .expect("api.py must contain 'from typing import'");
    let first_local_pos = api_py_content.find("from .").unwrap_or(api_py_content.len());
    assert!(
        typing_pos < first_local_pos,
        "api.py: 'from typing import' must come before 'from .' imports (isort I001);\ncontent:\n{api_py_content}"
    );

    // Bug 3: capsule types must have an explicit import so bare names resolve (ruff F821).
    // Both Language (sample_language.Language) and Parser (sample_language.Parser) share the
    // `sample_language` module, so a single `from sample_language import Language, Parser` is expected.
    assert!(
        api_py_content.contains("from sample_language import"),
        "api.py must contain 'from sample_language import' for capsule types; content:\n{api_py_content}"
    );
    assert!(
        api_py_content.contains("Language"),
        "api.py capsule import must include Language; content:\n{api_py_content}"
    );
    assert!(
        api_py_content.contains("Parser"),
        "api.py capsule import must include Parser; content:\n{api_py_content}"
    );
    // Capsule types must NOT be imported from ._native (they have no #[pyclass] there).
    let native_import_line = api_py_content
        .lines()
        .find(|l| l.contains("from ._sample_pack import") || l.contains("from ._native import"))
        .unwrap_or("");
    assert!(
        !native_import_line.contains("Language"),
        "api.py must NOT import Language from the native module; native line: {native_import_line:?}"
    );
    assert!(
        !native_import_line.contains("Parser"),
        "api.py must NOT import Parser from the native module; native line: {native_import_line:?}"
    );

    // Stub assertions: capsule types must not be declared as opaque classes in _native.pyi
    // and function stubs must use `Any` for capsule return types.
    let mut stubs_config = config.clone();
    if let Some(ref mut py) = stubs_config.python {
        py.stubs = Some(alef::core::config::StubsConfig {
            output: std::path::PathBuf::from("packages/python/sample_pack"),
            emit_docstrings: false,
        });
    }
    let stub_files = backend
        .generate_type_stubs(&api, &stubs_config)
        .expect("generate_type_stubs with capsule_types should succeed");
    assert_eq!(stub_files.len(), 1, "expected exactly one .pyi file");
    let stub_content = &stub_files[0].content;

    // Capsule types must NOT appear as standalone class declarations.
    assert!(
        !stub_content.contains("class Language:") && !stub_content.contains("class Language: ..."),
        "stub must NOT declare class Language; content:\n{stub_content}"
    );
    assert!(
        !stub_content.contains("class Parser:") && !stub_content.contains("class Parser: ..."),
        "stub must NOT declare class Parser; content:\n{stub_content}"
    );

    // Free function stubs must return `Any` for capsule types.
    assert!(
        stub_content.contains("def get_language(name: str) -> Any: ..."),
        "stub must contain 'def get_language(name: str) -> Any: ...'; content:\n{stub_content}"
    );
    assert!(
        stub_content.contains("def get_parser(name: str) -> Any: ..."),
        "stub must contain 'def get_parser(name: str) -> Any: ...'; content:\n{stub_content}"
    );

    // The stub must import Any from typing since it is now referenced.
    assert!(
        stub_content.contains("from typing import") && stub_content.contains("Any"),
        "stub must contain 'from typing import ... Any ...'; content:\n{stub_content}"
    );
}

/// capsule_types on impl-block methods:
/// - A type with a method returning a capsule type does NOT produce the non-existent struct.
/// - The method body uses PyCapsule_New (Capsule variant) or Python factory (ConstructFrom).
/// - The generated preamble includes #![allow(unsafe_code)].
#[test]
fn test_capsule_types_in_methods() {
    use alef::core::config::CapsuleTypeConfig;
    use alef::core::ir::{MethodDef, ReceiverKind};

    let backend = Pyo3Backend;

    // IR: an opaque LanguageRegistry type with two methods that return capsule types.
    let api = ApiSurface {
        crate_name: "sample_pack".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            // LanguageRegistry — the opaque registry that owns the Language/Parser getters
            TypeDef {
                name: "LanguageRegistry".to_string(),
                rust_path: "sample_pack::LanguageRegistry".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![
                    // get_language(&self, name: String) -> Result<Language, Error>
                    MethodDef {
                        name: "get_language".to_string(),
                        params: vec![ParamDef {
                            name: "name".to_string(),
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
                        return_type: TypeRef::Named("Language".to_string()),
                        is_async: false,
                        is_static: false,
                        error_type: Some("sample_pack::Error".to_string()),
                        doc: String::new(),
                        receiver: Some(ReceiverKind::Ref),
                        sanitized: false,
                        trait_source: None,
                        returns_ref: false,
                        returns_cow: false,
                        return_newtype_wrapper: None,
                        has_default_impl: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        version: Default::default(),
                    },
                ],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Language registry.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // Language — capsule round-trip type (no #[pyclass] emitted)
            TypeDef {
                name: "Language".to_string(),
                rust_path: "sample_pack::Language".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
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
    let mut capsule_map: HashMap<String, CapsuleTypeConfig> = HashMap::new();
    capsule_map.insert(
        "Language".to_string(),
        CapsuleTypeConfig::Capsule("sample_language.Language".to_string()),
    );
    config.python = Some(PythonConfig {
        module_name: Some("_sample_pack".to_string()),
        pip_name: None,
        async_runtime: None,
        stubs: None,
        features: None,
        serde_rename_all: None,
        capsule_types: capsule_map,
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
        .generate_bindings(&api, &config)
        .expect("generate_bindings with capsule_types on methods should succeed");

    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // The #[pymethods] impl block for LanguageRegistry must be present.
    // Regression guard: capsule-method rewriting was stripping the impl block header when the
    // first method returns a capsule type (`attr_start` was incorrectly walking past `#[pymethods]`
    // because `#[pymethods]impl Foo {` starts with `#[`).
    assert!(
        content.contains("#[pymethods]impl LanguageRegistry {")
            || content.contains("#[pymethods]\nimpl LanguageRegistry {"),
        "#[pymethods] impl block opening must be present for LanguageRegistry; content:\n{content}"
    );

    // Language must NOT appear as a standalone #[pyclass] struct — it is a capsule type.
    // Note: "struct LanguageRegistry" is expected; we must not match that as a false positive.
    assert!(
        !content.contains("pub struct Language {") && !content.contains("pub struct Language{"),
        "Language must not be emitted as a #[pyclass] struct; content:\n{content}"
    );

    // The get_language method must use PyCapsule_New (capsule round-trip).
    assert!(
        content.contains("PyCapsule_New"),
        "get_language method must call PyCapsule_New; content:\n{content}"
    );

    // The method must NOT reference the removed Language struct in its return type.
    assert!(
        !content.contains("-> PyResult<Language>"),
        "get_language method must not return PyResult<Language> (struct removed); content:\n{content}"
    );

    // The method must return PyResult<Py<PyAny>> instead.
    assert!(
        content.contains("-> pyo3::PyResult<pyo3::Py<pyo3::PyAny>>"),
        "get_language method must return pyo3::PyResult<pyo3::Py<pyo3::PyAny>>; content:\n{content}"
    );

    // The capsule name constant must be emitted with the configured name.
    assert!(
        content.contains("sample_language.Language"),
        "get_language method must embed the 'sample_language.Language' capsule name; content:\n{content}"
    );

    // The preamble must include #![allow(unsafe_code)].
    assert!(
        content.contains("allow(unsafe_code)"),
        "preamble must include #![allow(unsafe_code)]; content:\n{content}"
    );

    // Stub assertions: LanguageRegistry.get_language must return `Any` in .pyi.
    let mut stubs_config = config.clone();
    if let Some(ref mut py) = stubs_config.python {
        py.stubs = Some(alef::core::config::StubsConfig {
            output: std::path::PathBuf::from("packages/python/sample_pack"),
            emit_docstrings: false,
        });
    }
    let stub_files = backend
        .generate_type_stubs(&api, &stubs_config)
        .expect("generate_type_stubs with capsule_types on methods should succeed");
    assert_eq!(stub_files.len(), 1, "expected exactly one .pyi file");
    let stub_content = &stub_files[0].content;

    // Language must NOT appear as a standalone class declaration.
    assert!(
        !stub_content.contains("class Language:") && !stub_content.contains("class Language: ..."),
        "stub must NOT declare class Language; content:\n{stub_content}"
    );

    // LanguageRegistry must be declared (it is NOT a capsule type).
    assert!(
        stub_content.contains("class LanguageRegistry:"),
        "stub must declare class LanguageRegistry; content:\n{stub_content}"
    );

    // Within LanguageRegistry, get_language must return `Any`.
    assert!(
        stub_content.contains("def get_language(self, name: str) -> Any: ..."),
        "stub must contain 'def get_language(self, name: str) -> Any: ...'; content:\n{stub_content}"
    );

    // Any must be imported.
    assert!(
        stub_content.contains("from typing import") && stub_content.contains("Any"),
        "stub must contain 'from typing import ... Any ...'; content:\n{stub_content}"
    );
}
