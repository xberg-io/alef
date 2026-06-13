use super::*;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        is_ref,
        ..ParamDef::default()
    }
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        doc: "Callback method.".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        ..MethodDef::default()
    }
}

fn visitor_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_lib::visitor::{name}"),
        methods,
        is_trait: true,
        ..TypeDef::default()
    }
}

fn bridge_config(
    trait_name: &str,
    options_type: &str,
    context_type: Option<&str>,
    result_type: Option<&str>,
) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        options_type: Some(options_type.to_string()),
        context_type: context_type.map(str::to_string),
        result_type: result_type.map(str::to_string),
        ..TraitBridgeConfig::default()
    }
}

fn protocol_api(context_name: &str, result_name: &str, default_variant: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: context_name.to_string(),
            rust_path: format!("my_lib::visitor::{context_name}"),
            fields: vec![
                FieldDef {
                    name: "tag_name".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "depth".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Usize),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: result_name.to_string(),
            rust_path: format!("my_lib::visitor::{result_name}"),
            variants: vec![
                EnumVariant {
                    name: "Continue".to_string(),
                    is_default: default_variant == "Continue",
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Proceed".to_string(),
                    is_default: default_variant == "Proceed",
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    fields: vec![FieldDef {
                        name: "value".to_string(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    }],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        }],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}

#[test]
fn visitor_bindings_use_trait_name_and_callback_count_from_ir() {
    let trait_def = visitor_trait(
        "MarkdownVisitor",
        vec![method(
            "visit_text",
            vec![
                param("ctx", TypeRef::Named("NodeContext".to_string()), true),
                param("text", TypeRef::String, true),
            ],
            TypeRef::Named("VisitResult".to_string()),
        )],
    );

    let bridge_cfg = bridge_config(
        "MarkdownVisitor",
        "RenderOptions",
        Some("NodeContext"),
        Some("VisitResult"),
    );
    let api = protocol_api("NodeContext", "VisitResult", "Continue");
    let code = gen_visitor_bindings_with_api(
        "md",
        "my_lib",
        false,
        &trait_def,
        Some(&bridge_cfg),
        None,
        Some(&api),
        true,
    );

    assert!(code.contains("// Visitor / callback FFI — 1 MarkdownVisitor methods"));
    assert!(code.contains("dyn MarkdownVisitor + Send"));
    assert!(code.contains("options: *mut my_lib::RenderOptions"));
    assert!(code.contains("MD_VISIT_CUSTOM"));
    assert!(!code.contains("fn md_convert_with_visitor"));
    assert!(!code.contains("all 42 HtmlVisitor methods"));
    assert!(!code.contains("dyn HtmlVisitor + Send"));
    assert!(!code.contains("`HTM_VISIT_CUSTOM`"));
}

#[test]
fn visitor_bindings_skip_traits_without_node_context_visit_result_protocol() {
    let trait_def = visitor_trait(
        "PlainVisitor",
        vec![method(
            "visit_text",
            vec![
                param("context", TypeRef::Named("OtherContext".to_string()), true),
                param("text", TypeRef::String, true),
            ],
            TypeRef::String,
        )],
    );

    let bridge_cfg = bridge_config("PlainVisitor", "PlainOptions", None, None);
    let code = gen_visitor_bindings("pln", "my_lib", false, &trait_def, Some(&bridge_cfg), None);

    assert!(code.is_empty());
}

#[test]
fn visitor_bindings_use_configured_context_and_result_type_names() {
    let trait_def = visitor_trait(
        "RenderVisitor",
        vec![method(
            "visit_text",
            vec![
                param("context", TypeRef::Named("RenderContext".to_string()), true),
                param("text", TypeRef::String, true),
            ],
            TypeRef::Named("RenderDecision".to_string()),
        )],
    );
    let bridge_cfg = bridge_config(
        "RenderVisitor",
        "RenderOptions",
        Some("RenderContext"),
        Some("RenderDecision"),
    );
    let api = protocol_api("RenderContext", "RenderDecision", "Continue");

    let code = gen_visitor_bindings_with_api(
        "doc",
        "my_lib",
        false,
        &trait_def,
        Some(&bridge_cfg),
        None,
        Some(&api),
        true,
    );

    assert!(code.contains("ctx: &my_lib::visitor::RenderContext"));
    assert!(code.contains(") -> my_lib::visitor::RenderDecision"));
    assert!(code.contains("use my_lib::visitor::RenderDecision as VisitorResult"));
    assert!(code.contains("return my_lib::visitor::RenderDecision::Continue"));
    assert!(!code.contains("ctx: &my_lib::visitor::NodeContext"));
    assert!(!code.contains(") -> my_lib::visitor::VisitResult"));
}

#[test]
fn visitor_bindings_use_derived_default_result_variant() {
    let trait_def = visitor_trait(
        "RenderVisitor",
        vec![method(
            "visit_text",
            vec![param("context", TypeRef::Named("RenderContext".to_string()), true)],
            TypeRef::Named("RenderDecision".to_string()),
        )],
    );
    let bridge_cfg = bridge_config(
        "RenderVisitor",
        "RenderOptions",
        Some("RenderContext"),
        Some("RenderDecision"),
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "RenderContext".to_string(),
            rust_path: "my_lib::visitor::RenderContext".to_string(),
            fields: vec![FieldDef {
                name: "tag_name".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }],
        functions: vec![],
        enums: vec![EnumDef {
            name: "RenderDecision".to_string(),
            rust_path: "my_lib::visitor::RenderDecision".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Proceed".to_string(),
                    is_default: true,
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "ReplaceWith".to_string(),
                    fields: vec![FieldDef {
                        name: "value".to_string(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    }],
                    is_tuple: true,
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        }],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let code = gen_visitor_bindings_with_api(
        "doc",
        "my_lib",
        false,
        &trait_def,
        Some(&bridge_cfg),
        None,
        Some(&api),
        true,
    );

    assert!(code.contains("return my_lib::visitor::RenderDecision::Proceed"));
    assert!(code.contains("_ => my_lib::visitor::RenderDecision::Proceed"));
    assert!(!code.contains("RenderDecision::Continue"));
}
