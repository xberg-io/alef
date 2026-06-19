use crate::backends::go::type_map::{go_optional_type, go_type};
use crate::codegen::naming::{apply_serde_rename_all, go_type_name, to_go_name};
use crate::core::ir::{EnumDef, TypeRef};
use minijinja::context;

use super::helpers::{emit_type_doc, is_tuple_field};

pub(in crate::backends::go::gen_bindings) fn gen_enum_type(enum_def: &EnumDef, text_types: &[String]) -> String {
    let is_data_enum = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if !is_data_enum {
        return gen_unit_enum_type(enum_def);
    }

    // Detect "newtype-tuple" pattern: a data enum whose data variants contain only
    // positional tuple fields (all of which `is_tuple_field` returns true for).
    let all_data_fields_are_tuple = enum_def
        .variants
        .iter()
        .all(|v| v.fields.is_empty() || v.fields.iter().all(is_tuple_field));

    if all_data_fields_are_tuple {
        // Check if any tuple field has a Named (struct) type.
        // If so, use tagged union; otherwise use newtype tuple enum.
        let any_tuple_field_is_named_struct = enum_def.variants.iter().any(|v| {
            v.fields
                .iter()
                .any(|f| is_tuple_field(f) && matches!(&f.ty, TypeRef::Named(_)))
        });

        if any_tuple_field_is_named_struct {
            gen_tuple_tagged_union_type(enum_def)
        } else if is_passthrough_raw_message_enum(enum_def) {
            // Untagged enums whose variants mix scalar and collection shapes (e.g.
            // `Single(String) | Multiple(Vec<String>)`, `Text(String) | Parts(Vec<Foo>)`)
            // can't be modeled as `type X string` — Vec serializes to an array, not a
            // string, and any decoded JSON must round-trip without rejecting either shape.
            // Emit them as a `json.RawMessage` wrapper that passes the raw bytes through
            // unchanged (mirrors the napi `serde_json::Value` wrapper for the same case).
            gen_passthrough_raw_message_enum(enum_def, text_types)
        } else {
            gen_newtype_tuple_enum_type(enum_def)
        }
    } else {
        gen_data_enum_type(enum_def)
    }
}

