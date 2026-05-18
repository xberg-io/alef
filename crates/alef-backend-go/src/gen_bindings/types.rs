use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_type_name, to_go_name};
use alef_codegen::shared::binding_fields;
use alef_core::ir::{DefaultValue, EnumDef, FieldDef, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use minijinja::context;

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// Go structs require named fields, so these must be skipped.
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Apply a serde `rename_all` strategy to a field name.
/// Returns the field name transformed according to the strategy, or the
/// original name if no strategy is set.
pub(super) fn apply_serde_rename(field_name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("camelCase") => field_name.to_lower_camel_case(),
        Some("PascalCase") => field_name.to_pascal_case(),
        Some("SCREAMING_SNAKE_CASE") => field_name.to_uppercase(),
        // snake_case is the Rust default — field names are already snake_case.
        _ => field_name.to_string(),
    }
}

/// Returns true if a non-optional struct field should be emitted as a pointer type with
/// `omitempty` in a struct that has `has_default: true`.
///
/// This is necessary when the Go zero value for a field differs from the Rust `Default` value.
/// Without pointer+omitempty, unset fields serialize as their Go zero value (0, false, ""), which
/// the Rust FFI layer may reject or misinterpret (e.g., `request_timeout: 0` is invalid).
///
/// Cases that require pointer+omitempty:
/// - `TypeRef::Duration` — Duration zero is always invalid; real defaults are non-zero (e.g., 30s)
/// - `BoolLiteral(true)` — Rust default is `true`, Go zero is `false`
/// - `IntLiteral(n)` where n != 0 — Rust default is n, Go zero is 0
/// - `FloatLiteral(f)` where f != 0.0 — Rust default is f, Go zero is 0.0
/// - `StringLiteral(s)` where !s.is_empty() — Rust default is s, Go zero is ""
/// - `EnumVariant(_)` — Rust default is a specific variant, Go zero is ""
pub(super) fn needs_omitempty_pointer(field: &FieldDef) -> bool {
    // Duration fields always need pointer+omitempty: zero duration is invalid in Rust
    if matches!(field.ty, TypeRef::Duration) {
        return true;
    }
    match &field.typed_default {
        Some(DefaultValue::BoolLiteral(true)) => true,
        Some(DefaultValue::IntLiteral(n)) if *n != 0 => true,
        Some(DefaultValue::FloatLiteral(f)) if *f != 0.0 => true,
        Some(DefaultValue::StringLiteral(s)) if !s.is_empty() => true,
        Some(DefaultValue::EnumVariant(_)) => true,
        _ => false,
    }
}

/// Returns true if a struct type has at least one field with a non-zero default value.
///
/// Types with all-zero-default fields do not need the functional-options builder pattern;
/// idiomatic Go usage is direct struct literals like `&Span{StartByte: 1, EndByte: 5}`.
/// Types with at least one non-zero default benefit from the NewX/WithX pattern to ensure
/// proper initialization of non-zero defaults without requiring callers to remember every field.
pub(super) fn has_non_zero_default(typ: &TypeDef) -> bool {
    binding_fields(&typ.fields).any(|field| {
        // Duration fields always count as non-zero (zero duration is invalid).
        if matches!(field.ty, TypeRef::Duration) {
            return true;
        }
        // Check if the field has a non-zero default value.
        match &field.typed_default {
            Some(DefaultValue::BoolLiteral(true)) => true,
            Some(DefaultValue::IntLiteral(n)) => *n != 0,
            Some(DefaultValue::FloatLiteral(f)) => *f != 0.0,
            Some(DefaultValue::StringLiteral(s)) => !s.is_empty(),
            Some(DefaultValue::EnumVariant(_)) => true,
            _ => false,
        }
    })
}

/// Generate the package-level `unmarshalBytes` helper.
///
/// Emitted exactly once per generated `binding.go`. Methods and functions
/// returning `TypeRef::Bytes` reference this helper by name. The helper takes
/// a `*C.uint8_t` aliasing pointer (typically returned by an FFI accessor
/// that hands out a borrowed view into a parent handle's buffer) and produces
/// a freshly-allocated `*[]byte` copy. The caller MUST keep the parent handle
/// alive across the helper call; the returned slice is detached.
///
/// The helper does not free the input pointer because the FFI surface aliases
/// internal storage; freeing here would corrupt the parent handle.
pub(super) fn gen_unmarshal_bytes_helper() -> String {
    crate::template_env::render("unmarshal_bytes_helper.jinja", minijinja::Value::default())
}

/// Generate the lastError() helper function.
pub(super) fn gen_last_error_helper(ffi_prefix: &str) -> String {
    // Note: ctx is a borrowed pointer into thread-local storage, NOT a heap allocation.
    // Do NOT call free_string on it — that causes a double-free crash on the next FFI call.
    crate::template_env::render(
        "last_error_helper.jinja",
        context! {
            ffi_prefix => ffi_prefix,
        },
    )
}

/// Emit Go-convention doc comment lines for an exported symbol into `out`.
///
/// Go's revive linter requires that the first line of a doc comment starts with
/// the exported name (with an optional leading article). This function rewrites
/// verbatim docs that begin with an article ("A ", "An ", "The ") by prepending
/// the symbol name, and falls back to a generated comment when no doc is present.
///
/// Used for both types and methods/functions: the symbol name appears at the
/// start of the comment so `go doc`, `godoc`, and `pkg.go.dev` recognise the
/// item description.
///
/// Rustdoc sections are translated into Godoc-friendly prose:
/// - `# Arguments` → `// Arguments:` followed by `//   - name: desc` bullets
/// - `# Returns`   → `// Returns ...`
/// - `# Errors`    → `// Errors are returned when ...` (verbatim body if it
///   already reads naturally)
/// - `# Example` / `# Examples` → `//\n// Example:\n//   <indented code>`
///
/// Examples:
/// - `"A chat message."` on `Message` → `"// Message is a chat message."`
/// - `"Message represents…"` on `Message` → `"// Message represents…"` (unchanged)
/// - empty doc on `Message` → `"// Message <fallback>."`
pub(super) fn emit_type_doc(out: &mut String, type_name: &str, doc: &str, fallback: &str) {
    if doc.is_empty() {
        out.push_str(&crate::template_env::render(
            "type_doc_header.jinja",
            context! {
                type_name => type_name,
                doc => fallback,
            },
        ));
        return;
    }
    let sections = alef_codegen::doc_emission::parse_rustdoc_sections(doc);
    let summary = sections.summary.trim();
    if summary.is_empty() {
        // No summary prose, only sections — synthesise a header line then
        // append sections so the symbol still has a name-prefixed doc line.
        out.push_str(&crate::template_env::render(
            "type_doc_header.jinja",
            context! {
                type_name => type_name,
                doc => fallback,
            },
        ));
    } else {
        emit_godoc_summary(out, type_name, summary);
    }
    emit_godoc_sections(out, &sections);
}

