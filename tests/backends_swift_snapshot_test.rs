use alef::backends::swift::SwiftBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn assert_swift_snapshots(prefix: &str, files: &[GeneratedFile]) {
    for file in files {
        let path = file.path.to_string_lossy();
        if path.ends_with("Sources/RustBridgeC/RustBridgeC.h") {
            continue;
        }
        insta::assert_snapshot!(
            format!("{prefix}__{}", file.path.display().to_string().replace('/', "__")),
            &file.content
        );
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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
            version: Default::default(),
        }],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "demo::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    doc: "Inactive state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
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
            excluded_variants: vec![],
            version: Default::default(),
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
                    is_tuple: false,
                    doc: "Input validation failed.".to_string(),
                },
                ErrorVariant {
                    name: "ProcessingFailed".to_string(),
                    message_template: Some("processing failed".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                    doc: "Processing encountered an error.".to_string(),
                },
            ],
            doc: "Errors emitted by demo operations.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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
    assert_swift_snapshots("snapshot_basic", &files);
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    assert_swift_snapshots("snapshot_conversion_struct", &files);
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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
                    doc: "Success variant.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Error".to_string(),
                    fields: vec![make_field("message", TypeRef::String, false)],
                    doc: "Error variant.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
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
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    assert_swift_snapshots("snapshot_conversion_enum", &files);
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    assert_swift_snapshots("snapshot_conversion_vec", &files);
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
        version: Default::default(),
    }
}

#[test]
fn snapshot_trait_bridge_inbound() {
    // Simulates a sample_crate-style plugin trait with a Plugin super-trait, async fallible
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ImageConfig".to_string(),
                rust_path: "demo::ImageConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("language", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                doc: "Image configuration.".to_string(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ProcessingResult".to_string(),
                rust_path: "demo::ProcessingResult".to_string(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ImageProcessor".to_string(),
                rust_path: "demo::plugins::ImageProcessor".to_string(),
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
                                map_is_ahash: false,
                                map_key_is_cow: false,
                                vec_inner_is_ref: false,
                                map_is_btree: false,
                                core_wrapper: alef::core::ir::CoreWrapper::None,
                            },
                            ParamDef {
                                name: "config".into(),
                                ty: TypeRef::Named("ImageConfig".to_string()),
                                optional: false,
                                default: None,
                                sanitized: false,
                                typed_default: None,
                                is_ref: true,
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
                        TypeRef::Named("ProcessingResult".to_string()),
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
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                        }],
                        TypeRef::Primitive(PrimitiveType::Bool),
                        false,
                        false,
                    ),
                ],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "Image backend plugin trait.".to_string(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "ImageProcessor"
super_trait = "demo::plugins::Plugin"
registry_getter = "demo::plugins::registry::get_image_processor_registry"
register_fn = "register_image_processor"
unregister_fn = "unregister_image_processor"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    assert_swift_snapshots("snapshot_trait_bridge_inbound", &files);
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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
            version: Default::default(),
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
                is_tuple: false,
                doc: "Extraction encountered an error.".to_string(),
            }],
            doc: "Errors emitted by keyword extraction.".to_string(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    assert_swift_snapshots("snapshot_tuple_field", &files);
}

/// Snapshot the full output of a streaming adapter: the generated swift-bridge
/// Rust crate (which declares the opaque `StreamHandle` + `_start` + `next`)
/// and the Swift host wrapper (which exposes an `AsyncThrowingStream<Item, Error>`).
///
/// This locks down the public shape of the streaming codepath. Changes to the
/// emitted Rust shim or Swift wrapper require a deliberate snapshot review —
/// not a blanket accept — because downstream consumers with streaming fixtures
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
                version: Default::default(),
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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
    assert_swift_snapshots("snapshot_streaming", &files);
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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

    assert_swift_snapshots("snapshot_optional_field", &files);
}

