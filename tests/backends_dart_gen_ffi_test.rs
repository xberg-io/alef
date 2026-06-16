use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{DartConfig, DartStyle, ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};

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
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

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

fn make_config_ffi() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.dart]
style = "ffi"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

/// The generated FFI file contains a platform-aware `_libraryPath()` helper
/// with branches for macOS, Windows, and Linux.
#[test]
fn ffi_emits_library_load_helper_with_platform_branching() {
    let api = make_empty_api();
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file.content.contains("DynamicLibrary.open(_libraryPath())"),
        "missing DynamicLibrary.open"
    );
    assert!(ffi_file.content.contains("Platform.isMacOS"), "missing macOS branch");
    assert!(
        ffi_file.content.contains("Platform.isWindows"),
        "missing Windows branch"
    );
    assert!(ffi_file.content.contains(".dylib"), "missing dylib extension");
    assert!(ffi_file.content.contains(".dll"), "missing dll extension");
    assert!(ffi_file.content.contains(".so"), "missing so extension");
}

/// Each IR function gets a `lookupFunction` call with the correct C symbol name.
#[test]
fn each_function_gets_lookup_function_call() {
    let api = ApiSurface {
        functions: vec![
            make_function(
                "process_text",
                vec![make_param("input", TypeRef::String)],
                TypeRef::String,
                None,
            ),
            make_function("get_version", vec![], TypeRef::String, None),
        ],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

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
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

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
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

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

/// An async function in FFI mode emits an unsupported comment and is skipped.
#[test]
fn async_functions_emit_todo_comment_in_ffi_mode() {
    let mut f = make_function("stream_data", vec![], TypeRef::Unit, None);
    f.is_async = true;
    let api = ApiSurface {
        functions: vec![f],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file
            .content
            .contains("// Unsupported: async function 'stream_data'"),
        "missing unsupported comment for async function"
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Inactive".to_string(),
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
        serde_rename_all: None,
        serde_tag: None,
        serde_untagged: false,
        cfg: None,
        is_copy: false,
        has_serde: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let api = ApiSurface {
        enums: vec![en],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file.content.contains("enum Status {"),
        "missing Dart enum declaration"
    );
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

    let wrapper = files
        .iter()
        .find(|f| {
            let name = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(".dart") && !name.ends_with("_ffi.dart")
        })
        .unwrap();
    assert!(
        wrapper.content.contains("export"),
        "wrapper file should re-export the FFI module"
    );
}

/// `build_config_for` returns `BuildDependency::Ffi` for FFI style
/// and `BuildDependency::None` (FRB) for the default style.
#[test]
fn build_config_for_dispatches_on_dart_style() {
    use alef::core::backend::BuildDependency;

    let backend = DartBackend;

    let ffi_config = make_config_ffi();
    let bc_ffi = backend.build_config_for(&ffi_config).unwrap();
    assert_eq!(
        bc_ffi.build_dep,
        BuildDependency::Ffi,
        "FFI style should use BuildDependency::Ffi"
    );

    let mut frb_config = make_config_ffi();
    frb_config.dart = Some(DartConfig {
        style: DartStyle::Frb,
        ..DartConfig::default()
    });
    let bc_frb = backend.build_config_for(&frb_config).unwrap();
    assert_eq!(
        bc_frb.build_dep,
        BuildDependency::None,
        "FRB style should use BuildDependency::None"
    );
}

/// Product-type DTOs in FFI mode emit `@freezed` annotation with factory constructor
/// and fromJson factory for code generation via build_runner.
#[test]
fn product_type_single_field_emits_freezed_with_factory() {
    let api = ApiSurface {
        types: vec![make_type(
            "Point",
            vec![make_field("x", TypeRef::Primitive(PrimitiveType::I32), false)],
        )],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file.content.contains("@freezed"),
        "product-type DTO must have @freezed annotation"
    );
    assert!(
        ffi_file.content.contains("class Point with _$Point"),
        "product-type DTO must use with _$ClassName mixin"
    );
    assert!(
        ffi_file.content.contains("factory Point({"),
        "product-type DTO must have factory constructor"
    );
    assert!(
        ffi_file.content.contains("= _Point"),
        "product-type DTO must have = _ClassName assignment"
    );
    assert!(
        ffi_file.content.contains("factory Point.fromJson"),
        "product-type DTO must have fromJson factory"
    );
}

/// Product-type DTOs with multiple fields emit `@freezed` with named required parameters.
#[test]
fn product_type_multi_field_emits_freezed_with_named_params() {
    let api = ApiSurface {
        types: vec![make_type(
            "Config",
            vec![
                make_field("name", TypeRef::String, false),
                make_field("count", TypeRef::Primitive(PrimitiveType::I32), false),
            ],
        )],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file.content.contains("@freezed"),
        "multi-field DTO must have @freezed annotation"
    );
    assert!(
        ffi_file.content.contains("required String name"),
        "multi-field DTO must have required name parameter"
    );
    assert!(
        ffi_file.content.contains("required int count"),
        "multi-field DTO must have required count parameter"
    );
    assert!(
        ffi_file.content.contains("= _Config"),
        "multi-field DTO must have = _ClassName assignment"
    );
}

/// FFI generated files include part-of directives for freezed and json_serializable.
#[test]
fn ffi_file_includes_part_of_directives_for_freezed() {
    let api = ApiSurface {
        types: vec![make_type("Data", vec![make_field("value", TypeRef::String, false)])],
        ..make_empty_api()
    };
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file.content.contains("part 'demo_crate_ffi.freezed.dart'"),
        "FFI file must include part-of directive for freezed"
    );
    assert!(
        ffi_file.content.contains("part 'demo_crate_ffi.g.dart'"),
        "FFI file must include part-of directive for json_serializable"
    );
}

/// FFI generated files import json_annotation for @freezed DTO support.
#[test]
fn ffi_file_imports_json_annotation() {
    let api = make_empty_api();
    let config = make_config_ffi();

    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let ffi_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("_ffi.dart"))
        .unwrap();

    assert!(
        ffi_file
            .content
            .contains("import 'package:json_annotation/json_annotation.dart'"),
        "FFI file must import json_annotation for @freezed support"
    );
}

/// Scaffold for FFI Dart style includes freezed dev-dependencies for code generation.
#[test]
fn ffi_dart_scaffold_includes_freezed_dev_dependencies() {
    use alef::core::config::Language;
    use alef::scaffold::scaffold;

    let api = make_empty_api();
    let config = make_config_ffi();

    let files = scaffold(&api, &config, &[Language::Dart]).expect("scaffold must succeed");
    let pubspec = files
        .iter()
        .find(|f| {
            let fname = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            fname == "pubspec.yaml"
        })
        .expect("pubspec.yaml must be generated");

    assert!(
        pubspec.content.contains("freezed:"),
        "pubspec.yaml FFI style must include freezed dev-dependency"
    );
    assert!(
        pubspec.content.contains("build_runner:"),
        "pubspec.yaml FFI style must include build_runner dev-dependency"
    );
    assert!(
        pubspec.content.contains("json_serializable:"),
        "pubspec.yaml FFI style must include json_serializable dev-dependency"
    );
    assert!(
        pubspec.content.contains("freezed_annotation:"),
        "pubspec.yaml FFI style must include freezed_annotation dependency"
    );
    assert!(
        pubspec.content.contains("json_annotation:"),
        "pubspec.yaml FFI style must include json_annotation dependency"
    );
}
