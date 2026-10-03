use crate::backends::go::type_map::{go_optional_type, go_type};
use crate::codegen::naming::{apply_serde_rename_all, go_type_name, go_variant_name, to_go_name};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use minijinja::context;

use super::field_shape::go_data_enum_variant_field;
use super::helpers::{emit_type_doc, is_tuple_field};

/// Which Go declaration [`gen_enum_type`] emits for an IR enum — one variant per generator.
///
/// Exists so consumers outside this backend can ask what a `TypeRef::Named` resolving to an
/// `EnumDef` actually *is* in Go, instead of re-deriving the dispatch below and drifting from
/// it. The property they need is convertibility: Go converts an untyped string constant to a
/// defined type whose underlying type is `string` or `[]byte`, and to nothing else — so a
/// `sample.Enum("value")` expression compiles against the first three variants and is a
/// `cannot convert` compile error against the last three. ~keep
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GoEnumRepresentation {
    /// `type X string` plus a const block — [`gen_unit_enum_type`].
    UnitString,
    /// `type X string` plus a const block covering the unit variants only —
    /// [`gen_newtype_tuple_enum_type`].
    NewtypeTupleString,
    /// `type X json.RawMessage` — [`gen_passthrough_raw_message_enum`].
    RawMessage,
    /// `type X struct { .. }` — [`gen_adjacent_tagged_enum_type`].
    AdjacentTaggedStruct,
    /// `type X struct { .. }` — [`gen_tuple_tagged_union_type`].
    TupleTaggedStruct,
    /// `type X struct { .. }` — [`gen_externally_tagged_union_type`].
    ExternallyTaggedStruct,
    /// `type X interface { .. }` — [`gen_data_enum_type`].
    DataInterface,
}

impl GoEnumRepresentation {
    /// How the emitted `type X ...` line spells the underlying type, for diagnostics.
    pub(crate) fn go_declaration(self) -> &'static str {
        match self {
            Self::UnitString | Self::NewtypeTupleString => "string",
            Self::RawMessage => "json.RawMessage",
            Self::AdjacentTaggedStruct | Self::TupleTaggedStruct | Self::ExternallyTaggedStruct => "struct",
            Self::DataInterface => "interface",
        }
    }

    /// Whether `X(<Go string literal>)` is a legal conversion for this representation.
    ///
    /// `string` and `json.RawMessage` (underlying `[]byte`) are the only underlying types an
    /// untyped string constant converts to; a `struct` or `interface` target is rejected by
    /// the compiler with `cannot convert`. ~keep
    pub(crate) fn accepts_string_conversion(self) -> bool {
        matches!(self, Self::UnitString | Self::NewtypeTupleString | Self::RawMessage)
    }

    /// Whether the emitted declaration is followed by a const block naming the unit variants.
    pub(crate) fn has_named_constants(self) -> bool {
        matches!(self, Self::UnitString | Self::NewtypeTupleString)
    }
}

/// The single place that decides an IR enum's Go shape; [`gen_enum_type`] is a match over it.
pub(crate) fn go_enum_representation(enum_def: &EnumDef) -> GoEnumRepresentation {
    let is_data_enum = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if !is_data_enum {
        return GoEnumRepresentation::UnitString;
    }

    if enum_def.serde_tag.is_some() && enum_def.serde_content.is_some() {
        return GoEnumRepresentation::AdjacentTaggedStruct;
    }

    let all_data_fields_are_tuple = enum_def
        .variants
        .iter()
        .all(|v| v.fields.is_empty() || v.fields.iter().all(is_tuple_field));

    if !all_data_fields_are_tuple {
        return GoEnumRepresentation::DataInterface;
    }

    let any_tuple_field_is_named_struct = enum_def.variants.iter().any(|v| {
        v.fields
            .iter()
            .any(|f| is_tuple_field(f) && matches!(&f.ty, TypeRef::Named(_)))
    });

    if any_tuple_field_is_named_struct {
        // Field shape alone does not pick `TupleTaggedStruct`: `gen_tuple_tagged_union_type`
        // renders a discriminator field out of `serde_tag` and its marshalers unwrap that tag,
        // so classifying on shape while the emitter also demands a tag put one decision behind
        // two conditions and aborted the run on the shape serde reaches by DEFAULT — an enum
        // carrying neither attribute is externally tagged, which is a different wire form
        // (`{"Variant": payload}`) and therefore a different generator. ~keep
        if enum_def.serde_tag.is_some() || enum_def.serde_untagged {
            return GoEnumRepresentation::TupleTaggedStruct;
        }
        if is_externally_tagged_named_union(enum_def) {
            return GoEnumRepresentation::ExternallyTaggedStruct;
        }
        // Externally tagged, but with a variant no single-key struct can carry (a unit variant
        // serialises as the bare string `"Variant"`; a multi-field tuple variant serialises as
        // an array). Raw bytes round-trip every one of those forms; a struct would drop them.
        // [`gen_enum_type`] reports this narrowing, so the classifier stays free of side effects
        // for the many callers that only ask it a question. ~keep
        return GoEnumRepresentation::RawMessage;
    }

    // Serde's default external representation wraps every data variant in an object such as
    // `{"custom":"label"}`, while unit variants remain bare strings. A Go string alias cannot
    // accept both shapes, so preserve the exact JSON bytes instead. ~keep
    if enum_def.serde_tag.is_none() && !enum_def.serde_untagged {
        return GoEnumRepresentation::RawMessage;
    }

    if is_heterogeneous_shape_union(enum_def) {
        return GoEnumRepresentation::RawMessage;
    }

    GoEnumRepresentation::NewtypeTupleString
}

