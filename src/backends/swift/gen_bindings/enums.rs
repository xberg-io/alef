use crate::backends::swift::naming::swift_source_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use heck::{AsSnakeCase, ToLowerCamelCase};

/// Emits a custom Codable conformance for a serde-internally-tagged enum.
/// Handles variant-tag decoding/encoding and respects field renames.
pub(super) fn emit_serde_tagged_codable(en: &EnumDef, out: &mut String, mapper: &SwiftMapper) {
    let tag_key = en.serde_tag.as_deref().unwrap_or("type");

    let mut field_keys = std::collections::BTreeSet::new();
    for variant in &en.variants {
        for (idx, field) in variant.fields.iter().enumerate() {
            let swift_name = swift_associated_label(&field.name, idx);
            let rust_name = field.serde_rename.as_deref().unwrap_or(&field.name);
            field_keys.insert((swift_name, rust_name.to_string()));
        }
    }

    let mut coding_key_cases = String::new();
    for (swift_name, rust_name) in field_keys {
        coding_key_cases.push_str(&crate::backends::swift::template_env::render(
            "swift_tagged_coding_key_case.swift.jinja",
            minijinja::context! {
                swift_name => &swift_name,
                rust_name => &rust_name,
                has_custom_wire_name => swift_name != rust_name,
            },
        ));
    }

    let mut decode_cases = String::new();
    for variant in &en.variants {
        let variant_tag = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );

        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());

        if variant.fields.is_empty() {
            decode_cases.push_str(&crate::backends::swift::template_env::render(
                "swift_tagged_decode_unit_case.swift.jinja",
                minijinja::context! {
                    variant_tag => &variant_tag,
                    case_name => &case_name,
                },
            ));
        } else {
            let mut field_decoders = Vec::with_capacity(variant.fields.len());
            for (i, field) in variant.fields.iter().enumerate() {
                let label = swift_associated_label(&field.name, i);
                let already_optional = matches!(&field.ty, TypeRef::Optional(_));
                let is_optional = field.optional || already_optional;
                let ty = mapper.map_type(&field.ty);

                let decode_method = if is_optional { "decodeIfPresent" } else { "decode" };
                field_decoders.push(format!(
                    "{label}: try container.{decode_method}({ty}.self, forKey: .{label})"
                ));
            }
            decode_cases.push_str(&crate::backends::swift::template_env::render(
                "swift_tagged_decode_payload_case.swift.jinja",
                minijinja::context! {
                    variant_tag => &variant_tag,
                    case_name => &case_name,
                    field_decoders => field_decoders.join(", "),
                },
            ));
        }
    }

    let mut encode_cases = String::new();
    for variant in &en.variants {
        let variant_tag = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );

        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());

        if variant.fields.is_empty() {
            encode_cases.push_str(&crate::backends::swift::template_env::render(
                "swift_tagged_encode_unit_case.swift.jinja",
                minijinja::context! {
                    variant_tag => &variant_tag,
                    tag_key => tag_key,
                    case_name => &case_name,
                },
            ));
        } else {
            let mut bindings = Vec::new();
            for (i, field) in variant.fields.iter().enumerate() {
                let label = swift_associated_label(&field.name, i);
                bindings.push(format!("let {}", label));
            }

            let mut field_encoders = String::new();
            for (i, field) in variant.fields.iter().enumerate() {
                let label = swift_associated_label(&field.name, i);
                let already_optional = matches!(&field.ty, TypeRef::Optional(_));
                let is_optional = field.optional || already_optional;
                let encode_method = if is_optional { "encodeIfPresent" } else { "encode" };

                field_encoders.push_str(&crate::backends::swift::template_env::render(
                    "swift_tagged_encode_field.swift.jinja",
                    minijinja::context! {
                        encode_method => encode_method,
                        label => &label,
                    },
                ));
            }
            encode_cases.push_str(&crate::backends::swift::template_env::render(
                "swift_tagged_encode_payload_case.swift.jinja",
                minijinja::context! {
                    variant_tag => &variant_tag,
                    tag_key => tag_key,
                    case_name => &case_name,
                    bindings => bindings.join(", "),
                    field_encoders => field_encoders,
                },
            ));
        }
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_tagged_codable.swift.jinja",
        minijinja::context! {
            enum_name => &en.name,
            tag_key => tag_key,
            coding_key_cases => coding_key_cases,
            decode_cases => decode_cases,
            encode_cases => encode_cases,
        },
    ));
}

