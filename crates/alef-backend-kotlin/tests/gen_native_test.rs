use alef_backend_kotlin::KotlinBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, FfiConfig, KotlinConfig, KotlinTarget};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

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

fn make_function(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async,
        doc: String::new(),
        error_type: None,
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

fn make_fallible_function(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async: false,
        doc: String::new(),
        error_type: Some("DemoError".to_string()),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

/// Build a native-target config with a known FFI prefix.
fn make_native_config(crate_name: &str) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: crate_name.to_string(),
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
        ffi: Some(FfiConfig {
            prefix: Some("demo".to_string()),
            error_style: "last_error".to_string(),
            header_name: Some("demo.h".to_string()),
            lib_name: Some("demo_ffi".to_string()),
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: vec![],
            exclude_types: vec![],
            rename_fields: std::collections::HashMap::new(),
        }),
        gleam: None,
        go: None,
        java: None,
        kotlin: Some(KotlinConfig {
            package: Some("dev.kreuzberg.demo".to_string()),
            features: None,
            serde_rename_all: None,
            rename_fields: std::collections::HashMap::new(),
            exclude_functions: vec![],
            exclude_types: vec![],
            run_wrapper: None,
            extra_lint_paths: vec![],
            target: KotlinTarget::Native,
        }),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Native target emits three files: .kt, .def, and build.gradle.kts.
#[test]
fn native_emits_three_files() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 3, "expected 3 files (kt, def, gradle): {files:?}");
}

/// Struct fields map to Kotlin data class with camelCase property names.
#[test]
fn native_struct_emits_data_class() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "ConversionOptions",
            vec![
                make_field("max_width", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("output_path", TypeRef::Path, false),
            ],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files.iter().find(|f| f.path.extension().is_some_and(|e| e == "kt")).unwrap();
    let content = &kt.content;

    assert!(content.contains("package dev.kreuzberg.demo"), "missing package: {content}");
    assert!(content.contains("import kotlinx.cinterop.*"), "missing cinterop import: {content}");
    assert!(content.contains("data class ConversionOptions("), "missing data class: {content}");
    assert!(content.contains("val maxWidth: Int"), "missing maxWidth field: {content}");
    // Path → String in Native mode (no java.nio.file.Path).
    assert!(content.contains("val outputPath: String"), "Path field should be String in native: {content}");
}

/// Unit enum emits `enum class` with SCREAMING_SNAKE_CASE variants.
#[test]
fn native_unit_enum_emits_enum_class() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "demo::OutputFormat".to_string(),
            original_rust_path: String::new(),
            doc: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Markdown".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
                EnumVariant {
                    name: "PlainText".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
            ],
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files.iter().find(|f| f.path.extension().is_some_and(|e| e == "kt")).unwrap();
    let content = &kt.content;

    assert!(content.contains("enum class OutputFormat {"), "missing enum class: {content}");
    assert!(content.contains("MARKDOWN"), "missing MARKDOWN variant: {content}");
    assert!(content.contains("PLAIN_TEXT"), "missing PLAIN_TEXT variant: {content}");
}

/// Functions emit memScoped bodies that call the C FFI symbol.
#[test]
fn native_function_uses_mem_scoped() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_function(
            "convert_html",
            vec![make_param("input", TypeRef::String)],
            TypeRef::String,
            false,
        )],
        enums: vec![],
        errors: vec![],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files.iter().find(|f| f.path.extension().is_some_and(|e| e == "kt")).unwrap();
    let content = &kt.content;

    assert!(content.contains("fun convertHtml("), "missing function: {content}");
    assert!(content.contains("memScoped {"), "missing memScoped: {content}");
    assert!(content.contains("demo_convert_html("), "missing C call: {content}");
    // String param must be converted to a C string pointer.
    assert!(content.contains("inputC"), "missing string C conversion: {content}");
}

/// Fallible functions check last_error_code and throw on failure.
#[test]
fn native_fallible_function_checks_error_code() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_fallible_function(
            "parse_document",
            vec![make_param("source", TypeRef::String)],
            TypeRef::String,
        )],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            doc: String::new(),
            variants: vec![ErrorVariant {
                name: "ParseFailed".to_string(),
                is_unit: true,
                fields: vec![],
                message_template: None,
                has_source: false,
                has_from: false,
                doc: String::new(),
            }],
        }],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files.iter().find(|f| f.path.extension().is_some_and(|e| e == "kt")).unwrap();
    let content = &kt.content;

    assert!(content.contains("demo_last_error_code()"), "missing error code check: {content}");
    assert!(content.contains("throw RuntimeException("), "missing throw: {content}");
    assert!(content.contains("demo_last_error_context()"), "missing error context: {content}");
}

/// .def file references the correct header, prefix, and library name.
#[test]
fn native_def_file_has_correct_fields() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let def = files.iter().find(|f| f.path.extension().is_some_and(|e| e == "def")).unwrap();
    let content = &def.content;

    assert!(content.contains("headers = demo.h"), "missing header in .def: {content}");
    assert!(content.contains("headerFilter = demo_*"), "missing headerFilter in .def: {content}");
    assert!(content.contains("-ldemo_ffi"), "missing linker flag in .def: {content}");
}

/// .kt file is emitted under packages/kotlin-native/src/nativeMain/kotlin/<package>/.
#[test]
fn native_kt_file_is_at_correct_path() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files.iter().find(|f| f.path.extension().is_some_and(|e| e == "kt")).unwrap();
    let path_str = kt.path.to_string_lossy();

    assert!(
        path_str.contains("packages/kotlin-native/src/nativeMain/kotlin"),
        "wrong path for .kt file: {path_str}"
    );
    assert!(
        path_str.contains("dev/kreuzberg/demo"),
        "package path missing from .kt file path: {path_str}"
    );
    assert!(path_str.ends_with("MyCrate.kt"), "wrong filename for .kt file: {path_str}");
}