/// Whether every variant of an externally tagged enum is a single-field tuple wrapping a named
/// type — the only shape [`gen_externally_tagged_union_type`] renders losslessly.
///
/// serde's external tagging writes `{"Variant": payload}`, and a Go struct of `omitempty`
/// pointers keyed by the variant wire names is exactly that object. A unit variant (bare
/// `"Variant"` string) or a multi-field tuple variant (JSON array) has no place in that struct,
/// so those enums must not be classified here. ~keep
fn is_externally_tagged_named_union(enum_def: &EnumDef) -> bool {
    !enum_def.variants.is_empty()
        && enum_def.variants.iter().all(|variant| {
            variant.fields.len() == 1
                && is_tuple_field(&variant.fields[0])
                && matches!(&variant.fields[0].ty, TypeRef::Named(_))
        })
}

/// The Go struct field a struct-shaped enum declares for one of its variants.
///
/// Mirrors the two generators that render `tagged_union_variant_field.jinja`
/// ([`gen_tuple_tagged_union_type`] and [`gen_externally_tagged_union_type`]): both name the
/// field `go_variant_name(variant)`, and they differ only in the JSON key — the internally tagged
/// generator snake-cases the variant name for a wire-invisible container field, while the
/// externally tagged one must use serde's own variant wire name because that key *is* the
/// wire form. A consumer that re-derived either spelling would fill a field the binding does
/// not declare, which is a compile error rather than a wrong value. ~keep
pub(crate) struct GoStructEnumVariantField<'a> {
    pub variant: &'a EnumVariant,
    /// The exported Go identifier of the pointer field.
    pub field_name: String,
    /// The key the field's `json:"..."` tag carries.
    pub json_key: String,
    /// The IR field whose type the pointer points at.
    pub payload: &'a FieldDef,
}

/// Every variant a struct-shaped enum emits a pointer field for, in declaration order.
///
/// Empty for every representation that declares no per-variant field, so a caller can ask
/// without first matching on [`go_enum_representation`]. Declaration order is load-bearing:
/// it is the order `untagged_union_marshalers.jinja` tries variants in. ~keep
pub(crate) fn go_struct_enum_variant_fields(enum_def: &EnumDef) -> Vec<GoStructEnumVariantField<'_>> {
    let externally_tagged = match go_enum_representation(enum_def) {
        GoEnumRepresentation::TupleTaggedStruct => false,
        GoEnumRepresentation::ExternallyTaggedStruct => true,
        _ => return Vec::new(),
    };
    enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let payload = variant.fields.iter().find(|field| is_tuple_field(field))?;
            if !matches!(&payload.ty, TypeRef::Named(_)) {
                return None;
            }
            let json_key = if externally_tagged {
                crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                )
            } else {
                apply_serde_rename_all(
                    &crate::codegen::naming::pascal_to_snake(&variant.name),
                    enum_def.serde_rename_all.as_deref(),
                )
            };
            Some(GoStructEnumVariantField {
                variant,
                field_name: go_variant_name(&variant.name),
                json_key,
                payload,
            })
        })
        .collect()
}

/// The discriminator field a struct-shaped enum declares, as `(Go identifier, JSON key)`.
///
/// `None` when the emitted struct carries no tag field — an untagged union, or a
/// representation that is not a struct at all. The tag is not decoration: the emitted
/// `MarshalJSON` switches on it, so a literal that sets a variant pointer without also
/// setting the tag serialises to the footer's tag-only fallback. ~keep
pub(crate) fn go_struct_enum_tag_field(enum_def: &EnumDef) -> Option<(String, &str)> {
    match go_enum_representation(enum_def) {
        GoEnumRepresentation::TupleTaggedStruct | GoEnumRepresentation::AdjacentTaggedStruct => {
            let tag_name = enum_def.serde_tag.as_deref()?;
            Some((to_go_name(tag_name), tag_name))
        }
        _ => None,
    }
}

/// The constructor `adjacent_tagged_enum.jinja` declares for a variant, e.g. `NewShapeCircle`.
pub(crate) fn go_adjacent_tagged_constructor(enum_def: &EnumDef, variant: &EnumVariant) -> String {
    format!("New{}{}", go_type_name(&enum_def.name), go_variant_name(&variant.name))
}

/// The concrete Go struct [`gen_data_enum_type`] declares for a variant of a sealed-interface
/// enum, e.g. `ResponseFormatJSONSchema`. An interface value cannot be constructed directly,
/// so this name is the only way a snippet can produce one. ~keep
pub(crate) fn go_data_enum_variant_struct(enum_def: &EnumDef, variant: &EnumVariant) -> String {
    format!("{}{}", go_type_name(&enum_def.name), go_variant_name(&variant.name))
}

/// The single positional field an untagged sealed-interface variant stores in its `Value`
/// field, or `None` when the variant has no such field.
///
/// [`gen_data_enum_type`] consumes this so the condition has exactly one definition; a
/// consumer building a literal must set `Value` precisely when the emitter declared it. ~keep
pub(crate) fn go_data_enum_variant_scalar_tuple_field<'a>(
    enum_def: &EnumDef,
    variant: &'a EnumVariant,
) -> Option<&'a FieldDef> {
    if !enum_def.serde_untagged || variant.fields.len() != 1 || !is_tuple_field(&variant.fields[0]) {
        return None;
    }
    match &variant.fields[0].ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Primitive(_) => Some(&variant.fields[0]),
        _ => None,
    }
}