/// Emits a custom Codable conformance for a `#[serde(untagged)]` enum so
/// Swift round-trips the same wire shape as serde — i.e. a bare value
/// (`"foo"`, `[1,2,3]`, `{...}`) rather than the externally-tagged
/// `{"variant": payload}` shape that `Codable`'s auto-synthesised
/// implementation produces for enum data variants.
///
/// Each variant must have exactly one positional payload (the `field0`
/// label emitted by `swift_associated_label`). The init tries each variant
/// in declaration order via `singleValueContainer().decode(T.self)` until
/// one succeeds, mirroring serde's untagged deserialiser; if none match,
/// throws `DecodingError.dataCorruptedError`. Encoder writes the payload
/// value directly into a single-value container.
pub(super) fn emit_serde_untagged_codable(en: &EnumDef, out: &mut String, mapper: &SwiftMapper) {
    let mut decode_attempts = String::new();
    for variant in &en.variants {
        if variant.fields.len() != 1 {
            continue;
        }
        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
        let payload_ty = mapper.map_type(&variant.fields[0].ty);
        let label = swift_associated_label(&variant.fields[0].name, 0);
        decode_attempts.push_str(&crate::backends::swift::template_env::render(
            "swift_untagged_decode_attempt.swift.jinja",
            minijinja::context! {
                payload_type => &payload_ty,
                case_name => &case_name,
                label => &label,
            },
        ));
    }

    let mut encode_cases = String::new();
    for variant in &en.variants {
        if variant.fields.len() != 1 {
            continue;
        }
        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
        let label = swift_associated_label(&variant.fields[0].name, 0);
        encode_cases.push_str(&crate::backends::swift::template_env::render(
            "swift_untagged_encode_case.swift.jinja",
            minijinja::context! {
                case_name => &case_name,
                label => &label,
            },
        ));
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_untagged_codable.swift.jinja",
        minijinja::context! {
            decode_attempts => decode_attempts,
            encode_cases => encode_cases,
        },
    ));
}

