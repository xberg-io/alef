use super::*;

#[test]
fn test_basic_generation() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "Test configuration".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
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
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Process input".to_string(),
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
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_content: None,
            serde_untagged: false,
            serde_rename_all: None,
            rename_all_fields: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Failed to generate bindings");
    let files = result.unwrap();

    assert_eq!(files.len(), 1, "Expected 1 generated file");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().ends_with("lib.rs"),
        "Expected lib.rs file"
    );

    let content = &lib_file.content;

    assert!(
        content.contains("#[pyclass"),
        "Should contain #[pyclass] for Config type"
    );
    assert!(
        content.contains("#[pymethods]"),
        "Should contain #[pymethods] for Config methods"
    );
    assert!(
        content.contains("#[pyfunction]"),
        "Should contain #[pyfunction] for process function"
    );

    assert!(content.contains("struct Config"), "Should define Config struct");
    assert!(content.contains("enum Mode"), "Should define Mode enum");

    assert!(content.contains("#[pymodule]"), "Should contain #[pymodule] macro");
    assert!(
        content.contains("pub fn _test_lib"),
        "Should contain module init function with correct name"
    );

    assert!(content.contains("use pyo3::prelude::*"), "Should import pyo3::prelude");
}

#[test]
fn public_api_converters_accept_json_string_for_dict_coercion() {
    let backend = Pyo3Backend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StructuredExtraction".to_string(),
            rust_path: "test_lib::StructuredExtraction".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("schema", TypeRef::Json, true)],
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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
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
        functions: vec![FunctionDef {
            name: "extract_structured".to_string(),
            rust_path: "test_lib::extract_structured".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("StructuredExtraction".to_string()),
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
            return_type: TypeRef::Unit,
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_public_api(&api, &make_config()).unwrap();
    let api_py = files.iter().find(|f| f.path.ends_with("api.py")).unwrap();
    assert!(
        api_py.content.contains("import json"),
        "api.py must import json:\n{}",
        api_py.content
    );
    assert!(
        api_py
            .content
            .contains("if isinstance(value, str):\n        value = json.loads(value)"),
        "converter must parse JSON strings before dict/object coercion:\n{}",
        api_py.content
    );
}

#[test]
fn test_type_mapping() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "DataTypes".to_string(),
            rust_path: "test_lib::DataTypes".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("count", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("value", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("text", TypeRef::String, false),
                make_field("optional_text", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("items", TypeRef::Vec(Box::new(TypeRef::String)), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "Various data types".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    assert!(content.contains("struct DataTypes"), "Should define DataTypes struct");

    assert!(content.contains("count:"), "Should have count field");
    assert!(content.contains("value:"), "Should have value field");
    assert!(content.contains("text:"), "Should have text field");
    assert!(content.contains("optional_text:"), "Should have optional_text field");
    assert!(content.contains("items:"), "Should have items field");

    assert!(content.contains("#[pyclass"), "Type should have #[pyclass] macro");

    assert!(
        content.contains("From<") || content.contains("Into<"),
        "Should generate conversion traits"
    );
}

#[test]
fn test_enum_generation() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Complete".to_string(),
                    fields: vec![],
                    doc: "Completed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Status enum".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_content: None,
            serde_untagged: false,
            serde_rename_all: None,
            rename_all_fields: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    assert!(content.contains("enum Status"), "Should define Status enum");

    assert!(content.contains("Pending"), "Should have Pending variant");
    assert!(content.contains("Active"), "Should have Active variant");
    assert!(content.contains("Complete"), "Should have Complete variant");

    assert!(
        content.contains("#[pyclass") && content.contains("eq"),
        "Enum should have #[pyclass] with eq attribute"
    );

    assert!(
        content.contains("From<") || content.contains("Into<"),
        "Should generate enum conversion code"
    );
}

#[test]
fn test_generated_header() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    for file in &files {
        if file.path.to_string_lossy().ends_with("lib.rs") {
            assert!(
                file.content.contains("Code generated by Alef") || file.content.contains("DO NOT EDIT"),
                "Generated file should contain generation marker"
            );
        }
    }
}

#[test]
fn test_function_with_error_type() {
    let backend = Pyo3Backend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "validate".to_string(),
            rust_path: "test_lib::validate".to_string(),
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
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            error_type: Some("ValidationError".to_string()),
            doc: "Validate input".to_string(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert_eq!(files.len(), 1);

    let content = &files[0].content;

    // Check that the function is generated with #[pyfunction]
    assert!(
        content.contains("#[pyfunction]"),
        "Function should have #[pyfunction] macro"
    );
    assert!(content.contains("fn validate"), "Should generate validate function");

    assert!(
        content.contains("#[pyo3(signature"),
        "Function should have pyo3 signature attribute"
    );
}

/// Config identical to `make_config` but with stubs enabled, so one test can compare the
/// generated binding against the `.pyi` that documents it.
fn config_with_stubs() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/src/"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn keyword_field_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Registry".to_string(),
            rust_path: "test_lib::Registry".to_string(),
            fields: vec![
                make_field("global", TypeRef::String, false),
                make_field("label", TypeRef::String, false),
            ],
            is_clone: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// `global` is a hard Python keyword: `obj.global` is a SyntaxError in every identifier
/// position and Python offers no `r#`/backtick escape, so the only available fix is a rename.
/// The rename has to reach the *Python-visible* name — emitting `#[pyo3(get, name = "global")]`
/// over a Rust field called `global_` puts the keyword straight back on the class.
#[test]
fn should_rename_python_keyword_field_on_the_python_visible_surface() {
    let files = Pyo3Backend
        .generate_bindings(&keyword_field_surface(), &config_with_stubs())
        .expect("bindings generate");
    let content = &files[0].content;

    assert!(
        content.contains("global_"),
        "Rust binding field should be escaped: {content}"
    );
    assert!(
        content.contains(r#"pyo3(get, name = "global_")"#),
        "PyO3 must publish the escaped name, not the keyword: {content}"
    );
    // Anchored on the `pyo3(get, ` prefix deliberately. A bare `name = "global")` also matches
    // `serde(rename = "global")` -- the wire name the sibling test REQUIRES to stay unrenamed --
    // so the loose form fails on correct output, and the only way to satisfy it would be to move
    // the JSON key, which is the worse of the two bugs. ~keep
    assert!(
        !content.contains(r#"pyo3(get, name = "global")"#),
        "the bare keyword must not survive as the Python attribute name: {content}"
    );
}

/// The consistency half: a rename that moved the JSON key would be a worse bug than the
/// SyntaxError it fixes, because it silently breaks every peer binding on the wire. The
/// escape lives on the Python surface only — serde still spells the field `global`.
#[test]
fn should_keep_the_json_wire_name_when_a_python_keyword_field_is_renamed() {
    let files = Pyo3Backend
        .generate_bindings(&keyword_field_surface(), &config_with_stubs())
        .expect("bindings generate");
    let content = &files[0].content;

    assert!(
        content.contains(r#"serde(rename = "global")"#),
        "the wire name must stay on the unescaped keyword: {content}"
    );
    assert!(
        !content.contains(r#"serde(rename = "global_")"#),
        "the escape must not leak into the wire format: {content}"
    );
}

/// The other consistency half: the `.pyi` is what a type checker reads, so a binding whose
/// runtime attribute disagrees with the stub is indistinguishable from no fix at all — the
/// checker approves an attribute that raises `AttributeError`.
#[test]
fn should_declare_the_same_escaped_field_name_in_the_binding_and_the_stub() {
    let api = keyword_field_surface();
    let config = config_with_stubs();

    let binding = Pyo3Backend.generate_bindings(&api, &config).expect("bindings generate")[0]
        .content
        .clone();
    let stubs = Pyo3Backend.generate_type_stubs(&api, &config).expect("stubs generate");
    let stub = stubs
        .iter()
        .map(|f| f.content.as_str())
        .find(|content| content.contains("Registry"))
        .expect("a stub declaring Registry");

    assert!(
        stub.contains("global_"),
        "stub should declare the escaped attribute: {stub}"
    );
    assert!(
        binding.contains(r#"pyo3(get, name = "global_")"#) && stub.contains("global_"),
        "binding and stub must agree on the attribute name\nbinding: {binding}\nstub: {stub}"
    );
}

fn pyclass_surface_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        fields: vec![make_field("value", TypeRef::String, false)],
        is_clone: true,
        ..Default::default()
    }
}

fn assert_type_absent_from_runtime_conversions_and_stub(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    type_name: &str,
) {
    let files = Pyo3Backend
        .generate_bindings(api, config)
        .expect("bindings should generate");
    let binding = &files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated binding crate should include lib.rs")
        .content;
    assert!(
        !binding.contains(&format!("impl From<test_lib::{type_name}> for {type_name}")),
        "core-to-binding conversion must not target an absent pyclass:\n{binding}"
    );
    assert!(
        !binding.contains(&format!("impl From<{type_name}> for test_lib::{type_name}")),
        "binding-to-core conversion must not target an absent pyclass:\n{binding}"
    );

    let stub = Pyo3Backend
        .generate_type_stubs(api, config)
        .expect("stubs should generate")
        .into_iter()
        .find(|file| file.path.extension().is_some_and(|extension| extension == "pyi"))
        .expect("stub generation should emit a .pyi file")
        .content;
    assert!(
        !stub.contains(&format!("class {type_name}:")),
        "stub must not declare a class absent from the extension module:\n{stub}"
    );
}

#[test]
fn absent_pyclasses_do_not_receive_conversions_or_stub_classes() {
    use alef::core::config::CapsuleTypeConfig;

    for case in ["config", "capsule", "binding", "error"] {
        let type_name = match case {
            "config" => "ConfigExcluded",
            "capsule" => "CapsuleExcluded",
            "binding" => "BindingExcluded",
            "error" => "ConflictError",
            _ => unreachable!(),
        };
        let mut typ = pyclass_surface_type(type_name);
        let mut api = ApiSurface {
            crate_name: "test_lib".to_string(),
            types: vec![typ.clone()],
            ..Default::default()
        };
        let mut config = config_with_stubs();
        let python = config.python.as_mut().expect("test config enables Python");
        match case {
            "config" => python.exclude_types.push(type_name.to_string()),
            "capsule" => {
                python.capsule_types.insert(
                    type_name.to_string(),
                    CapsuleTypeConfig::Capsule(format!("test_lib.{type_name}")),
                );
            }
            "binding" => {
                typ.binding_excluded = true;
                api.types[0] = typ;
            }
            "error" => {
                api.errors.push(ErrorDef {
                    name: type_name.to_string(),
                    rust_path: format!("test_lib::{type_name}"),
                    original_rust_path: String::new(),
                    variants: Vec::new(),
                    doc: String::new(),
                    methods: Vec::new(),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                });
            }
            _ => unreachable!(),
        }
        assert_type_absent_from_runtime_conversions_and_stub(&api, &config, type_name);
    }
}