/// The JSON shape an untagged sealed-interface enum's decoder tests before it will decode a
/// variant, or `None` for a variant it never tries (one with no fields at all).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GoDataEnumShape {
    /// A JSON string.
    Text,
    /// A JSON array.
    Array,
    /// A JSON number or boolean — anything that is neither a string, an object nor an array.
    Scalar,
    /// A JSON object.
    Object,
}

impl GoDataEnumShape {
    /// The `firstByte` predicate `data_enum_unmarshal_shape_variant.jinja` is given.
    fn go_first_byte_check(self) -> &'static str {
        match self {
            Self::Text => "firstByte == '\"'",
            Self::Array => "firstByte == '['",
            Self::Scalar => "firstByte != '\"' && firstByte != '{' && firstByte != '['",
            Self::Object => "firstByte == '{'",
        }
    }

    /// Whether a fixture value has the shape this predicate admits. Mirrors the predicate
    /// above rather than restating it, so the emitted decoder and any statically constructed
    /// literal agree on which variant a value selects. ~keep
    fn matches(self, value: &serde_json::Value) -> bool {
        match self {
            Self::Text => value.is_string(),
            Self::Array => value.is_array(),
            Self::Scalar => value.is_number() || value.is_boolean(),
            Self::Object => value.is_object(),
        }
    }
}

/// The shape check [`gen_data_enum_type`] emits for an untagged variant, in declaration order.
fn go_data_enum_untagged_shape(variant: &EnumVariant) -> Option<GoDataEnumShape> {
    if variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]) {
        return Some(match &variant.fields[0].ty {
            TypeRef::String | TypeRef::Char | TypeRef::Path => GoDataEnumShape::Text,
            TypeRef::Vec(_) | TypeRef::Bytes => GoDataEnumShape::Array,
            TypeRef::Primitive(_) => GoDataEnumShape::Scalar,
            _ => GoDataEnumShape::Object,
        });
    }
    if variant.fields.is_empty() {
        return None;
    }
    Some(GoDataEnumShape::Object)
}

/// Whether the decoder [`gen_data_enum_type`] emits for an *untagged* sealed-interface enum
/// would try `variant` for a value of this JSON shape.
///
/// The shape predicate itself stays private: a consumer needs the verdict, not the `firstByte`
/// expression the template is handed, and exporting the latter would invite a second reading of
/// it. Declaration order is the caller's to preserve — the emitted decoder returns the first
/// variant that both matches the shape and decodes. ~keep
pub(crate) fn go_data_enum_untagged_variant_matches(variant: &EnumVariant, value: &serde_json::Value) -> bool {
    go_data_enum_untagged_shape(variant).is_some_and(|shape| shape.matches(value))
}

/// Emit the Go declaration for the variant [`go_enum_representation`] selected.
///
/// Kept as a bare dispatch so the classifier holds the only copy of the branch conditions;
/// anything that needs the emitted shape asks the classifier rather than restating it. ~keep
pub(in crate::backends::go::gen_bindings) fn gen_enum_type(enum_def: &EnumDef, text_types: &[String]) -> String {
    match go_enum_representation(enum_def) {
        GoEnumRepresentation::UnitString => gen_unit_enum_type(enum_def),
        GoEnumRepresentation::NewtypeTupleString => gen_newtype_tuple_enum_type(enum_def),
        GoEnumRepresentation::RawMessage => {
            if !is_heterogeneous_shape_union(enum_def) {
                tracing::warn!(
                    enum_name = %enum_def.name,
                    go_type = %go_type_name(&enum_def.name),
                    "externally tagged enum has a variant no Go struct field can carry (a unit \
                     variant or a multi-field tuple variant); emitting a json.RawMessage \
                     passthrough, so the variants are not individually typed in Go"
                );
            }
            gen_passthrough_raw_message_enum(enum_def, text_types)
        }
        GoEnumRepresentation::AdjacentTaggedStruct => gen_adjacent_tagged_enum_type(enum_def),
        GoEnumRepresentation::TupleTaggedStruct => gen_tuple_tagged_union_type(enum_def),
        GoEnumRepresentation::ExternallyTaggedStruct => gen_externally_tagged_union_type(enum_def),
        GoEnumRepresentation::DataInterface => gen_data_enum_type(enum_def),
    }
}

/// The qualified Go constant the binding declares for a unit variant, e.g. `sample.ModeAuto`.
///
/// Mirrors [`gen_unit_enum_type`]'s and [`gen_newtype_tuple_enum_type`]'s `const_name` —
/// `go_type_name(enum) + go_variant_name(variant)` — and their `wire_variant_value` lookup key,
/// which is the value the const is initialised to and therefore the value fixture JSON
/// carries. Only variants with no fields get a constant, so tuple/struct variants of a
/// `NewtypeTupleString` enum correctly find nothing here. ~keep
pub(crate) fn go_enum_constant_for_wire_value(enum_def: &EnumDef, wire_value: &str) -> Option<String> {
    if !go_enum_representation(enum_def).has_named_constants() {
        return None;
    }
    let go_enum_name = go_type_name(&enum_def.name);
    enum_def
        .variants
        .iter()
        .find(|variant| {
            variant.fields.is_empty()
                && crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ) == wire_value
        })
        .map(|variant| format!("{go_enum_name}{}", go_variant_name(&variant.name)))
}

