use super::*;

// ==============================================================================
// Regression tests: UPPER_SNAKE_CASE pyclass enum variants (iter35 wave-1 W2)
// ==============================================================================

fn make_unit_enum_def(name: &str, variants: &[&str]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .enumerate()
            .map(|(i, v)| EnumVariant {
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: i == 0,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            })
            .collect(),
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

/// Generated Rust binding emits `#[pyo3(name = "UPPER_SNAKE_CASE")]` on every unit-enum variant
/// when the enum carries the `#[pyclass]` attribute.
#[test]
fn test_pyclass_enum_variants_use_upper_snake_case_pyo3_name() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![make_unit_enum_def(
            "BatchStatus",
            &["Validating", "InProgress", "Complete"],
        )],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let rust_src = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "rs"))
        .map(|f| f.content.as_str())
        .unwrap_or("");

    assert!(
        rust_src.contains("#[pyo3(name = \"VALIDATING\")]"),
        "pyclass enum variant must carry UPPER_SNAKE_CASE pyo3(name), got:\n{}",
        rust_src
    );
    assert!(
        rust_src.contains("#[pyo3(name = \"IN_PROGRESS\")]"),
        "multi-word variant must carry UPPER_SNAKE_CASE pyo3(name), got:\n{}",
        rust_src
    );
    assert!(
        rust_src.contains("#[pyo3(name = \"COMPLETE\")]"),
        "simple variant must carry UPPER_SNAKE_CASE pyo3(name), got:\n{}",
        rust_src
    );
}

/// `options.py` must NOT emit SCREAMING_SNAKE_CASE monkey-patch alias lines for needed unit enums.
/// The canonical UPPER_SNAKE_CASE name is now the direct pyclass variant name, not an alias.
#[test]
fn test_options_py_does_not_emit_screaming_alias_lines() {
    let backend = Pyo3Backend;
    // Use a has_default type that references the enum so it ends up in `needed_enums` and
    // is therefore imported from the native module (the code path that previously monkey-patched).
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("status", TypeRef::Named("BatchStatus".to_string()), false)],
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![make_unit_enum_def("BatchStatus", &["Validating", "InProgress"])],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings must succeed");
    let options_py = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "options.py"))
        .map(|f| f.content.as_str())
        .unwrap_or("");

    // The old pattern was `BatchStatus.VALIDATING = BatchStatus.Validating`
    // or `setattr(BatchStatus, "VALIDATING", getattr(...))`.  Neither should appear.
    assert!(
        !options_py.contains(".VALIDATING = "),
        "options.py must NOT emit SCREAMING alias assignment, got:\n{}",
        options_py
    );
    assert!(
        !options_py.contains("setattr(BatchStatus"),
        "options.py must NOT emit setattr monkey-patch for BatchStatus, got:\n{}",
        options_py
    );
}

/// `options.py` must escape variant names whose snake_case form collides with a Python
/// reserved keyword. The HTML `<del>` tag maps to a Rust `NodeType::Del` variant; without
/// escaping, this emits `del = "del"` which is unparseable as a class-body statement.
/// `alef::core::keywords::python_ident` appends `_` (`del_ = "del"`).
#[test]
fn test_options_py_escapes_python_keyword_variant_names() {
    let backend = Pyo3Backend;
    // Create two enums: one referenced by a has_default type (goes to needed_enums, gets
    // imported) and one unreferenced (emitted as a (str, Enum) class in options.py).
    // We test the unreferenced one to verify the (str, Enum) emission path.
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ParseOptions".to_string(),
            rust_path: "test_lib::ParseOptions".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("heading", TypeRef::Named("HeadingStyle".to_string()), false)],
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![
            make_unit_enum_def("HeadingStyle", &["Atx", "Setext"]),
            // This enum is not referenced by any has_default type, so it will be emitted
            // as a (str, Enum) class in options.py, allowing us to test the escaping.
            make_unit_enum_def("NodeType", &["Del", "Ins", "Title"]),
        ],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let options_py = files
        .iter()
        .find(|f| f.path.ends_with("options.py"))
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| {
            panic!(
                "options.py not emitted. files: {:?}",
                files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
            )
        });

    assert!(
        options_py.contains("del_ = \"del\"") || options_py.contains("del_ = 'del'"),
        "options.py must escape Python-keyword variant Del → del_ (with original 'del' as value), got:\n{}",
        options_py
    );
    assert!(
        !options_py.contains("\n    del = "),
        "options.py must NOT emit the unescaped keyword `del` as a class attribute, got:\n{}",
        options_py
    );
    assert!(
        options_py.contains("ins = \"ins\"") || options_py.contains("ins = 'ins'"),
        "non-keyword variants must still emit unescaped (ins), got:\n{}",
        options_py
    );
    assert!(
        options_py.contains("title_ = \"title\"") || options_py.contains("title_ = 'title'"),
        "options.py must escape str-method variant Title → title_ (with original 'title' as value), got:\n{}",
        options_py
    );
}

