use alef::backends::csharp::CsharpBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;

fn make_sample_crate_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "sample_crate"
error_style = "last_error"
[crates.csharp]
namespace = "SampleCrate"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn test_generated_code_example() {
    let backend = CsharpBackend;

    let api = ApiSurface {
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "sample_crate::ExtractionConfig".to_string(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract_file_sync".to_string(),
            rust_path: "sample_crate::extract_file_sync".to_string(),
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
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
            version: Default::default(),
        }],
        enums: vec![EnumDef {
            name: "OcrBackend".to_string(),
            rust_path: "sample_crate::OcrBackend".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Tesseract".to_string(),
                    fields: vec![],
                    doc: "Tesseract OCR engine".to_string(),
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
                    name: "PaddleOcr".to_string(),
                    fields: vec![],
                    doc: "PaddleOCR engine".to_string(),
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
            doc: "Available OCR backends".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
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

    let config = make_sample_crate_config();

    let files = backend.generate_bindings(&api, &config).unwrap();

    // NativeMethods.cs should contain P/Invoke declarations
    let native_methods = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeMethods.cs"))
        .unwrap();

    assert!(native_methods.content.contains("[DllImport(LibName"));
    assert!(native_methods.content.contains("sample_crate_extract_file_sync"));
    assert!(native_methods.content.contains("internal static extern"));
    assert!(native_methods.content.contains("sample_crate_last_error_code"));
    assert!(native_methods.content.contains("sample_crate_last_error_context"));
    assert!(native_methods.content.contains("sample_crate_free_string"));

    // Exception class should be properly defined
    let exception = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SampleCrateException.cs"))
        .unwrap();

    assert!(
        exception
            .content
            .contains("public class SampleCrateException : Exception")
    );
    assert!(exception.content.contains("public int Code { get; }"));
    assert!(exception.content.contains("namespace SampleCrate"));

    // Wrapper class should have extraction methods
    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SampleCrateConverter.cs"))
        .unwrap();

    assert!(wrapper.content.contains("public static class SampleCrateConverter"));
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

// ──────────────────────────────────────────────────────────────────────────────
// untagged_union_text_types — Text() accessor emission in C# untagged wrappers
// ──────────────────────────────────────────────────────────────────────────────

fn make_untagged_enum(name: &str) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{name}"),
        original_rust_path: String::new(),
        methods: vec![],
        doc: "Multimodal assistant content.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: true,
        serde_rename_all: None,
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                doc: String::new(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
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
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Parts".to_string(),
                doc: String::new(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
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
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

fn make_config_with_text_types(text_types: &[&str]) -> ResolvedCrateConfig {
    let list = text_types
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["csharp"]
[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]
untagged_union_text_types = [{list}]
[crates.ffi]
prefix = "sample_crate"
error_style = "last_error"
[crates.csharp]
namespace = "SampleCrate"
"#
    ))
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Without `untagged_union_text_types`, no `Text()` method appears.
#[test]
fn csharp_untagged_wrapper_without_text_types_does_not_emit_text_method() {
    let backend = CsharpBackend;
    let config = make_sample_crate_config();

    let api = ApiSurface {
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![make_untagged_enum("AssistantContent")],
        ..ApiSurface::default()
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("AssistantContent.cs"))
        .expect("AssistantContent.cs must be emitted");

    assert!(
        wrapper.content.contains("class AssistantContent"),
        "wrapper class must be present"
    );
    assert!(
        !wrapper.content.contains("public string Text()"),
        "Text() must NOT be emitted when untagged_union_text_types is empty:\n{}",
        wrapper.content
    );
}

/// With `untagged_union_text_types = ["AssistantContent"]`, `Text()` is emitted.
#[test]
fn csharp_untagged_wrapper_with_text_types_emits_text_method() {
    let backend = CsharpBackend;
    let config = make_config_with_text_types(&["AssistantContent"]);

    let api = ApiSurface {
        crate_name: "sample_crate".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![make_untagged_enum("AssistantContent")],
        ..ApiSurface::default()
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let wrapper = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("AssistantContent.cs"))
        .expect("AssistantContent.cs must be emitted");

    let src = &wrapper.content;
    assert!(src.contains("public string Text()"), "Text() must be emitted:\n{src}");
    // Must handle JSON string
    assert!(
        src.contains("JsonValueKind.String"),
        "must handle JSON string variant:\n{src}"
    );
    // Must handle JSON array
    assert!(
        src.contains("JsonValueKind.Array"),
        "must handle JSON array variant:\n{src}"
    );
    // Must filter by type == "text"
    assert!(
        src.contains("GetString() == \"text\""),
        "must filter parts by type==\"text\":\n{src}"
    );
    // Returns empty string as fallback
    assert!(
        src.contains("string.Empty"),
        "must return string.Empty as fallback:\n{src}"
    );
}