/// Snapshot the OptionsField bind_via path: a trait bridge where Swift implements a Rust trait
/// and the resulting handle is threaded into a struct field on an options type.
///
/// This exercises the bidirectional `From` impl emission:
///   - `From<inner_path> for CallbackHandle`  (factory: `CallbackHandle::from(__inner)`)
///   - `From<CallbackHandle> for inner_path`  (helper: `<inner_path>::from(h)`)
///   - `From<core_options_path> for RenderOptions`  (helper: `RenderOptions::from(__core)`)
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
                name: "CallbackHandle".to_string(),
                rust_path: "demo::visitor::CallbackHandle".to_string(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // The options type that receives the visitor via a field.
            TypeDef {
                name: "RenderOptions".to_string(),
                rust_path: "demo::options::RenderOptions".to_string(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            // The visitor trait implemented by Swift.
            TypeDef {
                name: "MarkupVisitor".to_string(),
                rust_path: "demo::visitor::MarkupVisitor".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![make_method(
                    "visit_node",
                    vec![make_param("tag", TypeRef::String)],
                    TypeRef::Named("FlowDecision".to_string()),
                    false,
                    false,
                )],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                doc: "Visitor trait for markup nodes.".to_string(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![EnumDef {
            name: "FlowDecision".to_string(),
            rust_path: "demo::visitor::FlowDecision".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Accept".to_string(),
                    fields: vec![],
                    doc: "Accept the event.".to_string(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Stop".to_string(),
                    fields: vec![],
                    doc: "Stop processing.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
            ],
            doc: "Decision returned by a visitor callback.".to_string(),
            cfg: None,
            is_copy: true,
            has_serde: true,
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
        ..Default::default()
    };

    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "MarkupVisitor"
type_alias = "CallbackHandle"
param_name = "visitor"
bind_via = "options_field"
options_type = "RenderOptions"
result_type = "FlowDecision"
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
            .contains("impl From<demo::visitor::CallbackHandle> for CallbackHandle"),
        "forward From impl (core→wrapper) must be emitted for CallbackHandle:\n{}",
        lib_rs.content
    );
    assert!(
        lib_rs
            .content
            .contains("impl From<CallbackHandle> for demo::visitor::CallbackHandle"),
        "reverse From impl (wrapper→core) must be emitted for CallbackHandle:\n{}",
        lib_rs.content
    );
    assert!(
        lib_rs
            .content
            .contains("impl From<demo::options::RenderOptions> for RenderOptions"),
        "forward From impl (core→wrapper) must be emitted for RenderOptions:\n{}",
        lib_rs.content
    );

    assert_swift_snapshots("snapshot_trait_bridge_inbound_options_field", &files);
}

/// Verifies that `intoRust()` on a primitive-only first-class struct emits a direct
/// `RustBridge.{Type}(...)` bulk constructor call rather than the JSON roundtrip.
///
/// Span is the canonical PoC case from sample_language_pack: all fields are `usize` (Swift `UInt`),
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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
    // Top-level `spanFromJson` forwarder is still emitted (every from_json-eligible
    // type gets one — see `emit_from_json_forwarders`), but it must use the
    // JSONDecoder path, not route through `RustBridge.spanFromJson`. The intoRust
    // body must use the direct bulk constructor.
    assert!(
        !swift_file.content.contains("return try RustBridge.spanFromJson("),
        "intoRust must NOT call RustBridge.spanFromJson when the bulk constructor is available:\n{}",
        swift_file.content
    );

    assert_swift_snapshots("snapshot_intorust_bulk_primitives", &files);
}

/// Verifies that `intoRust()` is emitted only for directly bridgeable first-class DTOs.
///
/// Complex DTOs with nested wrappers or vectors remain RustBridge typealiases until
/// the Swift backend has complete bidirectional conversions for those shapes.
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
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
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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

    // Diagnostic (nested struct) and ProcessResult (vectors) are now emitted as
    // first-class Swift structs because all their fields are known DTO types.
    assert!(
        swift_file.content.contains("public struct Diagnostic:"),
        "Diagnostic must be a first-class struct:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("public struct ProcessResult:"),
        "ProcessResult must be a first-class struct:\n{}",
        swift_file.content
    );

    // Diagnostic.intoRust() uses direct constructor (has_default=true + Named field).
    assert!(
        swift_file
            .content
            .contains("return RustBridge.Diagnostic(RustString(self.message), try self.span.intoRust())"),
        "Diagnostic.intoRust must use direct bulk constructor with nested intoRust:\n{}",
        swift_file.content
    );

    // ProcessResult.init(_ rb:) must use .map conversions for Vec fields.
    assert!(
        swift_file
            .content
            .contains("try rb.diagnostics().map { try Diagnostic($0) }"),
        "ProcessResult init must convert Vec<Diagnostic> via .map:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("rb.tags().map { $0.as_str().toString() }"),
        "ProcessResult init must convert Vec<String> via .map:\n{}",
        swift_file.content
    );

    assert_swift_snapshots("snapshot_intorust_bulk_nested", &files);
}

/// Primitive-only serde DTOs without a `Default` impl (e.g. `Point { row: u32,
/// column: u32 }`, `ByteRange { start: usize, end: usize }`) must still get a
/// positional `fn new(...)` constructor extern emitted to the swift-bridge
/// extern block — and the Swift `intoRust()` must call it directly rather than
/// routing through a JSON-roundtrip path.
#[test]
fn snapshot_intorust_bulk_constructor_primitive_no_default() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Point".to_string(),
            rust_path: "demo::Point".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("row", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("column", TypeRef::Primitive(PrimitiveType::U32), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: true,
            doc: "Source-text position (row, column).".to_string(),
            cfg: None,
            is_trait: false,
            // Critical: serde-enabled but NO Default impl. Pre-fix this slipped into the
            // JSON-roundtrip path; post-fix the primitive-only fast path emits the bulk
            // constructor extern regardless.
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
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

    // Rust crate side: positional constructor extern emitted despite no Default impl.
    assert!(
        lib_rs.content.contains("fn new(row: u32, column: u32) -> Point"),
        "primitive-only serde DTO without Default must still declare a bulk-constructor extern:\n{}",
        lib_rs.content
    );
    // Rust crate side: from_json shim emitted so the top-level Swift fromJson
    // forwarder has a matching bridge symbol even though intoRust stays direct.
    assert!(
        lib_rs.content.contains("fn point_from_json("),
        "primitive-only DTO must emit a JSON-roundtrip shim for the fromJson forwarder:\n{}",
        lib_rs.content
    );

    // Swift side: direct positional construction; no JSONEncoder fallback.
    assert!(
        swift_file
            .content
            .contains("return RustBridge.Point(self.row, self.column)"),
        "intoRust must call positional constructor directly:\n{}",
        swift_file.content
    );
    // Top-level `pointFromJson` forwarder is still emitted (every from_json-eligible
    // type gets one — see `emit_from_json_forwarders`), but it must use JSONDecoder.
    // The intoRust body must use the direct bulk constructor.
    assert!(
        !swift_file.content.contains("return try RustBridge.pointFromJson("),
        "intoRust must NOT call RustBridge.pointFromJson when bulk constructor is available:\n{}",
        swift_file.content
    );
    assert!(
        !swift_file.content.contains("JSONEncoder().encode(self)"),
        "intoRust must NOT use the JSONEncoder fallback for Point:\n{}",
        swift_file.content
    );

    assert_swift_snapshots("snapshot_intorust_bulk_primitive_no_default", &files);
}

/// DTOs whose fields cannot be bridged through the positional constructor (e.g.
/// `HashMap<String, _>` — forces JSON bridging) must have a matching
/// `{type_snake}_from_json` shim emitted on the Rust crate side via the shared
/// `collect_json_fallback_types` predicate. This is the defensive symmetry that
/// keeps the binding side and the Rust crate side in lockstep: if the Swift host
/// would ever JSON-encode the DTO at runtime, the matching Rust symbol exists.
///
/// (Today's `can_emit_first_class_struct` gate keeps Map-bearing DTOs as
/// RustBridge typealiases so `intoRust()` is not even emitted on the Swift side
/// — but the shim is still pre-positioned so any future first-class emission
/// Just Works.)
#[test]
fn snapshot_intorust_json_fallback_shim_present_for_map_dto() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Metadata".to_string(),
            rust_path: "demo::Metadata".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("kind", TypeRef::String, false),
                // HashMap<String, String> forces JSON bridging on the field — which forces
                // default-construction → because there's no Default impl, the swift-bridge
                // bulk constructor extern cannot be emitted → Swift falls back to JSON.
                make_field(
                    "attrs",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Generic metadata bag.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    let lib_rs = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("lib.rs"))
        .expect("Rust lib.rs must be emitted");

    // The from_json shim must be declared inside the swift-bridge module …
    assert!(
        lib_rs
            .content
            .contains("fn metadata_from_json(json: String) -> Result<Metadata, String>"),
        "JSON-fallback DTO must have a matching Rust *_from_json extern declared:\n{}",
        lib_rs.content
    );
    // … and implemented as a pub free function so the swift-bridge codegen links it.
    assert!(
        lib_rs
            .content
            .contains("pub fn metadata_from_json(json: String) -> Result<Metadata, String>"),
        "JSON-fallback DTO must have a matching pub fn *_from_json implementation:\n{}",
        lib_rs.content
    );

    assert_swift_snapshots("snapshot_intorust_json_fallback_shim_present", &files);
}

/// Verifies that `Option<T>` fields stored as `(ty: T, optional: true)` in
/// extractor-produced IR are emitted as `T?` in the Swift enum case associated
/// values -- not as bare `T`.
///
/// The extractor unwraps `TypeRef::Optional(inner)` into `(inner, optional: true)`,
/// so `field.ty = TypeRef::Named("Chunk")` and `field.optional = true` for an
/// `Option<Chunk>` field. Previously `emit_variant_with_data` only called
/// `mapper.map_type(&f.ty)` without honoring `f.optional`, so nullable associated
/// values were emitted without `?`, losing the ability to express `null` on the
/// Swift API surface.
#[test]
fn snapshot_enum_variant_optional_field() {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Chunk".to_string(),
            rust_path: "demo::Chunk".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("text", TypeRef::String, false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A text chunk.".to_string(),
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "StreamEvent".to_string(),
            rust_path: "demo::StreamEvent".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                // Variant with optional Named field -- the extractor-unwrapped form:
                // field.ty = TypeRef::Named("Chunk"), field.optional = true.
                EnumVariant {
                    name: "Data".to_string(),
                    fields: vec![make_field("chunk", TypeRef::Named("Chunk".to_string()), true)],
                    doc: "A data event carrying an optional chunk.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                // Variant with optional String field -- same extractor form.
                EnumVariant {
                    name: "Error".to_string(),
                    fields: vec![make_field("message", TypeRef::String, true)],
                    doc: "An error event with an optional message.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                // Variant with non-optional field -- must stay bare.
                EnumVariant {
                    name: "Done".to_string(),
                    fields: vec![make_field("count", TypeRef::Primitive(PrimitiveType::U32), false)],
                    doc: "Stream completed with item count.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
            ],
            doc: "Streaming event enum.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
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
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    let swift_file = files
        .iter()
        .find(|f| f.path.extension().and_then(|e| e.to_str()) == Some("swift"))
        .expect("Swift source file must be emitted");

    assert!(
        swift_file.content.contains("chunk: Chunk?"),
        "optional Named field in enum variant must emit Type?, got:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("message: String?"),
        "optional String field in enum variant must emit String?, got:\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("count: UInt32"),
        "non-optional field in enum variant must stay bare, got:\n{}",
        swift_file.content
    );

    assert_swift_snapshots("snapshot_enum_variant_optional_field", &files);
}

#[test]
fn untagged_enum_field_uses_json_decoder_not_ref_init() {
    // Regression test: a struct whose field type is a `#[serde(untagged)]` enum must emit
    // a JSONDecoder decode expression in `init(_ rb: RustBridge.{Struct}Ref) throws`, NOT
    // the opaque-Ref path `try {EnumType}(rb.{field}())`.
    //
    // The bug: untagged enums were added to `known_dto_names` (correct — they are Codable),
    // but the field-init codegen emitted `try UserContent(rb.content())` as if a
    // `RustBridge.UserContentRef`-taking initializer existed. swift-bridge does not generate
    // an opaque Ref type for untagged enums; the accessor returns a plain `RustString`
    // (JSON-encoded payload). The fix routes these fields through `JSONDecoder`.
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Message".to_string(),
            rust_path: "demo::Message".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::Named("MessageContent".to_string()), false),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A chat message.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "MessageContent".to_string(),
            rust_path: "demo::MessageContent".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![make_field("field0", TypeRef::String, false)],
                    is_tuple: true,
                    doc: "Plain text content.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Parts".to_string(),
                    fields: vec![make_field("field0", TypeRef::Vec(Box::new(TypeRef::String)), false)],
                    is_tuple: true,
                    doc: "Array of parts.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
            ],
            doc: "Untagged content enum.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: None,
            serde_untagged: true,
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
        ..Default::default()
    };

    let config = make_basic_config();
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();

    let swift_file = files
        .iter()
        .find(|f| f.path.extension().and_then(|e| e.to_str()) == Some("swift"))
        .expect("Swift source file must be emitted");

    // The field-init for `content` must use JSONDecoder, not `try MessageContent(rb.content())`.
    assert!(
        !swift_file.content.contains("try MessageContent(rb.content())"),
        "untagged enum field must NOT emit Ref-based init — would fail with 'missing argument label \
         from:' / 'RustString does not conform to Decoder':\n{}",
        swift_file.content
    );
    assert!(
        swift_file.content.contains("JSONDecoder().decode(MessageContent.self"),
        "untagged enum field must decode via JSONDecoder:\n{}",
        swift_file.content
    );
    // Verify Message is still emitted as a first-class Swift struct (not a typealias).
    assert!(
        swift_file.content.contains("public struct Message:"),
        "Message must still be emitted as a first-class struct:\n{}",
        swift_file.content
    );
}