/// Emit the summary prose with the symbol name prefixed onto the first line.
///
/// Subsequent lines of the summary are emitted as plain `// <line>` continuation
/// comments. Article-stripping is applied only to the first sentence so
/// "A foo" becomes "Name is a foo".
fn emit_godoc_summary(out: &mut String, symbol_name: &str, summary: &str) {
    let mut lines = summary.lines();
    let first = lines.next().unwrap_or("").trim();
    // The template prepends `// {{ symbol_name }} `, so strip a leading
    // occurrence of `{symbol_name}` (plus an optional separator space) from
    // the rendered body — otherwise summaries that already start with the
    // exported name produce `// Name Name does ...` double-prefixes.
    let body = if let Some(rest) = first.strip_prefix(symbol_name) {
        rest.trim_start().to_string()
    } else {
        let rest = first
            .strip_prefix("A ")
            .or_else(|| first.strip_prefix("An "))
            .or_else(|| first.strip_prefix("The "))
            .unwrap_or(first);
        if rest.is_empty() {
            String::new()
        } else {
            let mut chars = rest.chars();
            match chars.next() {
                Some(c) => c.to_lowercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    };
    out.push_str(&crate::template_env::render(
        "type_doc_header.jinja",
        context! {
            type_name => symbol_name,
            doc => &body,
        },
    ));
    for line in lines {
        out.push_str(&crate::template_env::render(
            "go_doc_comment_line.jinja",
            context! { line => line.trim() },
        ));
    }
}

/// Push a blank `//` separator line if one isn't already at the end of `out`.
fn push_godoc_blank(out: &mut String) {
    if !out.ends_with("//\n") {
        out.push_str("//\n");
    }
}

/// Push `// <text>` line, or `//` when `text` is empty.
fn push_godoc_line(out: &mut String, text: &str) {
    if text.is_empty() {
        out.push_str("//\n");
    } else {
        out.push_str("// ");
        out.push_str(text);
        out.push('\n');
    }
}

/// Emit a section body prefixed with `lead` on the first line.
///
/// If the body already starts with the lead phrase (case-insensitive) the body
/// is emitted verbatim. Otherwise the first content word's leading character is
/// lowercased so `Returns` + `"The root node"` reads as `Returns the root node`
/// rather than `Returns The root node`.
fn emit_prefixed_section(out: &mut String, body: &str, lead: &str) {
    let trimmed = body.trim();
    let lead_first_word = lead.split_whitespace().next().unwrap_or(lead);
    let starts_with_lead = trimmed
        .split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case(lead_first_word));
    if starts_with_lead {
        for line in trimmed.lines() {
            push_godoc_line(out, line.trim());
        }
        return;
    }
    let mut lines = trimmed.lines();
    if let Some(first) = lines.next() {
        let first = first.trim();
        let first_lc = first
            .chars()
            .next()
            .map(|c| c.to_lowercase().to_string() + &first[c.len_utf8()..])
            .unwrap_or_default();
        push_godoc_line(out, &format!("{} {}", lead, first_lc));
    }
    for line in lines {
        push_godoc_line(out, line.trim());
    }
}

/// Emit `# Arguments`, `# Returns`, `# Errors`, `# Example` sections of a
/// rustdoc block as Godoc-friendly prose. Each section is separated from
/// preceding output by a blank `//` line so godoc tooling renders paragraphs.
fn emit_godoc_sections(out: &mut String, sections: &alef_codegen::doc_emission::RustdocSections) {
    if let Some(body) = sections.arguments.as_deref() {
        push_godoc_blank(out);
        push_godoc_line(out, "Arguments:");
        let bullets = alef_codegen::doc_emission::parse_arguments_bullets(body);
        if bullets.is_empty() {
            for line in body.lines() {
                push_godoc_line(out, line.trim());
            }
        } else {
            for (name, desc) in bullets {
                let bullet = if desc.is_empty() {
                    format!("  - {}", name)
                } else {
                    format!("  - {}: {}", name, desc)
                };
                push_godoc_line(out, &bullet);
            }
        }
    }
    if let Some(body) = sections.returns.as_deref() {
        push_godoc_blank(out);
        emit_prefixed_section(out, body, "Returns");
    }
    if let Some(body) = sections.errors.as_deref() {
        push_godoc_blank(out);
        emit_prefixed_section(out, body, "Errors are returned when");
    }
    if let Some(body) = sections.example.as_deref() {
        push_godoc_blank(out);
        push_godoc_line(out, "Example:");
        // Godoc renders indented blocks as preformatted code. Strip a single
        // ``` fence pair if present, then indent each line with two spaces.
        let mut in_fence = false;
        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if line.trim().is_empty() {
                out.push_str("//\n");
            } else {
                out.push_str("//   ");
                out.push_str(line.trim_end());
                out.push('\n');
            }
            let _ = in_fence;
        }
    }
}

/// Generate a Go enum type definition.
///
/// For unit enums (all variants have no fields): generates `type X string` with constants.
/// For newtype-tuple enums (all data variant fields are positional tuple fields with primitive types):
/// generates `type X string` with constants for named variants plus custom `MarshalJSON`/`UnmarshalJSON`.
/// For tuple-tagged-union enums (tuple fields with Named struct types): generates a struct with
/// one pointer field per variant, discriminated by a tag field, plus custom JSON marshaling.
/// For structural data enums (any variant has named fields): generates a flattened Go
/// struct with all variant fields collected and deduplicated, using pointer types for
/// fields not present in every variant.
pub(super) fn gen_enum_type(enum_def: &EnumDef) -> String {
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
            gen_passthrough_raw_message_enum(enum_def)
        } else {
            gen_newtype_tuple_enum_type(enum_def)
        }
    } else {
        gen_data_enum_type(enum_def)
    }
}

