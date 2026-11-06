//! Emitter tests for the opt-in lazy visitor-context accessor (ADR 0019, R port).

use super::super::ExtendrBackend;
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn config(accessor: Option<(&str, &str)>) -> crate::core::config::ResolvedCrateConfig {
    let accessor_table = accessor.map_or_else(String::new, |(context, method)| {
        format!("\n[crates.r.visitor_context_accessors]\n{context} = \"{method}\"\n")
    });
    let toml = format!(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

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
    let map = || TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    let context = TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "test_lib::NodeContext".to_string(),
        fields: vec![
            FieldDef {
                name: "tag".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "attributes".to_string(),
                ty: map(),
                ..Default::default()
            },
        ],
        private_fields: vec![FieldDef {
            name: "attributes".to_string(),
            ty: map(),
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: "attributes".to_string(),
            receiver: Some(ReceiverKind::Ref),
            return_type: map(),
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
    ExtendrBackend
        .generate_bindings(&api(), &config(accessor))
        .expect("r binding generation must succeed")
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("expected lib.rs")
        .content
        .clone()
}

#[test]
fn opted_in_context_gains_a_callable_and_scopes_the_callback() {
    let output = generated(Some(("NodeContext", "attributes")));

    assert!(
        output.contains("eval_string_with_params"),
        "the opted-in accessor must become a callable, not a copied list element:\n{output}"
    );
    assert!(
        output.contains("alef_visitor_scope_read_nodecontext"),
        "the callable must reach the registered reader:\n{output}"
    );
    assert!(
        output.contains("fn alef_visitor_scope_read_nodecontext;"),
        "the reader must be registered in extendr_module!:\n{output}"
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
        output.contains(r#"("tag", extendr_api::Robj::from"#),
        "every other context field must still be copied:\n{output}"
    );
}

#[test]
fn context_without_the_opt_in_keeps_the_eager_copy() {
    let output = generated(None);

    assert!(
        output.contains("ctx.attributes()"),
        "an unopted context must read and copy the value eagerly:\n{output}"
    );
    assert!(
        !output.contains("eval_string_with_params"),
        "an unopted context must not emit a lazy callable:\n{output}"
    );
    assert!(
        !output.contains("alef_call_with_scope_"),
        "an unopted context must not install a scope:\n{output}"
    );
}
