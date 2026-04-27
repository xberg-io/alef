use alef_backend_dart::DartBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, DartConfig, DartStyle};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef,
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

fn make_function(name: &str, params: Vec<ParamDef>, return_type: TypeRef, error_type: Option<String>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        doc: String::new(),
        error_type,
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

fn make_empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        enums: vec![],
        errors: vec![],
        functions: vec![],
    }
}

fn make_config_ffi() -> AlefConfig {
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
        dart: Some(DartConfig {
            style: DartStyle::Ffi,
            ..DartConfig::default()
        }),
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

/// The generated FFI file contains a platform-aware `_libraryPath()` helper
/// with branches for macOS, Windows, and Linux.
#[test]
fn ffi_emits_library_load_helper_with_platform_branching() {
    let api = make_empty_api();
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files.iter().find(|f| f.path.to_string_lossy().ends_with("_ffi.dart")).unwrap();

    assert!(ffi_file.content.contains("DynamicLibrary.open(_libraryPath())"), "missing DynamicLibrary.open");
    assert!(ffi_file.content.contains("Platform.isMacOS"), "missing macOS branch");
    assert!(ffi_file.content.contains("Platform.isWindows"), "missing Windows branch");
    assert!(ffi_file.content.contains(".dylib"), "missing dylib extension");
    assert!(ffi_file.content.contains(".dll"), "missing dll extension");
    assert!(ffi_file.content.contains(".so"), "missing so extension");
}

/// Each IR function gets a `lookupFunction` call with the correct C symbol name.
#[test]
fn each_function_gets_lookup_function_call() {
    let api = ApiSurface {
        functions: vec![
            make_function("process_text", vec![make_param("input", TypeRef::String)], TypeRef::String, None),
            make_function("get_version", vec![], TypeRef::String, None),
        ],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files.iter().find(|f| f.path.to_string_lossy().ends_with("_ffi.dart")).unwrap();

    assert!(
        ffi_file.content.contains("lookupFunction"),
        "missing lookupFunction calls"
    );
    assert!(
        ffi_file.content.contains("demo_crate_process_text"),
        "missing C symbol for process_text"
    );
    assert!(
        ffi_file.content.contains("demo_crate_get_version"),
        "missing C symbol for get_version"
    );
}

/// String parameters are marshalled via `toNativeUtf8()` and freed with `calloc.free`.
#[test]
fn string_params_marshal_via_to_native_utf8_and_calloc_free() {
    let api = ApiSurface {
        functions: vec![make_function(
            "convert",
            vec![make_param("text", TypeRef::String)],
            TypeRef::Unit,
            None,
        )],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files.iter().find(|f| f.path.to_string_lossy().ends_with("_ffi.dart")).unwrap();

    assert!(
        ffi_file.content.contains("toNativeUtf8()"),
        "missing toNativeUtf8 marshalling"
    );
    assert!(
        ffi_file.content.contains("calloc.free("),
        "missing calloc.free for string param"
    );
}

/// Functions with an error type emit a `_checkError()` call after the C invocation.
#[test]
fn result_returning_functions_check_last_error_code() {
    let api = ApiSurface {
        functions: vec![make_function(
            "parse",
            vec![make_param("src", TypeRef::String)],
            TypeRef::String,
            Some("ParseError".to_string()),
        )],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files.iter().find(|f| f.path.to_string_lossy().ends_with("_ffi.dart")).unwrap();

    assert!(ffi_file.content.contains("_checkError()"), "missing _checkError call");
    // The error helpers use the standard last-error symbols.
    assert!(
        ffi_file.content.contains("demo_crate_last_error_code"),
        "missing last_error_code symbol"
    );
    assert!(
        ffi_file.content.contains("demo_crate_last_error_context"),
        "missing last_error_context symbol"
    );
}

/// An async function in FFI mode emits a TODO comment and is skipped.
#[test]
fn async_functions_emit_todo_comment_in_ffi_mode() {
    let mut f = make_function("stream_data", vec![], TypeRef::Unit, None);
    f.is_async = true;
    let api = ApiSurface { functions: vec![f], ..make_empty_api() };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files.iter().find(|f| f.path.to_string_lossy().ends_with("_ffi.dart")).unwrap();

    assert!(
        ffi_file.content.contains("// TODO: async function 'stream_data'"),
        "missing TODO comment for async function"
    );
}

/// A unit-variant enum emits a standard Dart `enum` block.
#[test]
fn unit_enum_emits_dart_enum() {
    let en = EnumDef {
        name: "Status".to_string(),
        doc: String::new(),
        rust_path: "demo::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            is_tuple: false,
            },
            EnumVariant {
                name: "Inactive".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
            is_tuple: false,
            },
        ],
        serde_rename_all: None,
        serde_tag: None,
        cfg: None,
        is_copy: false,
        has_serde: false,
    };
    let api = ApiSurface { enums: vec![en], ..make_empty_api() };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files.iter().find(|f| f.path.to_string_lossy().ends_with("_ffi.dart")).unwrap();

    assert!(ffi_file.content.contains("enum Status {"), "missing Dart enum declaration");
    assert!(ffi_file.content.contains("active"), "missing active variant");
    assert!(ffi_file.content.contains("inactive"), "missing inactive variant");
}

/// FFI mode emits two files: `_ffi.dart` (implementation) and `.dart` (re-export).
#[test]
fn ffi_mode_emits_two_files_impl_and_reexport() {
    let api = make_empty_api();
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 2, "expected exactly 2 files for FFI mode");

    let has_ffi = files.iter().any(|f| f.path.to_string_lossy().ends_with("_ffi.dart"));
    let has_wrapper = files.iter().any(|f| {
        let name = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        name.ends_with(".dart") && !name.ends_with("_ffi.dart")
    });
    assert!(has_ffi, "missing _ffi.dart implementation file");
    assert!(has_wrapper, "missing .dart re-export wrapper file");

    let wrapper = files.iter().find(|f| {
        let name = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        name.ends_with(".dart") && !name.ends_with("_ffi.dart")
    }).unwrap();
    assert!(wrapper.content.contains("export"), "wrapper file should re-export the FFI module");
}

/// `build_config_for` returns `BuildDependency::Ffi` for FFI style
/// and `BuildDependency::None` (FRB) for the default style.
#[test]
fn build_config_for_dispatches_on_dart_style() {
    use alef_core::backend::BuildDependency;

    let backend = DartBackend;

    let ffi_config = make_config_ffi();
    let bc_ffi = backend.build_config_for(&ffi_config).unwrap();
    assert_eq!(bc_ffi.build_dep, BuildDependency::Ffi, "FFI style should use BuildDependency::Ffi");

    let mut frb_config = make_config_ffi();
    frb_config.dart = Some(DartConfig { style: DartStyle::Frb, ..DartConfig::default() });
    let bc_frb = backend.build_config_for(&frb_config).unwrap();
    assert_eq!(bc_frb.build_dep, BuildDependency::None, "FRB style should use BuildDependency::None");
}