/// Returns true if this enum should be emitted as a `json.RawMessage` passthrough
/// type — used for untagged enums with mixed scalar/collection variants.
pub(super) fn is_passthrough_raw_message_enum(enum_def: &EnumDef) -> bool {
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
fn gen_passthrough_raw_message_enum(enum_def: &EnumDef) -> String {
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
    out.push_str(&crate::template_env::render(
        "passthrough_raw_message_enum_body.jinja",
        context! {
            enum_name => &go_enum_name,
            variants => variant_names.join(", "),
        },
    ));
    out
}

/// Compute the wire value for a unit enum variant.
///
/// Priority order:
/// 1. Explicit `#[serde(rename = "...")]` on the variant (`serde_rename`).
/// 2. Enum-level `#[serde(rename_all = "...")]` applied to the variant name.
/// 3. Default: snake_case of the variant name.
fn enum_variant_wire_value(variant: &alef_core::ir::EnumVariant, enum_def: &EnumDef) -> String {
    if let Some(rename) = &variant.serde_rename {
        return rename.clone();
    }
    apply_serde_rename(
        &alef_codegen::naming::pascal_to_snake(&variant.name),
        enum_def.serde_rename_all.as_deref(),
    )
}

/// Generate a Go "newtype-tuple" enum as `type X string` with const block.
///
/// Used for Rust enums that have one or more unit variants plus one or more
/// "newtype" (single positional field) variants like `Custom(String)`.
/// The Go type is `type X string` — unit variants become named constants while
/// Custom/tuple variants are handled automatically because the underlying type
/// is `string` and any arbitrary string value round-trips through JSON as-is.
fn gen_newtype_tuple_enum_type(enum_def: &EnumDef) -> String {
    let mut out = String::with_capacity(1024);
    let go_enum_name = go_type_name(&enum_def.name);
    emit_type_doc(&mut out, &go_enum_name, &enum_def.doc, "is an enumeration type.");
    out.push_str(&crate::template_env::render(
        "string_type_decl.jinja",
        minijinja::context! {
            name => &go_enum_name,
        },
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "const_block_header.jinja",
        minijinja::Value::default(),
    ));
    for variant in &enum_def.variants {
        if !variant.fields.is_empty() {
            continue;
        }
        let const_name = format!("{}{}", go_enum_name, to_go_name(&variant.name));
        let wire_value = enum_variant_wire_value(variant, enum_def);
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
        out.push_str(&crate::template_env::render(
            "const_variant.jinja",
            minijinja::context! {
                const_name => &const_name,
                type_name => &go_enum_name,
                wire_value => &wire_value,
                doc_lines => &doc_lines,
            },
        ));
    }
    out.push_str(&crate::template_env::render(
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
    out.push_str(&crate::template_env::render(
        "tagged_union_struct_header.jinja",
        context! {
            go_enum_name => &go_enum_name,
            variants_list => variant_names.join(", "),
        },
    ));

    // Emit the serde tag discriminator field first (e.g. `FormatType string \`json:"format_type"\``).
    if let Some(tag_name) = &enum_def.serde_tag {
        let tag_field = to_go_name(tag_name);
        out.push_str(&crate::template_env::render(
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
                let json_field_name = apply_serde_rename(
                    &alef_codegen::naming::pascal_to_snake(&variant.name),
                    enum_def.serde_rename_all.as_deref(),
                );

                let doc_lines: Vec<&str> = if !variant.doc.is_empty() {
                    variant.doc.lines().map(|l| l.trim()).collect()
                } else {
                    vec![]
                };

                out.push_str(&crate::template_env::render(
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

    out.push_str(&crate::template_env::render(
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
                let wire_value = enum_variant_wire_value(variant, enum_def);
                out.push_str(&crate::template_env::render(
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

    out.push_str(&crate::template_env::render(
        "tagged_union_marshal_json_footer.jinja",
        context! {
            tag_name => tag_name,
            tag_field_name => &tag_field_name,
        },
    ));
    out.push('\n');

    out.push_str(&crate::template_env::render(
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
                let wire_value = enum_variant_wire_value(variant, enum_def);
                out.push_str(&crate::template_env::render(
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

    out.push_str(&crate::template_env::render(
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

    out.push_str(&crate::template_env::render(
        "untagged_union_marshalers.jinja",
        context! {
            enum_name => go_enum_name,
            variants => variants,
        },
    ));
}

/// Generate a Go unit enum as `type X string` with const block.
fn gen_unit_enum_type(enum_def: &EnumDef) -> String {
    let go_enum_name = go_type_name(&enum_def.name);

    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let const_name = format!("{}{}", go_enum_name, to_go_name(&v.name));
            let wire_value = enum_variant_wire_value(v, enum_def);

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
                wire_value => wire_value,
            }
        })
        .collect();

    crate::template_env::render(
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
fn gen_data_enum_type(enum_def: &EnumDef) -> String {
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
    out.push_str(&crate::template_env::render(
        "variant_comment.jinja",
        minijinja::context! {
            variants => variant_names.join(", "),
        },
    ));
    out.push_str(&format!(
        "// Sealed interface — use one of {}{}, {}{}.\n",
        go_enum_name,
        variant_names.first().unwrap_or(&""),
        go_enum_name,
        variant_names.get(1).unwrap_or(&"")
    ));
    out.push_str(&format!("type {go_enum_name} interface {{\n"));
    out.push_str(&format!("\tis{go_enum_name}()\n"));
    out.push_str("\tType() string\n");
    out.push_str("}\n\n");

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
        out.push_str(&format!("type {variant_struct_name} struct {{\n"));
        if let Some(field) = scalar_tuple_field {
            // Scalar tuple variant: emit `Value <type>` to hold the payload.
            // The `json:"-"` tag prevents encoding/json from picking it up
            // through default field marshalling — we emit a custom
            // MarshalJSON/UnmarshalJSON below that handles the bare scalar.
            out.push_str(&format!("\tValue {} `json:\"-\"`\n", go_type(&field.ty)));
        }
        for field in &variant.fields {
            if is_tuple_field(field) {
                continue;
            }
            let field_go_name = to_go_name(&field.name);
            let field_type = go_type(&field.ty);
            let json_name = apply_serde_rename(&field.name, enum_def.serde_rename_all.as_deref());
            let json_tag = format!("json:\"{}\"", json_name);

            let doc_lines: Vec<&str> = if !field.doc.is_empty() {
                field.doc.lines().map(|l| l.trim()).collect()
            } else {
                vec![]
            };
            out.push_str(&crate::template_env::render(
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
        out.push_str(&format!("func ({variant_struct_name}) is{go_enum_name}() {{}}\n\n"));

        // Implement the Type() method
        let wire_value = enum_variant_wire_value(variant, enum_def);
        out.push_str(&format!(
            "func ({variant_struct_name}) Type() string {{ return \"{}\" }}\n\n",
            wire_value
        ));

        if scalar_tuple_field.is_some() {
            // Scalar tuple variant: marshal as a bare JSON scalar (the inner Value),
            // and unmarshal a bare JSON scalar into Value. Skip the default
            // tag-wrapping aux-struct dance entirely.
            out.push_str(&format!(
                "func (v {variant_struct_name}) MarshalJSON() ([]byte, error) {{\n"
            ));
            out.push_str("\treturn json.Marshal(v.Value)\n");
            out.push_str("}\n\n");

            out.push_str(&format!(
                "func (v *{variant_struct_name}) UnmarshalJSON(data []byte) error {{\n"
            ));
            out.push_str("\treturn json.Unmarshal(data, &v.Value)\n");
            out.push_str("}\n\n");
        } else {
            // Implement MarshalJSON for the concrete struct
            out.push_str(&format!(
                "func (v {variant_struct_name}) MarshalJSON() ([]byte, error) {{\n"
            ));
            out.push_str("\ttype aux struct {\n");
            if let Some(tag_name) = &enum_def.serde_tag {
                let tag_json_name = tag_name.as_str();
                out.push_str(&format!(
                    "\t\t{} string `json:\"{}\"`\n",
                    to_go_name(tag_name),
                    tag_json_name
                ));
            }
            for field in &variant.fields {
                if is_tuple_field(field) {
                    continue;
                }
                let field_go_name = to_go_name(&field.name);
                let field_type = go_type(&field.ty);
                let json_name = apply_serde_rename(&field.name, enum_def.serde_rename_all.as_deref());
                out.push_str(&format!("\t\t{field_go_name} {field_type} `json:\"{json_name}\"`\n"));
            }
            out.push_str("\t}\n");
            out.push_str("\treturn json.Marshal(aux{\n");
            if let Some(tag_name) = &enum_def.serde_tag {
                out.push_str(&format!("\t\t{}: v.Type(),\n", to_go_name(tag_name)));
            }
            for field in &variant.fields {
                if is_tuple_field(field) {
                    continue;
                }
                let field_go_name = to_go_name(&field.name);
                out.push_str(&format!("\t\t{field_go_name}: v.{field_go_name},\n"));
            }
            out.push_str("\t})\n");
            out.push_str("}\n\n");
        }
    }

    // Emit the Unmarshal{EnumName} helper function.
    // Untagged enums use shape-discriminated try-each-variant unmarshalling.
    // Tagged enums (internally tagged with serde_tag) use the wire-struct
    // discriminator field switch.
    out.push_str(&format!(
        "// Unmarshal{go_enum_name} decodes JSON data into the appropriate concrete {go_enum_name} variant.\n"
    ));
    out.push_str(&format!(
        "func Unmarshal{go_enum_name}(data []byte) ({go_enum_name}, error) {{\n"
    ));

    if enum_def.serde_untagged {
        // Untagged path: sniff the first non-whitespace byte to filter candidates,
        // then try each variant in declared order.
        out.push_str("\tif len(data) == 0 {\n");
        out.push_str(&format!(
            "\t\treturn nil, fmt.Errorf(\"cannot unmarshal empty JSON into {go_enum_name}\")\n"
        ));
        out.push_str("\t}\n");
        out.push_str("\tvar firstByte byte\n");
        out.push_str("\tfor _, b := range data {\n");
        out.push_str("\t\tif b != ' ' && b != '\\t' && b != '\\n' && b != '\\r' {\n");
        out.push_str("\t\t\tfirstByte = b\n");
        out.push_str("\t\t\tbreak\n");
        out.push_str("\t\t}\n");
        out.push_str("\t}\n");

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
                out.push_str(&format!("\tif {check} {{\n"));
                out.push_str(&format!("\t\tvar v {variant_struct_name}\n"));
                out.push_str("\t\tif err := json.Unmarshal(data, &v); err == nil {\n");
                out.push_str("\t\t\treturn v, nil\n");
                out.push_str("\t\t}\n");
                out.push_str("\t}\n");
            }
        }

        out.push_str(&format!(
            "\treturn nil, fmt.Errorf(\"unknown {go_enum_name} shape: %s\", string(data))\n"
        ));
    } else {
        // Tagged path (internally-tagged or externally-tagged): read the discriminator
        // field from the wire struct, then switch on its value.
        out.push_str("\tvar wire struct {\n");
        if let Some(tag_name) = &enum_def.serde_tag {
            out.push_str(&format!(
                "\t\t{} string `json:\"{}\"`\n",
                to_go_name(tag_name),
                tag_name
            ));
        }
        out.push_str("\t}\n");
        out.push_str("\tif err := json.Unmarshal(data, &wire); err != nil {\n");
        out.push_str("\t\treturn nil, err\n");
        out.push_str("\t}\n\n");

        let tag_field = enum_def.serde_tag.as_ref().map(|tn| to_go_name(tn));
        let discriminator_field = tag_field.as_deref().unwrap_or("Type");

        out.push_str(&format!("\tswitch wire.{discriminator_field} {{\n"));
        for variant in &enum_def.variants {
            let wire_value = enum_variant_wire_value(variant, enum_def);
            let variant_struct_name = format!("{go_enum_name}{}", to_go_name(&variant.name));
            out.push_str(&format!("\tcase \"{}\":\n", wire_value));
            out.push_str(&format!("\t\tvar v {variant_struct_name}\n"));
            out.push_str("\t\tif err := json.Unmarshal(data, &v); err != nil {\n");
            out.push_str("\t\t\treturn nil, err\n");
            out.push_str("\t\t}\n");
            out.push_str("\t\treturn v, nil\n");
        }
        out.push_str("\t}\n");
        out.push_str(&format!(
            "\treturn nil, fmt.Errorf(\"unknown {go_enum_name} type: %q\", wire.{discriminator_field})\n"
        ));
    }

    out.push_str("}\n");

    out
}

/// Generate a Go opaque handle type wrapping an `unsafe.Pointer`.
///
/// Opaque types are not JSON-serializable — they are raw C pointers passed through
/// the FFI layer. The Go struct holds a pointer and exposes a `Free()` method.
/// Constructors are NOT emitted here — they are generated as free function wrappers
/// from `api.functions` entries that return this opaque type (e.g. `CreateClient`,
/// `CreateClientFromJson`). A zero-argument `New{TypeName}()` calling
/// `C.{prefix}_{type_snake}()` would reference a C function that does not exist in
/// the FFI layer.
pub(super) fn gen_opaque_type(typ: &TypeDef, ffi_prefix: &str) -> String {
    let type_snake = typ.name.to_snake_case();
    let go_name = go_type_name(&typ.name);
    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), typ.name);

    crate::template_env::render(
        "opaque_type.jinja",
        context! {
            go_name => go_name,
            ffi_prefix => ffi_prefix,
            type_snake => type_snake,
            c_type => c_type,
        },
    )
}

/// Generate only the `Free()` method for an opaque handle type whose struct definition
/// was already emitted by `gen_go_error_types`.
///
/// Error types share their name with their corresponding opaque handle (the C layer allocates
/// a `LiterLlmError*` handle that the Go binding holds as an opaque pointer). However the Go
/// error struct uses `Code`/`Message` string fields rather than a raw `ptr unsafe.Pointer`, so
/// we cannot generate the normal `Free()` using `h.ptr`. Instead we emit an unexported stub
/// that references the C symbols to keep them from being pruned, but does nothing at runtime —
/// Go error values are not heap-allocated C objects from the binding's perspective.
pub(super) fn gen_opaque_type_free_only(typ: &TypeDef, _ffi_prefix: &str) -> String {
    // Nothing to emit — the structured error type already has its Error() method and
    // the C-level free function is invoked transparently inside the FFI layer.
    // Returning an empty string avoids a duplicate struct definition and a broken Free().
    let _ = typ;
    String::new()
}

/// Generate a Go struct type definition with json tags for marshaling.
/// Accepts enum_names (unit enums) and data_enum_names (sealed-interface enums).
/// If any field has a data_enum type, emits custom UnmarshalJSON to dispatch to UnmarshalX().
pub(super) fn gen_struct_type(
    typ: &TypeDef,
    enum_names: &std::collections::HashSet<&str>,
    data_enum_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(1024);

    let go_name = go_type_name(&typ.name);
    emit_type_doc(&mut out, &go_name, &typ.doc, "is a type.");
    out.push_str(&crate::template_env::render(
        "struct_type_decl.jinja",
        minijinja::context! {
            name => &go_name,
        },
    ));

    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        // Special handling for Visitor field: use Visitor interface, not a handle type,
        // and mark as json:"-" since it's not serializable
        let is_visitor_field =
            field.name == "visitor" && matches!(&field.ty, TypeRef::Named(n) if n.contains("Visitor"));

        if is_visitor_field {
            let doc_lines: Vec<&str> = if !field.doc.is_empty() {
                field.doc.lines().map(|l| l.trim()).collect()
            } else {
                vec![]
            };
            if !doc_lines.is_empty() {
                out.push_str(&crate::template_env::render(
                    "visitor_field_doc.jinja",
                    minijinja::context! {
                        doc_lines => &doc_lines,
                    },
                ));
            }
            out.push_str(&crate::template_env::render(
                "visitor_field.jinja",
                minijinja::context! {
                    field_name => to_go_name(&field.name),
                },
            ));
            out.push('\n');
            continue;
        }

        // A non-optional field in a defaulted struct may still need pointer+omitempty when
        // the Go zero value differs from the Rust Default value (e.g., Duration, bool true, int != 0).
        let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);

        // Named types that map to Go string enums must also use omitempty (without pointer),
        // because the Go zero value "" is never a valid Rust enum variant. Without omitempty,
        // marshaling an empty struct sends `"field": ""` which fails Rust serde deserialization.
        let is_named_enum = !field.optional
            && !use_default_pointer
            && typ.has_default
            && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));

        // Sealed-interface enums are already nullable in Go (interface zero value is nil) —
        // they must never be wrapped in a pointer. `*AuthConfig` is "pointer to interface",
        // not "interface", and the two are not assignable. Emit the bare interface name
        // for both optional and non-optional positions.
        let is_sealed_interface = matches!(&field.ty, TypeRef::Named(n) if data_enum_names.contains(n.as_str()));

        let field_type = if is_sealed_interface {
            go_type(&field.ty)
        } else if field.optional {
            go_optional_type(&field.ty)
        } else if use_default_pointer {
            // Emit as pointer so that an unset field serializes as absent (omitempty),
            // letting Rust serde fill in the real default instead of seeing a zero value.
            go_optional_type(&field.ty)
        } else {
            go_type(&field.ty)
        };

        // Determine json tag - apply serde rename_all strategy.
        // Use omitempty for optional fields, slice/map types (nil slices serialize to null
        // in Go, which breaks Rust serde deserialization expecting an array), fields
        // where the Go zero value differs from the Rust Default value, and string enum
        // fields where "" is never a valid Rust enum variant.
        // Per-field `#[serde(rename = "...")]` wins over `rename_all`.
        let json_name = field
            .serde_rename
            .clone()
            .unwrap_or_else(|| apply_serde_rename(&field.name, typ.serde_rename_all.as_deref()));
        let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
        let json_tag = if field.optional || is_collection || use_default_pointer || is_named_enum {
            format!("json:\"{},omitempty\"", json_name)
        } else {
            format!("json:\"{}\"", json_name)
        };

        let doc_lines: Vec<&str> = if !field.doc.is_empty() {
            field.doc.lines().map(|l| l.trim()).collect()
        } else {
            vec![]
        };
        out.push_str(&crate::template_env::render(
            "struct_field.jinja",
            minijinja::context! {
                doc_lines => doc_lines,
                field_name => to_go_name(&field.name),
                field_type => &field_type,
                json_tag => &json_tag,
            },
        ));
    }

    out.push_str(&crate::template_env::render(
        "struct_type_end.jinja",
        minijinja::Value::default(),
    ));

    // If any field is a `[]byte` (Vec<u8>), emit custom MarshalJSON so the bytes
    // serialize as a JSON array of integers — matching what Rust's serde
    // `Vec<u8>` deserializer expects. Go's default `json.Marshal([]byte)` emits
    // base64, which Rust's `Deserialize for Vec<u8>` rejects with
    // `invalid type: string "...", expected a sequence`.
    let bytes_fields: Vec<&alef_core::ir::FieldDef> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| !is_tuple_field(f) && matches!(&f.ty, TypeRef::Bytes))
        .collect();
    if !bytes_fields.is_empty() {
        out.push('\n');
        out.push_str(&crate::template_env::render(
            "struct_marshal_json_header.jinja",
            context! {
                go_name => &go_name,
            },
        ));
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field =
                field.name == "visitor" && matches!(&field.ty, TypeRef::Named(n) if n.contains("Visitor"));
            if is_visitor_field {
                continue;
            }
            let go_field = to_go_name(&field.name);
            // Per-field `#[serde(rename = "...")]` wins over `rename_all`.
            let json_name = field
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_serde_rename(&field.name, typ.serde_rename_all.as_deref()));
            let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);
            let is_named_enum = !field.optional
                && !use_default_pointer
                && typ.has_default
                && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
            let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
            let json_tag = if field.optional || is_collection || use_default_pointer || is_named_enum {
                format!("json:\"{},omitempty\"", json_name)
            } else {
                format!("json:\"{}\"", json_name)
            };
            let go_field_type: String = if matches!(&field.ty, TypeRef::Bytes) {
                "[]int".to_string()
            } else if field.optional || use_default_pointer {
                go_optional_type(&field.ty).to_string()
            } else {
                go_type(&field.ty).to_string()
            };
            out.push_str(&crate::template_env::render(
                "struct_marshal_aux_field.jinja",
                context! {
                    field_name => &go_field,
                    field_type => &go_field_type,
                    json_tag => &json_tag,
                },
            ));
        }
        out.push_str(&crate::template_env::render(
            "struct_marshal_aux_init.jinja",
            minijinja::Value::default(),
        ));
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field =
                field.name == "visitor" && matches!(&field.ty, TypeRef::Named(n) if n.contains("Visitor"));
            if is_visitor_field {
                continue;
            }
            let go_field = to_go_name(&field.name);
            if matches!(&field.ty, TypeRef::Bytes) {
                let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);
                let is_pointer = field.optional || use_default_pointer;
                if is_pointer {
                    // Optional `*[]byte` field: only encode when non-nil.
                    out.push_str(&crate::template_env::render(
                        "struct_marshal_bytes_field_pointer.jinja",
                        context! {
                            go_field => &go_field,
                        },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "struct_marshal_bytes_field_nonpointer.jinja",
                        context! {
                            go_field => &go_field,
                        },
                    ));
                }
            } else {
                out.push_str(&crate::template_env::render(
                    "struct_marshal_regular_field.jinja",
                    context! {
                        go_field => &go_field,
                    },
                ));
            }
        }
        out.push_str(&crate::template_env::render(
            "struct_marshal_json_footer.jinja",
            minijinja::Value::default(),
        ));
    }

    // Collect fields whose type is a sealed-interface data enum (either direct or optional).
    // These cannot be unmarshalled by Go's default json.Unmarshal (interface types are opaque),
    // so we emit a custom UnmarshalJSON that reads every data-enum field as json.RawMessage
    // first, then dispatches via the generated UnmarshalX() helper.
    struct DataEnumField {
        go_name: String,
        enum_go_name: String,
        is_optional: bool,
        is_slice: bool,
    }
    let data_enum_fields: Vec<DataEnumField> = binding_fields(&typ.fields)
        .filter(|f| !is_tuple_field(f))
        .filter(|f| f.name != "visitor" || !matches!(&f.ty, TypeRef::Named(n) if n.contains("Visitor")))
        .filter_map(|f| {
            // Determine the inner Named type name, and whether the field is optional
            // and/or a slice. Slices of data enums (e.g. `Vec<RerankDocument>` where
            // `RerankDocument` is `#[serde(untagged)]`) need per-element dispatch
            // through the `Unmarshal<Enum>` helper — Go's default unmarshal of a
            // JSON array into `[]<sealed-interface>` fails because Go interfaces
            // are opaque to encoding/json.
            let (enum_name_str, is_optional, is_slice) = match &f.ty {
                TypeRef::Named(n) if data_enum_names.contains(n.as_str()) => (n.as_str(), false, false),
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(n) if data_enum_names.contains(n.as_str()) => (n.as_str(), true, false),
                    _ => return None,
                },
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) if data_enum_names.contains(n.as_str()) => (n.as_str(), false, true),
                    _ => return None,
                },
                _ => return None,
            };
            Some(DataEnumField {
                go_name: to_go_name(&f.name),
                enum_go_name: go_type_name(enum_name_str),
                is_optional,
                is_slice,
            })
        })
        .collect();

    if !data_enum_fields.is_empty() {
        out.push('\n');
        // Emit: func (s *StructName) UnmarshalJSON(data []byte) error {
        out.push_str(&format!("func (s *{go_name}) UnmarshalJSON(data []byte) error {{\n"));

        // Emit the anonymous helper struct with all fields,
        // replacing data-enum fields with json.RawMessage.
        out.push_str("\tvar raw struct {\n");
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field =
                field.name == "visitor" && matches!(&field.ty, TypeRef::Named(n) if n.contains("Visitor"));
            if is_visitor_field {
                continue;
            }
            let go_field_name = to_go_name(&field.name);
            let json_name = field
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_serde_rename(&field.name, typ.serde_rename_all.as_deref()));
            // Check if this field is a data enum field (direct, optional, or slice).
            let data_enum_def = data_enum_fields.iter().find(|def| def.go_name == go_field_name);
            if let Some(def) = data_enum_def {
                // For slice fields we keep the array shape so we can iterate
                // per-element; scalar/optional fields collapse to a single
                // json.RawMessage. Both use omitempty — nil-length checks
                // guard the decode loop below.
                let raw_type = if def.is_slice {
                    "[]json.RawMessage"
                } else {
                    "json.RawMessage"
                };
                out.push_str(&format!(
                    "\t\t{go_field_name} {raw_type} `json:\"{json_name},omitempty\"`\n"
                ));
            } else {
                // Use the normal field type and tag.
                let use_default_pointer = !field.optional && typ.has_default && needs_omitempty_pointer(field);
                let is_named_enum = !field.optional
                    && !use_default_pointer
                    && typ.has_default
                    && matches!(&field.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()));
                let is_collection = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
                let field_type = if field.optional || use_default_pointer {
                    go_optional_type(&field.ty)
                } else {
                    go_type(&field.ty)
                };
                let json_tag = if field.optional || is_collection || use_default_pointer || is_named_enum {
                    format!("json:\"{json_name},omitempty\"")
                } else {
                    format!("json:\"{json_name}\"")
                };
                out.push_str(&format!("\t\t{go_field_name} {field_type} `{json_tag}`\n"));
            }
        }
        out.push_str("\t}\n");
        out.push_str("\tif err := json.Unmarshal(data, &raw); err != nil {\n");
        out.push_str("\t\treturn err\n");
        out.push_str("\t}\n");

        // Copy all non-data-enum fields.
        for field in binding_fields(&typ.fields) {
            if is_tuple_field(field) {
                continue;
            }
            let is_visitor_field =
                field.name == "visitor" && matches!(&field.ty, TypeRef::Named(n) if n.contains("Visitor"));
            if is_visitor_field {
                continue;
            }
            let go_field_name = to_go_name(&field.name);
            let is_data_enum = data_enum_fields.iter().any(|def| def.go_name == go_field_name);
            if !is_data_enum {
                out.push_str(&format!("\ts.{go_field_name} = raw.{go_field_name}\n"));
            }
        }

        // Decode each data-enum field via its UnmarshalX helper.
        for def in &data_enum_fields {
            let unmarshal_fn = format!("Unmarshal{}", def.enum_go_name);
            if def.is_slice {
                // Slice field: iterate over the JSON array and dispatch per element
                // via the generated UnmarshalX helper. The struct field type is
                // `[]<sealed-interface>`, which encoding/json cannot populate
                // directly from a heterogeneous JSON array (interfaces are opaque).
                out.push_str(&format!("\tif len(raw.{}) > 0 {{\n", def.go_name));
                out.push_str(&format!(
                    "\t\ts.{} = make([]{}, 0, len(raw.{}))\n",
                    def.go_name, def.enum_go_name, def.go_name
                ));
                out.push_str(&format!("\t\tfor _, item := range raw.{} {{\n", def.go_name));
                out.push_str(&format!("\t\t\tv, err := {unmarshal_fn}(item)\n"));
                out.push_str("\t\t\tif err != nil {\n");
                out.push_str("\t\t\t\treturn err\n");
                out.push_str("\t\t\t}\n");
                out.push_str(&format!("\t\t\ts.{} = append(s.{}, v)\n", def.go_name, def.go_name));
                out.push_str("\t\t}\n");
                out.push_str("\t}\n");
            } else if def.is_optional {
                // Optional field: only decode when the raw bytes are non-nil/non-empty.
                // The struct field type is the bare sealed-interface (no `*`), since
                // Go interfaces are already nullable — so assign `v` directly.
                out.push_str(&format!("\tif len(raw.{}) > 0 {{\n", def.go_name));
                out.push_str(&format!("\t\tv, err := {unmarshal_fn}(raw.{})\n", def.go_name));
                out.push_str("\t\tif err != nil {\n");
                out.push_str("\t\t\treturn err\n");
                out.push_str("\t\t}\n");
                out.push_str(&format!("\t\ts.{} = v\n", def.go_name));
                out.push_str("\t}\n");
            } else {
                // Required field: always decode (raw is guaranteed non-nil by the struct unmarshal above).
                out.push_str(&format!("\tif len(raw.{}) > 0 {{\n", def.go_name));
                out.push_str(&format!("\t\tv, err := {unmarshal_fn}(raw.{})\n", def.go_name));
                out.push_str("\t\tif err != nil {\n");
                out.push_str("\t\t\treturn err\n");
                out.push_str("\t\t}\n");
                out.push_str(&format!("\t\ts.{} = v\n", def.go_name));
                out.push_str("\t}\n");
            }
        }

        out.push_str("\treturn nil\n");
        out.push_str("}\n");
    }

    out
}

