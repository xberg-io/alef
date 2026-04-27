use alef_backend_swift::gen_rust_crate;
use alef_core::config::{AlefConfig, CrateConfig, TraitBridgeConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType,
    ReceiverKind, TypeDef, TypeRef,
};
use alef_core::template_versions;

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
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

fn make_enum(name: &str, variants: Vec<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .into_iter()
            .map(|v| EnumVariant {
                name: v.to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                is_tuple: false,
            })
            .collect(),
        doc: String::new(),
        cfg: None,
        serde_tag: None,
        serde_rename_all: None,

        is_copy: false,
        has_serde: false,
    }
}

fn make_config() -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "demo-crate".to_string(),
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

// ── Cargo.toml tests ──────────────────────────────────────────────────────────

#[test]
fn cargo_toml_contains_swift_bridge_version() {
    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "1.2.3".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let expected_bridge = template_versions::cargo::SWIFT_BRIDGE;
    let expected_build = template_versions::cargo::SWIFT_BRIDGE_BUILD;

    assert!(
        cargo.content.contains(&format!("swift-bridge = \"{expected_bridge}\"")),
        "Cargo.toml missing swift-bridge version: {}",
        cargo.content
    );
    assert!(
        cargo
            .content
            .contains(&format!("swift-bridge-build = \"{expected_build}\"")),
        "Cargo.toml missing swift-bridge-build version: {}",
        cargo.content
    );
}

#[test]
fn cargo_toml_contains_crate_name_and_version() {
    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.5.1".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo.content.contains("name = \"my-lib-swift\""),
        "Cargo.toml missing package name: {}",
        cargo.content
    );
    assert!(
        cargo.content.contains("version = \"0.5.1\""),
        "Cargo.toml missing version: {}",
        cargo.content
    );
}

#[test]
fn cargo_toml_has_cdylib_and_staticlib() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo.content.contains("\"cdylib\""),
        "Cargo.toml missing cdylib: {}",
        cargo.content
    );
    assert!(
        cargo.content.contains("\"staticlib\""),
        "Cargo.toml missing staticlib: {}",
        cargo.content
    );
}

// ── lib.rs tests ──────────────────────────────────────────────────────────────

#[test]
fn lib_rs_contains_bridge_module() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("#[swift_bridge::bridge]"),
        "lib.rs missing bridge attribute: {}",
        lib.content
    );
    assert!(
        lib.content.contains("mod ffi {"),
        "lib.rs missing ffi module: {}",
        lib.content
    );
}

#[test]
fn lib_rs_has_extern_rust_block_per_type() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            make_type(
                "Point",
                vec![
                    make_field("x_coord", TypeRef::Primitive(PrimitiveType::I64)),
                    make_field("y_coord", TypeRef::Primitive(PrimitiveType::I64)),
                ],
            ),
            make_type("Empty", vec![]),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("extern \"Rust\""),
        "lib.rs missing extern Rust block: {}",
        lib.content
    );
    assert!(
        lib.content.contains("type Point;"),
        "lib.rs missing Point type decl: {}",
        lib.content
    );
    assert!(
        lib.content.contains("type Empty;"),
        "lib.rs missing Empty type decl: {}",
        lib.content
    );
}

#[test]
fn lib_rs_type_has_constructor_and_getters() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Point",
            vec![
                make_field("x_coord", TypeRef::Primitive(PrimitiveType::I64)),
                make_field("y_coord", TypeRef::Primitive(PrimitiveType::I64)),
            ],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("#[swift_bridge(init)]"),
        "lib.rs missing init attribute: {}",
        lib.content
    );
    assert!(
        lib.content.contains("fn new("),
        "lib.rs missing constructor: {}",
        lib.content
    );
    assert!(
        lib.content.contains("fn x_coord("),
        "lib.rs missing x_coord getter: {}",
        lib.content
    );
    assert!(
        lib.content.contains("fn y_coord("),
        "lib.rs missing y_coord getter: {}",
        lib.content
    );
}

#[test]
fn lib_rs_has_free_function_shim() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "fetch_data".into(),
            rust_path: "demo::fetch_data".into(),
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

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn fetch_data("),
        "lib.rs missing fetch_data shim: {}",
        lib.content
    );
    // The shim should delegate to the source crate
    assert!(
        lib.content.contains("demo::fetch_data("),
        "lib.rs shim not delegating to source crate: {}",
        lib.content
    );
}

