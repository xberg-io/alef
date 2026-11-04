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

fn input_result_tail(ty: TypeRef, optional_default: bool) -> String {
    context().input_expression(
        &ParamDef {
            name: "policy".into(),
            ty,
            ..Default::default()
        },
        optional_default,
    )
}

#[test]
fn fallible_named_input_returns_its_result_directly() {
    let actual = input_result_tail(TypeRef::Named("Policy".into()), false);
    assert_eq!(
        actual,
        concat!(
            "(policy).try_into().map_err(|error: serde_json::Error| ",
            "napi::Error::new(napi::Status::InvalidArg, format!(\"invalid JSON for sample_core::Policy: {error}\")))?"
        )
    );
}

#[test]
fn nested_fallible_input_returns_results_at_every_tail() {
    let actual = input_result_tail(
        TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::Named("Policy".into()))))),
        false,
    );
    assert_eq!(
        actual,
        format!(
            "{}{}?",
            concat!(
                "(policy).into_iter().map(|value| -> std::result::Result<_, serde_json::Error> { ",
                "(value).map(|value| -> std::result::Result<_, serde_json::Error> { (value).try_into() }).transpose() ",
                "}).collect::<std::result::Result<_, _>>()"
            ),
            invalid_serde_arg("sample_core::Policy")
        )
    );
}

#[test]
fn defaulted_fallible_input_keeps_unwrap_and_propagates_conversion_errors() {
    let actual = input_result_tail(TypeRef::Named("Policy".into()), true);
    assert_eq!(
        actual,
        format!(
            "{}{}?",
            concat!(
                "(|| -> std::result::Result<_, serde_json::Error> { ",
                "Ok((policy).map(|value| -> std::result::Result<_, serde_json::Error> { ",
                "(value).try_into() }).transpose()?.unwrap_or_default()) })()"
            ),
            invalid_arg("sample_core::Policy")
        )
    );
}
