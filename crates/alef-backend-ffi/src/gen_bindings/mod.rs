mod functions;
mod helpers;
mod types;

use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators;
use alef_core::backend::{Backend, BuildConfig, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

use functions::{gen_free_function, gen_method_wrapper};
use helpers::{gen_build_rs, gen_cbindgen_toml, gen_ffi_tokio_runtime, gen_free_string, gen_last_error, gen_version};
use types::{
    gen_enum_from_i32, gen_enum_to_i32, gen_field_accessor, gen_type_free, gen_type_from_json, gen_type_to_json,
};

pub struct FfiBackend;

impl FfiBackend {}

impl Backend for FfiBackend {
    fn name(&self) -> &str {
        "ffi"
    }

    fn language(&self) -> Language {
        Language::Ffi
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.ffi_prefix();
        let header_name = config.ffi_header_name();

        let output_dir = resolve_output_dir(
            config.output.ffi.as_ref(),
            &config.crate_config.name,
            "crates/{name}-ffi/src/",
        );

        let parent_dir = PathBuf::from(&output_dir)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();

        let files = vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join("lib.rs"),
                content: gen_lib_rs(api, &prefix, config),
                generated_header: false,
            },
            GeneratedFile {
                path: parent_dir.join("cbindgen.toml"),
                content: gen_cbindgen_toml(&prefix, api),
                generated_header: false,
            },
            GeneratedFile {
                path: parent_dir.join("build.rs"),
                content: gen_build_rs(&header_name),
                generated_header: false,
            },
        ];

        Ok(files)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-ffi",
            depends_on_ffi: false,
            post_build: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// lib.rs generation
// ---------------------------------------------------------------------------

fn gen_lib_rs(api: &ApiSurface, prefix: &str, config: &AlefConfig) -> String {
    let mut builder = RustFileBuilder::new().with_generated_header();

    // Imports
    builder.add_import("std::ffi::{c_char, CStr, CString}");
    builder.add_import("std::cell::RefCell");
    let core_import = config.core_import();

    // Build path map: short name -> full rust_path for all types and enums.
    // Normalize dashes to underscores since IR paths use Cargo package names (dashes)
    // but Rust source code requires crate names (underscores).
    let mut path_map = ahash::AHashMap::new();
    for t in api.types.iter().filter(|t| !t.is_trait) {
        path_map.insert(t.name.clone(), t.rust_path.replace('-', "_"));
    }
    for e in &api.enums {
        path_map.insert(e.name.clone(), e.rust_path.replace('-', "_"));
    }
    for err in &api.errors {
        path_map.insert(err.name.clone(), err.rust_path.replace('-', "_"));
    }

    // Import traits needed for trait method dispatch
    for trait_path in generators::collect_trait_imports(api) {
        builder.add_import(&trait_path);
    }
    // FFI backend uses fully qualified paths (e.g. html_to_markdown_rs::ConversionOptions)
    // for all core type references, so no named or glob imports from the core crate are
    // needed. Trait imports (collected above) are sufficient for method dispatch.

    // Only import serde_json when types need from_json deserialization or
    // when Json/Vec/Map fields/returns require serialization
    let has_from_json_types = api
        .types
        .iter()
        .any(|t| !t.is_opaque && !t.fields.iter().any(|f| f.sanitized));
    let has_serde_fields = api.types.iter().any(|t| {
        t.fields.iter().any(|f| {
            matches!(f.ty, alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
                || matches!(&f.ty, alef_core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)))
        })
    });
    let has_serde_returns = api.types.iter().any(|t| {
        t.methods.iter().any(|m| {
            matches!(m.return_type, alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
                || matches!(&m.return_type, alef_core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)))
        })
    }) || api.functions.iter().any(|f| {
        matches!(f.return_type, alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _))
            || matches!(&f.return_type, alef_core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), alef_core::ir::TypeRef::Json | alef_core::ir::TypeRef::Vec(_) | alef_core::ir::TypeRef::Map(_, _)))
    });
    if has_from_json_types || has_serde_fields || has_serde_returns {
        builder.add_import("serde_json");
    }

    // Custom module declarations
    let custom_mods = config.custom_modules.for_language(Language::Ffi);
    for module in custom_mods {
        builder.add_item(&format!("pub mod {module};"));
    }

    // Thread-local last_error infrastructure
    builder.add_item(&gen_last_error(prefix));

    // free_string helper
    builder.add_item(&gen_free_string(prefix));

    // version helper
    builder.add_item(&gen_version(prefix));

    // Struct opaque-handle functions (from_json + free + field accessors + methods)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        // Generate from_json/to_json for types that derive serde Serialize/Deserialize.
        // Opaque types and types without serde derives are skipped.
        // Note: sanitized fields do NOT block from_json/to_json generation because these
        // functions use serde for the full core type (bypassing field-level type mapping).
        if !typ.is_opaque && typ.has_serde {
            builder.add_item(&gen_type_from_json(typ, prefix, &core_import));
            // Generate to_json for types that support serialization.
            // Update types (partial update structs) typically only implement Deserialize,
            // not Serialize, so skip them.
            if !typ.name.ends_with("Update") {
                builder.add_item(&gen_type_to_json(typ, prefix, &core_import));
            }
        }
        builder.add_item(&gen_type_free(typ, prefix, &core_import));

        // Field accessors — skip sanitized fields (binding type differs from core)
        for field in &typ.fields {
            if !field.sanitized {
                builder.add_item(&gen_field_accessor(typ, field, prefix, &core_import));
            }
        }

        // Method wrappers
        for method in &typ.methods {
            builder.add_item(&gen_method_wrapper(typ, method, prefix, &core_import, &path_map));
        }
    }

    // Enum functions (from_i32 + to_i32) — only for simple unit-variant enums
    for enum_def in &api.enums {
        if alef_codegen::conversions::can_generate_enum_conversion(enum_def) {
            builder.add_item(&gen_enum_from_i32(enum_def, prefix, &core_import));
            builder.add_item(&gen_enum_to_i32(enum_def, prefix, &core_import));
        }
    }

    // Emit tokio runtime helper if any function or method is async
    let has_async_functions =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    if has_async_functions {
        builder.add_item(&gen_ffi_tokio_runtime());
    }

    // Free functions (async functions are wrapped with block_on via the runtime helper)
    for func in &api.functions {
        builder.add_item(&gen_free_function(func, prefix, &core_import, &path_map));
    }

    // Build adapter body map (consumed by generators via body substitution)
    let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Ffi).unwrap_or_default();

    // Visitor/callback FFI support — generated when `[ffi] visitor_callbacks = true`.
    // Note: the generated code uses std::rc::Rc fully qualified, so no extra import needed.
    if config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks) {
        builder.add_item(&crate::gen_visitor::gen_visitor_bindings(prefix, &core_import));
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::*;

    fn sample_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "Config".to_string(),
                rust_path: "my_lib::Config".to_string(),
                fields: vec![
                    FieldDef {
                        name: "timeout".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U64),
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                    },
                    FieldDef {
                        name: "name".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                    },
                    FieldDef {
                        name: "verbose".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Bool),
                        optional: true,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef_core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                    },
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                doc: "Configuration struct.".to_string(),
                cfg: None,
            }],
            functions: vec![FunctionDef {
                name: "extract".to_string(),
                rust_path: "my_lib::extract".to_string(),
                params: vec![ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                }],
                return_type: TypeRef::Named("ExtractionResult".to_string()),
                is_async: false,
                error_type: Some("MyError".to_string()),
                doc: "Extract content from a file.".to_string(),
                cfg: None,
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![EnumDef {
                name: "OutputFormat".to_string(),
                rust_path: "my_lib::OutputFormat".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "Text".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                    EnumVariant {
                        name: "Html".to_string(),
                        fields: vec![],
                        doc: String::new(),
                        is_default: false,
                        serde_rename: None,
                    },
                ],
                doc: "Output format.".to_string(),
                cfg: None,
                serde_tag: None,
                serde_rename_all: None,
            }],
            errors: vec![],
        }
    }

    fn sample_config() -> AlefConfig {
        toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn test_generates_lib_rs() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        assert!(files.iter().any(|f| f.path.ends_with("lib.rs")));

        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(lib.content.contains("extern \"C\""));
        assert!(lib.content.contains("my_lib_last_error_code"));
        assert!(lib.content.contains("my_lib_config_from_json"));
        assert!(lib.content.contains("my_lib_config_free"));
        assert!(lib.content.contains("my_lib_config_timeout"));
        assert!(lib.content.contains("my_lib_config_name"));
        assert!(lib.content.contains("my_lib_free_string"));
        assert!(lib.content.contains("my_lib_version"));
        assert!(lib.content.contains("my_lib_extract"));
        assert!(lib.content.contains("my_lib_output_format_from_i32"));
        assert!(lib.content.contains("my_lib_output_format_from_str"));
    }

    #[test]
    fn test_generates_cbindgen_toml() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
        assert!(cbindgen.content.contains("MY_LIB_H"));
        assert!(cbindgen.content.contains("language = \"C\""));
    }

    #[test]
    fn test_generates_build_rs() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
        assert!(build.content.contains("cbindgen::generate"));
        assert!(build.content.contains("my_lib.h"));
    }

    #[test]
    fn test_custom_prefix() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "ml"
            header_name = "mylib.h"
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
        assert!(lib.content.contains("ml_last_error_code"));
        assert!(lib.content.contains("ml_config_from_json"));

        let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
        assert!(build.content.contains("mylib.h"));
    }

    #[test]
    fn test_visitor_callbacks_disabled_by_default() {
        let api = sample_api();
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // When visitor_callbacks is not enabled, no visitor code should be generated
        assert!(!lib.content.contains("VisitorCallbacks"));
        assert!(!lib.content.contains("visit_text"));
        assert!(!lib.content.contains("_visitor_create"));
        assert!(!lib.content.contains("_visitor_free"));
        assert!(!lib.content.contains("_convert_with_visitor"));
    }

    #[test]
    fn test_visitor_callbacks_enabled() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Callback struct should be generated
        assert!(lib.content.contains("struct HtmVisitorCallbacks"));
        assert!(lib.content.contains("pub struct HtmNodeContext"));

        // Visit-result codes should be defined
        assert!(lib.content.contains("HTM_VISIT_CONTINUE"));
        assert!(lib.content.contains("HTM_VISIT_SKIP"));
        assert!(lib.content.contains("HTM_VISIT_PRESERVE_HTML"));
        assert!(lib.content.contains("HTM_VISIT_CUSTOM"));
        assert!(lib.content.contains("HTM_VISIT_ERROR"));

        // NodeContext fields
        assert!(lib.content.contains("node_type: i32"));
        assert!(lib.content.contains("tag_name: *const std::ffi::c_char"));
        assert!(lib.content.contains("depth: usize"));
        assert!(lib.content.contains("index_in_parent: usize"));
        assert!(lib.content.contains("parent_tag: *const std::ffi::c_char"));
        assert!(lib.content.contains("is_inline: i32"));
    }

    #[test]
    fn test_visitor_callbacks_visitor_handle_struct() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Visitor handle struct should exist
        assert!(lib.content.contains("pub struct HtmVisitor"));
        assert!(lib.content.contains("callbacks: HtmVisitorCallbacks"));
        assert!(lib.content.contains("_tag_scratch"));
    }

    #[test]
    fn test_visitor_callbacks_callback_fields() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Some key visitor callback fields (there are 40 total in the visitor module)
        assert!(lib.content.contains("visit_text"));
        assert!(lib.content.contains("visit_element_start"));
        assert!(lib.content.contains("visit_element_end"));
        assert!(lib.content.contains("visit_link"));
        assert!(lib.content.contains("visit_image"));
        assert!(lib.content.contains("visit_heading"));
        assert!(lib.content.contains("visit_code_block"));
        assert!(lib.content.contains("visit_code_inline"));
        assert!(lib.content.contains("visit_list_item"));
        assert!(lib.content.contains("visit_list_start"));
        assert!(lib.content.contains("visit_list_end"));
        assert!(lib.content.contains("visit_table_start"));
        assert!(lib.content.contains("visit_table_row"));
        assert!(lib.content.contains("visit_table_end"));
        assert!(lib.content.contains("visit_blockquote"));
        assert!(lib.content.contains("visit_strong"));
        assert!(lib.content.contains("visit_emphasis"));
        assert!(lib.content.contains("visit_strikethrough"));
    }

    #[test]
    fn test_visitor_callbacks_ffi_functions() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Public FFI entry points for visitor management
        assert!(lib.content.contains("htm_visitor_create"));
        assert!(lib.content.contains("htm_visitor_free"));
        assert!(lib.content.contains("htm_convert_with_visitor"));

        // Functions should be extern "C"
        assert!(lib.content.contains("extern \"C\" fn htm_visitor_create"));
        assert!(lib.content.contains("extern \"C\" fn htm_visitor_free"));
        assert!(lib.content.contains("extern \"C\" fn htm_convert_with_visitor"));
    }

    #[test]
    fn test_visitor_callbacks_callback_signatures() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Callback type signatures should be extern "C" function pointers
        assert!(lib.content.contains("extern \"C\" fn("));
        assert!(lib.content.contains("*const HtmNodeContext"));
        assert!(lib.content.contains("user_data: *mut std::ffi::c_void"));
        assert!(lib.content.contains("out_custom: *mut *mut std::ffi::c_char"));
        assert!(lib.content.contains("out_len: *mut usize"));

        // Return type should be i32
        assert!(lib.content.contains(") -> i32"));
    }

    #[test]
    fn test_visitor_callbacks_custom_prefix() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "ml"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Custom prefix should be used throughout (struct/function names and constants)
        assert!(lib.content.contains("MlVisitorCallbacks"));
        assert!(lib.content.contains("MlNodeContext"));
        assert!(lib.content.contains("ml_visitor_create"));
        assert!(lib.content.contains("ml_visitor_free"));
        assert!(lib.content.contains("ml_convert_with_visitor"));
        // Visit result constants use HTM_ prefix (hardcoded in gen_visitor)
        assert!(lib.content.contains("HTM_VISIT_CONTINUE"));
    }

    #[test]
    fn test_visitor_callbacks_visitor_ref_wrapper() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // VisitorRef wrapper for forwarding trait methods
        assert!(lib.content.contains("struct VisitorRef"));
        assert!(lib.content.contains("impl std::fmt::Debug for VisitorRef"));
        // VisitorRef should implement HtmlVisitor trait (core_import is my_lib for this test)
        assert!(lib.content.contains("impl my_lib::visitor::HtmlVisitor for VisitorRef"));
    }

    #[test]
    fn test_visitor_callbacks_safety_comments() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Should document safety invariants for unsafe blocks
        assert!(lib.content.contains("// SAFETY:"));
        assert!(lib.content.contains("unsafe"));
        assert!(lib.content.contains("unsafe extern \"C\" fn"));
    }

    #[test]
    fn test_visitor_callbacks_decode_visit_result() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper function to decode visit result codes back to Rust enum
        assert!(lib.content.contains("decode_visit_result"));
        assert!(lib.content.contains("VisitResult::Skip"));
        assert!(lib.content.contains("VisitResult::PreserveHtml"));
        assert!(lib.content.contains("VisitResult::Custom"));
        assert!(lib.content.contains("VisitResult::Error"));
    }

    #[test]
    fn test_visitor_callbacks_call_with_ctx() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper function for building and passing NodeContext to C callbacks
        assert!(lib.content.contains("call_with_ctx"));
        assert!(lib.content.contains("HtmNodeContext"));
        assert!(lib.content.contains("tag_cstring"));
        assert!(lib.content.contains("parent_cstring"));
    }

    #[test]
    fn test_visitor_callbacks_opt_str_to_c() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Helper to convert Option<&str> to C pointer (null or valid CString)
        assert!(lib.content.contains("opt_str_to_c"));
    }

    #[test]
    fn test_visitor_callbacks_repr_c() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // FFI-crossing types must use #[repr(C)]
        assert!(lib.content.contains("#[repr(C)]"));
    }

    #[test]
    fn test_visitor_callbacks_send_impl() {
        let api = sample_api();
        let config: AlefConfig = toml::from_str(
            r#"
            languages = ["ffi"]
            [crate]
            name = "my-lib"
            sources = ["src/lib.rs"]
            [ffi]
            prefix = "htm"
            visitor_callbacks = true
            "#,
        )
        .unwrap();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // VisitorCallbacks should be Send (safe to share across thread boundaries)
        assert!(lib.content.contains("unsafe impl Send for HtmVisitorCallbacks"));
    }

    /// Regression test: Option<Option<Primitive>> (update-struct pattern) must generate
    /// a getter that returns the primitive type — not *mut c_char — and collapses both
    /// None cases to the primitive's zero sentinel.
    #[test]
    fn test_option_option_primitive_getter_returns_primitive_type() {
        let api = ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            types: vec![TypeDef {
                name: "ConfigUpdate".to_string(),
                rust_path: "my_lib::ConfigUpdate".to_string(),
                fields: vec![FieldDef {
                    name: "max_depth".to_string(),
                    // field.ty = Optional(Primitive(Usize)), field.optional = true
                    // represents Rust type Option<Option<usize>>
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
                    optional: true,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                }],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                doc: String::new(),
                cfg: None,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let config = sample_config();
        let backend = FfiBackend;

        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

        // Return type must be `usize`, not `*mut std::ffi::c_char`
        assert!(
            lib.content.contains("-> usize"),
            "expected `-> usize` in getter but got:\n{}",
            lib.content
        );
        assert!(
            !lib.content.contains("-> *mut std::ffi::c_char"),
            "getter must not return *mut c_char for Option<Option<usize>>"
        );

        // Both None arms must return 0, not a pointer
        assert!(
            lib.content.contains("None => 0"),
            "expected `None => 0` sentinel in generated getter"
        );

        // The inner Some(inner_val) branch must dereference the usize
        assert!(
            lib.content.contains("*inner_val"),
            "expected `*inner_val` deref for inner primitive in generated getter"
        );
    }
}
