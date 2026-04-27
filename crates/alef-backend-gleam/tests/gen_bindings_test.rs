use alef_backend_gleam::GleamBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, GleamConfig, TraitBridgeConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef,
    ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
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
    }
}

fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "demo".to_string(),
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
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
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
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
    format: ::alef_core::config::FormatConfig::default(),
    format_overrides: ::std::collections::HashMap::new(),
    }
}

fn make_config_with_nif(nif_module: &str) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "demo".to_string(),
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
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: Some(GleamConfig {
            app_name: None,
            nif_module: Some(nif_module.to_string()),
            features: None,
            serde_rename_all: None,
            rename_fields: std::collections::HashMap::new(),
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
        }),
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
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
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
    format: ::alef_core::config::FormatConfig::default(),
    format_overrides: ::std::collections::HashMap::new(),
    }
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
        }],
        enums: vec![],
        errors: vec![],
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
                is_tuple: false,
                },
                EnumVariant {
                    name: "Inactive".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
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
                    doc: String::new(),
                },
                ErrorVariant {
                    name: "InvalidInput".into(),
                    message_template: None,
                    fields: vec![make_field("details", TypeRef::String, false)],
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    doc: String::new(),
                },
            ],
            doc: String::new(),
        }],
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
            is_tuple: false,
            }],
            doc: String::new(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
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
        }],
        enums: vec![],
        errors: vec![],
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
    }
}

fn make_bridge_cfg(trait_name: &str, register_fn: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some(format!("demo::get_{}_registry", trait_name.to_lowercase())),
        register_fn: Some(register_fn.to_string()),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
    }
}

fn make_config_with_bridges(bridges: Vec<TraitBridgeConfig>) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "demo".to_string(),
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
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: None,
        gleam: None,
        go: None,
        java: None,
        kotlin: None,
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
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
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: bridges,
        tools: alef_core::config::ToolsConfig::default(),
        format: ::alef_core::config::FormatConfig::default(),
        format_overrides: ::std::collections::HashMap::new(),
    }
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
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The NIF shim name follows `{trait_snake}_{method_snake}_response`.
    assert!(
        content.contains(
            "@external(erlang, \"Elixir.Demo.Native\", \"ocr_backend_process_image_response\")"
        ),
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
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![ocr_error],
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
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![my_error],
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
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let files = GleamBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("/// Send the `process_image` response back to the Rustler reply-registry."),
        "missing method doc comment: {content}"
    );
}