fn gen_adjacent_tagged_enum_type(enum_def: &EnumDef) -> String {
    let go_enum_name = go_type_name(&enum_def.name);
    let tag_name = enum_def.serde_tag.as_deref().expect("adjacent tag is present");
    let content_name = enum_def.serde_content.as_deref().expect("adjacent content is present");
    let tag_field = to_go_name(tag_name);
    let content_field = to_go_name(content_name);
    let payload_types: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .filter_map(|variant| variant.fields.first().map(|field| go_type(&field.ty).into_owned()))
        .collect();
    let homogeneous_payload_type = (payload_types.len() == 1)
        .then(|| payload_types.first().cloned())
        .flatten();
    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .map(|variant| {
            let wire_value = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            let constructor = format!("New{go_enum_name}{}", go_variant_name(&variant.name));
            let payload_type = variant.fields.first().map(|field| go_type(&field.ty).into_owned());
            minijinja::context! {
                wire_value,
                constructor,
                payload_type,
                has_payload => payload_type.is_some(),
            }
        })
        .collect();

    crate::backends::go::template_env::render(
        "adjacent_tagged_enum.jinja",
        context! {
            go_enum_name,
            tag_name,
            content_name,
            tag_field,
            content_field,
            homogeneous_payload_type,
            variants,
        },
    )
}

/// Returns true if this enum is emitted as a `json.RawMessage` passthrough type.
///
/// Delegates to [`go_enum_representation`] rather than restating its conditions: callers
/// outside this module partition enums with this predicate and must see the same answer the
/// emitter acted on, or a type is declared one way and referenced another. ~keep
pub(crate) fn is_passthrough_raw_message_enum(enum_def: &EnumDef) -> bool {
    go_enum_representation(enum_def) == GoEnumRepresentation::RawMessage
}

/// Whether an all-tuple enum with no named-struct payload mixes scalar and collection variants,
/// so no single Go type can describe the wire value.
fn is_heterogeneous_shape_union(enum_def: &EnumDef) -> bool {
    let is_data_enum = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    if !is_data_enum {
        return false;
    }
    let all_data_fields_are_tuple = enum_def
        .variants
        .iter()
        .all(|v| v.fields.is_empty() || v.fields.iter().all(is_tuple_field));
    if !all_data_fields_are_tuple {
        return false;
    }
    let any_tuple_field_is_named_struct = enum_def.variants.iter().any(|v| {
        v.fields
            .iter()
            .any(|f| is_tuple_field(f) && matches!(&f.ty, TypeRef::Named(_)))
    });
    if any_tuple_field_is_named_struct {
        return false;
    }
    enum_def.variants.iter().any(|v| {
        v.fields
            .iter()
            .any(|f| is_tuple_field(f) && matches!(&f.ty, TypeRef::Vec(_) | TypeRef::Map(_, _)))
    })
}

/// Generate a Go type that wraps `json.RawMessage` for an untagged enum whose
/// variants mix scalar and collection shapes — the wire form is whatever shape
/// the value happened to have, and the Go side passes the bytes through.
///
/// When `enum_def.name` appears in `text_types`, an additional `Text() string`
/// method is emitted that extracts the display text from the raw JSON bytes:
/// a JSON string is returned verbatim; a JSON array of `{"type":"text","text":"…"}`
/// objects has its `"text"` fields concatenated; anything else returns `""`.
fn gen_passthrough_raw_message_enum(enum_def: &EnumDef, text_types: &[String]) -> String {
    let mut out = String::new();
    let go_enum_name = go_type_name(&enum_def.name);
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();

    emit_type_doc(
        &mut out,
        &go_enum_name,
        &enum_def.doc,
        "is an untagged union type whose variants have heterogeneous JSON shapes \
         (scalar vs. array). Stored as raw JSON bytes so any variant round-trips.",
    );
    out.push_str(&crate::backends::go::template_env::render(
        "passthrough_raw_message_enum_body.jinja",
        context! {
            enum_name => &go_enum_name,
            variants => variant_names.join(", "),
        },
    ));

    if text_types.iter().any(|t| t == &enum_def.name) {
        out.push('\n');
        out.push_str(&crate::backends::go::template_env::render(
            "passthrough_raw_message_text_accessor.jinja",
            context! {
                enum_name => &go_enum_name,
            },
        ));
    }

    out
}

