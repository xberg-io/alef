//! SECURITY. `emit_decoder_init` decides whether an absent JSON key is filled with a Swift
//! type-based zero (`[]`, `[:]`, `0`) or left to throw a visible `DecodingError`. A field carrying
//! `#[serde(default = "path")]` reaches the IR as `DefaultValue::FunctionCall` — alef records the
//! function's *name*, never its return value — so the zero is a claim about a value alef does not
//! have, and the emitter correctly declines it.
//!
//! That refusal was unreachable in practice. `extract::extractor::types` blanket-overwrote every
//! field's `typed_default` with `DefaultValue::Empty` whenever the container derived `Default`,
//! and `Empty` licenses the type-based zero. A named allow-list or deny-list default therefore
//! decoded to `[]` — an allow-list that permits nothing, or a deny-list that fails open. These
//! tests pin the deferral against the IR shape the extractor now produces.
//!
//! Lives here rather than in `gen_bindings/dto.rs`'s inline `mod tests` because that file is at
//! its recorded file-size ceiling (`tests/file_size_baseline.txt`) and may not grow, as is
//! `gen_bindings/mod.rs` at the 1,000-line cap. ~keep

use crate::backends::swift::gen_bindings::dto::emit_decoder_init;
use crate::backends::swift::gen_bindings::zero_arg_default::compute_zero_arg_constructible_names;
use crate::backends::swift::type_map::SwiftMapper;
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::HashSet;

fn decode_body(name: &str, ty: TypeRef, typed_default: DefaultValue) -> String {
    let field = FieldDef {
        name: name.to_string(),
        ty,
        typed_default: Some(typed_default),
        ..Default::default()
    };
    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &HashSet::new(), &mut out);
    out
}

#[test]
fn a_named_serde_default_on_a_vec_defers_to_rust_instead_of_decoding_an_empty_array() {
    let out = decode_body(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::FunctionCall("default_scheme_allowlist".to_string()),
    );

    assert!(
        !out.contains("?? []"),
        "alef never evaluates default_scheme_allowlist(); `?? []` decodes an absent key into an \
         empty allow-list that permits nothing:\n{out}"
    );
    assert!(
        out.contains("try container.decode([String].self, forKey: .schemeAllowlist)"),
        "an unreadable default must leave the key required so an absent key is a visible \
         DecodingError:\n{out}"
    );
}

#[test]
fn a_named_serde_default_on_a_map_defers_to_rust_instead_of_decoding_an_empty_dictionary() {
    let out = decode_body(
        "header_overrides",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        DefaultValue::PublicFunctionCall("sample_crate::Policy::header_overrides".to_string()),
    );

    assert!(
        !out.contains("?? [:]"),
        "a resolved function-call default is still a value alef has not read:\n{out}"
    );
}

/// Discrimination control for both tests above. `Empty` genuinely IS `Default::default()`, so the
/// empty array and empty dictionary are exact for it and must still be emitted. Without this, a
/// change that stopped emitting `??` fallbacks for collections at all would satisfy the assertions
/// above while stripping the fallback off every ordinary `#[derive(Default)]` field. ~keep
#[test]
fn an_empty_default_still_decodes_to_the_swift_collection_zero() {
    let vec_out = decode_body(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::Empty,
    );
    assert!(
        vec_out.contains("?? []"),
        "`Empty` is the type's own default and keeps the empty-array fallback:\n{vec_out}"
    );

    let map_out = decode_body(
        "header_overrides",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        DefaultValue::Empty,
    );
    assert!(
        map_out.contains("?? [:]"),
        "`Empty` keeps the empty-dictionary fallback:\n{map_out}"
    );

    let scalar_out = decode_body(
        "redirect_limit",
        TypeRef::Primitive(PrimitiveType::U32),
        DefaultValue::Empty,
    );
    assert!(
        scalar_out.contains("?? 0"),
        "`Empty` keeps the scalar zero fallback:\n{scalar_out}"
    );
}

