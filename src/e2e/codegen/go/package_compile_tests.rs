use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};

use super::GoCodegen;
use super::go_batch::{GoBatchCase, GoBatchLayout, GoCaseOutcome, run_go_batch};

/// Assertion families whose generated package must compile and pass end to end.
const PASSING_ASSERTIONS: [(&str, &str); 5] = [
    ("equals", "value"),
    ("contains", "value"),
    ("starts_with", "\"value"),
    ("ends_with", "value\""),
    ("matches_regex", "value"),
];

/// Wrong expectations that must fail through the generated pipeline, paired with the
/// diagnostic the generated assertion is required to print. These are the batch's deliberate
/// failing controls: they share one `go test` with the passing cases and prove the single
/// invocation still reports each verdict separately. ~keep
const FAILING_ASSERTIONS: [(&str, &str, &str); 5] = [
    ("equals", "absent", "equals mismatch"),
    ("contains", "absent", "expected to contain"),
    ("starts_with", "absent", "expected to start"),
    ("ends_with", "absent", "expected to end"),
    ("matches_regex", "^absent$", "expected value to match regex"),
];

/// The generated helper must fail its test when a value cannot be marshalled.
const MARSHAL_FAILURE_CONTROL: &str = "helper_marshal_failure";

/// Total packages the batch runs: five passing families, five failing families, and the
/// marshal-failure control. Asserted as a set against what `go test` reported, since a batch
/// that silently selects fewer packages exits 0 and looks exactly like a pass. ~keep
const GENERATED_PACKAGE_CASE_COUNT: usize = 11;

const SAMPLE_PACKAGE_SOURCE: &str = "package sample\ntype Choice interface { isChoice() }\ntype ChoiceValue string\nfunc (ChoiceValue) isChoice() {}\ntype Envelope struct { Choice Choice }\nfunc Inspect() (*Envelope, error) { return &Envelope{Choice: ChoiceValue(\"value\")}, nil }\n";

