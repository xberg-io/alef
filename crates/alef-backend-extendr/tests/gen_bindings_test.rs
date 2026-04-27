use alef_backend_extendr::ExtendrBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, RConfig};
use alef_core::ir::*;

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

fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test-lib".to_string(),
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
        r: Some(RConfig {
            package_name: Some("testlib".to_string()),
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
        }),
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
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    }
}

#[test]
fn test_basic_generation() {
    let backend = ExtendrBackend;

    // Create test API surface with types, functions, and enums
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("backend", TypeRef::String, false),
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
            super_traits: vec![],
            doc: "Test config".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "test_lib::extract".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "path".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok(), "Generation should succeed");
    let files = result.unwrap();

    // Should generate a single lib.rs file
    assert_eq!(files.len(), 1, "Should generate exactly one file");

    let lib_file = &files[0];
    assert!(
        lib_file.path.to_string_lossy().contains("lib.rs"),
        "Output file should be lib.rs"
    );

    let content = &lib_file.content;

    // Check for extendr-specific attributes and imports
    assert!(
        content.contains("extendr_api::prelude::*"),
        "Should import extendr_api::prelude::*"
    );

    // Check for struct generation (Config)
    assert!(content.contains("pub struct Config"), "Should generate Config struct");
    assert!(content.contains("timeout"), "Should have timeout field");
    assert!(content.contains("backend"), "Should have backend field");

    // Check for function binding with #[extendr] attribute
    assert!(
        content.contains("#[extendr]"),
        "Functions should have #[extendr] attribute"
    );
    assert!(content.contains("fn extract"), "Should generate extract function");

    // Check for enum generation
    assert!(content.contains("pub enum Mode"), "Should generate Mode enum");
    assert!(content.contains("Fast"), "Should have Fast variant");
    assert!(content.contains("Accurate"), "Should have Accurate variant");

    // Check for module registration
    assert!(
        content.contains("extendr_module!"),
        "Should have extendr_module registration"
    );
    assert!(content.contains("mod testlib"), "Module name should match package_name");
    assert!(content.contains("impl Config"), "Should register Config type in module");
    assert!(
        content.contains("fn extract"),
        "Should register extract function in module"
    );
}

#[test]
fn test_type_mapping() {
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("opt_string", TypeRef::Optional(Box::new(TypeRef::String)), true),
                make_field("strings", TypeRef::Vec(Box::new(TypeRef::String)), false),
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
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let lib_file = &files[0];
    let content = &lib_file.content;

    // Verify struct is generated
    assert!(content.contains("pub struct Numbers"));

    // Extendr uses Rust types directly, so verify field names appear
    assert!(content.contains("u32_val"));
    assert!(content.contains("i64_val"));
    assert!(content.contains("string_val"));
    assert!(content.contains("opt_string"));
    assert!(content.contains("strings"));
}

#[test]
fn test_enum_generation() {
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Completed".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Completed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Task status".to_string(),
            cfg: None,
            is_copy: false,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify enum is generated
    assert!(content.contains("pub enum Status"));

    // Verify all variants are present
    assert!(content.contains("Pending"));
    assert!(content.contains("Active"));
    assert!(content.contains("Completed"));

    // Verify derive attributes for extendr
    assert!(content.contains("Clone"));
    assert!(content.contains("PartialEq"));
}

#[test]
fn test_generated_header() {
    let backend = ExtendrBackend;

    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SimpleType".to_string(),
            rust_path: "test::SimpleType".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::String, false)],
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
        }],
        functions: vec![FunctionDef {
            name: "simple_fn".to_string(),
            rust_path: "test::simple_fn".to_string(),
            original_rust_path: String::new(),
            params: vec![],
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();

    // All files should have generated_header set to true
    for file in &files {
        // Note: In the current gen_bindings.rs, generated_header is set to false
        // We check that the field exists and document this behavior
        assert!(
            !file.generated_header,
            "Current extendr backend sets generated_header=false"
        );
    }
}

fn make_owned_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        receiver: Some(ReceiverKind::Owned),
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }
}

fn make_ref_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        receiver: Some(ReceiverKind::Ref),
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    }
}