/// Generate a Go "newtype-tuple" enum as `type X string` with const block.
///
/// Used for Rust enums that have one or more unit variants plus one or more
/// "newtype" (single positional field) variants like `Custom(String)`.
/// The Go type is `type X string` — unit variants become named constants while
/// Custom/tuple variants are handled automatically because the underlying type
/// is `string` and any arbitrary string value round-trips through JSON as-is.
pub(in crate::backends::go::gen_bindings) fn gen_newtype_tuple_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);
    let go_enum_name = go_type_name(&enum_def.name);
    emit_type_doc(&mut out, &go_enum_name, &enum_def.doc, "is an enumeration type.");
    out.push_str(&crate::backends::go::template_env::render(
        "string_type_decl.jinja",
        minijinja::context! {
            name => &go_enum_name,
        },
    ));
    out.push('\n');
    out.push_str(&crate::backends::go::template_env::render(
        "const_block_header.jinja",
        minijinja::Value::default(),
    ));
    for variant in &enum_def.variants {
        if !variant.fields.is_empty() {
            continue;
        }
        let const_name = format!("{}{}", go_enum_name, go_variant_name(&variant.name));
        let const_value = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let doc_lines: Vec<String> = if !variant.doc.is_empty() {
            let mut lines = variant.doc.lines();
            let mut result = Vec::new();
            if let Some(first) = lines.next() {
                let trimmed = first.trim();
                let first_line = if trimmed.starts_with(&const_name) {
                    trimmed.to_string()
                } else {
                    let rest = {
                        let mut chars = trimmed.chars();
                        match chars.next() {
                            Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                            None => trimmed.to_string(),
                        }
                    };
                    format!("{} {}", const_name, rest)
                };
                result.push(first_line);
                result.extend(lines.map(|l| l.trim().to_string()));
            }
            result
        } else {
            vec![format!(
                "{} is the {} variant of {}.",
                const_name, variant.name, enum_def.name
            )]
        };
        out.push_str(&crate::backends::go::template_env::render(
            "const_variant.jinja",
            minijinja::context! {
                const_name => &const_name,
                type_name => &go_enum_name,
                wire_value => &const_value,
                doc_lines => &doc_lines,
            },
        ));
    }
    out.push_str(&crate::backends::go::template_env::render(
        "const_block_footer.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Generate a Go tagged union enum with Named struct fields.
///
/// Emits a struct with one pointer field per variant (containing the struct payload),
/// plus a discriminator tag field. For example, `FormatMetadata` with variants
/// `Pdf(PdfMetadata)`, `Excel(ExcelMetadata)` becomes:
///
/// ```go
/// type FormatMetadata struct {
///     FormatType string `json:"format_type"`
///     Pdf *PdfMetadata `json:"pdf_data,omitempty"`
///     Excel *ExcelMetadata `json:"excel_data,omitempty"`
///     ...
/// }
/// ```
///
/// Includes custom `UnmarshalJSON` that reads the tag first, then unmarshals
/// the payload into the correct pointer field.
fn gen_tuple_tagged_union_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(2048);
    let go_enum_name = go_type_name(&enum_def.name);
    let is_untagged = enum_def.serde_untagged;

    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();

    emit_type_doc(
        &mut out,
        &go_enum_name,
        &enum_def.doc,
        if is_untagged {
            "is an untagged union type (variants discriminated by JSON shape)."
        } else {
            "is a tagged union type (discriminated by format_type)."
        },
    );
    out.push_str(&crate::backends::go::template_env::render(
        "tagged_union_struct_header.jinja",
        context! {
            go_enum_name => &go_enum_name,
            variants_list => variant_names.join(", "),
        },
    ));

    if let Some(tag_name) = &enum_def.serde_tag {
        let tag_field = to_go_name(tag_name);
        out.push_str(&crate::backends::go::template_env::render(
            "tagged_union_tag_field.jinja",
            context! {
                tag_field => &tag_field,
                tag_name => tag_name,
            },
        ));
    }

    for variant in &enum_def.variants {
        if variant.fields.is_empty() {
            continue;
        }

        if let Some(field) = variant.fields.iter().find(|f| is_tuple_field(f))
            && let TypeRef::Named(struct_type_name) = &field.ty
        {
            let go_struct_type = go_type_name(struct_type_name);
            let field_name = go_variant_name(&variant.name);
            let json_field_name = apply_serde_rename_all(
                &crate::codegen::naming::pascal_to_snake(&variant.name),
                enum_def.serde_rename_all.as_deref(),
            );

            let doc_lines: Vec<&str> = if !variant.doc.is_empty() {
                variant.doc.lines().map(|l| l.trim()).collect()
            } else {
                vec![]
            };

            out.push_str(&crate::backends::go::template_env::render(
                "tagged_union_variant_field.jinja",
                context! {
                    doc_lines => doc_lines,
                    field_name => &field_name,
                    struct_type => &go_struct_type,
                    json_field_name => &json_field_name,
                },
            ));
        }
    }

    out.push_str("}\n\n");

    // Resolve the tag once, here, and hand it to the emitter. The emitter used to re-check it
    // with `.expect`, so "which marshalers" was decided on `serde_untagged` alone while the
    // tagged arm additionally required a tag — an enum with neither attribute took this else and
    // aborted the whole run with no mention of the offending type. `go_enum_representation` now
    // routes that enum elsewhere, and passing the tag down keeps the two in step. ~keep
    if let Some(tag_name) = enum_def.serde_tag.as_deref().filter(|_| !is_untagged) {
        emit_tagged_union_marshalers(&mut out, &go_enum_name, enum_def, tag_name);
    } else {
        emit_untagged_union_marshalers(&mut out, &go_enum_name, enum_def);
    }

    out
}

/// Generate a Go struct for an externally tagged enum — serde's DEFAULT representation, used
/// whenever neither `#[serde(tag = "...")]` nor `#[serde(untagged)]` is present.
///
/// The wire form is a single-key object, `{"Pdf": {..}}`, so a struct of `omitempty` variant
/// pointers keyed by each variant's serde wire name *is* that representation: `encoding/json`
/// writes exactly the one non-nil key and reads back exactly the key that was present. No
/// custom marshalers are emitted because a hand-written pair could only reproduce what the
/// default already does, and the key must be `wire_variant_value` — the variant name serde
/// itself writes — not a snake-cased field name, which is what the internally tagged generator
/// uses for its (wire-invisible) container fields. ~keep
///
/// [`is_externally_tagged_named_union`] is the precondition: every variant is a one-field tuple
/// wrapping a named type.
fn gen_externally_tagged_union_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);
    let go_enum_name = go_type_name(&enum_def.name);
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();

    emit_type_doc(
        &mut out,
        &go_enum_name,
        &enum_def.doc,
        "is an externally tagged union type: exactly one variant pointer is set, and the JSON \
         form is a single-key object keyed by that variant's name.",
    );
    out.push_str(&crate::backends::go::template_env::render(
        "tagged_union_struct_header.jinja",
        context! {
            go_enum_name => &go_enum_name,
            variants_list => variant_names.join(", "),
        },
    ));

    for variant in &enum_def.variants {
        let Some(field) = variant.fields.first() else {
            continue;
        };
        let TypeRef::Named(struct_type_name) = &field.ty else {
            continue;
        };
        let doc_lines: Vec<&str> = if variant.doc.is_empty() {
            vec![]
        } else {
            variant.doc.lines().map(|line| line.trim()).collect()
        };
        out.push_str(&crate::backends::go::template_env::render(
            "tagged_union_variant_field.jinja",
            context! {
                doc_lines => doc_lines,
                field_name => go_variant_name(&variant.name),
                struct_type => go_type_name(struct_type_name),
                json_field_name => crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                ),
            },
        ));
    }

    out.push_str("}\n");
    out
}

