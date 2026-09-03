use std::collections::HashSet;

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::test_support::toolchain;

use super::gen_struct_type;

/// Compile the generated struct with the real Go toolchain, or `None` when Go is not installed.
///
/// `None` is not a pass: every caller must return without asserting, and
/// [`toolchain::ToolchainGate::open`] has already counted the skip so the run reports how many of
/// these fixtures actually executed. ~keep
fn go_compile(generated: &str, declarations: &str) -> Option<std::process::Output> {
    let go = toolchain::GO.open()?;
    let directory = tempfile::tempdir().expect("create Go compile fixture");
    std::fs::write(directory.path().join("go.mod"), "module example.com/shape\n\ngo 1.24\n").expect("write Go module");
    std::fs::write(
        directory.path().join("shape.go"),
        format!("package shape\n\nimport \"encoding/json\"\n\n{declarations}\n{generated}"),
    )
    .expect("write generated Go source");
    Some(
        std::process::Command::new(go)
            .arg("test")
            .arg("./...")
            .current_dir(directory.path())
            .output()
            .expect("run Go compiler"),
    )
}

fn assert_go_compiles(generated: &str, declarations: &str) {
    let Some(output) = go_compile(generated, declarations) else {
        return;
    };
    assert!(
        output.status.success(),
        "generated Go failed to compile:\n{}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Name of the single `Test*` function the runtime fixture emits.
const RUNTIME_INVARIANT_TEST: &str = "TestGeneratedStructRuntimeInvariant";

/// Exactly how many Go tests a runtime fixture must execute.
const RUNTIME_INVARIANT_TEST_COUNT: usize = 1;

/// Write the runtime fixture and run `go test -v ./...`, returning the raw process output
/// without asserting anything about it.
///
/// `go_compile` only type-checks, and its name is the whole hazard: `go test ./...` on a
/// package with no `_test.go` files prints `[no test files]` and exits 0 *without running the
/// package's `init` functions*. A runtime invariant routed through `assert_go_compiles` is
/// therefore never evaluated. Runtime invariants belong here, where an emitted test file makes
/// them execute and the caller inspects the real exit status and `go test` output rather than
/// assuming success. Kept separate from `assert_go_runtime_invariant` so a negative control can
/// drive a deliberately failing invariant through this same path and observe the real failure —
/// see `generated_go_runtime_invariant_control_rejects_broken_invariant` below. ~keep
fn run_go_runtime_invariant(generated: &str, assertions: &str) -> Option<std::process::Output> {
    let go = toolchain::GO.open()?;
    let directory = tempfile::tempdir().expect("create Go runtime fixture");
    std::fs::write(directory.path().join("go.mod"), "module example.com/shape\n\ngo 1.24\n").expect("write Go module");
    std::fs::write(
        directory.path().join("shape.go"),
        format!("package shape\n\nimport \"encoding/json\"\n\n{generated}"),
    )
    .expect("write generated Go source");
    std::fs::write(
        directory.path().join("shape_test.go"),
        format!(
            "package shape\n\nimport (\n\t\"encoding/json\"\n\t\"testing\"\n)\n\nfunc {RUNTIME_INVARIANT_TEST}(t *testing.T) {{\n{assertions}}}\n"
        ),
    )
    .expect("write generated Go runtime test");
    Some(
        std::process::Command::new(go)
            .args(["test", "-v", "./..."])
            .current_dir(directory.path())
            .output()
            .expect("run Go runtime fixture"),
    )
}

/// Compile the generated struct and evaluate `assertions` inside a real `Test*` function,
/// asserting the invariant ran and passed.
fn assert_go_runtime_invariant(generated: &str, assertions: &str) {
    let Some(output) = run_go_runtime_invariant(generated, assertions) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The exact shape of the defect this helper replaces: no test file means the invariant
    // never runs, and `go test` reports that as a green `[no test files]`. ~keep
    assert!(
        !stdout.contains("[no test files]"),
        "the runtime fixture emitted no test file, so its invariant never ran:\n{stdout}"
    );
    assert!(
        output.status.success(),
        "generated Go runtime invariant failed:\n{stdout}\n{stderr}\n{generated}"
    );
    let executed: Vec<&str> = stdout
        .lines()
        .filter(|line| line.starts_with("--- PASS:") || line.starts_with("--- FAIL:") || line.starts_with("--- SKIP:"))
        .collect();
    assert_eq!(
        executed.len(),
        RUNTIME_INVARIANT_TEST_COUNT,
        "expected exactly {RUNTIME_INVARIANT_TEST_COUNT} Go test to run, got {executed:?}:\n{stdout}"
    );
    assert!(
        executed[0].starts_with(&format!("--- PASS: {RUNTIME_INVARIANT_TEST} ")),
        "the runtime invariant test did not run and pass: {executed:?}\n{stdout}"
    );
}

/// Negative control for `run_go_runtime_invariant`: feed it a deliberately false invariant and
/// prove the harness reports the real, non-green result — a nonzero process exit and a `go test`
/// `FAIL` line naming the runtime test — rather than the `[no test files]` false green this
/// helper exists to rule out. If this test ever passes without the harness actually observing a
/// failing `go test` run, the runtime gate above is not wired to anything. ~keep
#[test]
fn generated_go_runtime_invariant_control_rejects_broken_invariant() {
    // Both the generated struct and the assertion body must use `encoding/json` — the fixture
    // imports it unconditionally in both files, and an unused import is itself a compile error
    // that would report as a Go build failure rather than the runtime `FAIL` this control
    // exists to observe. Confirmed by first running a trivial `Envelope`/no-op assertion pair
    // through this exact harness by hand: it failed at `go build` with two "imported and not
    // used" errors and never reached `TestGeneratedStructRuntimeInvariant`, which is precisely
    // the false-negative shape this control must not have. ~keep
    let Some(output) = run_go_runtime_invariant(
        "type Envelope struct {\n\tPayload *json.RawMessage `json:\"payload,omitempty\"`\n}\n",
        "\t_ = json.RawMessage{}\n\tt.Fatalf(\"deliberately broken invariant\")\n",
    ) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "deliberately broken invariant unexpectedly reported success:\n{stdout}"
    );
    let expected_fail_line = format!("--- FAIL: {RUNTIME_INVARIANT_TEST} ");
    assert!(
        stdout.lines().any(|line| line.starts_with(&expected_fail_line)),
        "expected a `{expected_fail_line}` line in Go test output, got:\n{stdout}"
    );
}

#[test]
fn generated_go_compile_check_rejects_broken_source() {
    let Some(output) = go_compile("func broken() { missingSymbol() }", "") else {
        return;
    };
    assert!(!output.status.success(), "compile control unexpectedly passed");
}

fn envelope_with(field: FieldDef) -> TypeDef {
    TypeDef {
        name: "Envelope".into(),
        fields: vec![field],
        ..Default::default()
    }
}

#[test]
fn optional_data_enum_field_uses_non_pointer_interface() {
    let choice = EnumDef {
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
    };
    let type_def = envelope_with(FieldDef {
        name: "choice".into(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("Choice".into()))),
        optional: true,
        ..Default::default()
    });
    let output = gen_struct_type(
        &type_def,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([choice.name.as_str()]),
        &HashSet::from([type_def.name.as_str()]),
        &[],
    );

    assert!(output.contains("Choice Choice `json:\"choice,omitempty\"`"), "{output}");
    assert!(
        !output.contains("Choice *Choice"),
        "sealed interfaces are not pointers:\n{output}"
    );
}