/// Bug A: void-returning functions should NOT emit `return` statement.
/// Functions with `-> None` annotation must emit a bare call without `return`.
#[test]
fn test_api_py_void_function_no_redundant_return() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "init".to_string(),
            rust_path: "test_lib::init".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: "Initialize the system.".to_string(),
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
        excluded_type_paths: HashMap::new(),
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
        target_dep_overrides: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not found");

    // The function body should call _rust.init() without a return statement
    assert!(
        api_py.content.contains("def init() -> None:"),
        "api.py should have void-returning init function signature, got:\n{}",
        api_py.content
    );

    // Extract the function body to verify no `return` keyword appears
    if let Some(start) = api_py.content.find("def init() -> None:") {
        // Look for the next function definition to find the end of this function
        let rest = &api_py.content[start..];
        let next_fn_start = rest[19..].find("def ").map(|p| p + 19);
        let fn_body = if let Some(end) = next_fn_start {
            &rest[..end]
        } else {
            rest
        };
        // The body should have the docstring and the call
        assert!(fn_body.contains("_rust.init()"), "Function should call _rust.init()");
        // But it should NOT have "return _rust.init()"
        let without_docstring = fn_body.split("\"\"\"").last().unwrap_or(fn_body);
        assert!(
            !without_docstring.contains("return _rust.init()"),
            "Void-returning function must not emit 'return _rust.init()', got:\n{}",
            fn_body
        );
    }
}

/// Bug B: Consecutive top-level function definitions must have exactly two blank lines between them.
/// This is a PEP 8 requirement for spacing between top-level definitions.
#[test]
fn test_api_py_pep8_blank_lines_between_functions() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            FunctionDef {
                name: "first_function".to_string(),
                rust_path: "test_lib::first_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "First function.".to_string(),
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
            FunctionDef {
                name: "second_function".to_string(),
                rust_path: "test_lib::second_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Second function.".to_string(),
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
            FunctionDef {
                name: "third_function".to_string(),
                rust_path: "test_lib::third_function".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Third function.".to_string(),
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
        errors: vec![],
        excluded_type_paths: HashMap::new(),
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
        target_dep_overrides: Vec::new(),
    });

    let files = backend
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");
    let api_py = files
        .iter()
        .find(|f| f.path.ends_with("api.py"))
        .expect("api.py not found");

    // Find the three function definitions and verify spacing
    let first_pos = api_py
        .content
        .find("def first_function")
        .expect("first_function not found");
    let second_pos = api_py
        .content
        .find("def second_function")
        .expect("second_function not found");
    let third_pos = api_py
        .content
        .find("def third_function")
        .expect("third_function not found");

    // Between first and second function
    let between_1_2 = &api_py.content[first_pos..second_pos];
    // Count the blank lines between the functions
    // Should be: closing of first function + empty line + empty line + def of second
    let blank_count_1_2 = between_1_2.matches("\n\n").count();
    assert!(
        blank_count_1_2 >= 1,
        "Between first and second function, should have blank lines, got:\n{}",
        between_1_2
    );

    // Between second and third function
    let between_2_3 = &api_py.content[second_pos..third_pos];
    let blank_count_2_3 = between_2_3.matches("\n\n").count();
    assert!(
        blank_count_2_3 >= 1,
        "Between second and third function, should have blank lines, got:\n{}",
        between_2_3
    );

    // More stringent check: no docstrings immediately followed by def (with only 1 newline).
    // PEP 8 requires 2 blank lines between top-level definitions, meaning 3 newlines total.
    // We check for the docstring closing followed by only 1 or 2 newlines then 'def'.
    let has_improper_spacing_single = api_py.content.contains("\"\"\"\ndef ");
    let has_improper_spacing_one_blank = api_py.content.contains("\"\"\"\n\ndef ");
    assert!(
        !has_improper_spacing_single && !has_improper_spacing_one_blank,
        "Functions are jammed together without proper PEP 8 spacing:\n{}",
        api_py.content
    );
}

