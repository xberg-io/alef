use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::*;

pub(super) fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

pub(super) fn visitor_config_htm() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
context_type = "NodeContext"
result_type = "VisitResult"
"#,
    )
}

pub(super) fn visitor_config_ml() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
context_type = "NodeContext"
result_type = "VisitResult"
"#,
    )
}

pub(super) fn sample_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "my_lib::Config".to_string(),
            original_rust_path: String::new(),
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
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    original_type: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    original_type: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    original_type: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
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
            has_serde: true,
            super_traits: vec![],
            doc: "Configuration struct.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "my_lib::extract".to_string(),
            original_rust_path: String::new(),
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
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Named("ExtractionResult".to_string()),
            is_async: false,
            error_type: Some("MyError".to_string()),
            doc: "Extract content from a file.".to_string(),
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
            name: "OutputFormat".to_string(),
            rust_path: "my_lib::OutputFormat".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
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
                    name: "Html".to_string(),
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
            methods: vec![],
            doc: "Output format.".to_string(),
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
    }
}

/// Like `sample_api()` but includes a `SyntaxWalker` trait with representative methods.
///
/// Use this for tests that exercise visitor callback generation.  The methods cover each
/// `ParamKind` variant: Str, OptStr, Bool, U32, Usize, CellSlice, and no-params.
pub(super) fn visitor_api() -> ApiSurface {
    let mut api = sample_api();
    api.types.push(TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "my_lib::visitor::NodeContext".to_string(),
        fields: vec![
            FieldDef {
                name: "node_type".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::I32),
                ..FieldDef::default()
            },
            FieldDef {
                name: "tag_name".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..FieldDef::default()
            },
            FieldDef {
                name: "depth".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Usize),
                ..FieldDef::default()
            },
            FieldDef {
                name: "index_in_parent".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Usize),
                ..FieldDef::default()
            },
            FieldDef {
                name: "parent_tag".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..FieldDef::default()
            },
            FieldDef {
                name: "is_inline".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "HtmlVisitor".to_string(),
        rust_path: "my_lib::visitor::HtmlVisitor".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            MethodDef {
                name: "visit_text".to_string(),
                params: vec![
                    ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "text".to_string(),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Visit text nodes.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            MethodDef {
                name: "visit_element_start".to_string(),
                params: vec![ParamDef {
                    name: "ctx".to_string(),
                    ty: TypeRef::Named("NodeContext".to_string()),
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
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Called before entering any element.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            MethodDef {
                name: "visit_link".to_string(),
                params: vec![
                    ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "href".to_string(),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "title".to_string(),
                        ty: TypeRef::String,
                        optional: true,
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Visit anchor links.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            MethodDef {
                name: "visit_heading".to_string(),
                params: vec![
                    ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "level".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "text".to_string(),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Visit heading elements.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            MethodDef {
                name: "visit_blockquote".to_string(),
                params: vec![
                    ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "content".to_string(),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "depth".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Usize),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Visit blockquote elements.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            MethodDef {
                name: "visit_list_item".to_string(),
                params: vec![
                    ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "ordered".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Bool),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "text".to_string(),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Visit list items.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            MethodDef {
                name: "visit_table_row".to_string(),
                params: vec![
                    ParamDef {
                        name: "ctx".to_string(),
                        ty: TypeRef::Named("NodeContext".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "cells".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::String)),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "is_header".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Bool),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("VisitResult".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Visit table rows.".to_string(),
                receiver: Some(crate::core::ir::ReceiverKind::RefMut),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "HTML visitor trait.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });
    api.types.push(TypeDef {
        name: "RenderSettings".to_string(),
        rust_path: "my_lib::RenderSettings".to_string(),
        fields: vec![],
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "RenderedDocument".to_string(),
        rust_path: "my_lib::RenderedDocument".to_string(),
        fields: vec![],
        is_clone: true,
        is_return_type: true,
        ..TypeDef::default()
    });
    api.enums.push(EnumDef {
        name: "VisitResult".to_string(),
        rust_path: "my_lib::visitor::VisitResult".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                fields: vec![],
                is_default: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Skip".to_string(),
                fields: vec![],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "PreserveHtml".to_string(),
                fields: vec![],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![visitor_result_string_field("output")],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Error".to_string(),
                fields: vec![visitor_result_string_field("message")],
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        ..EnumDef::default()
    });
    api.functions.push(FunctionDef {
        name: "render_document".to_string(),
        rust_path: "my_lib::render_document".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                is_ref: false,
                ..ParamDef::default()
            },
            ParamDef {
                name: "settings".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("VisitorHandle".to_string()),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("RenderedDocument".to_string()),
        is_async: false,
        error_type: Some("RenderError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: true,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });
    api
}

pub(super) fn visitor_result_string_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
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
    }
}

pub(super) fn sample_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
    )
}
