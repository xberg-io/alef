use super::*;
use crate::core::ir::FieldDef;
use std::collections::HashSet;

fn payload_enum() -> EnumDef {
    EnumDef {
        name: "FormatMetadata".to_string(),
        has_serde: true,
        serde_rename_all: Some("snake_case".to_string()),
        variants: vec![
            EnumVariant {
                name: "Pdf".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Excel".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

#[test]
fn payload_carrying_enum_declares_a_to_string_wire_tag_accessor() {
    let en = payload_enum();
    let mapper = SwiftMapper;
    let known: HashSet<String> = HashSet::new();
    let cfg = EnumDeclarationCfg::new("sample", &[]);
    let mut out = String::new();
    emit_enum(&en, &mut out, &mapper, &known, &[], &cfg);
    // The exact call-site text the e2e generator's `swift_scalar_leaf_string_expr` appends
    // (`leaf_shape.rs`) is `.toString()` -- this method must exist under exactly that name.
    assert!(
        out.contains("public func toString() -> String {"),
        "expected a `toString()` wire-tag accessor, got:\n{out}"
    );
    // Same per-variant wire value `wire_variant_value` computes for the swift-bridge mirror's
    // OWN `to_string()` (`gen_rust_crate::enums`'s `rust_enum_to_string_variant.rs.jinja`) --
    // dropping the payload, keyed by the case identifier, not the tag JSON shape.
    assert!(out.contains("case .pdf:\n            return \"pdf\""), "got:\n{out}");
    assert!(
        out.contains("case .excel:\n            return \"excel\""),
        "got:\n{out}"
    );
    // Declared over the SAME variant set the enum's own `case` list uses just above, so the
    // switch stays exhaustive against the real declaration.
    assert!(out.contains("case pdf(field0: String)"), "got:\n{out}");
    assert!(out.contains("case excel(field0: String)"), "got:\n{out}");
}
