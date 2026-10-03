use super::enums::{GoEnumRepresentation, gen_enum_type, go_enum_representation};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use crate::test_support::toolchain;

fn pii_category_enum() -> EnumDef {
    EnumDef {
        name: "PiiCategory".to_string(),
        rust_path: "example::PiiCategory".to_string(),
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "Email".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

#[test]
fn externally_tagged_unit_and_string_enum_preserves_object_variant() {
    let enum_def = pii_category_enum();

    assert_eq!(
        go_enum_representation(&enum_def),
        GoEnumRepresentation::NewtypeTupleString
    );

    let generated = gen_enum_type(&enum_def, &[]);
    assert!(
        generated.contains("type PiiCategory string")
            && generated.contains(r#"PiiCategoryEmail PiiCategory = "email""#)
            && generated.contains("func (e PiiCategory) MarshalJSON()")
            && generated.contains("func (e *PiiCategory) UnmarshalJSON("),
        "the Go API must preserve its string alias and constants while accepting both serde wire shapes: {generated}"
    );
}

#[test]
fn externally_tagged_unit_and_string_enum_round_trips_both_wire_shapes() {
    let Some(go) = toolchain::GO.open() else {
        return;
    };
    let generated = gen_enum_type(&pii_category_enum(), &[]);
    let directory = tempfile::tempdir().expect("create Go runtime fixture");
    std::fs::write(directory.path().join("go.mod"), "module example.com/enum\n\ngo 1.24\n").expect("write Go module");
    std::fs::write(
        directory.path().join("enum.go"),
        format!("package sample\n\nimport \"encoding/json\"\n\n{generated}"),
    )
    .expect("write generated Go source");
    std::fs::write(
        directory.path().join("enum_test.go"),
        r#"package sample

import (
	"encoding/json"
	"testing"
)

func TestWireRoundTrip(t *testing.T) {
	for _, wire := range []string{`"email"`, `{"custom":"employee_id"}`} {
		var category PiiCategory
		if err := json.Unmarshal([]byte(wire), &category); err != nil {
			t.Fatalf("unmarshal %s: %v", wire, err)
		}
		encoded, err := json.Marshal(category)
		if err != nil {
			t.Fatalf("marshal %s: %v", wire, err)
		}
		if string(encoded) != wire {
			t.Fatalf("round trip mismatch: got %s, want %s", encoded, wire)
		}
	}
	if PiiCategoryEmail != PiiCategory("email") {
		t.Fatal("existing unit constant changed")
	}
}
"#,
    )
    .expect("write Go runtime test");

    let output = std::process::Command::new(go)
        .args(["test", "-run", "TestWireRoundTrip", "-count=1", "./..."])
        .current_dir(directory.path())
        .output()
        .expect("run Go runtime fixture");
    assert!(
        output.status.success(),
        "generated Go failed its wire round trip:\n{}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}
