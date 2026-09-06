//! Regression coverage for the call-site binding of a call whose success value is `()`.
//!
//! ~keep The generated suite is compiled under `-D warnings`, so `let _ = clear_backends();` --
//! where the expression evaluates to unit -- is a hard failure (`clippy::let_unit_value`), not a
//! cosmetic one. The three cases below pin the two independent conditions that must BOTH hold
//! before the binding may be dropped: the IR declaring the success value `()`, and nothing in the
//! rendered body reading the value. Dropping on either one alone regresses the other case.

use super::*;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::{Assertion, Fixture};

/// `clear_backends() -> Result<(), Error>`, `list_backends() -> Result<Vec<String>, Error>` and
/// `get_report() -> Result<Report, Error>` — one unit-returning call and two value-returning ones,
/// so the same fixture shape can be rendered against either.
fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef {
            name: "clear_backends".to_string(),
            return_type: TypeRef::Unit,
            error_type: Some("Error".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "list_backends".to_string(),
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            error_type: Some("Error".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "get_report".to_string(),
            return_type: TypeRef::Named("Report".to_string()),
            error_type: Some("Error".to_string()),
            ..FunctionDef::default()
        },
    ]
}

fn type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Report".to_string(),
        fields: vec![FieldDef {
            name: "title".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }]
}

fn not_error_assertion() -> Assertion {
    Assertion {
        skip: None,
        assertion_type: "not_error".to_string(),
        field: None,
        value: None,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn equals_assertion(field: &str, value: &str) -> Assertion {
    Assertion {
        skip: None,
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

fn render(function: &str, assertions: Vec<Assertion>) -> String {
    let call = CallConfig {
        function: function.to_string(),
        module: "sample_lib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        ..CallConfig::default()
    };
    let e2e_config = crate::e2e::config::E2eConfig {
        call,
        ..Default::default()
    };
    let fixture = Fixture {
        id: format!("{function}_fixture"),
        description: format!("sample_lib {function}"),
        assertions,
        ..Fixture::default()
    };

    let mut out = String::new();
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"sample_lib\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let test_config = cfg.resolve().unwrap().remove(0);
    render_test_function(
        &mut out,
        &fixture,
        &e2e_config,
        &test_config,
        &type_defs(),
        &[],
        &functions(),
        "sample_lib",
        None,
        None,
        false,
    );
    out
}

#[test]
fn should_emit_bare_statement_when_the_declared_result_is_unit() {
    let out = render("clear_backends", vec![not_error_assertion()]);
    assert!(
        out.contains("    clear_backends().expect(\"call failed\");"),
        "expected a bare `.expect` statement for a unit-returning call; got:\n{out}"
    );
    assert!(
        !out.contains("let _ ="),
        "a `let _ =` binding of a unit expression is clippy::let_unit_value; got:\n{out}"
    );
}

#[test]
fn should_keep_the_discard_binding_when_the_declared_result_is_not_unit() {
    let out = render("list_backends", vec![not_error_assertion()]);
    assert!(
        out.contains("    let _ = list_backends().expect(\"call failed\");"),
        "a value-returning call keeps its `let _` binding even when nothing reads it; got:\n{out}"
    );
}

#[test]
fn should_keep_the_named_binding_when_an_assertion_reads_the_result() {
    let out = render("get_report", vec![equals_assertion("title", "Quarterly")]);
    assert!(
        out.contains("    let result = get_report().expect(\"call failed\");"),
        "expected the result binding to survive for an assertion that reads it; got:\n{out}"
    );
}
