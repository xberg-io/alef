use alef_backend_swift::SwiftBackend;
use alef_core::backend::Backend;
use alef_core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
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

fn make_basic_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "demo::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("value", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("label", TypeRef::String, false),
                make_field("tag", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A demo configuration struct.".to_string(),
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
        }],
        functions: vec![FunctionDef {
            name: "process".into(),
            rust_path: "demo::process".into(),
            original_rust_path: String::new(),
            params: vec![
                make_param("input", TypeRef::String),
                make_param("count", TypeRef::Primitive(PrimitiveType::U32)),
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("DemoError".to_string()),
            doc: "Process input and return a result.".to_string(),
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
            name: "Status".to_string(),
            rust_path: "demo::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Active state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Inactive state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing status.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    message_template: Some("invalid input provided".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: "Input validation failed.".to_string(),
                },
                ErrorVariant {
                    name: "ProcessingFailed".to_string(),
                    message_template: Some("processing failed".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: "Processing encountered an error.".to_string(),
                },
            ],
            doc: "Errors emitted by demo operations.".to_string(),
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    }
}

fn make_basic_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

#[test]
fn snapshot_basic_struct_function_enum_error() {
    let api = make_basic_api();
    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!("snapshot_basic__{}", file.path.display().to_string().replace('/', "__")),
            &file.content
        );
    }
}