/// Return the CGo type name for a primitive type (e.g. `PrimitiveType::U64` → `"C.uint64_t"`).
///
/// CGo treats Go native types (`uint64`, `uint32`, …) and the corresponding C typedefs
/// (`C.uint64_t`, `C.uint32_t`, …) as distinct and will not implicitly convert between them
/// when passing values to C functions. Declaring optional-primitive temporaries with the CGo
/// type avoids an explicit cast at every call-site.
pub(super) fn cgo_type_for_primitive(prim: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType;
    match prim {
        PrimitiveType::U8 => "C.uint8_t",
        PrimitiveType::U16 => "C.uint16_t",
        PrimitiveType::U32 => "C.uint32_t",
        PrimitiveType::U64 => "C.uint64_t",
        PrimitiveType::Usize => "C.size_t",
        PrimitiveType::I8 => "C.int8_t",
        PrimitiveType::I16 => "C.int16_t",
        PrimitiveType::I32 => "C.int32_t",
        PrimitiveType::I64 => "C.int64_t",
        PrimitiveType::Isize => "C.ptrdiff_t",
        PrimitiveType::F32 => "C.float",
        PrimitiveType::F64 => "C.double",
        PrimitiveType::Bool => "C.int32_t",
    }
}

/// Return the Go expression for the maximum value of a primitive type, used as a sentinel
/// to signal "None" to FFI functions that use max-value sentinels for optional primitives.
pub(super) fn primitive_max_sentinel(prim: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType;
    match prim {
        PrimitiveType::U8 => "^uint8(0)",
        PrimitiveType::U16 => "^uint16(0)",
        PrimitiveType::U32 => "^uint32(0)",
        PrimitiveType::U64 => "^uint64(0)",
        PrimitiveType::Usize => "^uint(0)",
        PrimitiveType::I8 => "int8(127)",
        PrimitiveType::I16 => "int16(32767)",
        PrimitiveType::I32 => "int32(2147483647)",
        PrimitiveType::I64 => "int64(9223372036854775807)",
        PrimitiveType::Isize => "int(^uint(0) >> 1)",
        PrimitiveType::F32 => "float32(0)",
        PrimitiveType::F64 => "float64(0)",
        PrimitiveType::Bool => "false",
    }
}