/// Emit MarshalJSON / UnmarshalJSON for `#[serde(tag = "...")]` enums.
///
/// `tag_name` is supplied by the caller that already proved it exists, so this function has no
/// second opinion on whether the enum is tagged. ~keep
fn emit_tagged_union_marshalers(out: &mut String, go_enum_name: &str, enum_def: &EnumDef, tag_name: &str) {
    let tag_field_name = to_go_name(tag_name);

    out.push_str(&crate::backends::go::template_env::render(
        "tagged_union_marshal_json_header.jinja",
        context! {
            go_enum_name => go_enum_name,
            tag_field_name => &tag_field_name,
        },
    ));

    for variant in &enum_def.variants {
        if variant.fields.is_empty() {
            continue;
        }
        if let Some(field) = variant.fields.iter().find(|f| is_tuple_field(f))
            && let TypeRef::Named(_) = &field.ty
        {
            let variant_go_name = go_variant_name(&variant.name);
            let wire_value = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            out.push_str(&crate::backends::go::template_env::render(
                "tagged_union_marshal_variant.jinja",
                context! {
                    wire_value => &wire_value,
                    variant_go_name => &variant_go_name,
                    tag_name => tag_name,
                },
            ));
        }
    }

    out.push_str(&crate::backends::go::template_env::render(
        "tagged_union_marshal_json_footer.jinja",
        context! {
            tag_name => tag_name,
            tag_field_name => &tag_field_name,
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::go::template_env::render(
        "tagged_union_unmarshal_json_header.jinja",
        context! {
            go_enum_name => go_enum_name,
            tag_field_name => &tag_field_name,
            tag_name => tag_name,
        },
    ));

    for variant in &enum_def.variants {
        if variant.fields.is_empty() {
            continue;
        }
        if let Some(field) = variant.fields.iter().find(|f| is_tuple_field(f))
            && let TypeRef::Named(struct_type_name) = &field.ty
        {
            let go_struct_type = go_type_name(struct_type_name);
            let variant_go_name = go_variant_name(&variant.name);
            let wire_value = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            out.push_str(&crate::backends::go::template_env::render(
                "tagged_union_unmarshal_variant.jinja",
                context! {
                    wire_value => &wire_value,
                    variant_go_name => &variant_go_name,
                    go_struct_type => &go_struct_type,
                },
            ));
        }
    }

    out.push_str(&crate::backends::go::template_env::render(
        "tagged_union_unmarshal_json_footer.jinja",
        minijinja::Value::default(),
    ));
}

/// Emit MarshalJSON / UnmarshalJSON for `#[serde(untagged)]` enums.
///
/// Marshal: dispatch on the first non-nil variant pointer.
/// Unmarshal: try each variant in declaration order; return on first success.
/// Uses `var v T; t.Field = &v` to allocate so that variant types which are
/// string aliases (e.g. `type Mode string`) work alongside struct types.
fn emit_untagged_union_marshalers(out: &mut String, go_enum_name: &str, enum_def: &EnumDef) {
    let variants_with_types: Vec<(String, String)> = enum_def
        .variants
        .iter()
        .filter_map(|v| {
            if v.fields.is_empty() {
                return None;
            }
            v.fields.iter().find(|f| is_tuple_field(f)).and_then(|f| {
                if let TypeRef::Named(struct_type_name) = &f.ty {
                    Some((go_variant_name(&v.name), go_type_name(struct_type_name)))
                } else {
                    None
                }
            })
        })
        .collect();

    let variants: Vec<minijinja::Value> = variants_with_types
        .iter()
        .map(|(field, ty)| {
            context! {
                field => field,
                ty => ty,
            }
        })
        .collect();

    out.push_str(&crate::backends::go::template_env::render(
        "untagged_union_marshalers.jinja",
        context! {
            enum_name => go_enum_name,
            variants => variants,
        },
    ));
}

/// Generate a Go unit enum as `type X string` with const block.
pub(in crate::backends::go::gen_bindings) fn gen_unit_enum_type(enum_def: &EnumDef) -> String {
    let go_enum_name = go_type_name(&enum_def.name);

    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let const_name = format!("{}{}", go_enum_name, go_variant_name(&v.name));
            let const_value = crate::codegen::naming::wire_variant_value(
                &v.name,
                v.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );

            let mut doc_lines = Vec::new();
            let doc_first_line = if !v.doc.is_empty() {
                let mut lines = v.doc.lines();
                if let Some(first) = lines.next() {
                    let trimmed = first.trim();
                    let first_line = if trimmed.starts_with(&const_name) {
                        trimmed.to_string()
                    } else {
                        let rest = {
                            let mut chars = trimmed.chars();
                            match chars.next() {
                                Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                                None => trimmed.to_string(),
                            }
                        };
                        format!("{} {}", const_name, rest)
                    };
                    doc_lines = lines.map(|l| l.trim().to_string()).collect();
                    first_line
                } else {
                    String::new()
                }
            } else {
                format!("{} is the {} variant of {}.", const_name, v.name, enum_def.name)
            };

            context! {
                const_name => const_name,
                rust_name => v.name,
                doc_first_line => doc_first_line,
                doc_lines => doc_lines,
                wire_value => const_value,
            }
        })
        .collect();

    crate::backends::go::template_env::render(
        "unit_enum.jinja",
        context! {
            go_name => go_enum_name,
            enum_name => enum_def.name,
            variants => variants,
        },
    )
}