/// Reproduces a config struct whose `ngram_range: NgramRange` field carries `#[serde(default)]`
/// with a default of `NgramRange::default()`, which folds to
/// `typed_default = Empty` because the extractor cannot spell a Swift literal for a `Named`
/// struct. `NgramRange` itself is fully default-constructible (`min`/`max` both have literal
/// defaults), so it belongs in `zero_arg_constructible_names` and the field must decode as
/// optional-with-fallback rather than a required key.
#[test]
fn a_named_field_defaulting_to_type_default_uses_the_zero_arg_initializer() {
    let field = FieldDef {
        name: "ngram_range".to_string(),
        ty: TypeRef::Named("NgramRange".to_string()),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };
    let zero_arg_constructible_names: HashSet<String> = ["NgramRange".to_string()].into_iter().collect();

    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &zero_arg_constructible_names, &mut out);

    assert_eq!(
        out,
        "    public init(from decoder: any Decoder) throws {\n        \
         let container = try decoder.container(keyedBy: CodingKeys.self)\n        \
         self.ngramRange = try container.decodeIfPresent(NgramRange.self, forKey: .ngramRange) ?? NgramRange()\n    }\n",
        "an absent key must fall back to the type's own zero-arg initializer:\n{out}"
    );
}

/// The companion case in the same `KeywordConfig`: `algorithm: KeywordAlgorithm` also carries a
/// bare `#[serde(default)]` (`typed_default = Empty`), but `KeywordAlgorithm` is a Swift
/// `enum ... : String, Codable` with no zero-argument initializer — and its Rust `impl Default`
/// is gated on which of the `keywords-yake`/`keywords-rake` Cargo features is enabled, so alef
/// has no way to know which case a bare `KeywordAlgorithm()` should even mean. With
/// `zero_arg_constructible_names` empty (as it is for every enum, by construction of
/// `compute_zero_arg_constructible_names`), the field must keep the required `decode` so an
/// absent key throws a visible `DecodingError` instead of silently picking a case.
#[test]
fn an_enum_field_defaulting_to_type_default_stays_required() {
    let field = FieldDef {
        name: "algorithm".to_string(),
        ty: TypeRef::Named("KeywordAlgorithm".to_string()),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };

    let mut out = String::new();
    emit_decoder_init(&SwiftMapper, &[&field], &HashSet::new(), &mut out);

    assert!(
        out.contains("self.algorithm = try container.decode(KeywordAlgorithm.self, forKey: .algorithm)"),
        "an enum with no zero-arg initializer must stay a required decode:\n{out}"
    );
    assert!(
        !out.contains("decodeIfPresent"),
        "there is no safe fallback value to guess for a feature-gated enum default:\n{out}"
    );
}

/// `compute_zero_arg_constructible_names` must admit a struct whose every field already defaults
/// (the `NgramRange` shape: two `usize` fields with literal defaults) and reject one with a
/// required field (nothing to put after `??` in the memberwise init, so the type has no bare
/// `TypeName()` constructor).
#[test]
fn zero_arg_constructible_names_admits_fully_defaulted_structs_only() {
    let api = ApiSurface {
        types: vec![
            TypeDef {
                name: "NgramRange".to_string(),
                rust_path: "demo::NgramRange".to_string(),
                has_serde: true,
                fields: vec![
                    FieldDef {
                        name: "min".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Usize),
                        typed_default: Some(DefaultValue::IntLiteral(1)),
                        ..Default::default()
                    },
                    FieldDef {
                        name: "max".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::Usize),
                        typed_default: Some(DefaultValue::IntLiteral(3)),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            TypeDef {
                name: "YakeParams".to_string(),
                rust_path: "demo::YakeParams".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "window_size".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Usize),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let known_dto_names: HashSet<String> = ["NgramRange".to_string(), "YakeParams".to_string()]
        .into_iter()
        .collect();

    let admitted = compute_zero_arg_constructible_names(&api, &known_dto_names);

    assert!(
        admitted.contains("NgramRange"),
        "every field defaults, so the memberwise init takes zero arguments: {admitted:?}"
    );
    assert!(
        !admitted.contains("YakeParams"),
        "`window_size` has no default, so `YakeParams()` would not compile: {admitted:?}"
    );
}
