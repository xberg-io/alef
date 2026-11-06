//! Emitter tests for the opt-in lazy visitor-context accessor (ADR 0019, Node port).
//!
//! These run the real `NapiBackend::generate_bindings` path so the config key, the field
//! exclusion and the generated property getter are all exercised together. The generated crate
//! is not compiled here (it needs a consumer's core crate); the assertions pin the exact shape
//! that the consumer's `ctx.attributes` read depends on.

use super::NapiBackend;
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn config(accessor: Option<(&str, &str)>) -> crate::core::config::ResolvedCrateConfig {
    let accessor_table = accessor.map_or_else(String::new, |(context, method)| {
        format!("\n[crates.node.visitor_context_accessors]\n{context} = \"{method}\"\n")
    });
    let toml = format!(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "NodeVisitor"
type_alias = "NodeVisitorHandle"
context_type = "NodeContext"
result_type = "VisitResult"
{accessor_table}
"#
    );
    let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(&toml).expect("config must parse");
    cfg.resolve().expect("config must resolve").remove(0)
}

fn api() -> ApiSurface {
    let attributes = FieldDef {
        name: "attributes".to_string(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        ..Default::default()
    };
    let context = TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "test_lib::NodeContext".to_string(),
        fields: vec![
            FieldDef {
                name: "tag".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            attributes.clone(),
        ],
        private_fields: vec![FieldDef {
            name: "attributes".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: "attributes".to_string(),
            receiver: Some(ReceiverKind::Ref),
            return_type: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let visitor = TypeDef {
        name: "NodeVisitor".to_string(),
        rust_path: "test_lib::NodeVisitor".to_string(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "visit".to_string(),
            params: vec![ParamDef {
                name: "state".to_string(),
                ty: TypeRef::Named("NodeContext".to_string()),
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Named("VisitResult".to_string()),
            has_default_impl: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = crate::core::ir::EnumDef {
        name: "VisitResult".to_string(),
        rust_path: "test_lib::VisitResult".to_string(),
        variants: vec![crate::core::ir::EnumVariant {
            name: "Continue".to_string(),
            is_default: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    ApiSurface {
        crate_name: "test-lib".to_string(),
        types: vec![context, visitor],
        enums: vec![result],
        ..Default::default()
    }
}

fn generated(accessor: Option<(&str, &str)>) -> String {
    let files = NapiBackend
        .generate_bindings(&api(), &config(accessor))
        .expect("node binding generation must succeed");
    files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("expected native lib.rs")
        .content
        .clone()
}

#[test]
fn opted_in_context_defines_a_lazy_property_and_scopes_the_callback() {
    let output = generated(Some(("NodeContext", "attributes")));

    assert!(
        output.contains(r#".with_utf8_name("attributes")"#),
        "the opted-in accessor must become a JS property getter:\n{output}"
    );
    assert!(
        output.contains("with_getter_closure"),
        "the getter must read the callback scope rather than a copied field:\n{output}"
    );
    assert!(
        output.contains("alef_call_with_scope_nodecontext"),
        "the callback must install the scope under the context's identity:\n{output}"
    );
    assert!(
        output.contains("|| ctx.attributes()"),
        "the scope reader must call the core accessor only when the getter runs:\n{output}"
    );
    assert!(
        !output.contains(r#"obj.set_named_property("attributes""#),
        "the opted-in accessor must not also be copied eagerly:\n{output}"
    );
    assert!(
        output.contains(r#"obj.set_named_property("tag""#),
        "every other context field must still be copied:\n{output}"
    );
}

/// Negative control: without the opt-in the context keeps the eager copied property and emits no
/// scope, so the assertions above are not satisfied by code that is always present.
#[test]
fn context_without_the_opt_in_keeps_the_eager_copy() {
    let output = generated(None);

    assert!(
        output.contains(r#"obj.set_named_property("attributes""#),
        "an unopted context must keep the copied property:\n{output}"
    );
    assert!(
        !output.contains("with_getter_closure"),
        "an unopted context must not emit a lazy getter:\n{output}"
    );
    assert!(
        !output.contains("alef_call_with_scope_"),
        "an unopted context must not install a scope:\n{output}"
    );
}
