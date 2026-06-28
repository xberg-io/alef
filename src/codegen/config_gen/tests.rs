use super::*;
use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn make_test_type() -> TypeDef {
    TypeDef {
        name: "Config".to_string(),
        rust_path: "my_crate::Config".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
                optional: false,
                default: Some("30".to_string()),
                doc: "Timeout in seconds".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: Some(DefaultValue::IntLiteral(30)),
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "enabled".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                optional: false,
                default: None,
                doc: "Enable feature".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: Some(DefaultValue::BoolLiteral(true)),
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "Config name".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: Some(DefaultValue::StringLiteral("default".to_string())),
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
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
        doc: "Configuration type".to_string(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
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

fn simple_type_mapper(tr: &TypeRef) -> String {
    match tr {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            _ => "i64".to_string(),
        },
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", simple_type_mapper(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", simple_type_mapper(inner)),
        TypeRef::Named(n) => n.clone(),
        _ => "Value".to_string(),
    }
}

mod constructors;
mod defaults;
mod extendr;