#[test]
fn required_unresolved_named_field_uses_raw_message_pointer() {
    let type_def = TypeDef {
        name: "Envelope".into(),
        fields: vec![
            FieldDef {
                name: "payload".into(),
                ty: TypeRef::Named("ForeignPayload".into()),
                ..Default::default()
            },
            FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let output = gen_struct_type(
        &type_def,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([type_def.name.as_str()]),
        &[],
    );

    assert!(
        output.contains("Payload *json.RawMessage `json:\"payload,omitempty\"`"),
        "{output}"
    );
    assert_eq!(output.matches("Payload *json.RawMessage").count(), 2, "{output}");
    assert_go_runtime_invariant(
        &output,
        concat!(
            "\tdata, err := json.Marshal(Envelope{})\n",
            "\tif err != nil {\n\t\tt.Fatalf(\"marshal zero envelope: %v\", err)\n\t}\n",
            "\tvar object map[string]any\n",
            "\tif err := json.Unmarshal(data, &object); err != nil {\n",
            "\t\tt.Fatalf(\"unmarshal marshalled envelope: %v\", err)\n\t}\n",
            "\tif _, present := object[\"payload\"]; present {\n",
            "\t\tt.Fatalf(\"nil payload was not omitted: %s\", data)\n\t}\n",
        ),
    );
}

#[test]
fn optional_non_emitted_named_fields_use_raw_message_in_struct_and_marshal_aux() {
    for name in ["Excluded", "Opaque", "Foreign", "VisitorOwned"] {
        let type_def = TypeDef {
            name: "Envelope".into(),
            fields: vec![
                FieldDef {
                    name: "payload".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named(name.into()))),
                    optional: true,
                    ..Default::default()
                },
                FieldDef {
                    name: "bytes".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let output = gen_struct_type(
            &type_def,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([type_def.name.as_str()]),
            &[],
        );
        assert_eq!(
            output.matches("Payload *json.RawMessage").count(),
            2,
            "{name}:\n{output}"
        );
        assert_go_compiles(&output, "");
    }
}

#[test]
fn marshal_auxiliary_data_interface_uses_authoritative_type() {
    let (type_def, choice) = data_interface_with_bytes();
    let output = gen_struct_type(
        &type_def,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([choice.name.as_str()]),
        &HashSet::from([type_def.name.as_str()]),
        &[],
    );

    assert_eq!(output.matches("Choice Choice").count(), 2, "{output}");
    assert_go_compiles(
        &output,
        "type Choice interface{}\nfunc UnmarshalChoice(json.RawMessage) (Choice, error) { return nil, nil }",
    );
}

fn data_interface_with_bytes() -> (TypeDef, EnumDef) {
    let choice = EnumDef {
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
    };
    let type_def = TypeDef {
        name: "Envelope".into(),
        fields: vec![
            FieldDef {
                name: "choice".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("Choice".into()))),
                optional: true,
                ..Default::default()
            },
            FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    (type_def, choice)
}
