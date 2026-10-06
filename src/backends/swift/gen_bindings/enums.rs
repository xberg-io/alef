use crate::backends::swift::gen_bindings::adjacent_codable::{emit_serde_adjacent_codable, is_newtype_variant};
use crate::backends::swift::naming::swift_source_ident as swift_case_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::codegen::serde_enum_repr::{SerdeEnumRepr, serde_enum_repr};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use heck::{AsSnakeCase, ToLowerCamelCase};

/// Convert a Rust PascalCase enum variant name to a Swift `case` identifier, with correct
/// acronym-style segmentation and keyword escaping.
///
/// Use this instead of `heck::ToLowerCamelCase` for a variant name: heck re-segments
/// acronym runs incorrectly (`RDFa` -> `rdFa`), while [`crate::codegen::naming::pascal_to_camel`]
/// segments it the way `pascal_to_snake` does (`RDFa` -> `rdfa`). ~keep
pub(super) fn swift_variant_case(name: &str) -> String {
    swift_case_ident(&crate::codegen::naming::pascal_to_camel(name))
}

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
        let case_name = swift_variant_case(&variant.name);
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
        SerdeEnumRepr::External => emit_serde_external_codable(en, &mut body, mapper),
        SerdeEnumRepr::Untagged => {}
    }
    body
}

