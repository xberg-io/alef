//! Custom `Codable` conformance for adjacently tagged enums
//! (`#[serde(tag = "...", content = "...")]`).
//!
//! Swift's synthesised `Codable` writes its own externally tagged shape, and the
//! internally tagged emitter in [`super::enums`] writes the payload's fields at the top level.
//! Neither matches serde's adjacent form `{"tag":"variant","content":payload}`, so an enum with
//! both attributes needs its own encoder and decoder — otherwise Rust rejects every value with
//! `invalid type: ..., expected adjacently tagged enum ...`.

use crate::backends::swift::gen_bindings::enums::swift_associated_label;
use crate::backends::swift::naming::swift_source_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use heck::ToLowerCamelCase;

/// Render the `CodingKeys`, `init(from:)` and `encode(to:)` members for an adjacently tagged
/// enum. `tag` and `content` are the serde wire key names, taken straight from the IR.
pub(super) fn emit_serde_adjacent_codable(
    en: &EnumDef,
    tag: &str,
    content: &str,
    out: &mut String,
    mapper: &SwiftMapper,
) {
    let tag_ident = swift_case_ident(&tag.to_lower_camel_case());
    let content_ident = swift_case_ident(&content.to_lower_camel_case());

    let mut decode_cases = String::new();
    let mut encode_cases = String::new();
    for variant in &en.variants {
        let variant_wire = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
        emit_decode_case(
            variant,
            &variant_wire,
            &case_name,
            &content_ident,
            mapper,
            &mut decode_cases,
        );
        emit_encode_case(
            variant,
            &variant_wire,
            &case_name,
            &tag_ident,
            &content_ident,
            &mut encode_cases,
        );
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_adjacent_codable.swift.jinja",
        minijinja::context! {
            enum_name => &en.name,
            tag_ident => &tag_ident,
            tag_wire => tag,
            content_ident => &content_ident,
            content_wire => content,
            payload_key_cases => payload_key_cases(en),
            decode_cases => decode_cases,
            encode_cases => encode_cases,
        },
    ));
}

/// `PayloadKeys` covers the union of every struct variant's fields; variants with a single
/// positional payload put the value directly under the content key and need no nested keys, so
/// an enum made only of unit and newtype variants renders no `PayloadKeys` at all.
fn payload_key_cases(en: &EnumDef) -> String {
    let mut keys = std::collections::BTreeSet::new();
    for variant in &en.variants {
        if is_newtype_variant(variant) {
            continue;
        }
        for (idx, field) in variant.fields.iter().enumerate() {
            let swift_name = swift_associated_label(&field.name, idx);
            let wire_name = crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                en.rename_all_fields.as_deref(),
            );
            keys.insert((swift_name, wire_name));
        }
    }

    let mut cases = String::new();
    for (swift_name, wire_name) in keys {
        cases.push_str(&crate::backends::swift::template_env::render(
            "swift_adjacent_payload_key_case.swift.jinja",
            minijinja::context! {
                swift_name => &swift_name,
                wire_name => &wire_name,
            },
        ));
    }
    cases
}

/// A single positional payload: serde puts the value itself under the content key rather than
/// wrapping it in an object.
///
/// Shared with the internally tagged emitter in [`super::enums`], which needs the same answer for
/// the same reason — a positional payload is never a `"0"` key on the wire — so the two emitters
/// cannot drift apart on what counts as a newtype variant. ~keep
pub(super) fn is_newtype_variant(variant: &EnumVariant) -> bool {
    variant.is_tuple && variant.fields.len() == 1
}

fn is_optional_field(field: &crate::core::ir::FieldDef) -> bool {
    field.optional || matches!(&field.ty, TypeRef::Optional(_))
}

fn emit_decode_case(
    variant: &EnumVariant,
    variant_wire: &str,
    case_name: &str,
    content_ident: &str,
    mapper: &SwiftMapper,
    out: &mut String,
) {
    if variant.fields.is_empty() {
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_adjacent_decode_unit_case.swift.jinja",
            minijinja::context! {
                variant_wire => variant_wire,
                case_name => case_name,
            },
        ));
        return;
    }

    if is_newtype_variant(variant) {
        let field = &variant.fields[0];
        let optional = is_optional_field(field);
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_adjacent_decode_newtype_case.swift.jinja",
            minijinja::context! {
                variant_wire => variant_wire,
                case_name => case_name,
                label => swift_associated_label(&field.name, 0),
                payload_type => mapper.map_type(&field.ty),
                decode_method => if optional { "decodeIfPresent" } else { "decode" },
                content_ident => content_ident,
            },
        ));
        return;
    }

    assert_tuple_arity(variant);
    let field_decoders: Vec<String> = variant
        .fields
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let label = swift_associated_label(&field.name, idx);
            let method = if is_optional_field(field) {
                "decodeIfPresent"
            } else {
                "decode"
            };
            format!(
                "{label}: try payload.{method}({}.self, forKey: .{label})",
                mapper.map_type(&field.ty)
            )
        })
        .collect();
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_adjacent_decode_struct_case.swift.jinja",
        minijinja::context! {
            variant_wire => variant_wire,
            case_name => case_name,
            content_ident => content_ident,
            field_decoders => field_decoders.join(", "),
        },
    ));
}

