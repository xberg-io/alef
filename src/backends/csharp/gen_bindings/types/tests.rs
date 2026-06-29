use super::gen_record_type;
use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::HashSet;

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        original_type: None,
        cfg: None,
        typed_default: None,
        core_wrapper: Default::default(),
        vec_inner_core_wrapper: Default::default(),
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn record_type(fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "RenderOptions".to_string(),
        rust_path: "demo::RenderOptions".to_string(),
        original_rust_path: "demo::RenderOptions".to_string(),
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
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn record_type_maps_configured_bridge_alias_to_trait_interface() {
    let typ = record_type(vec![
        field(
            "walker",
            TypeRef::Optional(Box::new(TypeRef::Named("WalkerHandle".to_string()))),
        ),
        field("visitor_count", TypeRef::Primitive(PrimitiveType::U32)),
    ]);
    let bridge = TraitBridgeConfig {
        trait_name: "XmlWalker".to_string(),
        type_alias: Some("WalkerHandle".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("walker".to_string()),
        ..TraitBridgeConfig::default()
    };
    let aliases = HashSet::from(["WalkerHandle".to_string()]);

    let code = gen_record_type(
        &typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &aliases,
        &[bridge],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(code.contains("public IXmlWalker? Walker { get; init; } = null;"));
    assert!(code.contains("public uint VisitorCount"));
    assert!(!code.contains("IHtmlVisitor"));
    assert!(!code.contains("VisitorHandle"));
}
