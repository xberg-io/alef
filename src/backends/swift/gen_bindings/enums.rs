use crate::backends::swift::gen_bindings::adjacent_codable::{emit_serde_adjacent_codable, is_newtype_variant};
use crate::backends::swift::naming::swift_source_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::serde_enum_repr::{SerdeEnumRepr, serde_enum_repr};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use heck::{AsSnakeCase, ToLowerCamelCase};

/// What [`emit_enum`] needs to ask `gen_rust_crate::enums::declared_variants` which variants the
/// swift-bridge mirror enum actually declares for this build.
///
/// `configured_features` MUST be `codegen::cfg::enabled_features_for_language(config,
/// Language::Swift)` -- the same list `gen_rust_crate::emit` passes to `emit_enum_wrapper`, NOT
/// the wider `feature_gate::effective_swift_codegen_features` set the facade uses elsewhere. That
/// wider set is deliberately widened with every HOST-owned cfg feature name the surface mentions,
/// which is the wrong question for a FOREIGN variant's reachability; feeding it here would make
/// this facade's answer differ from the mirror's for exactly the variants this type exists to keep
/// in lockstep. ~keep
pub(super) struct EnumDeclarationCfg<'a> {
    source_crate: String,
    configured_features: &'a [String],
}

impl<'a> EnumDeclarationCfg<'a> {
    pub(super) fn new(crate_name: &str, configured_features: &'a [String]) -> Self {
        Self {
            source_crate: crate_name.replace('-', "_"),
            configured_features,
        }
    }
}