fn emit_encode_case(
    variant: &EnumVariant,
    variant_wire: &str,
    case_name: &str,
    tag_ident: &str,
    content_ident: &str,
    out: &mut String,
) {
    if variant.fields.is_empty() {
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_adjacent_encode_unit_case.swift.jinja",
            minijinja::context! {
                variant_wire => variant_wire,
                case_name => case_name,
                tag_ident => tag_ident,
            },
        ));
        return;
    }

    if is_newtype_variant(variant) {
        let field = &variant.fields[0];
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_adjacent_encode_newtype_case.swift.jinja",
            minijinja::context! {
                variant_wire => variant_wire,
                case_name => case_name,
                tag_ident => tag_ident,
                content_ident => content_ident,
                label => swift_associated_label(&field.name, 0),
                encode_method => if is_optional_field(field) { "encodeIfPresent" } else { "encode" },
            },
        ));
        return;
    }

    assert_tuple_arity(variant);
    let mut bindings = Vec::with_capacity(variant.fields.len());
    let mut field_encoders = String::new();
    for (idx, field) in variant.fields.iter().enumerate() {
        let label = swift_associated_label(&field.name, idx);
        bindings.push(format!("let {label}"));
        field_encoders.push_str(&crate::backends::swift::template_env::render(
            "swift_adjacent_encode_field.swift.jinja",
            minijinja::context! {
                label => &label,
                key => &label,
                encode_method => if is_optional_field(field) { "encodeIfPresent" } else { "encode" },
            },
        ));
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_adjacent_encode_struct_case.swift.jinja",
        minijinja::context! {
            variant_wire => variant_wire,
            case_name => case_name,
            tag_ident => tag_ident,
            content_ident => content_ident,
            bindings => bindings.join(", "),
            field_encoders => field_encoders.trim_end_matches('\n'),
        },
    ));
}