/// Emits a Swift enum or typealias for the given `EnumDef`.
/// Non-Codable enums (`has_serde: false`) become typealiases to RustBridge.X.
/// Codable enums (`has_serde: true`) are emitted as native Swift enums.
///
/// When `needs_codable` is `true` (the enum is a field type of a streaming item
/// Codable struct), all-unit enums receive `: String, Codable` conformance so
/// `JSONDecoder` can decode them. The raw value for each case is the `serde`
/// serialized form derived from `serde_rename_all` (or the camelCase variant
/// name when no rename strategy is set).
pub(super) fn emit_enum(
    en: &EnumDef,
    out: &mut String,
    mapper: &SwiftMapper,
    known_dto_names: &std::collections::HashSet<String>,
    text_types: &[String],
) {
    super::client::emit_doc_comment(&en.doc, "", out);

    if !en.has_serde {
        out.push_str(&crate::backends::swift::template_env::render(
            "typealias.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        return;
    }

    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());

    if all_unit {
        let _ = mapper;
        let mut cases = String::new();
        for variant in &en.variants {
            super::client::emit_doc_comment(&variant.doc, "    ", &mut cases);
            let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
            let raw_value = unit_enum_wire_value(variant, en.serde_rename_all.as_deref());
            if raw_value == case_name.trim_matches('`') {
                cases.push_str(&crate::backends::swift::template_env::render(
                    "enum_case_unit.jinja",
                    minijinja::context! {
                        case_name => &case_name,
                    },
                ));
            } else {
                cases.push_str(&crate::backends::swift::template_env::render(
                    "enum_case_raw_value.swift.jinja",
                    minijinja::context! {
                        case_name => &case_name,
                        raw_value => &raw_value,
                    },
                ));
            }
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_raw_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                cases => cases,
            },
        ));
        emit_enum_into_rust_extension(&en.name, out);
        return;
    }
    if !all_variants_codable_safe(en, known_dto_names) {
        out.push_str(&crate::backends::swift::template_env::render(
            "typealias.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        return;
    }

    let has_serde_tag = en.serde_tag.is_some() && !en.serde_untagged;
    let is_serde_untagged = en.serde_untagged
        && en.variants.iter().any(|v| !v.fields.is_empty())
        && en
            .variants
            .iter()
            .filter(|v| !v.fields.is_empty())
            .all(|v| v.fields.len() == 1);

    if has_serde_tag {
        let mut variants = String::new();
        for variant in &en.variants {
            emit_variant_with_data(variant, &mut variants, mapper);
        }
        let mut codable_body = String::new();
        emit_serde_tagged_codable(en, &mut codable_body, mapper);
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                variants => variants,
                codable_body => codable_body,
            },
        ));
    } else if is_serde_untagged {
        let mut variants = String::new();
        for variant in &en.variants {
            emit_variant_with_data(variant, &mut variants, mapper);
        }
        let mut codable_body = String::new();
        emit_serde_untagged_codable(en, &mut codable_body, mapper);
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                variants => variants,
                codable_body => codable_body,
            },
        ));
    } else {
        let mut variants = String::new();
        for variant in &en.variants {
            emit_variant_with_data(variant, &mut variants, mapper);
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                variants => variants,
                codable_body => "",
            },
        ));
    }

    if is_serde_untagged && text_types.iter().any(|t| t == &en.name) {
        emit_swift_text_accessor(en, out);
    }

    emit_enum_into_rust_extension(&en.name, out);
}

