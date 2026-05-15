use alef_backend_swift::gen_rust_crate;
use alef_core::config::{ResolvedCrateConfig, TraitBridgeConfig, new_config::NewAlefConfig};
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
        serde_rename: None,
        serde_flatten: false,
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
        serde_untagged: false,
        serde_rename_all: None,

        is_copy: false,
        has_serde: false,
    }
}

fn make_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Enum types ARE declared as opaque `type T;` in their own extern block.
    // This is required so that the enum can be used as a function parameter
    // (e.g. `fn new(content: Status, ...)`); without the declaration swift-bridge
    // rejects any function whose signature mentions the enum.
    // Struct-field getters that return enum-typed fields still serialize to String
    // (via to_string()) rather than returning the opaque handle, so that the
    // swift-bridge Vec<T> Vectorizable conformance does not affect field access.
    assert!(
        lib.content.contains("type Status;"),
        "lib.rs must contain Status opaque type declaration: {}",
        lib.content
    );
    assert!(
        lib.content.contains("pub enum Status {"),
        "lib.rs missing Status wrapper enum: {}",
        lib.content
    );
    assert!(
        lib.content.contains("pub fn to_string"),
        "lib.rs missing Status to_string impl: {}",
        lib.content
    );
}

#[test]
fn lib_rs_struct_with_enum_field_returns_string() {
    // A struct with an enum-typed field must have that getter return `String`,
    // not the opaque enum wrapper type. The extern block must declare `fn foo(&self) -> String`
    // (not `fn foo(&self) -> Status`), and the wrapper impl must call Status::from(...).to_string().
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![{
            let mut t = make_type("Item", vec![make_field("status", TypeRef::Named("Status".to_string()))]);
            t.has_serde = true;
            t
        }],
        functions: vec![],
        enums: vec![make_enum("Status", vec!["Active", "Inactive"])],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Getter must be declared as String in the extern block.
    assert!(
        lib.content.contains("fn status(&self) -> String;"),
        "extern block must declare status() -> String, not the opaque enum type: {}",
        lib.content
    );
    // Wrapper impl must convert enum to String via to_string().
    assert!(
        lib.content.contains("Status::from(") && lib.content.contains(".to_string()"),
        "wrapper impl must call Status::from(...).to_string(): {}",
        lib.content
    );
    // Opaque enum type IS declared in its own extern block (required for parameter usage).
    // It must NOT appear inside the struct's extern block (where the getter returns String).
    assert!(
        lib.content.contains("type Status;"),
        "lib.rs must declare Status as opaque extern type (needed for param usage): {}",
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

fn config_with_bridge(trait_name: &str) -> ResolvedCrateConfig {
    let mut cfg = make_config();
    cfg.trait_bridges = vec![TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
    // Result-returning method must return a JSON envelope for swift-bridge transport.
    assert!(
        lib.content.contains(r#"format!("{{\"ok\": {}}}""#) && lib.content.contains(r#"format!("{{\"err\": {}}}""#),
        "async Result trampoline must emit JSON envelope: {}",
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
        generated_header: None,
        precommit: None,
        cargo: None,
    });

    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
fn cargo_toml_includes_serde_json_dep() {
    // serde_json is always required: the generated lib.rs may emit
    // `::serde_json::to_value(...)` / `::serde_json::from_value(...)` for types
    // that carry `has_serde: true` with Vec/Primitive fields (Codable propagation).
    // Without the dep the generated crate fails to compile with E0433.
    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo.content.contains("serde_json"),
        "Cargo.toml must list serde_json (required for Codable propagation); got:\n{}",
        cargo.content
    );
}

#[test]
fn cargo_toml_serde_json_dep_present_when_has_serde_type_with_vec_field() {
    // Specifically reproduce the kreuzberg bug: a type with has_serde=true and a
    // Vec field causes the generator to emit ::serde_json::to_value calls in the
    // wrapper impl, but the dep was missing from Cargo.toml → E0433 compile error.
    let serde_type = TypeDef {
        name: "DeviceInfo".to_string(),
        rust_path: "demo::DeviceInfo".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "device_id".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
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
            serde_rename: None,
            serde_flatten: false,
        }],
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
        has_serde: true, // triggers ::serde_json::to_value path in wrappers.rs
        super_traits: vec![],
    };

    let api = ApiSurface {
        crate_name: "my-lib".into(),
        version: "0.1.0".into(),
        types: vec![serde_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let cargo = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The lib.rs must emit the serde_json call for this case.
    assert!(
        lib.content.contains("serde_json"),
        "lib.rs should emit serde_json calls for has_serde type with Vec field; got:\n{}",
        lib.content
    );
    // The Cargo.toml must declare the dep so the crate compiles.
    assert!(
        cargo.content.contains("serde_json"),
        "Cargo.toml must list serde_json when lib.rs emits serde_json calls; got:\n{}",
        cargo.content
    );
}

// ---------------------------------------------------------------------------
// gen_unregistration_fn / gen_clear_fn tests
// ---------------------------------------------------------------------------

fn config_with_full_bridge(
    trait_name: &str,
    unregister_fn: Option<&str>,
    clear_fn: Option<&str>,
) -> ResolvedCrateConfig {
    let mut cfg = make_config();
    cfg.trait_bridges = vec![TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: None,
        registry_getter: Some("demo::plugins::registry::get_test_registry".to_string()),
        register_fn: Some("register_test_plugin".to_string()),
        unregister_fn: unregister_fn.map(str::to_string),
        clear_fn: clear_fn.map(str::to_string),
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: alef_core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    cfg
}

fn make_minimal_trait_api(trait_name: &str) -> ApiSurface {
    let trait_def = make_trait_type(trait_name, &format!("demo::{trait_name}"), vec![]);
    ApiSurface {
        crate_name: "demo-crate".into(),
        version: "0.1.0".into(),
        types: vec![trait_def],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    }
}

/// When `unregister_fn` and `clear_fn` are both configured, the generated lib.rs
/// must contain both functions with the configured names and the correct signatures.
#[test]
fn trait_bridge_unregister_and_clear_fns_emitted_when_both_configured() {
    let api = make_minimal_trait_api("Analyzer");
    let cfg = config_with_full_bridge("Analyzer", Some("unregister_analyzer"), Some("clear_analyzers"));

    let files = gen_rust_crate::emit(&api, &cfg).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // unregister_fn must be present with the configured name and String arg.
    assert!(
        lib.content
            .contains("pub fn unregister_analyzer(name: String) -> Result<(), String>"),
        "lib.rs must contain unregister_analyzer signature; got:\n{}",
        lib.content
    );
    // unregister body must call the registry getter.
    assert!(
        lib.content.contains("demo::plugins::registry::get_test_registry()"),
        "unregister_analyzer body must call registry getter; got:\n{}",
        lib.content
    );

    // clear_fn must be present with the configured name and no args.
    assert!(
        lib.content.contains("pub fn clear_analyzers() -> Result<(), String>"),
        "lib.rs must contain clear_analyzers signature; got:\n{}",
        lib.content
    );
    // clear body must also call the registry getter.
    assert!(
        lib.content.contains("pub fn clear_analyzers() -> Result<(), String>"),
        "clear_analyzers body must be emitted; got:\n{}",
        lib.content
    );

    // Both names must appear in the extern "Rust" block for swift-bridge visibility.
    assert!(
        lib.content
            .contains("fn unregister_analyzer(name: String) -> Result<(), String>;"),
        "extern Rust block must declare unregister_analyzer; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn clear_analyzers() -> Result<(), String>;"),
        "extern Rust block must declare clear_analyzers; got:\n{}",
        lib.content
    );
}

/// When `unregister_fn` and `clear_fn` are both `None`, neither function must
/// appear in the generated lib.rs.
#[test]
fn trait_bridge_no_unregister_or_clear_when_both_none() {
    let api = make_minimal_trait_api("Analyzer");
    let cfg = config_with_full_bridge("Analyzer", None, None);

    let files = gen_rust_crate::emit(&api, &cfg).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("unregister_"),
        "lib.rs must not emit any unregister fn when unregister_fn is None; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("clear_"),
        "lib.rs must not emit any clear fn when clear_fn is None; got:\n{}",
        lib.content
    );
}

// ── streaming adapter bridge function tests ───────────────────────────────────

/// When a `[[crates.adapters]]` entry has `pattern = "streaming"` and
/// `owner_type` set to an opaque-handle type, the generated `lib.rs` must:
///
/// 1. Declare the opaque `{Owner}{Adapter}StreamHandle` type plus its `_start`
///    free function and `next` method inside the `#[swift_bridge::bridge]` module.
/// 2. Emit a concrete Rust handle struct + `_start` free function (returning
///    `Result<Handle, String>`) + `impl Handle { fn next(&mut self) -> Result<String, String> }`
///    that drives the underlying stream and JSON-encodes each chunk.
///
/// These three pieces back the host-side `AsyncThrowingStream<Item, Error>` wrapper
/// emitted by `gen_bindings::emit_streaming_client_method`. swift-bridge generates the
/// matching Swift `class` shadow with `deinit` so no explicit `_free` is needed.
#[test]
fn streaming_adapter_emits_extern_block_and_rust_shim() {
    use alef_core::ir::ReceiverKind;

    let config = {
        let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo_lib"
sources = ["src/lib.rs"]

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "demo_lib::chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"
error_type = "DemoError"

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#;
        let cfg: alef_core::config::new_config::NewAlefConfig = toml::from_str(toml).expect("test config must parse");
        cfg.resolve().expect("test config must resolve").remove(0)
    };

    let api = ApiSurface {
        crate_name: "demo_lib".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "demo_lib::DefaultClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "chat".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: true,
                is_static: false,
                error_type: Some("DemoError".to_string()),
                doc: String::new(),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                receiver: Some(ReceiverKind::Ref),
                trait_source: None,
                has_default_impl: false,
            }],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // 1. The extern "Rust" block must declare the opaque handle type and the
    //    `_start` free function that returns it.
    assert!(
        lib.content.contains("type DefaultClientChatStreamStreamHandle;"),
        "extern block must declare DefaultClientChatStreamStreamHandle; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn default_client_chat_stream_start("),
        "extern block must declare default_client_chat_stream_start; got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("Result<DefaultClientChatStreamStreamHandle, String>"),
        "_start must return Result<Handle, String>; got:\n{}",
        lib.content
    );
    // _next must appear as a method on the handle in the extern "Rust" block.
    assert!(
        lib.content
            .contains("fn next(self: &mut DefaultClientChatStreamStreamHandle) -> Result<String, String>"),
        "extern block must declare `next(&mut self) -> Result<String, String>`; got:\n{}",
        lib.content
    );
    // The Swift wrapper references `defaultClientChatStreamStart`, so the
    // camelCased swift_name must be emitted.
    assert!(
        lib.content.contains("defaultClientChatStreamStart"),
        "extern block must set swift_name = \"defaultClientChatStreamStart\"; got:\n{}",
        lib.content
    );

    // 2. The concrete Rust struct + functions must be emitted.
    assert!(
        lib.content.contains("pub struct DefaultClientChatStreamStreamHandle"),
        "lib.rs must emit a concrete DefaultClientChatStreamStreamHandle struct; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("pub fn default_client_chat_stream_start("),
        "lib.rs must emit a concrete _start function; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("impl DefaultClientChatStreamStreamHandle"),
        "lib.rs must emit an `impl` block defining `next` on the handle; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("pub fn next(&mut self)"),
        "handle impl must define `next(&mut self)`; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("tokio::runtime::Builder"),
        "stream shim must construct a Tokio runtime; got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains(".block_on("),
        "stream shim must call block_on; got:\n{}",
        lib.content
    );
    // EOF sentinel: empty-string return on clean stream end.
    assert!(
        lib.content.contains("Ok(String::new())"),
        "next() must return Ok(String::new()) on clean EOF; got:\n{}",
        lib.content
    );
}

/// When no adapters are configured (or none with pattern = streaming), no
/// extra extern blocks or shims must be emitted.
/// Methods with `ReceiverKind::RefMut` on an opaque type must emit `client: &mut TypeName`
/// in both the `extern "Rust"` block declaration and the Rust free-function shim,
/// so that `client.0.method()` can borrow the inner value mutably.
#[test]
fn opaque_type_refmut_method_emits_mut_receiver_in_extern_and_shim() {
    let set_language_method = MethodDef {
        name: "set_language".to_string(),
        params: vec![ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::RefMut),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Parser".to_string(),
            rust_path: "demo::Parser".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![set_language_method],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The extern "Rust" block must declare the method with &mut receiver.
    assert!(
        lib.content.contains("client: &mut Parser"),
        "extern block and shim must use &mut Parser for RefMut method; got:\n{}",
        lib.content
    );
    // The pub fn shim must also take &mut.
    assert!(
        lib.content.contains("pub fn parser_set_language(client: &mut Parser"),
        "shim must declare `client: &mut Parser` for RefMut receiver; got:\n{}",
        lib.content
    );
    // Must NOT use an immutable reference for the RefMut method.
    assert!(
        !lib.content.contains("pub fn parser_set_language(client: &Parser"),
        "shim must not use immutable &Parser for a RefMut receiver; got:\n{}",
        lib.content
    );
}

#[test]
fn no_streaming_adapters_emits_no_extra_blocks() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The only extern block present should be the module declaration itself.
    // A bare config with no adapters must not mention "chat_stream" or streaming symbols.
    assert!(
        !lib.content.contains("chat_stream"),
        "lib.rs must not contain streaming symbols when no adapters configured; got:\n{}",
        lib.content
    );
}

fn make_simple_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
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

fn make_opaque_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,
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

/// `Option<Named(T)>` return types on opaque-type methods must NOT be wrapped with
/// `serde_json::to_string` — they should pass through as `Option<T>` using `.map(T)`.
/// This is the Class A fix: capsule_types / handle-returned-type passthrough.
#[test]
fn option_named_return_on_method_uses_map_not_serde_json() {
    // Build a Node type with a `parent()` method returning Option<Node>.
    let parent_method = make_simple_method(
        "parent",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("Node".to_string()))),
    );
    let child_method = make_simple_method(
        "child",
        vec![ParamDef {
            name: "index".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }],
        TypeRef::Optional(Box::new(TypeRef::Named("Node".to_string()))),
    );

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_opaque_type("Node", vec![parent_method, child_method])],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Must use .map(Node) not serde_json::to_string for Option<Node> returns.
    assert!(
        lib.content.contains(".map(Node)"),
        "Option<Named> return must use .map(T) not serde_json; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("serde_json::to_string(&(client.0.parent()))"),
        "parent() must not serialize Option<Node> via serde_json; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("serde_json::to_string(&(client.0.child("),
        "child() must not serialize Option<Node> via serde_json; got:\n{}",
        lib.content
    );
    // The extern block must declare Option<Node> not String for these methods.
    assert!(
        lib.content.contains("fn node_parent(client: &Node) -> Option<Node>"),
        "extern block must declare node_parent -> Option<Node>, not String; got:\n{}",
        lib.content
    );
}

/// Method with a `Bytes` param where `is_ref = true` must pass `&name` (not `name`)
/// so the core method receives `&[u8]` instead of `Vec<u8>`.
/// This is Class B bug 1.
#[test]
fn bytes_ref_param_on_method_passes_borrowed_slice() {
    let parse_bytes_method = {
        let mut m = make_simple_method(
            "parse_bytes",
            vec![ParamDef {
                name: "source".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            TypeRef::Optional(Box::new(TypeRef::Named("Tree".to_string()))),
        );
        m.receiver = Some(ReceiverKind::RefMut);
        m
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            make_opaque_type("Tree", vec![]),
            make_opaque_type("Parser", vec![parse_bytes_method]),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("client.0.parse_bytes(&source)"),
        "Bytes+is_ref param must pass &source; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("client.0.parse_bytes(source)"),
        "Bytes+is_ref param must NOT pass owned source; got:\n{}",
        lib.content
    );
}

/// Method with a `Path` param (not is_ref) must call `PathBuf::from(name)` not `&name`.
/// This is Class B bug 2.
#[test]
fn path_param_on_method_converts_to_pathbuf() {
    let add_dir_method = make_simple_method(
        "add_extra_libs_dir",
        vec![ParamDef {
            name: "dir".to_string(),
            ty: TypeRef::Path,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }],
        TypeRef::Unit,
    );

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_opaque_type("Registry", vec![add_dir_method])],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("::std::path::PathBuf::from(dir)"),
        "Path param must convert via PathBuf::from; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("add_extra_libs_dir(&dir)"),
        "Path param must NOT pass &dir (that yields &String not PathBuf); got:\n{}",
        lib.content
    );
}

/// Method with a `Named` param where `is_ref = true` must pass `&name.0` not `name.0`.
/// This is Class B bug 3.
#[test]
fn named_ref_param_on_method_passes_borrow_of_inner() {
    let process_method = {
        let mut m = make_simple_method(
            "process",
            vec![
                ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ProcessConfig".to_string()),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
            ],
            TypeRef::Named("ProcessResult".to_string()),
        );
        m.error_type = Some("Error".to_string());
        m
    };

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            make_type("ProcessConfig", vec![]),
            make_type("ProcessResult", vec![]),
            make_opaque_type("Registry", vec![process_method]),
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("&config.0"),
        "Named+is_ref param must pass &config.0; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains(".process(&source, config.0)"),
        "Named+is_ref param must NOT pass owned config.0; got:\n{}",
        lib.content
    );
}

/// Method with a `Vec<String>` param where `is_ref = true` must emit
/// `&name.iter().map(|s| s.as_str()).collect::<Vec<_>>()` so the core
/// method receives `&[&str]` instead of `Vec<String>`.
/// This is Class B bug 4.
#[test]
fn vec_string_ref_param_on_method_converts_to_str_slice() {
    let ensure_languages_method = make_simple_method(
        "ensure_languages",
        vec![ParamDef {
            name: "names".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }],
        TypeRef::Unit,
    );

    let mut m = ensure_languages_method;
    m.error_type = Some("Error".to_string());

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_opaque_type("Downloader", vec![m])],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let files = gen_rust_crate::emit(&api, &make_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("&names.iter().map(|s| s.as_str()).collect::<Vec<_>>()"),
        "Vec<String>+is_ref param must convert to &[&str]; got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("ensure_languages(names)"),
        "Vec<String>+is_ref must NOT pass owned names directly; got:\n{}",
        lib.content
    );
}