/// Get a type name suitable for a function suffix (e.g., unmarshalFoo).
pub(super) fn type_name(ty: &TypeRef) -> String {
    match ty {
        // IR Named types are already PascalCase from Rust source. Avoid
        // ToPascalCase to preserve all-caps acronyms (GraphQL, JSON, HTTP).
        TypeRef::Named(n) => n.clone(),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Bytes".to_string(),
        TypeRef::Optional(inner) => type_name(inner),
        TypeRef::Vec(inner) => format!("List{}", type_name(inner)),
        TypeRef::Map(_, v) => format!("Map{}", type_name(v)),
        TypeRef::Json => "JSON".to_string(),
        TypeRef::Path => "Path".to_string(),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Duration => "U64".to_string(),
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "Bool".to_string(),
            alef_core::ir::PrimitiveType::U8 => "U8".to_string(),
            alef_core::ir::PrimitiveType::U16 => "U16".to_string(),
            alef_core::ir::PrimitiveType::U32 => "U32".to_string(),
            alef_core::ir::PrimitiveType::U64 => "U64".to_string(),
            alef_core::ir::PrimitiveType::I8 => "I8".to_string(),
            alef_core::ir::PrimitiveType::I16 => "I16".to_string(),
            alef_core::ir::PrimitiveType::I32 => "I32".to_string(),
            alef_core::ir::PrimitiveType::I64 => "I64".to_string(),
            alef_core::ir::PrimitiveType::F32 => "F32".to_string(),
            alef_core::ir::PrimitiveType::F64 => "F64".to_string(),
            alef_core::ir::PrimitiveType::Usize => "Usize".to_string(),
            alef_core::ir::PrimitiveType::Isize => "Isize".to_string(),
        },
    }
}