/// Generate a Go data enum as sealed-interface with per-variant concrete structs.
///
/// For an externally-tagged enum (serde default with no `#[serde(tag)]`):
/// - Emits an interface with unexported `is{EnumName}()` marker method
/// - One concrete struct per variant with only its fields (no nullables)
/// - MarshalJSON/UnmarshalJSON on each concrete struct type
/// - An Unmarshal{EnumName}([]byte) helper to dispatch to the right variant
///
/// This pattern is type-safe: callers construct {EnumName}Variant{} directly,
/// and invalid combinations are impossible (no nullable fields).
pub(in crate::backends::go::gen_bindings) fn gen_data_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(2048);
    let go_enum_name = go_type_name(&enum_def.name);
    let variant_names: Vec<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
    // serde's default for a data-carrying enum with neither `#[serde(tag = ..)]` nor
    // `#[serde(untagged)]` is EXTERNAL tagging: `{"Variant": <inner>}`. That is a distinct wire
    // shape from both internal tagging (tag folded into the payload object) and untagged (no
    // wrapper at all) -- it must not be conflated with either. ~keep
    let is_externally_tagged = enum_def.serde_tag.is_none() && !enum_def.serde_untagged;

    emit_type_doc(
        &mut out,
        &go_enum_name,
        &enum_def.doc,
        "is a tagged union type (discriminated by type field).",
    );
    out.push_str(&crate::backends::go::template_env::render(
        "variant_comment.jinja",
        minijinja::context! {
            variants => variant_names.join(", "),
        },
    ));
    // Every variant, cased with the same `go_variant_name` initialism rule the concrete struct
    // declarations use below (e.g. `ImageUrl` -> `ImageURL`), so the doc comment names the
    // real emitted identifiers instead of a raw-cased approximation. ~keep
    let all_variant_names: Vec<String> = variant_names
        .iter()
        .map(|name| format!("{go_enum_name}{}", go_variant_name(name)))
        .collect();
    out.push_str(&crate::backends::go::template_env::render(
        "data_enum_interface.jinja",
        minijinja::context! {
            go_enum_name => &go_enum_name,
            variant_names => all_variant_names.join(", "),
        },
    ));

    for variant in &enum_def.variants {
        let variant_struct_name = format!("{go_enum_name}{}", go_variant_name(&variant.name));

        emit_type_doc(
            &mut out,
            &variant_struct_name,
            &variant.doc,
            &format!("is the {} variant of {}.", variant.name, enum_def.name),
        );

        let scalar_tuple_field = go_data_enum_variant_scalar_tuple_field(enum_def, variant);

        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_struct_header.jinja",
            minijinja::context! {
                variant_struct_name => &variant_struct_name,
            },
        ));
        if let Some(field) = scalar_tuple_field {
            let field_type = go_type(&field.ty);
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_scalar_tuple_field.jinja",
                minijinja::context! {
                    field_type => &field_type,
                },
            ));
        }
        for field in &variant.fields {
            let Some((field_go_name, json_name)) = go_data_enum_variant_field(enum_def, field) else {
                continue;
            };
            let field_type = if field.optional {
                go_optional_type(&field.ty)
            } else {
                go_type(&field.ty)
            };
            let json_tag = if field.optional {
                format!("json:\"{},omitempty\"", json_name)
            } else {
                format!("json:\"{}\"", json_name)
            };

            let doc_lines: Vec<&str> = if !field.doc.is_empty() {
                field.doc.lines().map(|l| l.trim()).collect()
            } else {
                vec![]
            };
            out.push_str(&crate::backends::go::template_env::render(
                "struct_field.jinja",
                minijinja::context! {
                    doc_lines => doc_lines,
                    field_name => &field_go_name,
                    field_type => &field_type,
                    json_tag => &json_tag,
                },
            ));
        }
        out.push_str("}\n\n");

        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_marker_method.jinja",
            minijinja::context! {
                variant_struct_name => &variant_struct_name,
                go_enum_name => &go_enum_name,
            },
        ));

        let wire_value = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_type_method.jinja",
            minijinja::context! {
                variant_struct_name => &variant_struct_name,
                wire_value => &wire_value,
            },
        ));

        if scalar_tuple_field.is_some() {
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_scalar_marshalers.jinja",
                minijinja::context! {
                    variant_struct_name => &variant_struct_name,
                },
            ));
        } else {
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_marshal_json_header.jinja",
                minijinja::context! {
                    variant_struct_name => &variant_struct_name,
                },
            ));
            if let Some(tag_name) = &enum_def.serde_tag {
                let tag_json_name = tag_name.as_str();
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_aux_field.jinja",
                    minijinja::context! {
                        field_go_name => to_go_name(tag_name),
                        field_type => "string",
                        json_tag => format!("json:\"{tag_json_name}\""),
                    },
                ));
            }
            for field in &variant.fields {
                let Some((field_go_name, json_name)) = go_data_enum_variant_field(enum_def, field) else {
                    continue;
                };
                let field_type = if field.optional {
                    go_optional_type(&field.ty)
                } else {
                    go_type(&field.ty)
                };
                let json_tag = if field.optional {
                    format!("json:\"{json_name},omitempty\"")
                } else {
                    format!("json:\"{json_name}\"")
                };
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_aux_field.jinja",
                    minijinja::context! {
                        field_go_name => &field_go_name,
                        field_type => &field_type,
                        json_tag => &json_tag,
                    },
                ));
            }
            if is_externally_tagged {
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_json_external_values_header.jinja",
                    minijinja::context! {
                        wire_value => &wire_value,
                    },
                ));
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_json_values_header.jinja",
                    minijinja::Value::default(),
                ));
            }
            if let Some(tag_name) = &enum_def.serde_tag {
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_aux_value.jinja",
                    minijinja::context! {
                        field_go_name => to_go_name(tag_name),
                        value_expr => "v.Type()",
                    },
                ));
            }
            for field in &variant.fields {
                if is_tuple_field(field) {
                    continue;
                }
                let field_go_name = to_go_name(&field.name);
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_aux_value.jinja",
                    minijinja::context! {
                        field_go_name => &field_go_name,
                        value_expr => format!("v.{field_go_name}"),
                    },
                ));
            }
            if is_externally_tagged {
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_json_external_footer.jinja",
                    minijinja::Value::default(),
                ));
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_marshal_json_footer.jinja",
                    minijinja::Value::default(),
                ));
            }
        }
    }

    out.push_str(&crate::backends::go::template_env::render(
        "data_enum_unmarshal_header.jinja",
        minijinja::context! {
            go_enum_name => &go_enum_name,
        },
    ));

    if enum_def.serde_untagged {
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_empty_check.jinja",
            minijinja::context! {
                go_enum_name => &go_enum_name,
            },
        ));

        for variant in &enum_def.variants {
            let variant_struct_name = format!("{go_enum_name}{}", go_variant_name(&variant.name));

            let shape_check = go_data_enum_untagged_shape(variant).map(GoDataEnumShape::go_first_byte_check);

            if let Some(check) = shape_check {
                out.push_str(&crate::backends::go::template_env::render(
                    "data_enum_unmarshal_shape_variant.jinja",
                    minijinja::context! {
                        check => check,
                        variant_struct_name => &variant_struct_name,
                    },
                ));
            }
        }

        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_unknown_shape.jinja",
            minijinja::context! {
                go_enum_name => &go_enum_name,
            },
        ));
    } else if let Some(tag_name) = &enum_def.serde_tag {
        let discriminator_field = to_go_name(tag_name);

        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_wire_header.jinja",
            minijinja::context! {
                tag_field => Some(discriminator_field.as_str()),
                tag_name => Some(tag_name.as_str()),
                discriminator_field => &discriminator_field,
            },
        ));
        for variant in &enum_def.variants {
            let wire_value = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            let variant_struct_name = format!("{go_enum_name}{}", go_variant_name(&variant.name));
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_unmarshal_wire_variant.jinja",
                minijinja::context! {
                    wire_value => &wire_value,
                    variant_struct_name => &variant_struct_name,
                },
            ));
        }
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_unknown_type.jinja",
            minijinja::context! {
                go_enum_name => &go_enum_name,
                discriminator_field => &discriminator_field,
            },
        ));
    } else {
        // External tagging (serde's default): the JSON is `{"Variant": <inner>}`, so the
        // discriminator is the object's sole key, not a field inside the payload. ~keep
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_external_header.jinja",
            minijinja::context! {
                go_enum_name => &go_enum_name,
            },
        ));
        for variant in &enum_def.variants {
            let wire_value = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            let variant_struct_name = format!("{go_enum_name}{}", go_variant_name(&variant.name));
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_unmarshal_external_variant.jinja",
                minijinja::context! {
                    wire_value => &wire_value,
                    variant_struct_name => &variant_struct_name,
                },
            ));
        }
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_external_footer.jinja",
            minijinja::context! {
                go_enum_name => &go_enum_name,
            },
        ));
    }

    out
}