#[test]
fn lib_rs_async_function_blocks_on_tokio_runtime() {
    // swift-bridge v0.1.x has no `async` attribute or async-fn extern support
    // (the build script's parser rejects `#[swift_bridge(async)]`). Async
    // source functions are bridged via a sync wrapper that blocks on a tokio
    // current-thread runtime, so Swift sees a normal sync call.
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "load_async".into(),
            rust_path: "demo::load_async".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
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

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("#[swift_bridge(async)]"),
        "swift_bridge(async) is not a real attribute in v0.1.x: {}",
        lib.content
    );
    // The wrapper is sync — it spins up a tokio runtime and `block_on` the future.
    assert!(
        lib.content.contains("pub fn load_async("),
        "wrapper should be sync (block_on a tokio runtime): {}",
        lib.content
    );
    assert!(
        lib.content.contains("tokio::runtime::Builder"),
        "wrapper should construct a tokio runtime: {}",
        lib.content
    );
    assert!(
        lib.content.contains(".block_on("),
        "wrapper should call block_on on the future: {}",
        lib.content
    );
}

#[test]
fn lib_rs_result_function_has_map_err_chain() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "parse_input".into(),
            rust_path: "demo::parse_input".into(),
            original_rust_path: String::new(),
            params: vec![make_param("raw", TypeRef::String)],
            return_type: TypeRef::Primitive(PrimitiveType::I32),
            is_async: false,
            error_type: Some("ParseError".into()),
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

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains(".map_err(|e| e.to_string())"),
        "lib.rs missing map_err chain: {}",
        lib.content
    );
    assert!(
        lib.content.contains("Result<"),
        "lib.rs missing Result return type: {}",
        lib.content
    );
}

// ── build.rs tests ────────────────────────────────────────────────────────────

#[test]
fn build_rs_calls_parse_bridges() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();

    assert!(
        build.content.contains("swift_bridge_build::parse_bridges"),
        "build.rs missing parse_bridges call: {}",
        build.content
    );
    assert!(
        build.content.contains("OUT_DIR"),
        "build.rs missing OUT_DIR: {}",
        build.content
    );
}

// ── file count and path tests ─────────────────────────────────────────────────

#[test]
fn emit_returns_three_files() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    assert_eq!(files.len(), 3, "expected 3 generated files, got {}", files.len());

    let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with("Cargo.toml")),
        "missing Cargo.toml in {:?}",
        paths
    );
    assert!(
        paths.iter().any(|p| p.ends_with("src/lib.rs")),
        "missing src/lib.rs in {:?}",
        paths
    );
    assert!(
        paths.iter().any(|p| p.ends_with("build.rs")),
        "missing build.rs in {:?}",
        paths
    );
}

#[test]
fn lib_rs_has_generated_header_comment() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("// Generated by alef. Do not edit by hand."),
        "lib.rs missing generated header comment: {}",
        lib.content
    );
}

#[test]
fn lib_rs_has_wrapper_newtype_for_type() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "Point",
            vec![
                make_field("x_coord", TypeRef::Primitive(PrimitiveType::I64)),
                make_field("y_coord", TypeRef::Primitive(PrimitiveType::I64)),
            ],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Wrapper newtype should reference the source crate
    assert!(
        lib.content.contains("pub struct Point("),
        "lib.rs missing Point wrapper newtype: {}",
        lib.content
    );
    assert!(
        lib.content.contains("demo::Point"),
        "lib.rs wrapper not referencing source crate: {}",
        lib.content
    );
}

#[test]
fn lib_rs_enum_extern_block_and_wrapper() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![make_enum("Status", vec!["Active", "Inactive"])],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("type Status;"),
        "lib.rs missing Status extern type: {}",
        lib.content
    );
    assert!(
        lib.content.contains("pub enum Status {"),
        "lib.rs missing Status wrapper enum: {}",
        lib.content
    );
}

// ── trait bridge tests ────────────────────────────────────────────────────────

