use alef_backend_kotlin::KotlinBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
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
        binding_excluded: false,
        binding_exclusion_reason: None,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

/// Build a native-target config with a known FFI prefix.
fn make_native_config(crate_name: &str) -> ResolvedCrateConfig {
    resolved_one(&format!(
        r#"
[workspace]
languages = ["kotlin", "ffi"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"
header_name = "demo.h"
lib_name = "demo_ffi"

[crates.kotlin]
package = "dev.kreuzberg.demo"
target = "native"
"#
    ))
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "kt"))
        .unwrap();
    let content = &kt.content;

    assert!(
        content.contains("package dev.kreuzberg.demo"),
        "missing package: {content}"
    );
    assert!(
        content.contains("import kotlinx.cinterop.*"),
        "missing cinterop import: {content}"
    );
    assert!(
        content.contains("data class ConversionOptions("),
        "missing data class: {content}"
    );
    assert!(
        content.contains("val maxWidth: Int"),
        "missing maxWidth field: {content}"
    );
    // Path → String in Native mode (no java.nio.file.Path).
    assert!(
        content.contains("val outputPath: String"),
        "Path field should be String in native: {content}"
    );
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
            serde_untagged: false,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "kt"))
        .unwrap();
    let content = &kt.content;

    assert!(
        content.contains("enum class OutputFormat {"),
        "missing enum class: {content}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "kt"))
        .unwrap();
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
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "kt"))
        .unwrap();
    let content = &kt.content;

    assert!(
        content.contains("demo_last_error_code()"),
        "missing error code check: {content}"
    );
    assert!(content.contains("throw RuntimeException("), "missing throw: {content}");
    assert!(
        content.contains("demo_last_error_context()"),
        "missing error context: {content}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let def = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "def"))
        .unwrap();
    let content = &def.content;

    assert!(
        content.contains("headers = demo.h"),
        "missing header in .def: {content}"
    );
    assert!(
        content.contains("headerFilter = demo_*"),
        "missing headerFilter in .def: {content}"
    );
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };
    let config = make_native_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let kt = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "kt"))
        .unwrap();
    let path_str = kt.path.to_string_lossy();

    assert!(
        path_str.contains("packages/kotlin-native/src/nativeMain/kotlin"),
        "wrong path for .kt file: {path_str}"
    );
    assert!(
        path_str.contains("dev/kreuzberg/demo"),
        "package path missing from .kt file path: {path_str}"
    );
    assert!(
        path_str.ends_with("MyCrate.kt"),
        "wrong filename for .kt file: {path_str}"
    );
}
