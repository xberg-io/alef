use alef_backend_csharp::CsharpBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::*;

fn make_kreuzberg_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "kreuzberg"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "kreuzberg"
error_style = "last_error"
[crates.csharp]
namespace = "Kreuzberg"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn test_generated_code_example() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "kreuzberg".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "kreuzberg::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "ocr_backend".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    default: None,
                    doc: "OCR backend to use".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "timeout".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U64),
                    optional: true,
                    default: None,
                    doc: "Timeout in milliseconds".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
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
            doc: "Configuration for text extraction".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![FunctionDef {
            name: "extract_file_sync".to_string(),
            rust_path: "kreuzberg::extract_file_sync".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
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
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("ExtractionConfig".to_string()))),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text from a file synchronously".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![EnumDef {
            name: "OcrBackend".to_string(),
            rust_path: "kreuzberg::OcrBackend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Tesseract".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Tesseract OCR engine".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "PaddleOCR engine".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Available OCR backends".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
    };

    let config = make_kreuzberg_config();

    let files = backend.generate_bindings(&api, &config).unwrap();

    // NativeMethods.cs should contain P/Invoke declarations
    let native_methods = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .unwrap();

    assert!(native_methods.content.contains("[DllImport(LibName"));
    assert!(native_methods.content.contains("kreuzberg_extract_file_sync"));
    assert!(native_methods.content.contains("internal static extern"));
    assert!(native_methods.content.contains("kreuzberg_last_error_code"));
    assert!(native_methods.content.contains("kreuzberg_last_error_context"));
    assert!(native_methods.content.contains("kreuzberg_free_string"));

    // Exception class should be properly defined
    let exception = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("KreuzbergException.cs"))
        .unwrap();

    assert!(
        exception
            .content
            .contains("public class KreuzbergException : Exception")
    );
    assert!(exception.content.contains("public int Code { get; }"));
    assert!(exception.content.contains("namespace Kreuzberg"));

    // Wrapper class should have extraction methods
    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("KreuzbergLib.cs"))
        .unwrap();

    assert!(wrapper.content.contains("public static class KreuzbergLib"));
    assert!(wrapper.content.contains("public static string ExtractFileSync"));
    assert!(wrapper.content.contains("NativeMethods."));
    assert!(wrapper.content.contains("GetLastError()"));

    // Type definition should use records
    let config_type = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ExtractionConfig.cs"))
        .unwrap();

    assert!(config_type.content.contains("public sealed record ExtractionConfig"));
    assert!(config_type.content.contains("string? OcrBackend"));
    assert!(config_type.content.contains("ulong? Timeout"));
    assert!(config_type.content.contains("Configuration for text extraction"));

    // Enum definition
    let enum_type = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("OcrBackend.cs"))
        .unwrap();

    assert!(enum_type.content.contains("public enum OcrBackend"));
    assert!(enum_type.content.contains("Tesseract,"));
    assert!(enum_type.content.contains("PaddleOcr,"));
    assert!(enum_type.content.contains("Available OCR backends"));
}