/// serde writes a multi-field *tuple* variant's content as a JSON array, which the keyed
/// `PayloadKeys` container cannot express. Emitting the object form anyway would produce JSON
/// Rust silently refuses at run time, so refuse at generation time instead.
fn assert_tuple_arity(variant: &EnumVariant) {
    assert!(
        !variant.is_tuple,
        "adjacently tagged variant `{}` has {} positional payload fields; serde encodes that \
         content as a JSON array, which the Swift backend does not generate. Give the variant \
         named fields, or wrap the payload in a single struct.",
        variant.name,
        variant.fields.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, PrimitiveType};

    fn render(en: &EnumDef) -> String {
        let mut out = String::new();
        emit_serde_adjacent_codable(en, "kind", "body", &mut out, &SwiftMapper);
        out
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn unit_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            ..EnumVariant::default()
        }
    }

    fn newtype_variant(name: &str, ty: TypeRef) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            is_tuple: true,
            fields: vec![field("0", ty)],
            ..EnumVariant::default()
        }
    }

    fn struct_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            ..EnumVariant::default()
        }
    }

    fn enum_of(variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: "Outcome".to_string(),
            has_serde: true,
            serde_tag: Some("kind".to_string()),
            serde_content: Some("body".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants,
            ..EnumDef::default()
        }
    }

    #[test]
    fn should_key_the_coding_keys_on_the_serde_tag_and_content_names() {
        let out = render(&enum_of(vec![unit_variant("KeepGoing")]));
        assert!(out.contains("case kind = \"kind\""), "{out}");
        assert!(out.contains("case body = \"body\""), "{out}");
    }

    #[test]
    fn should_write_only_the_tag_for_a_unit_variant() {
        let out = render(&enum_of(vec![unit_variant("KeepGoing")]));
        assert!(
            out.contains("case \"keep_going\":\n            self = .keepGoing"),
            "{out}"
        );
        assert!(
            out.contains("case .keepGoing:\n            try container.encode(\"keep_going\", forKey: .kind)"),
            "{out}"
        );
        assert!(
            !out.contains("forKey: .body"),
            "a unit variant has no payload to write: {out}"
        );
    }

    #[test]
    fn should_put_a_newtype_payload_directly_under_the_content_key() {
        let out = render(&enum_of(vec![newtype_variant("Replace", TypeRef::String)]));
        assert!(
            out.contains("self = .replace(field0: try container.decode(String.self, forKey: .body))"),
            "{out}"
        );
        assert!(out.contains("try container.encode(field0, forKey: .body)"), "{out}");
        assert!(
            !out.contains("PayloadKeys"),
            "a positional payload is the content itself, not an object: {out}"
        );
    }

    #[test]
    fn should_nest_a_struct_variants_fields_under_the_content_key() {
        let fields = vec![
            field("depth", TypeRef::Primitive(PrimitiveType::I32)),
            field("label", TypeRef::Optional(Box::new(TypeRef::String))),
        ];
        let out = render(&enum_of(vec![struct_variant("Descend", fields)]));
        assert!(out.contains("private enum PayloadKeys: String, CodingKey"), "{out}");
        assert!(out.contains("case depth = \"depth\""), "{out}");
        assert!(
            out.contains("let payload = try container.nestedContainer(keyedBy: PayloadKeys.self, forKey: .body)"),
            "{out}"
        );
        assert!(
            out.contains("var payload = container.nestedContainer(keyedBy: PayloadKeys.self, forKey: .body)"),
            "{out}"
        );
        assert!(out.contains("try payload.encode(depth, forKey: .depth)"), "{out}");
        assert!(
            out.contains("try payload.encodeIfPresent(label, forKey: .label)"),
            "an optional field must not be forced onto the wire: {out}"
        );
    }

    #[test]
    fn should_apply_a_fields_own_serde_rename_to_its_struct_variant_payload_key() {
        let mut fields = vec![field("depth", TypeRef::Primitive(PrimitiveType::I32))];
        fields[0].serde_rename = Some("customDepth".to_string());
        let out = render(&enum_of(vec![struct_variant("Descend", fields)]));
        assert!(out.contains("case depth = \"customDepth\""), "{out}");
        assert!(!out.contains("\"depth\""), "{out}");
    }

    /// `rename_all_fields` (struct-variant field casing) is a distinct serde namespace from
    /// `rename_all` (variant-name casing, already applied via `serde_rename_all` on `enum_of`).
    /// A field with no `serde_rename` of its own must still pick up the container's field rule.
    #[test]
    fn should_case_a_struct_variant_payload_key_with_the_enums_rename_all_fields() {
        let fields = vec![field("depth", TypeRef::Primitive(PrimitiveType::I32))];
        let mut en = enum_of(vec![struct_variant("Descend", fields)]);
        en.rename_all_fields = Some("SCREAMING_SNAKE_CASE".to_string());
        let out = render(&en);
        assert!(
            out.contains("case depth = \"DEPTH\""),
            "the enum's rename_all_fields must case the payload key: {out}"
        );
        assert!(!out.contains("case depth = \"depth\""), "{out}");
    }

    #[test]
    fn should_prefer_a_fields_own_serde_rename_over_the_enums_rename_all_fields() {
        let mut fields = vec![field("depth", TypeRef::Primitive(PrimitiveType::I32))];
        fields[0].serde_rename = Some("customDepth".to_string());
        let mut en = enum_of(vec![struct_variant("Descend", fields)]);
        en.rename_all_fields = Some("SCREAMING_SNAKE_CASE".to_string());
        let out = render(&en);
        assert!(out.contains("case depth = \"customDepth\""), "{out}");
        assert!(!out.contains("\"DEPTH\""), "{out}");
        assert!(!out.contains("case depth = \"depth\""), "{out}");
    }

    #[test]
    fn should_honour_a_variant_level_serde_rename_over_the_container_strategy() {
        let mut variant = unit_variant("KeepGoing");
        variant.serde_rename = Some("CONTINUE".to_string());
        let out = render(&enum_of(vec![variant]));
        assert!(out.contains("case \"CONTINUE\":"), "{out}");
        assert!(!out.contains("keep_going"), "{out}");
    }

    #[test]
    #[should_panic(expected = "positional payload fields")]
    fn should_refuse_a_multi_field_tuple_variant_rather_than_emit_the_wrong_shape() {
        let mut variant = newtype_variant("Pair", TypeRef::String);
        variant.fields.push(field("1", TypeRef::String));
        render(&enum_of(vec![variant]));
    }
}