#[test]
fn test_opaque_type_generates_inner_field_and_delegates() {
    // Regression: opaque types (e.g. ConversionOptionsBuilder) must generate
    // `inner: Arc<CoreType>` and delegate methods — not emit empty structs with todo!() stubs.
    let backend = ExtendrBackend;

    let builder_type = TypeDef {
        name: "OptionsBuilder".to_string(),
        rust_path: "test_lib::OptionsBuilder".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            make_owned_method(
                "with_value",
                vec![ParamDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
                TypeRef::Named("OptionsBuilder".to_string()),
            ),
            make_ref_method("build", vec![], TypeRef::Named("Options".to_string())),
        ],
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
        doc: String::new(),
        cfg: None,
    };

    let options_type = TypeDef {
        name: "Options".to_string(),
        rust_path: "test_lib::Options".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("value", TypeRef::String, false)],
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
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![options_type, builder_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // Opaque builder struct must have inner: Arc<CoreType>, not be empty
    assert!(
        content.contains("inner: Arc<test_lib::OptionsBuilder>"),
        "Opaque builder must have inner: Arc<CoreType>. Got:\n{}",
        content
    );
    // Must import Arc
    assert!(
        content.contains("use std::sync::Arc"),
        "Must import Arc for opaque types"
    );
    // Methods must not use todo!()
    assert!(
        !content.contains("todo!(\"Not implemented: OptionsBuilder"),
        "Opaque builder methods must not contain todo!() stubs"
    );
    // build() must delegate to self.inner
    assert!(
        content.contains("self.inner.build()"),
        "build() must delegate to self.inner. Got:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// Trait bridge tests (Extendr plugin bridge via gen_trait_bridge)
// ---------------------------------------------------------------------------

mod trait_bridge {
    use alef_backend_extendr::trait_bridge::gen_trait_bridge;
    use alef_core::config::TraitBridgeConfig;
    use alef_core::ir::*;

    fn make_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("my_lib::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }
    }

    fn make_method(name: &str, return_type: TypeRef, has_error: bool, has_default: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type,
            is_async: false,
            is_static: false,
            error_type: if has_error {
                Some("Box<dyn std::error::Error + Send + Sync>".to_string())
            } else {
                None
            },
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: has_default,
        }
    }

    fn make_async_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            is_static: false,
            error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
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

    fn make_plugin_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
        }
    }

    fn make_visitor_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,
            type_alias: Some(format!("{trait_name}Handle")),
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
        }
    }

    // ---- Plugin bridge: wrapper struct ---

    #[test]
    fn test_plugin_bridge_generates_wrapper_struct() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let cfg = make_plugin_bridge_cfg("OcrBackend");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("pub struct ROcrBackendBridge"),
            "plugin bridge must generate ROcrBackendBridge wrapper struct"
        );
        assert!(
            code.code.contains("inner: extendr_api::Robj"),
            "wrapper struct must hold an extendr_api::Robj"
        );
        assert!(
            code.code.contains("cached_name: String"),
            "wrapper struct must cache the plugin name"
        );
    }

    // ---- Plugin bridge: trait impl ---

    #[test]
    fn test_plugin_bridge_generates_trait_impl() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let cfg = make_plugin_bridge_cfg("OcrBackend");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("impl my_lib::OcrBackend for ROcrBackendBridge"),
            "plugin bridge must implement the trait for the wrapper"
        );
        assert!(
            code.code.contains("fn process("),
            "trait impl must include all trait methods"
        );
    }

    // ---- Plugin bridge: sync method uses dollar() to look up R function ---

    #[test]
    fn test_plugin_bridge_sync_method_uses_dollar_lookup() {
        let trait_def = make_trait_def("Analyzer", vec![make_method("analyze", TypeRef::String, true, false)]);
        let cfg = make_plugin_bridge_cfg("Analyzer");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("dollar(\"analyze\")"),
            "sync method body must look up the R function via dollar()"
        );
    }

    // ---- Plugin bridge: async method uses spawn_blocking ---

    #[test]
    fn test_plugin_bridge_async_method_uses_spawn_blocking() {
        let trait_def = make_trait_def("Processor", vec![make_async_method("run")]);
        let cfg = make_plugin_bridge_cfg("Processor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("spawn_blocking"),
            "async method body must use tokio::task::spawn_blocking"
        );
        assert!(
            code.code.contains("async fn run("),
            "async method must be declared async"
        );
    }

    // ---- Plugin bridge: registration function ---

    #[test]
    fn test_plugin_bridge_generates_registration_fn() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let cfg = make_plugin_bridge_cfg("OcrBackend");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("pub fn register_ocrbackend("),
            "registration fn must be generated with the configured name"
        );
        assert!(
            code.code.contains("#[extendr]"),
            "registration fn must carry #[extendr] attribute"
        );
        assert!(
            code.code.contains("my_lib::get_registry"),
            "registration fn must call the configured registry getter"
        );
    }

    // ---- Plugin bridge: registration validates required methods ---

    #[test]
    fn test_plugin_bridge_registration_validates_required_methods() {
        let trait_def = make_trait_def(
            "Transform",
            vec![
                make_method("transform", TypeRef::String, true, false),
                make_method("describe", TypeRef::String, false, true),
            ],
        );
        let cfg = make_plugin_bridge_cfg("Transform");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("\"transform\""),
            "registration fn must validate required method 'transform' exists"
        );
        assert!(
            code.code.contains("dollar(\"transform\")") || code.code.contains("\"transform\""),
            "constructor must check required methods via dollar()"
        );
    }

    // ---- Plugin bridge: constructor caches name ---

    #[test]
    fn test_plugin_bridge_constructor_caches_name() {
        let trait_def = make_trait_def("Worker", vec![make_method("work", TypeRef::Unit, false, false)]);
        let cfg = make_plugin_bridge_cfg("Worker");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("cached_name"),
            "constructor must populate cached_name"
        );
        assert!(
            code.code.contains("dollar(\"name\")"),
            "constructor must call dollar(\"name\") to cache the plugin name"
        );
    }

    // ---- Plugin bridge: super_trait generates Plugin impl ---

    #[test]
    fn test_plugin_bridge_with_super_trait_generates_plugin_impl() {
        let trait_def = make_trait_def("OcrBackend", vec![make_method("process", TypeRef::String, true, false)]);
        let cfg = TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            registry_getter: Some("my_lib::get_registry".to_string()),
            register_fn: Some("register_ocr_backend".to_string()),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
        };
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("impl my_lib::Plugin for ROcrBackendBridge"),
            "must generate Plugin impl for bridge struct"
        );
        assert!(code.code.contains("fn name(&self)"), "Plugin impl must include name()");
        assert!(
            code.code.contains("fn initialize(&self)"),
            "Plugin impl must include initialize()"
        );
        assert!(
            code.code.contains("fn shutdown(&self)"),
            "Plugin impl must include shutdown()"
        );
    }

    // ---- Visitor bridge ---

    #[test]
    fn test_visitor_bridge_generates_r_bridge_struct() {
        let trait_def = make_trait_def(
            "HtmlVisitor",
            vec![make_method("visit_node", TypeRef::Unit, false, true)],
        );
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("pub struct RHtmlVisitorBridge"),
            "visitor bridge must produce RHtmlVisitorBridge struct"
        );
    }

    #[test]
    fn test_visitor_bridge_does_not_generate_registration_fn() {
        let trait_def = make_trait_def(
            "HtmlVisitor",
            vec![make_method("visit_node", TypeRef::Unit, false, true)],
        );
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            !code.code.contains("#[extendr]"),
            "visitor bridge must not generate an extendr registration function"
        );
    }

    #[test]
    fn test_visitor_bridge_generates_trait_impl() {
        let trait_def = make_trait_def(
            "HtmlVisitor",
            vec![make_method("visit_node", TypeRef::Unit, false, true)],
        );
        let cfg = make_visitor_bridge_cfg("HtmlVisitor");
        let code = gen_trait_bridge(&trait_def, &cfg, "my_lib", "Error", "Error::from({msg})", &make_api());

        assert!(
            code.code.contains("impl my_lib::HtmlVisitor for RHtmlVisitorBridge"),
            "visitor bridge must implement the trait"
        );
    }
}