/// The `case` list for an all-unit Swift enum, restricted to the variants the swift-bridge mirror
/// enum declares.
///
/// A `case` the mirror dropped is a case no conversion can produce (the mirror's `to_string` match
/// is built from that same list) and none can accept (its `__alef_*_from_swift_string` arm was
/// dropped too) -- the facade would advertise a value the binding cannot represent. Asking
/// `gen_rust_crate::enums::declared_variants` rather than re-deriving the rule is what keeps the
/// two emitted surfaces from drifting apart again. ~keep
fn unit_enum_cases(en: &EnumDef, cfg: &EnumDeclarationCfg<'_>) -> String {
    let declared = crate::backends::swift::gen_rust_crate::enums::declared_variants(
        en,
        &cfg.source_crate,
        Some(cfg.configured_features),
    );
    let mut cases = String::new();
    for variant in declared {
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
    cases
}

/// Render the custom `Codable` members an enum needs so its JSON matches serde's, or an empty
/// string when serde's representation is already what Swift synthesises.
///
/// This is the only place the choice is made: every Swift enum emitter routes through it, so the
/// first-class enum and the trait-bridge result enum (which used to emit no `Codable` body at
/// all, silently accepting Swift's externally tagged synthesis) can no longer disagree.
fn serde_codable_body(en: &EnumDef, repr: &SerdeEnumRepr, mapper: &SwiftMapper) -> String {
    let mut body = String::new();
    match repr {
        SerdeEnumRepr::Internal { .. } => emit_serde_tagged_codable(en, &mut body, mapper),
        SerdeEnumRepr::Adjacent { tag, content } => {
            emit_serde_adjacent_codable(en, tag, content, &mut body, mapper);
        }
        SerdeEnumRepr::Untagged if has_untagged_single_payloads(en) => {
            emit_serde_untagged_codable(en, &mut body, mapper);
        }
        SerdeEnumRepr::External | SerdeEnumRepr::Untagged => {}
    }
    body
}

/// `emit_serde_untagged_codable` decodes each variant through a single-value container, which
/// only works when every data variant carries exactly one payload.
fn has_untagged_single_payloads(en: &EnumDef) -> bool {
    en.variants.iter().any(|v| !v.fields.is_empty())
        && en
            .variants
            .iter()
            .filter(|v| !v.fields.is_empty())
            .all(|v| v.fields.len() == 1)
}

/// serde flattens an internally tagged *newtype* variant's payload into the very object that
/// carries the tag (`{"format_type":"excel","sheet_count":2}`), so there is no `"0"` key to decode
/// from: the payload has to be handed the same decoder, and encode into the same container.
///
/// The optionality guard exists only because `T?(from:)` is not expressible in Swift. serde cannot
/// internally tag an `Option` payload either, so the keyed path such a variant keeps is no more
/// wrong than it already was. ~keep
fn is_flattened_newtype_variant(variant: &EnumVariant) -> bool {
    if !is_newtype_variant(variant) {
        return false;
    }
    let field = &variant.fields[0];
    !field.optional && !matches!(&field.ty, TypeRef::Optional(_))
}

/// Emits a custom Codable conformance for a serde-internally-tagged enum.
/// Handles variant-tag decoding/encoding and respects field renames.
pub(super) fn emit_serde_tagged_codable(en: &EnumDef, out: &mut String, mapper: &SwiftMapper) {
    let tag_wire = crate::codegen::serde_enum_repr::tagged_object_tag_key(en);
    let tag_ident = swift_case_ident(&tag_wire.to_lower_camel_case());

    let mut field_keys = std::collections::BTreeSet::new();
    for variant in &en.variants {
        if is_flattened_newtype_variant(variant) {
            continue;
        }
        for (idx, field) in variant.fields.iter().enumerate() {
            let swift_name = swift_associated_label(&field.name, idx);
            // `rename_all_fields` renames the fields INSIDE a struct variant, a separate namespace
            // from `rename_all`, which renames the variants themselves. Reading only
            // `serde_rename` ignored it and emitted CodingKeys that do not match the wire. The
            // adjacent emitter already asks `wire_field_name` for this; both now do. ~keep
            let rust_name = crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                en.rename_all_fields.as_deref(),
            );
            field_keys.insert((swift_name, rust_name));
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
        } else if is_flattened_newtype_variant(variant) {
            let field = &variant.fields[0];
            let label = swift_associated_label(&field.name, 0);
            let payload_ty = mapper.map_type(&field.ty);
            decode_cases.push_str(&crate::backends::swift::template_env::render(
                "swift_tagged_decode_payload_case.swift.jinja",
                minijinja::context! {
                    variant_tag => &variant_tag,
                    case_name => &case_name,
                    field_decoders => format!("{label}: try {payload_ty}(from: decoder)"),
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
                    tag_key => &tag_ident,
                    case_name => &case_name,
                },
            ));
        } else if is_flattened_newtype_variant(variant) {
            let label = swift_associated_label(&variant.fields[0].name, 0);
            encode_cases.push_str(&crate::backends::swift::template_env::render(
                "swift_tagged_encode_payload_case.swift.jinja",
                minijinja::context! {
                    variant_tag => &variant_tag,
                    tag_key => &tag_ident,
                    case_name => &case_name,
                    bindings => format!("let {label}"),
                    field_encoders => format!("            try {label}.encode(to: encoder)\n"),
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
                    tag_key => &tag_ident,
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
            tag_ident => &tag_ident,
            tag_wire => tag_wire,
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
    cfg: &EnumDeclarationCfg<'_>,
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
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_raw_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                cases => unit_enum_cases(en, cfg),
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

    let repr = serde_enum_repr(en);
    let is_serde_untagged = matches!(repr, SerdeEnumRepr::Untagged) && has_untagged_single_payloads(en);

    let mut variants = String::new();
    for variant in &en.variants {
        emit_variant_with_data(variant, &mut variants, mapper);
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_enum_decl.swift.jinja",
        minijinja::context! {
            name => &en.name,
            variants => variants,
            codable_body => serde_codable_body(en, &repr, mapper),
        },
    ));

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
    cfg: &EnumDeclarationCfg<'_>,
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
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_enum_raw_decl.swift.jinja",
            minijinja::context! {
                name => &en.name,
                cases => unit_enum_cases(en, cfg),
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
                codable_body => serde_codable_body(en, &serde_enum_repr(en), mapper),
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

#[cfg(test)]
mod tagged_codable_tests {
    use super::*;
    use crate::core::ir::FieldDef;

    fn render_tagged(en: &EnumDef) -> String {
        let mut out = String::new();
        emit_serde_tagged_codable(en, &mut out, &SwiftMapper);
        out
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
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

    fn tagged_enum(tag: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: "FormatMetadata".to_string(),
            has_serde: true,
            serde_tag: Some(tag.to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            variants,
            ..EnumDef::default()
        }
    }

    #[test]
    fn should_decode_an_internally_tagged_newtype_payload_from_the_same_decoder() {
        let en = tagged_enum(
            "format_type",
            vec![newtype_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
        );
        let out = render_tagged(&en);
        assert!(
            out.contains("self = .excel(field0: try ExcelMetadata(from: decoder))"),
            "{out}"
        );
    }

    #[test]
    fn should_encode_an_internally_tagged_newtype_payload_into_the_tag_container() {
        let en = tagged_enum(
            "format_type",
            vec![newtype_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
        );
        let out = render_tagged(&en);
        assert!(out.contains("case .excel(let field0):"), "{out}");
        assert!(
            out.contains("try container.encode(\"excel\", forKey: .formatType)"),
            "{out}"
        );
        assert!(out.contains("try field0.encode(to: encoder)"), "{out}");
    }

    #[test]
    fn should_not_give_an_internally_tagged_newtype_payload_a_positional_coding_key() {
        let en = tagged_enum(
            "format_type",
            vec![newtype_variant("Excel", TypeRef::Named("ExcelMetadata".to_string()))],
        );
        let out = render_tagged(&en);
        assert!(
            !out.contains("field0 = \"0\""),
            "serde flattens the payload; there is no \"0\" key on the wire: {out}"
        );
        assert!(
            !out.contains("forKey: .field0"),
            "serde flattens the payload; there is no \"0\" key on the wire: {out}"
        );
    }

    #[test]
    fn should_keep_the_keyed_form_for_an_internally_tagged_struct_variant() {
        let en = tagged_enum(
            "type",
            vec![struct_variant(
                "Basic",
                vec![field("username", TypeRef::String), field("password", TypeRef::String)],
            )],
        );
        let out = render_tagged(&en);
        // No `= "wire"` alias when the Swift label already equals the wire name -- that is the
        // shape `AuthConfig` ships with today, and the alias is only emitted when they differ. ~keep
        assert!(out.contains("case username"), "{out}");
        assert!(out.contains("case password"), "{out}");
        assert!(!out.contains("case username = "), "{out}");
        assert!(
            out.contains(
                "self = .basic(username: try container.decode(String.self, forKey: .username), \
                 password: try container.decode(String.self, forKey: .password))"
            ),
            "{out}"
        );
        assert!(
            out.contains("try container.encode(username, forKey: .username)"),
            "{out}"
        );
        assert!(
            !out.contains("(from: decoder)"),
            "a named-field variant is not a flattened newtype: {out}"
        );
    }

    #[test]
    fn should_keep_the_keyed_form_for_an_optional_newtype_payload() {
        let en = tagged_enum(
            "format_type",
            vec![newtype_variant(
                "Excel",
                TypeRef::Optional(Box::new(TypeRef::Named("ExcelMetadata".to_string()))),
            )],
        );
        let out = render_tagged(&en);
        assert!(
            out.contains("field0: try container.decodeIfPresent("),
            "`T?(from:)` is not expressible in Swift, so the keyed path is kept: {out}"
        );
        assert!(out.contains("forKey: .field0"), "{out}");
        assert!(out.contains("case field0 = \"0\""), "{out}");
        assert!(!out.contains("(from: decoder)"), "{out}");
    }

    #[test]
    fn should_alias_a_snake_case_tag_key_onto_a_camel_case_swift_identifier() {
        let en = tagged_enum("format_type", vec![struct_variant("Excel", vec![])]);
        let out = render_tagged(&en);
        assert!(out.contains("case formatType = \"format_type\""), "{out}");
        assert!(out.contains("forKey: .formatType"), "{out}");
        assert!(!out.contains("case format_type"), "{out}");
        assert!(!out.contains("forKey: .format_type"), "{out}");
    }

    #[test]
    fn should_alias_a_hyphenated_tag_key_onto_a_legal_swift_identifier() {
        let en = tagged_enum("format-type", vec![struct_variant("Excel", vec![])]);
        let out = render_tagged(&en);
        assert!(out.contains("case formatType = \"format-type\""), "{out}");
        assert!(out.contains("forKey: .formatType"), "{out}");
        assert!(!out.contains("case format-type"), "{out}");
    }

    #[test]
    fn should_backtick_escape_a_tag_key_that_collides_with_a_swift_keyword() {
        let en = tagged_enum("protocol", vec![struct_variant("Excel", vec![])]);
        let out = render_tagged(&en);
        assert!(out.contains("case `protocol` = \"protocol\""), "{out}");
        assert!(out.contains("forKey: .`protocol`"), "{out}");
    }

    #[test]
    fn should_give_the_conventional_type_tag_key_an_explicit_raw_value() {
        let en = tagged_enum("type", vec![struct_variant("Excel", vec![])]);
        let out = render_tagged(&en);
        assert!(out.contains("case type = \"type\""), "{out}");
        assert!(out.contains("forKey: .type"), "{out}");
    }
}