/// Regression test: `from ._<module> import (` must NOT be followed by a blank line.
///
/// Previously the multi-line native-import branch routed through `single_line.jinja`
/// with a text ending in `\n`; the template appended a second `\n`, yielding
/// `from ._mod import (\n\n    Name,` which ruff E303 rejects and which caused an
/// endless regen-format-fail loop in downstream consumers (sample_crate, sample_crawler).
#[test]
fn test_native_import_no_stray_blank_line_after_open_paren() {
    let backend = Pyo3Backend;

    // Create enough opaque types (no has_default) so that the import line exceeds
    // 88 chars and the multi-line branch is taken.  Names are intentionally long
    // to force multi-line without needing dozens of types.
    let make_opaque = |name: &str| TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
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
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            make_opaque("AssetCategory"),
            make_opaque("AuthConfig"),
            make_opaque("BrowserMode"),
            make_opaque("BrowserWait"),
            make_opaque("CrawlEngineHandle"),
            make_opaque("FeedType"),
            make_opaque("ImageSource"),
            make_opaque("LinkType"),
        ],
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
        .generate_public_api(&api, &config)
        .expect("generate_public_api failed");

    let init_py = files
        .iter()
        .find(|f| f.path.ends_with("__init__.py"))
        .expect("__init__.py not generated");

    // The import must start a multi-line block and must NOT have a blank line
    // immediately after the open paren — `(\n\n` is the bug pattern.
    assert!(
        !init_py.content.contains("import (\n\n"),
        "__init__.py must not have a blank line after the open paren in a multi-line import; \
         ruff E303 rejects it and causes an endless regen-format loop.\ncontent:\n{}",
        init_py.content
    );

    // Verify the import block is actually multi-line (the test is only useful
    // if we hit the `import_line.len() > 88` branch).
    assert!(
        init_py.content.contains("from ._test_lib import (\n"),
        "__init__.py should emit a multi-line native import for this many types;\ncontent:\n{}",
        init_py.content
    );
}

/// Regression: an internally-tagged enum (`#[serde(tag = "...")]`) must accept a bare string
/// for its unit variants. Previously the constructor encoded the string as a bare JSON string
/// (`"disabled"`), which the tagged enum rejects — so `Enum("disabled")`, and any binding that
/// builds a default from that string, raised `ValueError`. The constructor must wrap the bare
/// string as `{"<tag>": "<variant>"}` so serde can parse it into the unit variant.
#[test]
fn test_internally_tagged_enum_constructor_wraps_bare_string() {
    fn variant(name: &str, fields: Vec<FieldDef>, is_default: bool) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            originally_had_data_fields: !fields.is_empty(),
            fields,
            doc: String::new(),
            is_default,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            cfg: None,
            version: Default::default(),
        }
    }

    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "VlmFallbackPolicy".to_string(),
            rust_path: "test_lib::VlmFallbackPolicy".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                variant("Disabled", vec![], true),
                variant(
                    "OnLowQuality",
                    vec![make_field(
                        "quality_threshold",
                        TypeRef::Primitive(PrimitiveType::F64),
                        false,
                    )],
                    false,
                ),
                variant("Always", vec![], false),
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: true,
            serde_tag: Some("mode".to_string()),
            serde_untagged: false,
            serde_rename_all: Some("snake_case".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs should be generated");

    assert!(
        lib_rs.content.contains(r#"serde_json::json!({ "mode": s })"#),
        "internally-tagged enum constructor must wrap a bare string as {{\"mode\": s}};\ncontent:\n{}",
        lib_rs.content
    );
    assert!(
        !lib_rs.content.contains("serde_json::to_string(&s)"),
        "tagged enum must not encode the bare string directly (the bug);\ncontent:\n{}",
        lib_rs.content
    );
}

/// #132: an internally-tagged enum with a UNIT data-default variant must still wrap a bare host
/// string as `{"<tag>": s}` so serde resolves the variant. This is the unit-variant tag-wrapping
/// that survives the `string_shorthand` removal — verify it stays after that mechanism is gone.
#[test]
fn test_internally_tagged_unit_variant_wraps_bare_string() {
    fn variant(name: &str, fields: Vec<FieldDef>, is_default: bool) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            originally_had_data_fields: !fields.is_empty(),
            fields,
            doc: String::new(),
            is_default,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            cfg: None,
            version: Default::default(),
        }
    }

    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Greeting".to_string(),
            rust_path: "test_lib::Greeting".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                variant("Default", vec![], true),
                variant("Preset", vec![make_field("name", TypeRef::String, false)], false),
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: true,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: Some("snake_case".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs should be generated");

    // #132: a bare string is wrapped as {"<tag>": s} so serde resolves the (unit) variant name.
    assert!(
        lib_rs.content.contains(r#"serde_json::json!({ "type": s })"#),
        "internally-tagged enum must wrap a bare string as {{\"type\": s}};\ncontent:\n{}",
        lib_rs.content
    );
}