/// Generate a Go expression that converts a C return value (`ptr`) to the correct Go type.
///
/// For primitives like Bool, this produces inline conversion (e.g., `func() *bool { v := ptr != 0; return &v }()`).
/// For Named types (opaque handles), this uses `_to_json` to serialize then `json.Unmarshal` in Go.
/// For strings, this calls `C.GoString`.
/// The `ffi_prefix` is used to construct C type names for Named types.
pub(super) fn go_return_expr(
    ty: &TypeRef,
    var_name: &str,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    value_only_types: &std::collections::HashSet<String>,
) -> String {
    go_return_expr_inner(ty, var_name, ffi_prefix, opaque_names, value_only_types)
}

fn go_return_expr_inner(
    ty: &TypeRef,
    var_name: &str,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    // value_only_types was previously used to skip _to_json for all-primitive structs.
    // The FFI backend now emits _to_json for all non-opaque non-Update types, so this
    // set is no longer consulted. The parameter is kept for API compatibility.
    _value_only_types: &std::collections::HashSet<String>,
) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            alef_core::ir::PrimitiveType::Bool => {
                format!("func() *bool {{ v := {} != 0; return &v }}()", var_name)
            }
            _ => {
                // Numeric primitives: cast and take address
                let go_ty = go_type(ty);
                format!("func() *{go_ty} {{ v := {go_ty}({var_name}); return &v }}()")
            }
        },
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // Opaque types: wrap the raw C pointer in the Go handle struct.
                // IR name is already PascalCase from Rust; preserve all-caps
                // acronyms (GraphQLError stays GraphQLError, not GraphQlError).
                format!(
                    "&{go_type}{{ptr: unsafe.Pointer({var_name})}}",
                    go_type = name,
                    var_name = var_name,
                )
            } else {
                // Full conversion: serialize C handle to JSON, then unmarshal into Go struct.
                // The FFI backend emits _to_json for all non-opaque types (including those whose
                // fields are all primitives/strings), so we always use the JSON path here.
                let type_snake = name.to_snake_case();
                format!(
                    "func() *{go_type} {{\n\
                     \tjsonPtr := C.{ffi_prefix}_{type_snake}_to_json({var_name})\n\
                     \tif jsonPtr == nil {{ return nil }}\n\
                     \tdefer C.{ffi_prefix}_free_string(jsonPtr)\n\
                     \tvar result {go_type}\n\
                     \tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {{ return nil }}\n\
                     \treturn &result\n\
                     }}()",
                    go_type = name,
                    ffi_prefix = ffi_prefix,
                    type_snake = type_snake,
                    var_name = var_name,
                )
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            format!(
                "func() *string {{ if {var} == nil {{ return nil }}; v := C.GoString({var}); return &v }}()",
                var = var_name
            )
        }
        TypeRef::Json => {
            format!(
                "func() *json.RawMessage {{ if {var_name} == nil {{ return nil }}; v := json.RawMessage(C.GoString({var_name})); return &v }}()"
            )
        }
        TypeRef::Bytes => {
            format!("unmarshalBytes({})", var_name)
        }
        TypeRef::Optional(inner) => go_return_expr_inner(inner, var_name, ffi_prefix, opaque_names, _value_only_types),
        TypeRef::Vec(inner) => {
            // Vec types are returned as JSON strings from FFI. Deserialize inline.
            // Return []T (not *[]T) — slices are already reference types in Go.
            let go_elem = go_type(inner);
            format!(
                "func() []{go_elem} {{\n\
                 \tif {var_name} == nil {{ return nil }}\n\
                 \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                 \tvar result []{go_elem}\n\
                 \tif err := json.Unmarshal([]byte(C.GoString({var_name})), &result); err != nil {{ return nil }}\n\
                 \treturn result\n\
                 }}()",
                go_elem = go_elem,
                var_name = var_name,
                ffi_prefix = ffi_prefix,
            )
        }
        TypeRef::Map(k, v) => {
            // Map types are returned as JSON strings from FFI. Deserialize inline.
            let go_k = go_type(k);
            let go_v = go_type(v);
            format!(
                "func() *map[{go_k}]{go_v} {{\n\
                 \tif {var_name} == nil {{ return nil }}\n\
                 \tdefer C.{ffi_prefix}_free_string({var_name})\n\
                 \tvar result map[{go_k}]{go_v}\n\
                 \tif err := json.Unmarshal([]byte(C.GoString({var_name})), &result); err != nil {{ return nil }}\n\
                 \treturn &result\n\
                 }}()",
                go_k = go_k,
                go_v = go_v,
                var_name = var_name,
                ffi_prefix = ffi_prefix,
            )
        }
        _ => format!("unmarshal{}({})", type_name(ty), var_name),
    }
}

