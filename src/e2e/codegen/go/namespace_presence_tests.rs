use std::collections::HashSet;
use std::path::PathBuf;

use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::go_batch::{GoBatchCase, GoBatchLayout, GoCaseOutcome, run_go_batch};
use super::test_function::{GoTestFunctionContext, render_test_function};

const MODULE: &str = "example.com/namespacepresence";

fn result_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".into(),
            fields: vec![FieldDef {
                name: "action_results".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("ActionResult".into()))),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "ActionResult".into(),
            fields: vec![FieldDef {
                name: "data".into(),
                ty: TypeRef::Json,
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

#[test]
fn namespace_presence_pointer_shape_matches_rendered_accessor() {
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &HashSet::from(["action_results".into()]),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts(&result_types(), "go"),
        Some("Envelope".into()),
    );
    for path in ["action_results[0].data", "interaction.action_results[0].data"] {
        assert_eq!(resolver.target_field_is_pointer(path), Some(true), "{path}");
    }
}

fn render_presence(path: &str, assertion_type: &str) -> String {
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: MODULE.into(),
            returns_result: true,
            result_fields: HashSet::from(["action_results".into()]),
            ..Default::default()
        },
        fields_optional: HashSet::from(["action_results[].data".into()]),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "namespace_presence".into(),
        description: "namespace presence".into(),
        assertions: vec![Assertion {
            assertion_type: assertion_type.into(),
            field: Some(path.into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let functions = [FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    let mut output = String::new();
    render_test_function(
        &mut output,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample",
            e2e_config: &config,
            adapters: &[],
            data_enum_names: &HashSet::new(),
            config: &Default::default(),
            type_defs: &result_types(),
            enums: &[],
            errors: &[],
            functions: &functions,
        },
    );
    output
}

#[test]
fn namespace_presence_raw_message_checks_compile_and_reject_empty_payloads() {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for (prefix, path) in [
        ("plain", "action_results[0].data"),
        ("namespace", "interaction.action_results[0].data"),
    ] {
        for assertion in ["not_empty", "is_empty"] {
            let rendered = render_presence(path, assertion);
            for (state, setup, nonempty) in [
                ("nil", "var raw *json.RawMessage", false),
                ("empty", "value := json.RawMessage{}; raw := &value", false),
                (
                    "nonempty",
                    "value := json.RawMessage(`{\"value\":1}`); raw := &value",
                    true,
                ),
            ] {
                let name = format!("{prefix}_{assertion}_{state}");
                let source = format!(
                    "package sample\nimport \"encoding/json\"\ntype ActionResult struct {{ Data *json.RawMessage }}\ntype Envelope struct {{ ActionResults []ActionResult }}\nfunc Inspect() (*Envelope,error) {{ {setup}; return &Envelope{{ActionResults: []ActionResult{{{{Data:raw}}}}}},nil }}\n"
                );
                cases.push(GoBatchCase {
                    name: name.clone(),
                    files: vec![
                        ("sample.go".into(), source),
                        (
                            "presence_test.go".into(),
                            format!(
                                "package sample_test\nimport (\"testing\"; sample \"{MODULE}/{name}\")\n{rendered}"
                            ),
                        ),
                    ],
                });
                expected.push((
                    name,
                    if (assertion == "not_empty") == nonempty {
                        GoCaseOutcome::Passed
                    } else {
                        GoCaseOutcome::Failed
                    },
                ));
            }
        }
    }
    let layout = GoBatchLayout {
        root_files: vec![(PathBuf::from("go.mod"), format!("module {MODULE}\ngo 1.23\n"))],
        module_dir: PathBuf::new(),
        module_path: MODULE.into(),
        extra_args: vec![],
    };
    let Some(report) = run_go_batch(&layout, &cases) else {
        return;
    };
    for (name, outcome) in expected {
        report.assert_outcome(&name, outcome);
        if outcome == GoCaseOutcome::Failed {
            report.assert_output_contains(&name, "expected");
        }
    }
    assert_eq!(report.total_test_cases(), 12);
}