/// Emit `init(from:)`/`encode(to:)` for an EXTERNALLY tagged enum (serde's default) that carries
/// data on at least one variant.
///
/// Without this the enum fell through to Swift's compiler-synthesized `Codable`, which uses a
/// keyed container for EVERY variant -- `{"plain":{}}` for a unit variant and
/// `{"custom":{"field0":"x"}}` for a newtype one. serde writes neither: a unit variant is a bare
/// string `"plain"`, and a newtype variant is a single-keyed object whose value IS the payload,
/// `{"custom":"x"}`. Decoding real wire data therefore threw `typeMismatch` on every value of such
/// a type, and the compiler could not see it. Confirmed against a real binding: every
/// bridge-constructed value carrying one of these enums failed to decode.
///
/// An all-unit externally tagged enum never reaches here -- it is emitted as a `String`-backed
/// `RawRepresentable` by the `swift_enum_raw_decl` branch, whose raw values already match serde. ~keep
pub(super) fn emit_serde_external_codable(en: &EnumDef, out: &mut String, mapper: &SwiftMapper) {
    let wire = |variant: &EnumVariant| {
        crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        )
    };
    let esc = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");

    let mut unit_decode = String::new();
    let mut keyed_decode = String::new();
    let mut encode_cases = String::new();
    let mut untagged_attempts = String::new();

    for variant in &en.variants {
        let case_name = swift_variant_case(&variant.name);
        let tag = esc(&wire(variant));

        // serde honours `#[serde(untagged)]` on a SINGLE variant, not just on the whole enum
        // (`EnumVariant::serde_untagged`), and `serde_enum_repr` only reads the enum-level flag --
        // so an otherwise externally tagged enum can still carry one variant whose payload is
        // written bare, with no tag wrapper at all. `OutputFormat::Custom(String)` is exactly
        // that: `Custom("latex")` serializes as `"latex"`, NOT `{"custom":"latex"}`. Emitting the
        // keyed form for it would produce a Codable that cannot read its own wire. ~keep
        if variant.serde_untagged && variant.fields.len() == 1 {
            let payload_ty = mapper.map_type(&variant.fields[0].ty);
            let label = swift_associated_label(&variant.fields[0].name, 0);
            untagged_attempts.push_str(&format!(
                "        if let single = try? decoder.singleValueContainer(), let value = try? single.decode({payload_ty}.self) {{\n            self = .{case_name}({label}: value)\n            return\n        }}\n"
            ));
            encode_cases.push_str(&format!(
                "        case .{case_name}(let value):\n            var single = encoder.singleValueContainer()\n            try single.encode(value)\n"
            ));
            continue;
        }

        if variant.fields.is_empty() {
            unit_decode.push_str(&format!(
                "            case \"{tag}\":\n                self = .{case_name}\n                return\n"
            ));
            encode_cases.push_str(&format!(
                "        case .{case_name}:\n            var single = encoder.singleValueContainer()\n            try single.encode(\"{tag}\")\n"
            ));
            continue;
        }

        if variant.is_tuple && variant.fields.len() == 1 {
            let label = swift_associated_label(&variant.fields[0].name, 0);
            let (decode_fn, encode_fn, payload_ty) = match &variant.fields[0].ty {
                TypeRef::Optional(inner) => ("decodeIfPresent", "encodeIfPresent", mapper.map_type(inner)),
                other if variant.fields[0].optional => ("decodeIfPresent", "encodeIfPresent", mapper.map_type(other)),
                other => ("decode", "encode", mapper.map_type(other)),
            };
            keyed_decode.push_str(&format!(
                "        case \"{tag}\":\n            self = .{case_name}({label}: try container.{decode_fn}({payload_ty}.self, forKey: key))\n"
            ));
            encode_cases.push_str(&format!(
                "        case .{case_name}(let value):\n            var container = encoder.container(keyedBy: __AlefExternalTagKey.self)\n            try container.{encode_fn}(value, forKey: __AlefExternalTagKey(\"{tag}\"))\n"
            ));
            continue;
        }

        let mut binds = Vec::new();
        let mut reads = String::new();
        let mut writes = String::new();
        for (index, field) in variant.fields.iter().enumerate() {
            let label = swift_associated_label(&field.name, index);
            let field_ty = mapper.map_type(&field.ty);
            let field_wire = esc(&crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                None,
            ));
            binds.push(format!("let {label}"));
            // serde omits a `None` field entirely (`skip_serializing_if`), so a non-present key is
            // absence, not corruption -- `decode` throws `keyNotFound` on it. ~keep
            // Must use the SAME optionality predicate `emit_variant_with_data` used to DECLARE
            // this associated value (`f.optional || TypeRef::Optional`) -- a declaration that says
            // `String?` decoded with `decode(String.self)` throws `keyNotFound` on the absent key
            // serde writes for `None`, and the two drifting apart is invisible to the compiler. ~keep
            let (decode_fn, encode_fn, decoded_ty) = match &field.ty {
                TypeRef::Optional(inner) => ("decodeIfPresent", "encodeIfPresent", mapper.map_type(inner)),
                other if field.optional => ("decodeIfPresent", "encodeIfPresent", mapper.map_type(other)),
                _ => ("decode", "encode", field_ty.clone()),
            };
            reads.push_str(&format!(
                "                {label}: try nested.{decode_fn}({decoded_ty}.self, forKey: __AlefExternalTagKey(\"{field_wire}\")),\n"
            ));
            writes.push_str(&format!(
                "            try nested.{encode_fn}({label}, forKey: __AlefExternalTagKey(\"{field_wire}\"))\n"
            ));
        }
        let reads = reads.trim_end().trim_end_matches(',').to_string();
        keyed_decode.push_str(&format!(
            "        case \"{tag}\":\n            let nested = try container.nestedContainer(keyedBy: __AlefExternalTagKey.self, forKey: key)\n            self = .{case_name}(\n{reads}\n            )\n"
        ));
        encode_cases.push_str(&format!(
            "        case .{case_name}({}):\n            var container = encoder.container(keyedBy: __AlefExternalTagKey.self)\n            var nested = container.nestedContainer(keyedBy: __AlefExternalTagKey.self, forKey: __AlefExternalTagKey(\"{tag}\"))\n{writes}",
            binds.join(", ")
        ));
    }

    // With an untagged fallback present an unrecognized string is NOT an error -- it is that
    // variant's payload -- so the unit-tag switch must fall through instead of throwing. ~keep
    let unknown_tag_arm = if untagged_attempts.is_empty() {
        "                throw DecodingError.dataCorrupted(\n                    .init(codingPath: decoder.codingPath, debugDescription: \"unknown variant tag \\(raw)\"))".to_string()
    } else {
        "                break".to_string()
    };
    out.push_str(&crate::backends::swift::template_env::render(
        "swift_external_codable.swift.jinja",
        minijinja::context! {
            unit_decode => unit_decode,
            keyed_decode => keyed_decode,
            encode_cases => encode_cases,
            untagged_attempts => untagged_attempts,
            unknown_tag_arm => unknown_tag_arm,
        },
    ));
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

        let case_name = swift_variant_case(&variant.name);

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

        let case_name = swift_variant_case(&variant.name);

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
        let case_name = swift_variant_case(&variant.name);
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
        let case_name = swift_variant_case(&variant.name);
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

    emit_swift_wire_tag_accessor(en, out);

    emit_enum_into_rust_extension(&en.name, out);
}