/// Generate functional options pattern for Go config types with defaults.
/// Produces ConfigOption type and WithFieldName constructors.
pub(super) fn gen_config_options(
    typ: &TypeDef,
    enum_names: &std::collections::HashSet<&str>,
    passthrough_enum_names: &std::collections::HashSet<&str>,
    data_enum_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    // ConfigOption type definition
    let go_name = go_type_name(&typ.name);
    out.push_str(&crate::template_env::render(
        "config_option_type_header.jinja",
        context! {
            go_name => &go_name,
        },
    ));
    out.push('\n');

    // Generate WithFieldName constructors for each field
    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);

        // Match the struct's special-cased visitor field (typed as the user-facing
        // `Visitor` interface, not the opaque `VisitorHandle`). The With option must
        // accept Visitor too — passing a VisitorHandle and assigning &v yielded a
        // *VisitorHandle, which doesn't satisfy the Visitor interface and broke the
        // Go build whenever the visitor pattern was active.
        let is_visitor_field =
            field.name == "visitor" && matches!(&field.ty, TypeRef::Named(n) if n.contains("Visitor"));

        // For the function parameter, always accept the direct type (not wrapped in optional)
        let param_type = if is_visitor_field {
            std::borrow::Cow::Borrowed("Visitor")
        } else {
            go_type(&field.ty)
        };

        out.push_str(&crate::template_env::render(
            "config_with_option_comment.jinja",
            context! {
                go_name => &go_name,
                field_go_name => &field_go_name,
                field_name => &field.name,
            },
        ));
        // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults) both
        // store pointer types in the struct, so we must take the address of v when assigning.
        // Exception: slice (Vec) and map types are reference types in Go — go_optional_type
        // returns []T and map[K]V (not *[]T / *map[K]V), so no address-of is needed.
        // Sealed-interface (data enum) fields are also already-nullable interface values; their
        // struct field is `T` (not `*T`), so the assignment must not take the address.
        let is_slice_or_map = matches!(&field.ty, TypeRef::Vec(_) | TypeRef::Map(_, _));
        let is_sealed_interface = matches!(&field.ty, TypeRef::Named(n) if data_enum_names.contains(n.as_str()));
        let use_ptr = !is_visitor_field
            && (field.optional || needs_omitempty_pointer(field))
            && !is_slice_or_map
            && !is_sealed_interface;
        let assign_val = if use_ptr { "&v" } else { "v" };
        out.push_str(&crate::template_env::render(
            "config_with_option_signature.jinja",
            context! {
                go_name => &go_name,
                field_go_name => &field_go_name,
                param_type => param_type.as_ref(),
                assign_val => assign_val,
            },
        ));
        out.push('\n');
    }

    // Generate NewConfig constructor
    out.push_str(&crate::template_env::render(
        "config_new_constructor_header.jinja",
        context! {
            go_name => &go_name,
        },
    ));

    // Set default values for fields
    for field in binding_fields(&typ.fields) {
        if is_tuple_field(field) {
            continue;
        }

        let field_go_name = to_go_name(&field.name);
        let default_val = if field.optional || needs_omitempty_pointer(field) {
            // Optional fields and fields that use pointer+omitempty (to preserve Rust defaults)
            // are pointer types. Set to nil so they serialize as absent, letting Rust serde
            // fill in the real default instead of seeing a Go zero value.
            "nil".to_string()
        } else {
            let mut val = alef_codegen::config_gen::default_value_for_field(field, "go");
            // Passthrough json.RawMessage-backed enum: zero value is `nil` (a nil
            // []byte slice). Override unconditionally — config_gen would otherwise
            // return `""` for `String` defaults baked into the IR for these types.
            if let TypeRef::Named(name) = &field.ty {
                if passthrough_enum_names.contains(name.as_str()) {
                    val = "nil".to_string();
                }
            }
            // config_gen returns "nil" for Named types with Empty default, but in Go
            // non-optional Named types are value types. Fix up based on whether the
            // Named type is a string-based enum or a struct.
            if val == "nil" {
                if let TypeRef::Named(name) = &field.ty {
                    if passthrough_enum_names.contains(name.as_str()) {
                        // already handled above; keep nil
                    } else if enum_names.contains(name.as_str()) {
                        // String-typed enum — zero value is empty string
                        val = "\"\"".to_string();
                    } else if data_enum_names.contains(name.as_str()) {
                        // Sealed-interface (data enum) — zero value is nil interface.
                        // Composite literal `T{}` is invalid for interface types.
                        val = "nil".to_string();
                    } else {
                        // Struct — zero value is TypeName{}
                        val = format!("{}{{}}", go_type_name(name));
                    }
                }
            }
            val
        };
        out.push_str(&crate::template_env::render(
            "config_default_field.jinja",
            context! {
                field_go_name => &field_go_name,
                default_val => &default_val,
            },
        ));
    }

    out.push_str(&crate::template_env::render(
        "config_new_constructor_footer.jinja",
        minijinja::Value::default(),
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

    fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    #[test]
    fn test_is_tuple_field_detects_positional_names() {
        let positional = simple_field("_0", TypeRef::String);
        assert!(is_tuple_field(&positional));
        let named = simple_field("value", TypeRef::String);
        assert!(!is_tuple_field(&named));
    }

    #[test]
    fn test_apply_serde_rename_camel_case() {
        assert_eq!(apply_serde_rename("my_field", Some("camelCase")), "myField");
        assert_eq!(apply_serde_rename("my_field", None), "my_field");
    }

    #[test]
    fn test_gen_unit_enum_type_produces_type_string_and_const_block() {
        let enum_def = EnumDef {
            name: "Status".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            variants: vec![EnumVariant {
                name: "Active".to_string(),
                doc: String::new(),
                fields: vec![],
                is_default: false,
                serde_rename: None,
                is_tuple: false,
            }],
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let out = gen_unit_enum_type(&enum_def);
        assert!(out.contains("type Status string"));
        assert!(out.contains("const ("));
        assert!(out.contains("StatusActive"));
    }

    #[test]
    fn test_gen_struct_type_emits_json_tags() {
        let typ = TypeDef {
            name: "MyConfig".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            fields: vec![simple_field("timeout", TypeRef::Primitive(PrimitiveType::U64))],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let out = gen_struct_type(
            &typ,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(out.contains("type MyConfig struct"));
        assert!(out.contains("json:\"timeout\""));
    }

    #[test]
    fn test_gen_data_enum_sealed_interface() {
        // Test tagged-data enum (named fields): emits sealed interface pattern
        let enum_def = EnumDef {
            name: "AuthConfig".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: "Authentication configuration.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            variants: vec![
                EnumVariant {
                    name: "Basic".to_string(),
                    doc: "Basic auth variant.".to_string(),
                    fields: vec![
                        simple_field("username", TypeRef::String),
                        simple_field("password", TypeRef::String),
                    ],
                    is_default: false,
                    serde_rename: Some("basic".to_string()),
                    is_tuple: false,
                },
                EnumVariant {
                    name: "Bearer".to_string(),
                    doc: "Bearer token variant.".to_string(),
                    fields: vec![simple_field("token", TypeRef::String)],
                    is_default: false,
                    serde_rename: Some("bearer".to_string()),
                    is_tuple: false,
                },
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let out = gen_data_enum_type(&enum_def);
        // Should emit sealed interface
        assert!(out.contains("type AuthConfig interface"));
        assert!(out.contains("isAuthConfig()"));
        assert!(out.contains("Type() string"));
        // Should emit concrete structs per variant, not flat struct with all nullables
        assert!(out.contains("type AuthConfigBasic struct"));
        assert!(out.contains("type AuthConfigBearer struct"));
        // Basic variant should have username/password non-null fields
        assert!(out.contains("Username string"));
        assert!(out.contains("Password string"));
        // Bearer variant should have token field
        assert!(out.contains("Token string"));
        // No nullable fields — each struct has only its own fields
        assert!(!out.contains("*string `json:\"username,omitempty\""));
        // Should emit Unmarshal helper
        assert!(out.contains("func UnmarshalAuthConfig(data []byte)"));
        assert!(out.contains("case \"basic\""));
        assert!(out.contains("case \"bearer\""));
    }

    /// Regression: an `Option<Bytes>` field becomes a non-pointer `[]byte` in the Go
    /// struct (slices are already nullable in Go). The MarshalJSON helper must not
    /// dereference `v.Data` with `*v.Data` — that produced
    /// `invalid operation: cannot indirect v.Data (variable of type []byte)`.
    #[test]
    fn gen_struct_type_marshal_optional_bytes_field_does_not_dereference() {
        let mut data_field = simple_field("data", TypeRef::Bytes);
        data_field.optional = true;
        let typ = TypeDef {
            name: "EmailAttachment".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            fields: vec![data_field],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let out = gen_struct_type(
            &typ,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert!(!out.contains("*v.Data"), "expected no `*v.Data` dereference in:\n{out}");
        assert!(
            out.contains("len(v.Data)") && out.contains("range v.Data"),
            "expected `len(v.Data)` and `range v.Data` (no dereference) in:\n{out}"
        );
    }

    /// Regression: a non-optional field whose type is a sealed-interface (data) enum
    /// must default to `nil` (the interface zero value), NOT `TypeName{}` — composite
    /// literals are not valid for interface types in Go.
    #[test]
    fn gen_config_options_defaults_data_enum_field_to_nil_not_composite_literal() {
        let typ = TypeDef {
            name: "ChunkingConfig".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            fields: vec![simple_field("sizing", TypeRef::Named("ChunkSizing".to_string()))],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let mut data_enum_names = std::collections::HashSet::new();
        data_enum_names.insert("ChunkSizing");
        let out = gen_config_options(
            &typ,
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &data_enum_names,
        );
        // BUG fixed: previously emitted `Sizing: ChunkSizing{}` which is a Go compile
        // error (`invalid composite literal type ChunkSizing` — ChunkSizing is an
        // interface). Verify the constructor now uses the interface zero value `nil`.
        assert!(
            !out.contains("Sizing: ChunkSizing{}") && !out.contains("Sizing:                ChunkSizing{}"),
            "expected no `Sizing: ChunkSizing{{}}` in:\n{out}"
        );
        assert!(
            out.contains("Sizing:") && out.contains("nil"),
            "expected `Sizing: ... nil` default in:\n{out}"
        );
    }
}
