use alef::backends::gleam::GleamBackend;
use alef::core::backend::Backend;
use alef::core::config::{GleamConfig, ResolvedCrateConfig, TraitBridgeConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
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
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
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
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,

        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn make_config_with_nif(nif_module: &str) -> ResolvedCrateConfig {
    let mut config = make_config();
    config.gleam = Some(GleamConfig {
        app_name: None,
        nif_module: Some(nif_module.to_string()),
        features: None,
        serde_rename_all: None,
        rename_fields: std::collections::HashMap::new(),
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        element_constructors: Vec::new(),
        json_object_wrapper: None,
    });
    config
}

#[test]
fn struct_emits_record_type() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Point",
            vec![
                make_field("x", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("y", TypeRef::Primitive(PrimitiveType::I32), false),
            ],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GleamBackend.generate_bindings(&api, &make_config()).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    assert!(content.contains("pub type Point {"), "missing type decl: {content}");
    assert!(content.contains("Point("), "missing constructor: {content}");
    assert!(content.contains("x: Int"));
    assert!(content.contains("y: Int"));
}

#[test]
fn function_emits_external_binding() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "greet".into(),
            rust_path: "demo::greet".into(),
            original_rust_path: String::new(),
            params: vec![make_param("who", TypeRef::String)],
            return_type: TypeRef::String,
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

    let files = GleamBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"greet\")"),
        "missing external annotation: {content}"
    );
    assert!(content.contains("pub fn greet(who: String) -> String"));
}

#[test]
fn enum_emits_custom_type() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".into(),
            rust_path: "demo::Status".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".into(),
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
                },
                EnumVariant {
                    name: "Inactive".into(),
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
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
            has_default: false,
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

    let files = GleamBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("pub type Status {"));
    assert!(content.contains("Active"));
    assert!(content.contains("Inactive"));
}

#[test]
fn optional_field_imports_option() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Maybe",
            vec![make_field("value", TypeRef::Optional(Box::new(TypeRef::String)), false)],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GleamBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(content.contains("import gleam/option.{type Option}"));
    assert!(content.contains("value: Option(String)"));
}

#[test]
fn error_emits_custom_type() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".into(),
            rust_path: "demo::DemoError".into(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".into(),
                    message_template: None,
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "InvalidInput".into(),
                    message_template: None,
                    fields: vec![make_field("details", TypeRef::String, false)],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                    doc: String::new(),
                },
            ],
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
    };

    let files = GleamBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("pub type DemoError {"),
        "missing error type decl: {content}"
    );
    assert!(content.contains("NotFound"), "missing NotFound variant: {content}");
    assert!(
        content.contains("InvalidInput("),
        "missing InvalidInput constructor: {content}"
    );
    assert!(content.contains("details: String"), "missing details field: {content}");
}

#[test]
fn enum_tuple_variant_emits_unlabeled_field() {
    // Rust tuple variants like `Pdf(String)` produce fields named `_0`, `_1`, etc.
    // Gleam constructor arguments cannot have labels starting with `_`, so these
    // must be emitted as unlabeled positional arguments: `Pdf(String)` not `Pdf(_0: String)`.
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Wrapper".into(),
            rust_path: "demo::Wrapper".into(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Inner".into(),
                fields: vec![make_field("_0", TypeRef::String, false)],
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
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
            has_default: false,
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

    let files = GleamBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;
    assert!(
        !content.contains("_0:"),
        "positional field `_0` must not appear as a label: {content}"
    );
    assert!(
        content.contains("Inner(\n    String\n  )") || content.contains("Inner(\n    String"),
        "unlabeled String argument expected: {content}"
    );
}

#[test]
fn nif_module_override_uses_custom_name() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "greet".into(),
            rust_path: "demo::greet".into(),
            original_rust_path: String::new(),
            params: vec![make_param("who", TypeRef::String)],
            return_type: TypeRef::String,
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

    let config = make_config_with_nif("custom_nif_atom");
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("@external(erlang, \"custom_nif_atom\", \"greet\")"),
        "should use custom nif_module: {content}"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
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
    }
}

fn make_method_with_types(name: &str, return_type: TypeRef, error_type: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: error_type.map(str::to_string),
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
    }
}

fn make_trait_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: false,

        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    }
}

fn make_bridge_cfg(trait_name: &str, register_fn: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some(format!("demo::get_{}_registry", trait_name.to_lowercase())),
        register_fn: Some(register_fn.to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

fn make_bridge_cfg_full(
    trait_name: &str,
    register_fn: &str,
    unregister_fn: Option<&str>,
    clear_fn: Option<&str>,
) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some(format!("demo::get_{}_registry", trait_name.to_lowercase())),
        register_fn: Some(register_fn.to_string()),
        unregister_fn: unregister_fn.map(str::to_string),
        clear_fn: clear_fn.map(str::to_string),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }
}