/// Emit a `func text() -> String` extension on an untagged-union Swift enum that
/// extracts the plain-text display value, mirroring Rust's `Display`.
///
/// - String newtype variant: return the associated value verbatim.
/// - `Vec<T>` newtype variant: serialize each element to JSON and concatenate the
///   `"text"` field of every element whose `"type"` equals `"text"`, skipping
///   non-text parts (images, audio, refusals). The JSON round-trip keeps the
///   accessor independent of the element type's concrete Swift shape, matching the
///   Kotlin backend's JsonNode-based extraction.
/// - Any other variant (unit, primitive, struct, object): return an empty string.
pub(super) fn emit_swift_text_accessor(en: &EnumDef, out: &mut String) {
    let name = &en.name;
    out.push_str("extension ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("    /// Returns the plain-text display value of this content.\n");
    out.push_str("    ///\n");
    out.push_str("    /// - If the value is a string, it is returned verbatim.\n");
    out.push_str("    /// - If the value is an array, the `text` field of every element whose\n");
    out.push_str("    ///   `type` equals `\"text\"` is concatenated in order; non-text parts\n");
    out.push_str("    ///   (images, audio, refusals, etc.) are skipped.\n");
    out.push_str("    /// - Otherwise returns an empty string.\n");
    out.push_str("    public func text() -> String {\n");
    out.push_str("        switch self {\n");

    for variant in &en.variants {
        let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
        if variant.fields.len() == 1 && is_tuple_variant_field(&variant.fields[0].name) {
            let label = swift_associated_label(&variant.fields[0].name, 0);
            match &variant.fields[0].ty {
                TypeRef::String => {
                    out.push_str("        case .");
                    out.push_str(&case_name);
                    out.push_str("(let ");
                    out.push_str(&label);
                    out.push_str("):\n            return ");
                    out.push_str(&label);
                    out.push('\n');
                }
                TypeRef::Vec(elem_ty) if matches!(**elem_ty, TypeRef::Named(_) | TypeRef::Json) => {
                    out.push_str("        case .");
                    out.push_str(&case_name);
                    out.push_str("(let ");
                    out.push_str(&label);
                    out.push_str("):\n");
                    out.push_str("            var result = \"\"\n");
                    out.push_str("            for part in ");
                    out.push_str(&label);
                    out.push_str(" {\n");
                    out.push_str("                guard let data = try? JSONEncoder().encode(part),\n");
                    out.push_str(
                        "                    let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],\n",
                    );
                    out.push_str("                    object[\"type\"] as? String == \"text\",\n");
                    out.push_str("                    let textValue = object[\"text\"] as? String\n");
                    out.push_str("                else { continue }\n");
                    out.push_str("                result += textValue\n");
                    out.push_str("            }\n");
                    out.push_str("            return result\n");
                }
                _ => emit_swift_text_empty_case(out, &case_name),
            }
        } else if variant.fields.is_empty() {
            emit_swift_text_empty_case(out, &case_name);
        } else {
            out.push_str("        case .");
            out.push_str(&case_name);
            out.push_str(":\n            return \"\"\n");
        }
    }

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a `case .<name>: return ""` branch for the Swift text accessor.
fn emit_swift_text_empty_case(out: &mut String, case_name: &str) {
    out.push_str("        case .");
    out.push_str(case_name);
    out.push_str(":\n            return \"\"\n");
}

/// Returns `true` when a variant field name denotes a positional (tuple) payload —
/// i.e. a bare or underscore-prefixed digit (`0`, `_0`, …). Such variants render as
/// `case foo(field0: T)` in Swift and have a single unlabelled associated value.
fn is_tuple_variant_field(name: &str) -> bool {
    let stripped = name.strip_prefix('_').unwrap_or(name);
    !stripped.is_empty() && stripped.bytes().all(|b| b.is_ascii_digit())
}

/// Returns `true` when every associated value type across `en.variants` is safe to
/// reference inside a Codable+Sendable+Hashable Swift enum — i.e. the type is a
/// primitive, String/Char/Path/Json/Bytes/Duration, or a Named type already known
/// to be a first-class Codable struct or unit serde enum.
pub(super) fn all_variants_codable_safe(en: &EnumDef, known_dto_names: &std::collections::HashSet<String>) -> bool {
    fn supported(ty: &TypeRef, known: &std::collections::HashSet<String>) -> bool {
        match ty {
            TypeRef::Primitive(_)
            | TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Unit
            | TypeRef::Bytes
            | TypeRef::Duration => true,
            TypeRef::Named(n) => known.contains(n),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => supported(inner, known),
            TypeRef::Map(k, v) => supported(k, known) && supported(v, known),
        }
    }
    en.variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .all(|f| supported(&f.ty, known_dto_names))
}

/// Emits a Swift enum for a trait-bridge result type without an intoRust() extension.
/// Result-type enums are first-class enums that JSON-decode locally in Swift and do NOT
/// call a Rust-side from_json function, so they don't need the FFI round-trip mechanism.
pub(super) fn emit_enum_without_into_rust(
    en: &EnumDef,
    out: &mut String,
    mapper: &SwiftMapper,
    known_dto_names: &std::collections::HashSet<String>,
) {
    super::client::emit_doc_comment(&en.doc, "", out);

    if !en.has_serde {
        out.push_str(&crate::backends::swift::template_env::render(
            "typealias.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        return;
    }

    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());

    if all_unit {
        let _ = mapper;
        let mut cases = String::new();
        for variant in &en.variants {
            super::client::emit_doc_comment(&variant.doc, "    ", &mut cases);
            let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
            let raw_value = unit_enum_wire_value(variant, en.serde_rename_all.as_deref());
            if raw_value == case_name.trim_matches('`') {
                cases.push_str(&crate::backends::swift::template_env::render(
                    "enum_case_unit.jinja",
                    minijinja::context! {
                        case_name => &case_name,
                    },
                ));
            } else {
                cases.push_str(&crate::backends::swift::template_env::render(
                    "enum_case_raw_value.swift.jinja",
                    minijinja::context! {
                        case_name => &case_name,
                        raw_value => &raw_value,
                    },
                ));
            }
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_raw_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                cases => cases,
            },
        ));
        return;
    }

    if all_variants_codable_safe(en, known_dto_names) {
        let mut variants = String::new();
        for variant in &en.variants {
            emit_variant_with_data(variant, &mut variants, mapper);
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                variants => variants,
                codable_body => "",
            },
        ));
    } else {
        out.push_str(&crate::backends::swift::template_env::render(
            "typealias.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
    }
}

/// Emits an `extension {EnumName} { func intoRust() throws -> RustBridge.{EnumName} { ... } }`
/// block that converts a public Swift enum back to its opaque FFI-bridge counterpart.
///
/// The bridge enum (a swift-bridge `extern "Rust" { type {Name}; }`) is exposed as an
/// opaque Swift class with no public initializer — there is no way to construct one from
/// individual cases. To round-trip across the FFI boundary, the extension JSON-encodes
/// `self` and delegates to the `{enum_snake}_from_json` swift-bridge shim emitted by the
/// Rust bridge crate. This mirrors the JSON-fallback path used in struct `intoRust()`
/// emission (see `emit_first_class_struct`) and gives every enum the same shape of
/// reverse-conversion that first-class structs already have.
///
/// `throws` because `JSONEncoder.encode` and the Rust shim both fail on malformed
/// input. The extension is internal-visibility (no `public` keyword): callers are other
/// generated bindings inside the same module, never end-users.
pub(super) fn emit_enum_into_rust_extension(name: &str, out: &mut String) {
    let from_json_fn = format!("{}_from_json", AsSnakeCase(name)).to_lower_camel_case();
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_enum_into_rust.swift.jinja",
        minijinja::context! {
            name => name,
            from_json_fn => from_json_fn,
        },
    ));
}

