//! Whole-file coverage for the Go typed-error assertion: the rendered test must declare the
//! pointer-typed error the generated binding returns, import `errors` for it, and still keep the
//! message check for an error type that carries no `ErrorType` field.
//!
//! Split out of `tests.rs`, which is near the 1000-line cap and may not grow. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ErrorDef, ErrorVariant, MethodDef, TypeRef};
use crate::e2e::codegen::go::{GoTestFileContext, render_test_file};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn error_fixture(declared: &str) -> Fixture {
    Fixture {
        id: "budget_enforced".to_string(),
        description: "Budget is enforced".to_string(),
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        assertions: vec![
            Assertion {
                assertion_type: "error".to_string(),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(declared.to_string())),
                ..Assertion::default()
            },
        ],
        ..Fixture::default()
    }
}

fn errors(with_error_type: bool) -> Vec<ErrorDef> {
    let methods = if with_error_type {
        vec![MethodDef {
            name: "error_type".to_string(),
            return_type: TypeRef::String,
            ..MethodDef::default()
        }]
    } else {
        Vec::new()
    };
    vec![ErrorDef {
        name: "SampleError".to_string(),
        rust_path: "sample::SampleError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "BudgetExceeded".to_string(),
            error_code: Some(100),
            is_unit: true,
            ..ErrorVariant::default()
        }],
        doc: String::new(),
        methods,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }]
}

fn render(fixture: &Fixture, errors: &[ErrorDef]) -> String {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat".to_string(),
            module: "github.com/example/sample".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    render_test_file(
        "budget",
        &[fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/sample",
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &[],
            enums: &[],
            errors,
            functions: &[],
            crate_facts: None,
        },
    )
}

#[test]
fn a_named_variant_is_asserted_through_the_pointer_typed_error() {
    let out = render(&error_fixture("BudgetExceeded"), &errors(true));

    assert!(out.contains("\t\"errors\"\n"), "{out}");
    assert!(out.contains("var typedError *pkg.Error"), "{out}");
    assert!(out.contains("errors.As(err, &typedError)"), "{out}");
    assert!(out.contains("typedError.ErrorType != `BudgetExceeded`"), "{out}");
    assert!(!out.contains("strings.Contains(err.Error()"), "{out}");
    assert!(out.contains("\tpkg \"github.com/example/sample\""), "{out}");
}

/// Negative control for the whole file: no `ErrorType` field means no `errors` import and the
/// pre-existing message-or-type-name check, so an unused import cannot break compilation.
#[test]
fn an_error_type_without_the_field_keeps_the_message_check_and_imports_no_errors() {
    let out = render(&error_fixture("BudgetExceeded"), &errors(false));

    assert!(out.contains("strings.Contains(err.Error(), `BudgetExceeded`)"), "{out}");
    assert!(!out.contains("errors.As("), "{out}");
    assert!(!out.contains("\"errors\""), "{out}");
}

/// The generated file is parsed by `gofmt -e` so a malformed block or import list is caught here
/// rather than in a consumer's `go test`.
#[test]
fn the_typed_error_test_file_parses_as_go() {
    let out = render(&error_fixture("BudgetExceeded"), &errors(true));
    let Ok(mut child) = std::process::Command::new("gofmt")
        .arg("-e")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    use std::io::Write as _;
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(out.as_bytes())
        .expect("write Go test file");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(
        output.status.success(),
        "{}\n{out}",
        String::from_utf8_lossy(&output.stderr)
    );
}