fn make_config_with_bridges(bridges: Vec<TraitBridgeConfig>) -> ResolvedCrateConfig {
    let mut config = make_config();
    config.trait_bridges = bridges;
    config
}

// ---------------------------------------------------------------------------
// Trait bridge: single-method trait emits registration shim + support NIFs
// ---------------------------------------------------------------------------

#[test]
fn trait_bridge_single_method_emits_register_and_support_nifs() {
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process")]);
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;

    // Registration shim must reference the correct Rustler NIF name
    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"register_ocr_backend\")"),
        "missing register shim: {content}"
    );
    assert!(
        content.contains("pub fn register_ocr_backend(pid: Dynamic, plugin_name: String) -> Nil"),
        "missing register fn signature: {content}"
    );

    // Support NIFs must be emitted once
    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"complete_trait_call\")"),
        "missing complete_trait_call shim: {content}"
    );
    assert!(
        content.contains("pub fn complete_trait_call(reply_id: Int, result_json: String) -> Nil"),
        "missing complete_trait_call signature: {content}"
    );
    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"fail_trait_call\")"),
        "missing fail_trait_call shim: {content}"
    );

    // Dynamic import must be present
    assert!(
        content.contains("import gleam/dynamic.{type Dynamic}"),
        "missing Dynamic import: {content}"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge: multi-method trait emits only one set of support NIFs
// ---------------------------------------------------------------------------

#[test]
fn trait_bridge_multiple_bridges_emit_support_nifs_only_once() {
    let ocr_type = make_trait_type("OcrBackend", vec![make_method("process"), make_method("name")]);
    let embedding_type = make_trait_type("EmbeddingBackend", vec![make_method("embed")]);
    let ocr_bridge = make_bridge_cfg("OcrBackend", "register_ocr_backend");
    let embedding_bridge = make_bridge_cfg("EmbeddingBackend", "register_embedding_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![ocr_type, embedding_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![ocr_bridge, embedding_bridge]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Both registration shims must be present
    assert!(
        content.contains("pub fn register_ocr_backend("),
        "missing ocr register fn: {content}"
    );
    assert!(
        content.contains("pub fn register_embedding_backend("),
        "missing embedding register fn: {content}"
    );

    // Support NIFs emitted exactly once (count occurrences)
    let complete_count = content.matches("pub fn complete_trait_call(").count();
    assert_eq!(
        complete_count, 1,
        "complete_trait_call must be emitted exactly once, found {complete_count}: {content}"
    );
    let fail_count = content.matches("pub fn fail_trait_call(").count();
    assert_eq!(
        fail_count, 1,
        "fail_trait_call must be emitted exactly once, found {fail_count}: {content}"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge: per-method response shims
// ---------------------------------------------------------------------------

#[test]
fn trait_bridge_emits_per_method_response_shim() {
    // A trait with one method returning Unit and no error type should emit a shim
    // with `Result(Nil, String)` (fallback error type).
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process_image")]);
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The NIF shim name follows `{trait_snake}_{method_snake}_response`.
    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"ocr_backend_process_image_response\")"),
        "missing response shim @external: {content}"
    );
    assert!(
        content.contains(
            "pub fn ocr_backend_process_image_response(call_id: Dynamic, result: Result(Nil, String)) -> Nil"
        ),
        "missing response shim fn signature: {content}"
    );
}

#[test]
fn trait_bridge_response_shim_uses_typed_return_and_error() {
    // A method returning String with a named error type should emit typed Result.
    // The error type must appear in the declared errors list so resolve_gleam_error_type
    // can match it; otherwise it falls back to String.
    let method = make_method_with_types("process_image", TypeRef::String, Some("OcrError"));
    let trait_type = make_trait_type("OcrBackend", vec![method]);
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");
    let ocr_error = ErrorDef {
        name: "OcrError".into(),
        rust_path: "demo::OcrError".into(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![ocr_error],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains(
            "pub fn ocr_backend_process_image_response(call_id: Dynamic, result: Result(String, OcrError)) -> Nil"
        ),
        "wrong typed response shim signature: {content}"
    );
}

#[test]
fn trait_bridge_response_shim_unit_return_emits_nil() {
    // Explicit Unit return type must map to Nil in the ok branch.
    let method = make_method_with_types("ping", TypeRef::Unit, Some("MyError"));
    let trait_type = make_trait_type("MyTrait", vec![method]);
    let bridge_cfg = make_bridge_cfg("MyTrait", "register_my_trait");
    let my_error = ErrorDef {
        name: "MyError".into(),
        rust_path: "demo::MyError".into(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![my_error],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("Result(Nil, MyError)"),
        "Unit return must become Nil in result ok branch: {content}"
    );
}

#[test]
fn trait_bridge_multiple_methods_emit_one_shim_each() {
    // Two methods on one trait → two distinct response shims.
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![
            make_method_with_types("process_image", TypeRef::String, Some("OcrError")),
            make_method_with_types("get_name", TypeRef::String, None),
        ],
    );
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub fn ocr_backend_process_image_response("),
        "missing process_image shim: {content}"
    );
    assert!(
        content.contains("pub fn ocr_backend_get_name_response("),
        "missing get_name shim: {content}"
    );

    // Each method gets exactly one pub fn shim declaration.
    assert_eq!(
        content.matches("pub fn ocr_backend_process_image_response(").count(),
        1,
        "process_image shim must appear exactly once: {content}"
    );
    assert_eq!(
        content.matches("pub fn ocr_backend_get_name_response(").count(),
        1,
        "get_name shim must appear exactly once: {content}"
    );
}

#[test]
fn trait_bridge_response_shim_includes_doc_comment() {
    // The per-method shim must include a doc comment with method name guidance.
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process_image")]);
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("/// Send the `process_image` response back to the Rustler reply-registry."),
        "missing method doc comment: {content}"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge: unregistration function
// ---------------------------------------------------------------------------

#[test]
fn trait_bridge_emits_unregistration_fn_when_configured() {
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process")]);
    let bridge_cfg = make_bridge_cfg_full(
        "OcrBackend",
        "register_ocr_backend",
        Some("unregister_ocr_backend"),
        None,
    );

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"unregister_ocr_backend\")"),
        "missing unregister @external annotation: {content}"
    );
    assert!(
        content.contains("pub fn unregister_ocr_backend(name: String) -> Result(Nil, String)"),
        "missing unregister fn signature: {content}"
    );
}

#[test]
fn trait_bridge_omits_unregistration_fn_when_not_configured() {
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process")]);
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        !content.contains("unregister_ocr_backend"),
        "unregister fn must not appear when unregister_fn is None: {content}"
    );
}

// ---------------------------------------------------------------------------
// Trait bridge: clear function
// ---------------------------------------------------------------------------

#[test]
fn trait_bridge_emits_clear_fn_when_configured() {
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process")]);
    let bridge_cfg = make_bridge_cfg_full("OcrBackend", "register_ocr_backend", None, Some("clear_ocr_backends"));

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("@external(erlang, \"Elixir.Demo.Native\", \"clear_ocr_backends\")"),
        "missing clear @external annotation: {content}"
    );
    assert!(
        content.contains("pub fn clear_ocr_backends() -> Result(Nil, String)"),
        "missing clear fn signature: {content}"
    );
}

