//! `php_field_can_be_constructor_param`'s `Json` arm: a bare `serde_json::Value` field is now a
//! real constructor parameter too, symmetric with how a Json field's GETTER already returns
//! `Option<String>` (serialized JSON) because `serde_json::Value` has no ext-php-rs `FromZval`
//! impl (`ty_is_or_wraps_json` in `types.rs`). The constructor takes the SAME JSON `String` shape
//! and decodes it back with `serde_json::from_str`, which is fallible on malformed input --
//! unlike the `Vec<Named>` per-element decode, there is no template involved: the decode
//! expression lives directly in `representable_field_init` (`constructor_init.rs`).
//!
//! Modelled after the real trigger for this widening:
//! `<core>::StructuredExtractionConfig.schema: serde_json::Value` (required) and
//! `<core>::OcrPipelineStage.paddle_ocr_config: Option<serde_json::Value>` (optional).

use super::*;
use crate::backends::php::type_map::PhpMapper;

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

fn mapper() -> PhpMapper {
    PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    }
}

/// The genuinely new-behaviour case: before this widening, bare `Json` fell through to
/// `is_php_prop_scalar_with_enums`'s `TypeRef::Named(_) | TypeRef::Json | TypeRef::Bytes |
/// TypeRef::Unit => false` arm and answered `false` unconditionally -- there was no other arm
/// that could also produce `true` here (unlike the enum-named case elsewhere in this test suite),
/// so this assertion directly proves the new `TypeRef::Json => true` arm exists.
#[test]
fn bare_json_is_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Json,
        &AHashSet::new(),
        &AHashSet::new(),
        &AHashSet::new(),
    ));
}

/// `TypeRef::Optional` already recurses into the `Json` arm above, so `Option<Json>` (in the
/// extractor's unwrapped-optional convention: `optional: true`, bare `ty: Json`) needs no
/// separate arm -- this pins that the recursion path specifically still reaches `true` for Json,
/// not just for the already-covered `Named`/`Vec` cases.
#[test]
fn optional_json_is_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Optional(Box::new(TypeRef::Json)),
        &AHashSet::new(),
        &AHashSet::new(),
        &AHashSet::new(),
    ));
}

