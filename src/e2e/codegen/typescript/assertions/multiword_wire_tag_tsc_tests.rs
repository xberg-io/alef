//! Real `tsc`-parse and `node`-execute oracle for a WIRE VALUE shaped like a realistic
//! `#[serde(rename = "...")]` tag: multiple words, with a double quote embedded inside it.
//!
//! ~keep Every fix in this range (numeric wildcard containment, the struct-element fixture
//! rename, the numeric-array text-surface leak, the wasm enum wire comparison) was proven only at
//! the Rust string-equality level or, at most, against a single-token fixture value. None of them
//! actually fed the emitted TypeScript through a real compiler and a real runtime for a value an
//! interpolation bug could break -- which is exactly the shape `render_wasm_enum_assertion`'s
//! `format!("... .toBe(\"{wire}\");\n")` had: `wire` is a `#[serde(rename)]` string a consumer
//! controls, and it was spliced raw between hand-written double quotes with no escaping. A rename
//! containing `"` breaks the emitted string literal outright; this oracle pins that both `tsc`
//! (syntax/type) and `node` (runtime) stay clean once the value is routed through `json_to_js`,
//! and -- via the sabotage test below -- proves the oracle itself is wired to catch the bug it
//! exists for, not just accompanying it.

use super::render_assertion;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

/// A realistic multi-word serde rename with a double quote embedded in it -- the exact shape that
/// broke the pre-fix raw `"{wire}"` interpolation. ~keep
const HOSTILE_MULTIWORD_WIRE_TAG: &str = "needs \"final\" review";

fn enums() -> Vec<EnumDef> {
    vec![EnumDef {
        name: "Status".to_string(),
        variants: vec![
            EnumVariant {
                name: "NeedsReview".to_string(),
                serde_rename: Some(HOSTILE_MULTIWORD_WIRE_TAG.to_string()),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Done".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }]
}

fn type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Report".to_string(),
        fields: vec![FieldDef {
            name: "status".to_string(),
            ty: TypeRef::Named("Status".to_string()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }]
}

/// Renders the real `render_assertion` -> `render_wasm_enum_assertion` path for a struct field
/// whose enum wire value is `HOSTILE_MULTIWORD_WIRE_TAG`, exactly as the wasm backend would.
fn render() -> String {
    let defs = type_defs();
    let enum_defs = enums();
    let result_fields: HashSet<String> = ["status".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&defs, &enum_defs),
        Some("Report".to_string()),
    );
    let config = HashMap::from([("status".to_string(), "Status".to_string())]);
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("status".to_string()),
        value: Some(serde_json::Value::String("NeedsReview".to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out, &assertion, "result", &resolver, false, &config, "wasm", false, false, false,
    );
    out
}

/// Wraps one emitted assertion line in a self-contained, dependency-free TypeScript program: a
/// minimal `expect().toBe()` shim, plus a `result` object holding the TRUE runtime wire value
/// (serialized independently via `serde_json`, not via the code under test), so a pass proves the
/// emitted comparison is both syntactically valid TypeScript and semantically correct against the
/// real value the wasm binding would hand JavaScript.
fn harness(generated_assertion: &str) -> String {
    let true_wire_value_literal =
        serde_json::to_string(HOSTILE_MULTIWORD_WIRE_TAG).expect("a plain string always serializes to JSON");
    format!(
        "function expect(actual: unknown) {{\n  return {{\n    toBe(expected: unknown) {{\n      if (actual !== \
         expected) {{\n        throw new Error(`expected ${{JSON.stringify(actual)}} to be \
         ${{JSON.stringify(expected)}}`);\n      }}\n      console.log(\"PASS\");\n    }},\n  }};\n}}\nconst result \
         = {{ status: {true_wire_value_literal} }};\n{generated_assertion}"
    )
}

fn tsc(source: &std::path::Path) -> std::process::Output {
    crate::core::tool_command("tsc")
        .args(["--strict", "--noEmit", "--target", "ES2022"])
        .arg(source)
        .output()
        .unwrap_or_else(|error| panic!("tsc is required for this generated-code regression: {error}"))
}

fn node(source: &std::path::Path) -> std::process::Output {
    std::process::Command::new("node")
        .arg(source)
        .output()
        .unwrap_or_else(|error| panic!("node is required for this generated-code regression: {error}"))
}

#[test]
fn a_multiword_wire_tag_with_an_embedded_quote_parses_and_executes_clean() {
    let generated = render();
    let expected_literal =
        serde_json::to_string(HOSTILE_MULTIWORD_WIRE_TAG).expect("a plain string always serializes to JSON");
    assert_eq!(
        generated,
        format!("    expect(result.status).toBe({expected_literal});\n"),
        "got: {generated}"
    );

    let directory = tempfile::tempdir().expect("temporary TypeScript project");
    let source_path = directory.path().join("wire_tag.ts");
    std::fs::write(&source_path, harness(&generated)).expect("write generated TypeScript");

    let type_check = tsc(&source_path);
    assert!(
        type_check.status.success(),
        "tsc rejected the emitted assertion:\n{}\n{}",
        String::from_utf8_lossy(&type_check.stdout),
        String::from_utf8_lossy(&type_check.stderr)
    );

    let run = node(&source_path);
    assert!(
        run.status.success(),
        "node failed to execute the emitted assertion:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("PASS"),
        "node did not report a passing assertion: {run:?}"
    );
}

/// BROKEN ON PURPOSE. Replays the pre-fix raw interpolation
/// (`format!("... .toBe(\"{{wire}}\");\n")`) this oracle exists to catch, so a green run above can
/// never be mistaken for a check that examines nothing: the same harness must reject this text. ~keep
#[test]
fn the_pre_fix_raw_interpolation_is_rejected_by_the_oracle() {
    let naive_pre_fix_render = format!("    expect(result.status).toBe(\"{HOSTILE_MULTIWORD_WIRE_TAG}\");\n");
    let directory = tempfile::tempdir().expect("temporary TypeScript project");
    let source_path = directory.path().join("sabotaged_wire_tag.ts");
    std::fs::write(&source_path, harness(&naive_pre_fix_render)).expect("write sabotaged TypeScript");

    let type_check = tsc(&source_path);
    assert!(
        !type_check.status.success(),
        "the pre-fix raw interpolation must fail tsc -- if this passes, the oracle is not wired to the bug it \
         exists to catch"
    );
}
