use alef::backends::go::trait_bridge::gen_trait_bridges_file;
use alef::core::config::TraitBridgeConfig;
use alef::core::ir::*;

use super::{make_api_with_type, make_config_with_bridges, make_trait_method, make_trait_param, make_trait_type};

// ---------------------------------------------------------------------------
// Trait bridge typed params (regression: D1 - interface{} instead of concrete types)
// ---------------------------------------------------------------------------

#[test]
fn test_trait_bridge_string_param_emitted_as_string_not_interface() {
    // Regression test D1: path: String should emit "path string", not "path interface{}"
    let trait_type = make_trait_type(
        "Backend",
        vec![make_trait_method(
            "process_file",
            vec![make_trait_param("path", TypeRef::String)],
            TypeRef::String,
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Backend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go interface method signature should emit "path string", NOT "path interface{}"
    assert!(
        code.contains("ProcessFile(path string"),
        "String parameter must emit as 'string', not 'interface{{}}' in trait interface method\nGenerated code:\n{code}"
    );
}

#[test]
fn test_trait_bridge_named_config_param_emitted_as_concrete_type() {
    // Regression test D1: config: OcrConfig should emit "config OcrConfig", not "config map[string]interface{}"
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method(
            "process_image",
            vec![
                make_trait_param("image_bytes", TypeRef::Bytes),
                make_trait_param("config", TypeRef::Named("OcrConfig".to_string())),
            ],
            TypeRef::Named("OcrResult".to_string()),
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);

    // Add OcrConfig and OcrResult structs to the API
    let mut api = make_api_with_type(trait_type);
    api.types.push(TypeDef {
        name: "OcrConfig".to_string(),
        rust_path: "my_lib::OcrConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
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
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });
    api.types.push(TypeDef {
        name: "OcrResult".to_string(),
        rust_path: "my_lib::OcrResult".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
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
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go interface method signature should emit "config OcrConfig", NOT "config map[string]interface{}"
    assert!(
        code.contains("ProcessImage(") && code.contains("config OcrConfig"),
        "Named config parameter must emit as concrete type 'OcrConfig', not 'map[string]interface{{}}' in trait interface method\nGenerated code:\n{code}"
    );
}

#[test]
fn test_trait_bridge_enum_return_type_emitted_as_concrete_type() {
    // Regression test D1: return BackendType should emit "OcrBackendType", not "map[string]interface{}"
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method(
            "backend_type",
            vec![],
            TypeRef::Named("OcrBackendType".to_string()),
            false,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);

    // Add OcrBackendType enum to the API
    let mut api = make_api_with_type(trait_type);
    api.enums.push(EnumDef {
        name: "OcrBackendType".to_string(),
        rust_path: "my_lib::OcrBackendType".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Tesseract".to_string(),
            fields: vec![],
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
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    });

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go interface method signature should emit "OcrBackendType", NOT "map[string]interface{}"
    assert!(
        code.contains("BackendType() OcrBackendType"),
        "Named return type must emit as concrete enum type 'OcrBackendType', not 'map[string]interface{{}}' in trait interface method\nGenerated code:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Excluded-type substitution (regression: sample_crate's InternalDocument)
// ---------------------------------------------------------------------------

/// Regression: when a trait method references a type that was extracted from Rust
/// but excluded from the public binding (e.g. `#[cfg_attr(alef, alef(skip))]`),
/// the Go trait interface and trampoline must fall back to `json.RawMessage`
/// — otherwise the generated Go code refers to an undefined type and the build
/// fails with `undefined: <Name>`.
#[test]
fn test_trait_bridge_substitutes_excluded_named_types_with_json_raw_message() {
    let trait_type = make_trait_type(
        "Renderer",
        vec![make_trait_method(
            "render",
            vec![make_trait_param("doc", TypeRef::Named("InternalDocument".to_string()))],
            TypeRef::String,
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        super_trait: None,
        registry_getter: Some("get_renderer_registry".to_string()),
        register_fn: Some("register_renderer".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let mut api = make_api_with_type(trait_type);
    // Mark InternalDocument as excluded — this is what `#[cfg_attr(alef, alef(skip))]`
    // produces in the real sample_crate IR.
    api.excluded_type_paths.insert(
        "InternalDocument".to_string(),
        "sample_crate::types::internal::InternalDocument".to_string(),
    );

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go trait interface and trampoline must NOT name `InternalDocument` — that
    // type was never emitted into binding.go and the build would fail with
    // `undefined: InternalDocument`.
    assert!(
        !code.contains("InternalDocument"),
        "trait_bridges.go must not reference excluded type InternalDocument\nGenerated code:\n{code}"
    );
    // The trampoline parameter declaration must use json.RawMessage instead.
    assert!(
        code.contains("json.RawMessage"),
        "expected json.RawMessage fallback for excluded named type\nGenerated code:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Function deduplication (regression: D2 - snake_case + PascalCase duplicates)
// ---------------------------------------------------------------------------

#[test]
fn test_trait_bridge_dedup_snake_case_unregister_functions() {
    // Regression test D2: when unregister_fn is set to snake_case version of Unregister{Trait},
    // don't emit both versions — only emit the PascalCase standard function.
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process_image", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()), // snake_case — should NOT emit
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Must have the PascalCase function
    assert!(
        code.contains("func UnregisterOcrBackend(name string) error {"),
        "Must emit PascalCase UnregisterOcrBackend function"
    );

    // Must NOT have the snake_case duplicate
    assert!(
        !code.contains("func unregister_ocr_backend(name string) error {"),
        "Must NOT emit snake_case unregister_ocr_backend function — Go convention is PascalCase only"
    );

    // Count occurrences of unregister to ensure only one version is present
    let unregister_count = code.matches("func Unregister").count();
    assert_eq!(
        unregister_count, 1,
        "Must emit exactly one Unregister function (PascalCase), got {unregister_count}"
    );
}

// ---------------------------------------------------------------------------
// Trait-bridge config marshalling (T1.5 — avoid interface{} for typed configs)
// ---------------------------------------------------------------------------

#[test]
fn test_trait_bridge_unmarshals_config_into_concrete_type() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method(
            "process_image",
            vec![
                make_trait_param("image_bytes", TypeRef::Bytes),
                make_trait_param("config", TypeRef::Named("OcrConfig".to_string())),
            ],
            TypeRef::String,
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("get_ocr_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Debug: print what was generated
    eprintln!("Full generated code:\n{}", &code);
    eprintln!("---");
    if let Some(pos) = code.find("go") {
        eprintln!("Code starting at 'go': {}", &code[pos..pos.min(pos + 500)]);
    }

    // Assert: config parameter should unmarshal directly into OcrConfig, not interface{}
    assert!(
        code.contains("var goConfig OcrConfig"),
        "trampoline must declare config variable as concrete OcrConfig type"
    );
    assert!(
        code.contains("json.Unmarshal([]byte(C.GoString(config)), &goConfig)"),
        "trampoline must unmarshal directly into concrete OcrConfig type"
    );

    // Assert: the generated code should NOT contain the problematic interface{} pattern
    // for config parameter unmarshalling
    let problem_pattern = "var rawData interface{}\n\t\t\tjson.Unmarshal([]byte(C.GoString(config))";
    assert!(
        !code.contains(problem_pattern),
        "trampoline callback body must not use 'var rawData interface{{}}' for typed config params"
    );
}