#[test]
fn trait_bridge_omits_clear_fn_when_not_configured() {
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process")]);
    let bridge_cfg = make_bridge_cfg("OcrBackend", "register_ocr_backend");

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        !content.contains("clear_ocr_backends"),
        "clear fn must not appear when clear_fn is None: {content}"
    );
}

#[test]
fn trait_bridge_emits_all_three_fns_when_fully_configured() {
    let trait_type = make_trait_type("OcrBackend", vec![make_method("process")]);
    let bridge_cfg = make_bridge_cfg_full(
        "OcrBackend",
        "register_ocr_backend",
        Some("unregister_ocr_backend"),
        Some("clear_ocr_backends"),
    );

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub fn register_ocr_backend(pid: Dynamic, plugin_name: String) -> Nil"),
        "missing register fn: {content}"
    );
    assert!(
        content.contains("pub fn unregister_ocr_backend(name: String) -> Result(Nil, String)"),
        "missing unregister fn: {content}"
    );
    assert!(
        content.contains("pub fn clear_ocr_backends() -> Result(Nil, String)"),
        "missing clear fn: {content}"
    );
}

// ---------------------------------------------------------------------------
// Regression: a non-trait type that has methods must be emitted ONCE as an
// opaque resource. It must NOT also be emitted as a regular phantom/record
// type by the data-type emission pass (gleam rejects duplicate type defs).
// ---------------------------------------------------------------------------

#[test]
fn non_trait_type_with_methods_emits_opaque_resource_only_once() {
    let mut client = make_type("DefaultClient", vec![]);
    client.methods = vec![make_method("chat")];

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_nif("Elixir.Demo.Native");
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let normal_defs = content.matches("pub type DefaultClient").count();
    let opaque_defs = content.matches("pub opaque type DefaultClient").count();
    assert_eq!(opaque_defs, 1, "expected exactly one opaque emission: {content}");
    // `pub type` is a substring of `pub opaque type`? No — they're distinct prefixes.
    assert_eq!(
        normal_defs, 0,
        "non-trait type with methods must not be emitted as a regular type: {content}"
    );
}