/// Returns true if this enum should be emitted as a `json.RawMessage` passthrough
/// type — used for untagged enums with mixed scalar/collection variants.
pub(in crate::backends::go::gen_bindings) fn is_passthrough_raw_message_enum(enum_def: &EnumDef) -> bool {
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
        let const_name = format!("{}{}", go_enum_name, to_go_name(&variant.name));
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

    // Collect variant names for the doc comment
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

    // Emit the serde tag discriminator field first (e.g. `FormatType string \`json:"format_type"\``).
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

    // Emit one pointer field per variant
    for variant in &enum_def.variants {
        if variant.fields.is_empty() {
            continue;
        }

        // Find the first (and typically only) tuple field
        if let Some(field) = variant.fields.iter().find(|f| is_tuple_field(f)) {
            if let TypeRef::Named(struct_type_name) = &field.ty {
                let go_struct_type = go_type_name(struct_type_name);
                let field_name = to_go_name(&variant.name);
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
    }

    out.push_str("}\n\n");

    if is_untagged {
        emit_untagged_union_marshalers(&mut out, &go_enum_name, enum_def);
    } else {
        emit_tagged_union_marshalers(&mut out, &go_enum_name, enum_def);
    }

    out
}

/// Emit MarshalJSON / UnmarshalJSON for `#[serde(tag = "...")]` enums.
fn emit_tagged_union_marshalers(out: &mut String, go_enum_name: &str, enum_def: &EnumDef) {
    let tag_name = enum_def
        .serde_tag
        .as_deref()
        .expect("emit_tagged_union_marshalers called for untagged enum");
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
        if let Some(field) = variant.fields.iter().find(|f| is_tuple_field(f)) {
            if let TypeRef::Named(_) = &field.ty {
                let variant_go_name = to_go_name(&variant.name);
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
        if let Some(field) = variant.fields.iter().find(|f| is_tuple_field(f)) {
            if let TypeRef::Named(struct_type_name) = &field.ty {
                let go_struct_type = go_type_name(struct_type_name);
                let variant_go_name = to_go_name(&variant.name);
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
                    Some((to_go_name(&v.name), go_type_name(struct_type_name)))
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
            let const_name = format!("{}{}", go_enum_name, to_go_name(&v.name));
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

    // Emit the sealed interface
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
    let first_variant = format!("{}{}", go_enum_name, variant_names.first().unwrap_or(&""));
    let second_variant = format!("{}{}", go_enum_name, variant_names.get(1).unwrap_or(&""));
    out.push_str(&crate::backends::go::template_env::render(
        "data_enum_interface.jinja",
        minijinja::context! {
            go_enum_name => &go_enum_name,
            first_variant => &first_variant,
            second_variant => &second_variant,
        },
    ));

    // Emit one concrete struct per variant
    for variant in &enum_def.variants {
        let variant_struct_name = format!("{go_enum_name}{}", to_go_name(&variant.name));

        // Doc comment for the concrete struct
        emit_type_doc(
            &mut out,
            &variant_struct_name,
            &variant.doc,
            &format!("is the {} variant of {}.", variant.name, enum_def.name),
        );

        // Detect "scalar-tuple" variants (single positional tuple field carrying a
        // primitive, String, Char, or Path). For untagged enums these need a
        // named Go field (`Value <type>`) to hold the payload — the default
        // tuple-field skip would emit an empty struct that loses the payload
        // entirely. Surfaced on `enum RerankDocument { Text(String), Object { … } }`
        // where the Text variant otherwise lost its inner String content.
        let scalar_tuple_field =
            if enum_def.serde_untagged && variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]) {
                match &variant.fields[0].ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Primitive(_) => Some(&variant.fields[0]),
                    _ => None,
                }
            } else {
                None
            };

        // Struct definition with only this variant's fields
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_struct_header.jinja",
            minijinja::context! {
                variant_struct_name => &variant_struct_name,
            },
        ));
        if let Some(field) = scalar_tuple_field {
            // Scalar tuple variant: emit `Value <type>` to hold the payload.
            // The `json:"-"` tag prevents encoding/json from picking it up
            // through default field marshalling — we emit a custom
            // MarshalJSON/UnmarshalJSON below that handles the bare scalar.
            let field_type = go_type(&field.ty);
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_scalar_tuple_field.jinja",
                minijinja::context! {
                    field_type => &field_type,
                },
            ));
        }
        for field in &variant.fields {
            if is_tuple_field(field) {
                continue;
            }
            let field_go_name = to_go_name(&field.name);
            let field_type = if field.optional {
                go_optional_type(&field.ty)
            } else {
                go_type(&field.ty)
            };
            let json_name = apply_serde_rename_all(&field.name, enum_def.serde_rename_all.as_deref());
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

        // Implement the sealed marker method
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_marker_method.jinja",
            minijinja::context! {
                variant_struct_name => &variant_struct_name,
                go_enum_name => &go_enum_name,
            },
        ));

        // Implement the Type() method
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
            // Scalar tuple variant: marshal as a bare JSON scalar (the inner Value),
            // and unmarshal a bare JSON scalar into Value. Skip the default
            // tag-wrapping aux-struct dance entirely.
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_scalar_marshalers.jinja",
                minijinja::context! {
                    variant_struct_name => &variant_struct_name,
                },
            ));
        } else {
            // Implement MarshalJSON for the concrete struct
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
                if is_tuple_field(field) {
                    continue;
                }
                let field_go_name = to_go_name(&field.name);
                let field_type = if field.optional {
                    go_optional_type(&field.ty)
                } else {
                    go_type(&field.ty)
                };
                let json_name = apply_serde_rename_all(&field.name, enum_def.serde_rename_all.as_deref());
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
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_marshal_json_values_header.jinja",
                minijinja::Value::default(),
            ));
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
            out.push_str(&crate::backends::go::template_env::render(
                "data_enum_marshal_json_footer.jinja",
                minijinja::Value::default(),
            ));
        }
    }

    // Emit the Unmarshal{EnumName} helper function.
    // Untagged enums use shape-discriminated try-each-variant unmarshalling.
    // Tagged enums (internally tagged with serde_tag) use the wire-struct
    // discriminator field switch.
    out.push_str(&crate::backends::go::template_env::render(
        "data_enum_unmarshal_header.jinja",
        minijinja::context! {
            go_enum_name => &go_enum_name,
        },
    ));

    if enum_def.serde_untagged {
        // Untagged path: sniff the first non-whitespace byte to filter candidates,
        // then try each variant in declared order.
        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_empty_check.jinja",
            minijinja::context! {
                go_enum_name => &go_enum_name,
            },
        ));

        for variant in &enum_def.variants {
            let variant_struct_name = format!("{go_enum_name}{}", to_go_name(&variant.name));

            // Determine the expected JSON shape for this variant.
            // Tuple/newtype variants: shape is determined by the single inner field type.
            // Struct variants with named fields: always an object.
            let shape_check = if variant.fields.len() == 1 && is_tuple_field(&variant.fields[0]) {
                match &variant.fields[0].ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => Some("firstByte == '\"'"),
                    TypeRef::Vec(_) | TypeRef::Bytes => Some("firstByte == '['"),
                    TypeRef::Primitive(_) => Some("firstByte != '\"' && firstByte != '{' && firstByte != '['"),
                    // Named types, maps, and everything else are assumed to be objects.
                    _ => Some("firstByte == '{'"),
                }
            } else if variant.fields.is_empty() {
                // Unit variant in an untagged context — skip; cannot match a JSON value.
                None
            } else {
                // Struct variant with named fields is always an object.
                Some("firstByte == '{'")
            };

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
    } else {
        // Tagged path (internally-tagged or externally-tagged): read the discriminator
        // field from the wire struct, then switch on its value.
        let tag_field = enum_def.serde_tag.as_ref().map(|tn| to_go_name(tn));
        let discriminator_field = tag_field.as_deref().unwrap_or("Type");

        out.push_str(&crate::backends::go::template_env::render(
            "data_enum_unmarshal_wire_header.jinja",
            minijinja::context! {
                tag_field => tag_field.as_deref(),
                tag_name => enum_def.serde_tag.as_deref(),
                discriminator_field => discriminator_field,
            },
        ));
        for variant in &enum_def.variants {
            let wire_value = crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                );
            let variant_struct_name = format!("{go_enum_name}{}", to_go_name(&variant.name));
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
                discriminator_field => discriminator_field,
            },
        ));
    }

    out
}
