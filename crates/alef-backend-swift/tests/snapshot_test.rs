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
