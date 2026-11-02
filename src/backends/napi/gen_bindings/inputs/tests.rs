use super::*;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef};

fn context() -> ContainerContext {
    ContainerContext::new(
        &ApiSurface {
            enums: vec![EnumDef {
                name: "Policy".into(),
                rust_path: "sample_core::Policy".into(),
                serde_untagged: true,
                variants: vec![EnumVariant {
                    name: "Text".into(),
                    fields: vec![FieldDef {
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
        "sample_core",
    )
}

#[test]
fn optional_borrowed_inputs_convert_the_owned_local_before_borrowing_it() {
    let parameter = ParamDef {
        name: "policy".into(),
        ty: TypeRef::Named("Policy".into()),
        optional: true,
        is_ref: true,
        ..Default::default()
    };
    let bindings = prepare_bindings(
        "let policy_owned: Option<sample_core::Policy> = policy.map(Into::into);\n\
         let policy_core = policy_owned.as_ref();"
            .into(),
        Some(&context()),
        std::slice::from_ref(&parameter),
        std::slice::from_ref(&parameter),
        false,
    );
    let block: syn::Block = syn::parse_str(&format!("{{{bindings}}}")).unwrap();
    assert_eq!(block.stmts.len(), 2);
    let syn::Stmt::Local(owned) = &block.stmts[0] else {
        panic!("owned local")
    };
    assert!(
        owned
            .init
            .as_ref()
            .unwrap()
            .expr
            .to_token_stream()
            .to_string()
            .contains("try_into")
    );
    let syn::Stmt::Local(borrowed) = &block.stmts[1] else {
        panic!("borrow projection")
    };
    assert_eq!(
        borrowed.init.as_ref().unwrap().expr.to_token_stream().to_string(),
        "policy_owned . as_ref ()"
    );
    assert_eq!(bindings.matches("try_into").count(), 1);
}

#[test]
fn input_errors_keep_the_core_type_and_original_serde_detail() {
    let expression = context().input_expression(
        &ParamDef {
            name: "policy".into(),
            ty: TypeRef::Named("Policy".into()),
            ..Default::default()
        },
        false,
    );
    assert!(
        expression.contains("invalid JSON for sample_core::Policy: {error}"),
        "{expression}"
    );
    let conversion = serde_try_from_impl("JsPolicy", "sample_core::Policy", "serde_json::from_value(val.0)");
    assert!(
        conversion.contains("invalid JSON for sample_core::Policy: {error}"),
        "{conversion}"
    );
}