#[test]
fn snapshot_conversion_struct_with_named_types() {
    // Test struct with Named types as fields, requiring conversion init
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Output".to_string(),
                rust_path: "demo::Output".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("result", TypeRef::Primitive(PrimitiveType::I32), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Output struct.".to_string(),
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
            },
            TypeDef {
                name: "Wrapper".to_string(),
                rust_path: "demo::Wrapper".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("output", TypeRef::Named("Output".to_string()), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Wrapper containing a named type.".to_string(),
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
            },
        ],
        functions: vec![FunctionDef {
            name: "processAndWrap".into(),
            rust_path: "demo::processAndWrap".into(),
            original_rust_path: String::new(),
            params: vec![make_param("input", TypeRef::String)],
            return_type: TypeRef::Named("Wrapper".to_string()),
            is_async: false,
            error_type: None,
            doc: "Process and wrap result.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_conversion_struct__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

#[test]
fn snapshot_conversion_enum_with_data() {
    // Test enum with data variants that need conversion
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Data".to_string(),
            rust_path: "demo::Data".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("value", TypeRef::Primitive(PrimitiveType::I32), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Data struct.".to_string(),
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
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Result".to_string(),
            rust_path: "demo::Result".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Success".to_string(),
                    fields: vec![make_field("data", TypeRef::Named("Data".to_string()), false)],
                    is_tuple: false,
                    doc: "Success variant.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Error".to_string(),
                    fields: vec![make_field("message", TypeRef::String, false)],
                    is_tuple: false,
                    doc: "Error variant.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Result enum with data.".to_string(),
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
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_conversion_enum__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

#[test]
fn snapshot_conversion_vec_of_named() {
    // Test function returning Vec<Named> type
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Item".to_string(),
            rust_path: "demo::Item".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Item struct.".to_string(),
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
        }],
        functions: vec![FunctionDef {
            name: "getItems".into(),
            rust_path: "demo::getItems".into(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
            is_async: false,
            error_type: None,
            doc: "Get all items.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_conversion_vec__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool, fallible: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: if fallible { Some("DemoError".to_string()) } else { None },
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

#[test]
fn snapshot_trait_bridge_inbound() {
    // Simulates a kreuzberg-style plugin trait with a Plugin super-trait, async fallible
    // method using a Named param/return, plus a sync method returning a primitive.
    // Verifies the inbound (extern "Swift") code path: extern block, wrapper struct,
    // Plugin impl, Trait impl, and register/unregister entry points.
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Plugin".to_string(),
                rust_path: "demo::plugins::Plugin".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "Base plugin trait.".to_string(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            TypeDef {
                name: "OcrConfig".to_string(),
                rust_path: "demo::OcrConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("language", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "OCR configuration.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            TypeDef {
                name: "ExtractionResult".to_string(),
                rust_path: "demo::ExtractionResult".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("text", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Result of extraction.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            TypeDef {
                name: "OcrBackend".to_string(),
                rust_path: "demo::plugins::OcrBackend".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![
                    make_method(
                        "process_image",
                        vec![
                            ParamDef {
                                name: "image_bytes".into(),
                                ty: TypeRef::Bytes,
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
                                name: "config".into(),
                                ty: TypeRef::Named("OcrConfig".to_string()),
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
                        TypeRef::Named("ExtractionResult".to_string()),
                        true,
                        true,
                    ),
                    make_method(
                        "supports_language",
                        vec![ParamDef {
                            name: "lang".into(),
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
                        TypeRef::Primitive(PrimitiveType::Bool),
                        false,
                        false,
                    ),
                ],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "OCR backend plugin trait.".to_string(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec!["Plugin".to_string()],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "demo::plugins::Plugin"
registry_getter = "demo::plugins::registry::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_trait_bridge_inbound__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

/// Snapshot a struct whose IR contains a sanitized homogeneous-tuple field.
///
/// In the real pipeline `parse_homogeneous_tuple` rewrites `(usize, usize)` to
/// `Vec<Primitive(Usize)>` and sets `sanitized = true` on the `FieldDef`.  The
/// swift backend must **not** emit a direct assignment (`__target.ngram_range =
/// ngram_range;`) because the source field is still `(usize, usize)` — that
/// would be a type-mismatch compile error.  Instead it must emit the serde
/// JSON round-trip:
///
/// ```text
/// if let Ok(__v) = ::serde_json::to_value(ngram_range) {
///     if let Ok(t) = ::serde_json::from_value(__v) { __target.ngram_range = t; }
/// }
/// ```
///
/// This test locks down that code path so a future refactor cannot regress it.
#[test]
fn snapshot_tuple_field_as_vec() {
    // Build a field whose IR type is Vec<Primitive(Usize)> with sanitized=true.
    // This is the representation produced by `parse_homogeneous_tuple` for `(usize, usize)`.
    let mut ngram_range_field = make_field(
        "ngram_range",
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
        false,
    );
    ngram_range_field.sanitized = true;

    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "KeywordConfig".to_string(),
            rust_path: "demo::KeywordConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("max_keywords", TypeRef::Primitive(PrimitiveType::Usize), false),
                ngram_range_field,
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Keyword extraction configuration.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![FunctionDef {
            name: "extract_keywords".into(),
            rust_path: "demo::extract_keywords".into(),
            original_rust_path: String::new(),
            params: vec![
                make_param("text", TypeRef::String),
                make_param("config", TypeRef::Named("KeywordConfig".to_string())),
            ],
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            is_async: false,
            error_type: Some("DemoError".to_string()),
            doc: "Extract keywords from text.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "ExtractionFailed".to_string(),
                message_template: Some("keyword extraction failed".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                doc: "Extraction encountered an error.".to_string(),
            }],
            doc: "Errors emitted by keyword extraction.".to_string(),
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_tuple_field__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

/// Snapshot the full output of a streaming adapter: the generated swift-bridge
/// Rust crate (which declares the opaque `StreamHandle` + `_start` + `next`)
/// and the Swift host wrapper (which exposes an `AsyncThrowingStream<Item, Error>`).
///
/// This locks down the public shape of the streaming codepath. Changes to the
/// emitted Rust shim or Swift wrapper require a deliberate snapshot review —
/// not a blanket accept — because downstream consumers (kreuzcrawl, liter-llm)
/// depend on the exact Swift surface for their hand-written facade layer.
#[test]
fn snapshot_streaming_adapter() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "demo::DefaultClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "ping".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                receiver: Some(ReceiverKind::Ref),
                trait_source: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            doc: "Streaming-capable demo client.".to_string(),
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "demo::chat_stream"
owner_type = "DefaultClient"
item_type = "ChatChunk"
error_type = "DemoError"

[[crates.adapters.params]]
name = "req"
type = "demo::ChatRequest"

[crates.swift]
client_constructor_body.DefaultClient = "Self { inner: ::demo::DefaultClient::new(api_key, base_url) }"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_streaming__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

/// Verifies that Option<T> fields stored as (ty: T, optional: true) in extractor-produced IR
/// are emitted as T? in the Swift first-class struct -- not as bare T.
///
/// The extractor calls unwrap_optional which strips TypeRef::Optional(inner) into
/// (inner, true), so field.ty = TypeRef::String and field.optional = true.
/// Previously the emitter only checked matches!(&field.ty, TypeRef::Optional(_)) which
/// was always false for extractor IR, causing optional fields to be emitted as non-optional.
#[test]
fn snapshot_first_class_struct_optional_field() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "SystemMessage".to_string(),
            rust_path: "demo::SystemMessage".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                // optional: true with unwrapped inner type -- what the extractor produces
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A system-role chat message.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    let swift_file = files
        .iter()
        .find(|f| f.path.extension().and_then(|e| e.to_str()) == Some("swift"))
        .expect("Swift source file must be emitted");

    assert!(
        swift_file.content.contains("public let name: String?"),
        "optional field must emit String?, got:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("name: String? = nil"),
        "optional init param must include = nil, got:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("self.name = rb.name()?.toString()"),
        "FFI bridge must chain through ?., got:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("public let content: String\n"),
        "non-optional field must emit bare String, got:\n{}",
        swift_file.content
    );

    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_optional_field__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

/// Snapshot the OptionsField bind_via path: a trait bridge where Swift implements a Rust trait
/// and the resulting handle is threaded into a struct field on an options type.
///
/// This exercises the bidirectional `From` impl emission:
///   - `From<inner_path> for VisitorHandle`  (factory: `VisitorHandle::from(__inner)`)
///   - `From<VisitorHandle> for inner_path`  (helper: `<inner_path>::from(h)`)
///   - `From<core_options_path> for ConversionOptions`  (helper: `ConversionOptions::from(__core)`)
///
/// Without these three impls the generated lib.rs does not compile (E0308 / E0277).
#[test]
fn snapshot_trait_bridge_inbound_options_field() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            // The type alias — a newtype wrapping the inner Arc<Mutex<dyn Trait + Send>> path.
            TypeDef {
                name: "VisitorHandle".to_string(),
                rust_path: "demo::visitor::VisitorHandle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: true,
                is_copy: false,
                doc: "Visitor handle type alias.".to_string(),
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
            },
            // The options type that receives the visitor via a field.
            TypeDef {
                name: "ConversionOptions".to_string(),
                rust_path: "demo::options::ConversionOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("timeout_ms", TypeRef::Primitive(PrimitiveType::U32), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Options for conversion.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            // The visitor trait implemented by Swift.
            TypeDef {
                name: "HtmlVisitor".to_string(),
                rust_path: "demo::visitor::HtmlVisitor".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![make_method(
                    "visit_node",
                    vec![make_param("tag", TypeRef::String)],
                    TypeRef::Unit,
                    false,
                    false,
                )],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "Visitor trait for HTML nodes.".to_string(),
                cfg: None,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
bind_via = "options_field"
options_type = "ConversionOptions"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    // Verify the three required From impls are present in the generated lib.rs.
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_str().is_some_and(|p| p.ends_with("lib.rs")))
        .expect("lib.rs must be generated");

    assert!(
        lib_rs
            .content
            .contains("impl From<demo::visitor::VisitorHandle> for VisitorHandle"),
        "forward From impl (core→wrapper) must be emitted for VisitorHandle:\n{}",
        lib_rs.content
    );
    assert!(
        lib_rs
            .content
            .contains("impl From<VisitorHandle> for demo::visitor::VisitorHandle"),
        "reverse From impl (wrapper→core) must be emitted for VisitorHandle:\n{}",
        lib_rs.content
    );
    assert!(
        lib_rs
            .content
            .contains("impl From<demo::options::ConversionOptions> for ConversionOptions"),
        "forward From impl (core→wrapper) must be emitted for ConversionOptions:\n{}",
        lib_rs.content
    );

    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_trait_bridge_inbound_options_field__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

/// Verifies that `intoRust()` on a primitive-only first-class struct emits a direct
/// `RustBridge.{Type}(...)` bulk constructor call rather than the JSON roundtrip.
///
/// Span is the canonical PoC case from tslp: all fields are `usize` (Swift `UInt`),
/// type has a `Default` impl, so the swift-bridge `#[swift_bridge(init)] fn new(...)`
/// extern is emitted and the host wrapper can call it directly.
#[test]
fn snapshot_into_rust_bulk_constructor_primitives() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Span".to_string(),
            rust_path: "demo::Span".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("start_byte", TypeRef::Primitive(PrimitiveType::Usize), false),
                make_field("end_byte", TypeRef::Primitive(PrimitiveType::Usize), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Source span.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    let swift_file = files
        .iter()
        .find(|f| f.path.extension().and_then(|e| e.to_str()) == Some("swift"))
        .expect("Swift source file must be emitted");
    let lib_rs = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("lib.rs"))
        .expect("Rust lib.rs must be emitted");

    // Bulk constructor extern must be emitted on the Rust side.
    assert!(
        lib_rs
            .content
            .contains("fn new(start_byte: usize, end_byte: usize) -> Span"),
        "Rust extern must declare a bulk-constructor extern for Span:\n{}",
        lib_rs.content
    );

    // The Swift host wrapper must call the constructor directly — NOT JSONEncoder.
    assert!(
        swift_file
            .content
            .contains("return RustBridge.Span(self.startByte, self.endByte)"),
        "intoRust must emit a direct bulk-constructor call:\n{}",
        swift_file.content
    );
    assert!(
        !swift_file.content.contains("JSONEncoder().encode(self)"),
        "intoRust must NOT use the JSONEncoder fallback for Span:\n{}",
        swift_file.content
    );
    assert!(
        !swift_file.content.contains("spanFromJson"),
        "intoRust must NOT call spanFromJson when the bulk constructor is available:\n{}",
        swift_file.content
    );

    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_intorust_bulk_primitives__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}

/// Verifies that `intoRust()` on a nested DTO (Vec<Named>, nested struct, primitive fields)
/// emits a direct bulk constructor call that builds RustVec for each Vec field and recurses
/// via `try self.field.intoRust()` for nested struct fields.
///
/// Models the tslp `ProcessResult` and `CodeChunk` shape — a top-level DTO that owns
/// nested first-class DTOs and Vec<DTO> collections.
#[test]
fn snapshot_into_rust_bulk_constructor_nested() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "Span".to_string(),
                rust_path: "demo::Span".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("start_byte", TypeRef::Primitive(PrimitiveType::Usize), false),
                    make_field("end_byte", TypeRef::Primitive(PrimitiveType::Usize), false),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Source span.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            TypeDef {
                name: "Diagnostic".to_string(),
                rust_path: "demo::Diagnostic".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("message", TypeRef::String, false),
                    make_field("span", TypeRef::Named("Span".to_string()), false),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Parse diagnostic.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
            TypeDef {
                name: "ProcessResult".to_string(),
                rust_path: "demo::ProcessResult".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("language", TypeRef::String, false),
                    make_field(
                        "diagnostics",
                        TypeRef::Vec(Box::new(TypeRef::Named("Diagnostic".to_string()))),
                        false,
                    ),
                    make_field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
                ],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Top-level processing result.".to_string(),
                cfg: None,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    let swift_file = files
        .iter()
        .find(|f| f.path.extension().and_then(|e| e.to_str()) == Some("swift"))
        .expect("Swift source file must be emitted");

    // Span (primitive-only): direct call.
    assert!(
        swift_file
            .content
            .contains("return RustBridge.Span(self.startByte, self.endByte)"),
        "Span.intoRust must use direct bulk constructor:\n{}",
        swift_file.content
    );

    // Diagnostic (nested struct): recurse via `try self.span.intoRust()`.
    assert!(
        swift_file
            .content
            .contains("return RustBridge.Diagnostic(self.message, try self.span.intoRust())"),
        "Diagnostic.intoRust must recurse via nested intoRust:\n{}",
        swift_file.content
    );

    // ProcessResult: Vec<Named> via per-element intoRust, Vec<String> via raw push.
    assert!(
        swift_file
            .content
            .contains("let __diagnostics = RustVec<RustBridge.Diagnostic>()"),
        "ProcessResult.intoRust must materialise RustVec<RustBridge.Diagnostic>:\n{}",
        swift_file.content
    );
    assert!(
        swift_file
            .content
            .contains("__diagnostics.push(value: try __elem.intoRust())"),
        "ProcessResult.intoRust must push per-element intoRust() into the Vec<Named>:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("let __tags = RustVec<String>()"),
        "ProcessResult.intoRust must materialise RustVec<String> for Vec<String>:\n{}",
        swift_file.content
    );
    assert!(
        !swift_file.content.contains("JSONEncoder().encode(self)"),
        "no DTO in this surface should use the JSONEncoder fallback:\n{}",
        swift_file.content
    );

    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_intorust_bulk_nested__{}",
                file.path.display().to_string().replace('/', "__")
            ),
            &file.content
        );
    }
}