/// Returns the serde wire value for a unit enum variant.
///
/// Priority:
/// 1. Per-variant `serde_rename` override (verbatim).
/// 2. Enum-level `serde_rename_all` strategy applied to the Rust PascalCase variant name.
/// 3. The Rust PascalCase variant name unchanged (serde default).
///
/// Supported `serde_rename_all` values: `"snake_case"`, `"camelCase"`, `"SCREAMING_SNAKE_CASE"`,
/// `"kebab-case"`. Unknown strategies fall back to the PascalCase variant name.
pub(super) fn unit_enum_wire_value(variant: &crate::core::ir::EnumVariant, rename_all: Option<&str>) -> String {
    crate::codegen::naming::wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all)
}

/// Emits a single enum case, with or without associated values.
pub(super) fn emit_variant_with_data(variant: &EnumVariant, out: &mut String, mapper: &SwiftMapper) {
    super::client::emit_doc_comment(&variant.doc, "    ", out);
    let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
    if variant.fields.is_empty() {
        out.push_str(&crate::backends::swift::template_env::render(
            "enum_case_unit.jinja",
            minijinja::context! {
                case_name => &case_name,
            },
        ));
    } else {
        let assoc: Vec<String> = variant
            .fields
            .iter()
            .enumerate()
            .map(|(idx, f)| {
                let already_optional = matches!(&f.ty, TypeRef::Optional(_));
                let ty_str = mapper.map_type(&f.ty);
                let ty_with_opt = if f.optional && !already_optional {
                    format!("{ty_str}?")
                } else {
                    ty_str
                };
                let label = swift_associated_label(&f.name, idx);
                format!("{label}: {ty_with_opt}")
            })
            .collect();
        out.push_str(&crate::backends::swift::template_env::render(
            "enum_case_with_data.jinja",
            minijinja::context! {
                case_name => &case_name,
                associated_values => assoc.join(", "),
            },
        ));
    }
}

/// Resolves a Swift associated-value label for an enum case field.
///
/// - Empty, all-digit, or `_<digits>` names (positional tuple variants) become
///   `field0`, `field1`, …
/// - Otherwise lowerCamelCase + Swift-idiomatic backtick keyword escaping
///   (associated-value labels appear in emitted Swift source).
pub(super) fn swift_associated_label(name: &str, idx: usize) -> String {
    let stripped = name.trim_start_matches('_');
    if stripped.is_empty() || stripped.chars().all(|c| c.is_ascii_digit()) {
        return format!("field{idx}");
    }
    swift_case_ident(&name.to_lower_camel_case())
}