/// End-to-end regression pin, shaped after `StructuredExtractionConfig.schema`: a required `Json`
/// field must
///   - render its param as an owned `String` (matching the getter's own return shape, and
///     `gen_php_function_params`'s pre-existing `Json` arm),
///   - decode it via `serde_json::from_str`, `?`-propagating a parse failure rather than silently
///     defaulting to `Value::Null` (the fabrication this whole feature refuses elsewhere),
///   - wrap the constructor's return type in `PhpResult<Self>` for that `?` to type-check, and
///   - never mention `Default::default()` for this field.
#[test]
fn required_json_field_wraps_constructor_in_php_result_and_propagates_parse_errors() {
    let typ = TypeDef {
        name: "StructuredExtractionConfig".to_string(),
        rust_path: "test_lib::StructuredExtractionConfig".to_string(),
        fields: vec![
            field("schema_name", TypeRef::String, false),
            field("schema", TypeRef::Json, false),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper(),
        true,
        "test_lib",
        &AHashSet::new(),
        &AHashSet::new(),
        &[],
        &[],
        &AHashSet::new(),
        &[],
        &AHashSet::new(),
        &[],
    )
    .expect("struct methods generate");

    let new_fn = out
        .split("#[php(constructor)]")
        .nth(1)
        .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
    let ctor_only = new_fn
        .split("\n\n")
        .next()
        .unwrap_or_else(|| panic!("constructor body has no blank-line terminator:\n{new_fn}"));

    assert!(
        ctor_only.contains("schema: String") && !ctor_only.contains("schema: &"),
        "a required Json field must be an owned String param, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains(") -> PhpResult<Self>"),
        "a fallible Json decode must wrap the constructor in PhpResult, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains("schema: serde_json::from_str(&schema).map_err(")
            && ctor_only.contains("PhpException::default(e.to_string()))?"),
        "malformed JSON must be refused via a propagated parse error, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains("Ok(Self {"),
        "the final value must be built inside Ok(..), got:\n{ctor_only}"
    );
    assert!(
        !ctor_only.contains("schema: Default::default()") && !ctor_only.contains("unwrap_or_default"),
        "a representable field must never silently fabricate Value::Null, got:\n{ctor_only}"
    );
}

/// Same pin as the required case, for `Option<Json>` (`OcrPipelineStage.paddle_ocr_config`):
/// `Option<String>` param, and the decode chain that keeps `None` on an omitted value while still
/// propagating a parse error for a PROVIDED-but-malformed one, rather than silently collapsing a
/// bad string into `None` (which `.ok()`-swallowing would do, and which this arm deliberately
/// does not use).
#[test]
fn optional_json_field_also_wraps_constructor_in_php_result() {
    let typ = TypeDef {
        name: "OcrPipelineStage".to_string(),
        rust_path: "test_lib::OcrPipelineStage".to_string(),
        fields: vec![
            field("engine", TypeRef::String, false),
            field("paddle_ocr_config", TypeRef::Json, true),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper(),
        true,
        "test_lib",
        &AHashSet::new(),
        &AHashSet::new(),
        &[],
        &[],
        &AHashSet::new(),
        &[],
        &AHashSet::new(),
        &[],
    )
    .expect("struct methods generate");

    let new_fn = out
        .split("#[php(constructor)]")
        .nth(1)
        .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
    let ctor_only = new_fn
        .split("\n\n")
        .next()
        .unwrap_or_else(|| panic!("constructor body has no blank-line terminator:\n{new_fn}"));

    assert!(
        ctor_only.contains("paddleOcrConfig: Option<String>"),
        "an optional Json field must be Option<String>, got:\n{ctor_only}"
    );
    assert!(ctor_only.contains(") -> PhpResult<Self>"), "got:\n{ctor_only}");
    assert!(
        ctor_only.contains("paddle_ocr_config: paddleOcrConfig.map(|s| serde_json::from_str(&s)).transpose().map_err(")
            && ctor_only.contains("PhpException::default(e.to_string()))?"),
        "must decode via a fallible transpose chain, not silently swallow a parse error, got:\n{ctor_only}"
    );
    assert!(
        !ctor_only.contains(".ok())") && !ctor_only.contains("unwrap_or_default"),
        "a provided-but-malformed value must never be silently collapsed, got:\n{ctor_only}"
    );
}

/// Negative control for the `PhpResult` wrapping, mirroring the equivalent Vec<Named> control:
/// an all-infallible constructor (no Json, no fallible Vec<Named>) must keep the bare `Self`
/// return type even when routed through the SAME per-field-filtered branch that can add
/// `PhpResult`. Without this, `needs_php_result` could be miscomputed as unconditionally `true`
/// and the tests above would still pass for the wrong reason.
#[test]
fn struct_without_json_field_keeps_bare_self_return() {
    let typ = TypeDef {
        name: "PlainSchemaHolder".to_string(),
        rust_path: "test_lib::PlainSchemaHolder".to_string(),
        fields: vec![
            field("schema_name", TypeRef::String, false),
            field("profile", TypeRef::Named("Outcome".to_string()), false),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper(),
        true,
        "test_lib",
        &AHashSet::new(),
        &AHashSet::new(),
        &[],
        &[],
        &AHashSet::new(),
        &[],
        &AHashSet::new(),
        &[],
    )
    .expect("struct methods generate");

    let new_fn = out
        .split("#[php(constructor)]")
        .nth(1)
        .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
    let ctor_only = new_fn
        .split("\n\n")
        .next()
        .unwrap_or_else(|| panic!("constructor body has no blank-line terminator:\n{new_fn}"));

    assert!(
        ctor_only.contains("profile: &Outcome") && ctor_only.contains("profile: profile.clone()"),
        "test setup must route through the per-field-filtered branch, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains(") -> Self {") && !ctor_only.contains("PhpResult"),
        "a constructor with no Json/fallible field must keep the bare Self return type, got:\n{ctor_only}"
    );
}