fn make_method(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    error_type: Option<&str>,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: error_type.map(|s| s.to_string()),
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

fn make_trait_type(name: &str, rust_path: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
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

fn config_with_bridge(trait_name: &str) -> AlefConfig {
    let mut cfg = make_config();
    cfg.trait_bridges = vec![TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
    }];
    cfg
}

/// Test 1: a trait with only sync unit methods emits the correct extern block and wrapper.
#[test]
fn trait_bridge_sync_unit_methods_emits_box_type_and_trampolines() {
    let trait_def = make_trait_type(
        "Validator",
        "demo::Validator",
        vec![
            make_method(
                "validate",
                vec![make_param("score", TypeRef::Primitive(PrimitiveType::F64))],
                TypeRef::Primitive(PrimitiveType::Bool),
                false,
                None,
            ),
            make_method("priority", vec![], TypeRef::Primitive(PrimitiveType::I32), false, None),
        ],
    );
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![trait_def],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let cfg = config_with_bridge("Validator");
    let files = gen_rust_crate::emit(&api, &cfg).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The extern block must declare the opaque box type.
    assert!(
        lib.content.contains("type ValidatorBox;"),
        "lib.rs missing ValidatorBox extern type: {}",
        lib.content
    );
    // Trampoline functions must appear in the extern block.
    assert!(
        lib.content.contains("fn validator_call_validate("),
        "lib.rs missing validator_call_validate trampoline: {}",
        lib.content
    );
    assert!(
        lib.content.contains("fn validator_call_priority("),
        "lib.rs missing validator_call_priority trampoline: {}",
        lib.content
    );
    // Wrapper struct must wrap Box<dyn Trait>.
    assert!(
        lib.content
            .contains("pub struct ValidatorBox(pub Box<dyn demo::Validator"),
        "lib.rs missing ValidatorBox wrapper struct: {}",
        lib.content
    );
    // Trampoline implementations must delegate to this.0.{method}.
    assert!(
        lib.content.contains("this.0.validate("),
        "lib.rs trampoline not delegating to this.0.validate: {}",
        lib.content
    );
    assert!(
        lib.content.contains("this.0.priority("),
        "lib.rs trampoline not delegating to this.0.priority: {}",
        lib.content
    );
}

/// Test 2: a trait with an async method emits a tokio block_on wrapper.
#[test]
fn trait_bridge_async_method_emits_block_on() {
    let trait_def = make_trait_type(
        "Processor",
        "demo::Processor",
        vec![make_method(
            "run",
            vec![make_param("input", TypeRef::String)],
            TypeRef::String,
            true,
            Some("MyError"),
        )],
    );
    let api = ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![trait_def],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let cfg = config_with_bridge("Processor");
    let files = gen_rust_crate::emit(&api, &cfg).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Extern block and wrapper struct.
    assert!(
        lib.content.contains("type ProcessorBox;"),
        "lib.rs missing ProcessorBox extern type: {}",
        lib.content
    );
    assert!(
        lib.content
            .contains("pub struct ProcessorBox(pub Box<dyn demo::Processor"),
        "lib.rs missing ProcessorBox wrapper struct: {}",
        lib.content
    );
    // Async method trampoline must use tokio block_on.
    assert!(
        lib.content.contains("tokio::runtime::Builder"),
        "async trait method trampoline must use tokio runtime: {}",
        lib.content
    );
    assert!(
        lib.content.contains(".block_on("),
        "async trait method trampoline must call block_on: {}",
        lib.content
    );
    // Result-returning method must have map_err.
    assert!(
        lib.content.contains("map_err(|e| e.to_string())"),
        "async Result trampoline must have map_err: {}",
        lib.content
    );
}

#[test]
fn cargo_toml_has_license_field() {
    use alef_core::config::ScaffoldConfig;

    let mut config = make_config();
    config.scaffold = Some(ScaffoldConfig {
        description: Some("Demo library".to_string()),
        license: Some("Apache-2.0".to_string()),
        repository: None,
        homepage: None,
        authors: vec![],
        keywords: vec![],
    });

    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &config).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo.content.contains("license = \"Apache-2.0\""),
        "Cargo.toml must include license field; got:\n{}",
        cargo.content
    );
}

#[test]
fn cargo_toml_license_defaults_to_mit_when_scaffold_absent() {
    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo.content.contains("license = \"MIT\""),
        "Cargo.toml must default license to MIT when scaffold config is absent; got:\n{}",
        cargo.content
    );
}

#[test]
fn cargo_toml_does_not_include_serde_json() {
    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        !cargo.content.contains("serde_json"),
        "Cargo.toml must not list serde_json (unused dep); got:\n{}",
        cargo.content
    );
}