/// Emit a `public func toString() -> String` extension on a payload-carrying (associated-value)
/// Swift enum that returns the SAME serde wire tag the swift-bridge opaque mirror's own
/// `to_string()` returns (`gen_rust_crate::enums`'s `rust_enum_to_string_impl.rs.jinja`:
/// `Self::{variant} => "{serde_name}".to_string()`), dropping any payload exactly as that mirror
/// method does.
///
/// Before this existed, a promoted payload-carrying enum declared NEITHER `.rawValue` (only the
/// all-unit shape gets that, see `unit_enum_cases`) NOR `.toString()` -- e2e assertion generators
/// that called `.toString()` on such a leaf (assuming the pre-promotion opaque shape) produced a
/// hard Swift compile error. Rather than refuse those assertions (they have a real, reproducible
/// answer -- the wire tag is data alef already computes), this gives the promoted type the same
/// accessor name and the same value, so existing `.toString()` call sites keep compiling AND keep
/// asserting the same thing they always did.
///
/// Reuses [`crate::codegen::naming::wire_variant_value`] -- the SAME function
/// [`emit_serde_tagged_codable`]'s own encode/decode cases call for a variant's tag -- so this
/// accessor and the enum's own Codable wire form can never disagree about what a variant's tag is.
///
/// Declared over ALL of `en.variants`, not `declared_variants`'s cfg-filtered subset: the case
/// list just emitted above (`emit_variant_with_data`'s loop, also over `en.variants` unfiltered)
/// is what this `switch` must stay exhaustive against, and filtering here independently would
/// leave a real declared case with no matching arm -- `error: switch must be exhaustive`. ~keep
pub(super) fn emit_swift_wire_tag_accessor(en: &EnumDef, out: &mut String) {
    out.push_str("extension ");
    out.push_str(&en.name);
    out.push_str(" {\n");
    out.push_str("    /// Returns the serde wire tag identifying this variant -- the same value\n");
    out.push_str("    /// the pre-promotion opaque binding's `to_string()` returned -- dropping\n");
    out.push_str("    /// any associated payload.\n");
    out.push_str("    public func toString() -> String {\n");
    out.push_str("        switch self {\n");
    for variant in &en.variants {
        let case_name = swift_variant_case(&variant.name);
        let wire = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("        case .");
        out.push_str(&case_name);
        out.push_str(":\n            return \"");
        out.push_str(&wire.replace('\\', "\\\\").replace('"', "\\\""));
        out.push_str("\"\n");
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
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
        let case_name = swift_variant_case(&variant.name);
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
    let case_name = swift_variant_case(&variant.name);
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
mod tagged_codable_tests;

/// Pins `emit_swift_wire_tag_accessor`'s declared method name (`toString`) and per-variant switch
/// body against the exact call-site text the Swift e2e generator is independently tested to emit
/// for a payload-carrying (promoted) enum leaf --
/// `e2e::codegen::swift::first_class_render_gate_tests::payload_carrying_enum_leaf_renders_a_real_assertion_not_a_skip`
/// (and its sibling gate tests) assert the literal substring `"payload.toString()"` in RENDERED
/// e2e output. Together these two independently-failing tests are what "fails if either moves
/// alone" means in practice here: this test fails if the BINDING declaration stops being named
/// `toString` (or stops matching the mirror's own wire-tag match), and the e2e gate tests fail if
/// the CALL SITE stops emitting `.toString()` -- neither can drift without a red test, without
/// requiring the two to share Rust code across the binding/e2e module boundary. ~keep
#[cfg(test)]
mod wire_tag_accessor_tests;

/// Pins the externally tagged (serde default) Codable body against the three wire shapes serde
/// actually writes. Before `emit_serde_external_codable` existed these enums fell through to
/// Swift's synthesized `Codable`, which keys EVERY variant -- `{"plain":{}}` for a unit variant,
/// `{"custom":{"field0":"x"}}` for a newtype one -- so no real wire value decoded. Verified against
/// a built binding with `swiftc`: `"plain"` and `{"custom":"foo"}` both round-trip now and both
/// threw `typeMismatch` before. ~keep
#[cfg(test)]
mod external_codable_tests;