fn sealed_choice_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let enums = vec![EnumDef {
        name: "Choice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            fields: vec![FieldDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }];
    let functions = vec![FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    (types, enums, functions)
}

fn fixture_with_assertion(assertion_type: &str, expected: &str) -> FixtureGroup {
    FixtureGroup {
        category: "shape".into(),
        fixtures: vec![Fixture {
            id: format!("choice_{assertion_type}"),
            description: "sealed choice assertion".into(),
            assertions: vec![Assertion {
                assertion_type: assertion_type.into(),
                field: Some("choice".into()),
                value: Some(serde_json::json!(expected)),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn generate_package(assertion_type: &str, expected: &str) -> Vec<crate::core::backend::GeneratedFile> {
    let (types, enums, functions) = sealed_choice_ir();
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        ..Default::default()
    };
    GoCodegen
        .generate(
            &[fixture_with_assertion(assertion_type, expected)],
            &config,
            &Default::default(),
            &types,
            &enums,
            &functions,
            &[],
        )
        .expect("generate complete Go e2e package")
}

/// The generated `go.mod` of one case, plus the rest of its generated files rebased onto the
/// case's own package directory. The manifest is lifted out so every case can share one
/// module root; the cases stay separately compiled packages inside it.
struct GeneratedPackageCase {
    case: GoBatchCase,
    go_mod: String,
    module_dir: PathBuf,
}

fn generated_package_case(name: &str, assertion_type: &str, expected: &str) -> GeneratedPackageCase {
    let files = generate_package(assertion_type, expected);
    let manifest = files
        .iter()
        .find(|file| file.path.file_name() == Some(OsStr::new("go.mod")))
        .expect("generated Go e2e package includes go.mod");
    let module_dir = manifest
        .path
        .parent()
        .expect("generated go.mod has a parent directory")
        .to_path_buf();
    let mut case_files = Vec::new();
    for file in &files {
        let relative = file
            .path
            .strip_prefix(&module_dir)
            .expect("generated Go e2e files live beside the generated go.mod");
        if relative == Path::new("go.mod") {
            continue;
        }
        case_files.push((relative.to_string_lossy().into_owned(), file.content.clone()));
    }
    GeneratedPackageCase {
        case: GoBatchCase {
            name: name.to_owned(),
            files: case_files,
        },
        go_mod: manifest.content.clone(),
        module_dir,
    }
}

fn marshal_failure_control_case() -> GoBatchCase {
    let helper = super::render_helpers_test_go();
    assert!(
        helper.contains("func jsonString(t *testing.T, value any) string"),
        "{helper}"
    );
    GoBatchCase {
        name: MARSHAL_FAILURE_CONTROL.to_owned(),
        files: vec![
            ("helpers_test.go".to_owned(), helper),
            (
                "failure_test.go".to_owned(),
                "package e2e_test\nimport \"testing\"\nfunc TestMarshalFailure(t *testing.T) { jsonString(t, make(chan int)) }\n"
                    .to_owned(),
            ),
        ],
    }
}

fn module_path_of(go_mod: &str) -> String {
    go_mod
        .lines()
        .find_map(|line| line.strip_prefix("module "))
        .expect("generated go.mod declares a module path")
        .trim()
        .to_owned()
}

#[test]
fn generated_data_interface_packages_compile_and_run_in_one_go_test() {
    let mut generated = Vec::new();
    for (assertion_type, expected) in PASSING_ASSERTIONS {
        generated.push(generated_package_case(
            &format!("pass_{assertion_type}"),
            assertion_type,
            expected,
        ));
    }
    for (assertion_type, expected, _) in FAILING_ASSERTIONS {
        generated.push(generated_package_case(
            &format!("fail_{assertion_type}"),
            assertion_type,
            expected,
        ));
    }

    // Sharing one module root only drops per-case coverage if the cases disagree about the
    // manifest, so prove they do not rather than assuming it. ~keep
    let manifest = generated[0].go_mod.clone();
    let module_dir = generated[0].module_dir.clone();
    for case in &generated {
        assert_eq!(
            case.go_mod, manifest,
            "case `{}` generated a different go.mod; it can no longer share a module root",
            case.case.name
        );
        assert_eq!(
            case.module_dir, module_dir,
            "case `{}` moved output_base",
            case.case.name
        );
    }

    let mut cases: Vec<GoBatchCase> = generated.into_iter().map(|entry| entry.case).collect();
    cases.push(marshal_failure_control_case());
    let inventory: Vec<String> = cases.iter().map(|case| case.name.clone()).collect();
    assert_eq!(
        inventory.len(),
        GENERATED_PACKAGE_CASE_COUNT,
        "the generated-package inventory changed; update GENERATED_PACKAGE_CASE_COUNT deliberately"
    );

    let layout = GoBatchLayout {
        root_files: vec![
            (
                PathBuf::from("packages/go/go.mod"),
                "module example.com/sample\n\ngo 1.26\n".to_owned(),
            ),
            (PathBuf::from("packages/go/sample.go"), SAMPLE_PACKAGE_SOURCE.to_owned()),
            (module_dir.join("go.mod"), manifest.clone()),
        ],
        module_dir,
        module_path: module_path_of(&manifest),
        extra_args: vec!["-mod=mod".to_owned()],
    };
    let Some(report) = run_go_batch(&layout, &cases) else {
        return;
    };

    report.assert_inventory(&inventory);
    for (assertion_type, _) in PASSING_ASSERTIONS {
        let name = format!("pass_{assertion_type}");
        report.assert_outcome(&name, GoCaseOutcome::Passed);
        assert!(
            report.case(&name).test_case_count >= 1,
            "case `{name}` selected no Go test to run:\n{}",
            report.case(&name).output
        );
    }
    for (assertion_type, _, diagnostic) in FAILING_ASSERTIONS {
        let name = format!("fail_{assertion_type}");
        report.assert_outcome(&name, GoCaseOutcome::Failed);
        report.assert_output_contains(&name, diagnostic);
    }
    report.assert_outcome(MARSHAL_FAILURE_CONTROL, GoCaseOutcome::Failed);
    report.assert_output_contains(MARSHAL_FAILURE_CONTROL, "marshal assertion value as JSON");
    assert_eq!(
        report.total_test_cases(),
        GENERATED_PACKAGE_CASE_COUNT,
        "the batch executed a different number of Go tests than it has cases"
    );
}

#[test]
fn generated_equals_data_interface_emits_json_helper() {
    let files = generate_package("equals", "value");
    assert!(
        files.iter().any(|file| file.path.ends_with("helpers_test.go")),
        "equals emits jsonString and must emit its package helper"
    );
    let shape = files
        .iter()
        .find(|file| file.path.ends_with("shape_test.go"))
        .expect("shape_test.go is generated");
    assert_eq!(
        shape.content.matches("jsonString(t, result.Choice)").count(),
        1,
        "generated assertion must call the real JSON helper:\n{}",
        shape.content
    );
}
