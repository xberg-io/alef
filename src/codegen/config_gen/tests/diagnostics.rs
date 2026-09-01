use crate::codegen::config_gen::validate_rust_default_functions;
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, TypeDef, TypeRef};

fn unrecoverable_type(name: &str, field_name: &str, default_path: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        fields: vec![FieldDef {
            name: field_name.to_string(),
            ty: TypeRef::Named("NestedSettings".to_string()),
            typed_default: Some(DefaultValue::FunctionCall(default_path.to_string())),
            ..FieldDef::default()
        }],
        has_serde: false,
        ..TypeDef::default()
    }
}

#[test]
fn reports_all_unrecoverable_serde_default_functions_together() {
    let api = ApiSurface {
        types: vec![
            unrecoverable_type("ClientSettings", "policy", "private_policy"),
            unrecoverable_type("ServerSettings", "retries", "Self::default_retries"),
        ],
        ..ApiSurface::default()
    };

    let error = validate_rust_default_functions(&api).expect_err("both defaults must fail closed");
    let message = error.to_string();

    for expected in [
        "cannot preserve 2 serde default function(s)",
        "sample_core::ClientSettings::policy",
        "private_policy",
        "sample_core::ServerSettings::retries",
        "Self::default_retries",
        "public, unconditional, zero-argument static method",
        "not `Self::default_retry_limit`",
    ] {
        assert!(message.contains(expected), "missing `{expected}` from: {message}");
    }
}
