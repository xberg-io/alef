use crate::backends::swift::naming::swift_rust_shim_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use heck::{AsSnakeCase, ToLowerCamelCase};

/// Emits a custom Codable conformance for a serde-internally-tagged enum.
/// Handles variant-tag decoding/encoding and respects field renames.
pub(super) fn emit_serde_tagged_codable(en: &EnumDef, out: &mut String, mapper: &SwiftMapper) {
    let tag_key = en.serde_tag.as_deref().unwrap_or("type");

    // Collect all unique field names across all variants. Use swift_associated_label
    // so positional tuple-variant fields (named "0", "1", … or "_0", "_1", …) become
    // `field0`, `field1`, … — bare digits are invalid Swift identifiers, and the
    // init(from:)/encode(to:) bodies below already use the label-synthesized form
    // via swift_associated_label, so the CodingKeys cases must match.
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

                // For Optional types, use decodeIfPresent with the type as-is.
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
        // All-unit serde enums receive `: String, Codable, Sendable, Hashable` conformance
        // so that structs with enum-typed fields can derive `Codable` automatically.
        // Raw values are the serde-serialized variant names (respecting serde_rename_all
        // and per-variant serde_rename overrides) so JSONDecoder/JSONEncoder round-trip
        // correctly against the Rust wire format.
        let _ = mapper; // mapper unused for case bodies — suppress unused warning
        let mut cases = String::new();
        for variant in &en.variants {
            super::client::emit_doc_comment(&variant.doc, "    ", &mut cases);
            let case_name = swift_case_ident(&variant.name.to_lower_camel_case());
            let raw_value = unit_enum_wire_value(variant, en.serde_rename_all.as_deref());
            if raw_value == case_name.trim_matches('`') {
                // Raw value matches the Swift case name — no explicit annotation needed.
                cases.push_str(&crate::backends::swift::template_env::render(
                    "enum_case_unit.jinja",
                    minijinja::context! {
                        case_name => &case_name,
                    },
                ));
            } else {
                // Explicit raw-value annotation required (e.g. `case toolCalls = "tool_calls"`).
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
    // Data-variant enum (has_serde + not all_unit): emit as native Swift enum with associated values.
    //
    // If the enum has serde_tag (internally-tagged), emit custom Codable conformance
    // to handle the {"type": "variant", ...} wire format instead of Swift's default
    // {"variant": {...}} externally-tagged form.
    //
    // Otherwise, rely on Swift's auto-synthesized Codable when all associated value
    // types are Codable-safe (primitives, String, or known first-class structs).
    // When any associated value is not Codable-safe, fall back to a typealias to
    // RustBridge.X — users lose Swift-side pattern matching but everything else works.
    if !all_variants_codable_safe(en, known_dto_names) {
        out.push_str(&crate::backends::swift::template_env::render(
            "typealias.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        return;
    }

    // Check if this enum uses serde tagging (internally-tagged) or untagged.
    let has_serde_tag = en.serde_tag.is_some() && !en.serde_untagged;
    let is_serde_untagged = en.serde_untagged
        && en.variants.iter().any(|v| !v.fields.is_empty())
        && en
            .variants
            .iter()
            .filter(|v| !v.fields.is_empty())
            .all(|v| v.fields.len() == 1);

    if has_serde_tag {
        // Emit as enum with custom Codable for internally-tagged format
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
        // Emit as enum with custom Codable that matches serde's untagged
        // wire format (bare value, no `{"variant": ...}` wrapper).
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
        // Emit as enum with auto-synthesized Codable (externally-tagged)
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

    emit_enum_into_rust_extension(&en.name, out);
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
        // Emit all-unit enum without intoRust() extension (unlike emit_enum).
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
        // Do NOT call emit_enum_into_rust_extension — that's the key difference.
        return;
    }

    // For data-variant enums, check if all variants are Codable-safe.
    if all_variants_codable_safe(en, known_dto_names) {
        // Emit as a native Swift enum with associated values, without intoRust().
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
        // Do NOT call emit_enum_into_rust_extension — that's the key difference.
    } else {
        // Fall back to typealias (same as emit_enum) if not all variants are Codable-safe.
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
                // Honor field.optional (extractor-unwrapped form) in addition to
                // TypeRef::Optional(inner) — both encode "nullable" in the IR.
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
