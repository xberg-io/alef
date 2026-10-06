//! WASM tagged-enum conversion coverage, split from tests.rs.

use super::tests::*;
use super::{gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

#[test]
fn gen_tagged_enum_core_to_binding_hidden_variant_uses_safe_default() {
    let mut e = make_tagged_tuple_enum();
    e.excluded_variants.push(EnumVariant {
        serde_untagged: false,
        name: "Internal".to_string(),
        fields: vec![],
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: true,
        binding_exclusion_reason: Some("not part of the public binding".to_string()),
        is_tuple: false,
        sensitive: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    });

    let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");

    assert!(result.contains("_ => Self::default(),"));
    assert!(!result.contains("panic!("));
}

/// Regression: tagged struct variants whose source field type is already `Option<T>`
/// must preserve that option layer. The flat wasm struct stores every variant field as
/// `Option<T>`; wrapping an already-optional core field in `Some(...)` produces
/// `Option<Option<T>>`, and unwrapping it in the reverse direction produces `T`.
#[test]
fn gen_tagged_enum_struct_variant_preserves_optional_fields() {
    let field = |name: &str, ty: TypeRef, optional: bool| FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: crate::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let e = EnumDef {
        name: "SecuritySchemeInfo".to_string(),
        rust_path: "test_lib::SecuritySchemeInfo".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            serde_untagged: false,
            name: "Http".to_string(),
            fields: vec![
                field("scheme", TypeRef::String, false),
                field("bearer_format", TypeRef::String, true),
            ],
            doc: String::new(),
            is_default: false,
            serde_rename: Some("http".to_string()),
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            sensitive: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let binding_to_core = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains("bearer_format: val.bearer_format.clone()"),
        "binding→core must preserve Option<String>;\nactual:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("bearer_format: val.bearer_format.clone().unwrap_or_default()"),
        "binding→core must not unwrap source Option<String>;\nactual:\n{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
    // The destructured local already carries the field's value unchanged, so the initializer is
    // field-init shorthand: `bearer_format: bearer_format` is `clippy::redundant_field_names`,
    // denied in a consumer building generated bindings under `-D warnings`. ~keep
    assert!(
        core_to_binding.contains("                bearer_format,"),
        "core→binding must pass Option<String> through as field-init shorthand;\nactual:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("bearer_format: bearer_format"),
        "core→binding must not emit a redundant field name;\nactual:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("bearer_format: Some(bearer_format)"),
        "core→binding must not create Option<Option<String>>;\nactual:\n{core_to_binding}"
    );
}

/// Regression: tuple-variant enums with positional `_0` fields must not emit
/// `set__0` as the setter name — that double-underscore form is rejected by the
/// `non_snake_case` lint under `RUSTFLAGS="-D warnings"`.  The generated Rust
/// identifier must be `set_field_0` (getter: `field_0`) while the JS-visible
/// name is controlled by `js_name` and remains unchanged.
///
/// Adjacently tagged, because an INTERNALLY tagged newtype variant no longer gets a positional
/// accessor at all — serde flattens its payload, so the `"0"` key this test pins is one no wire
/// form of that enum carries (see
/// `should_not_emit_a_positional_js_accessor_for_a_flattened_newtype_variant`). Adjacent tagging
/// keeps a real key for the payload, so it is where the naming rule still has something to name.
#[test]
fn gen_tagged_enum_as_struct_positional_field_setter_snake_case() {
    use super::gen_tagged_enum_as_struct;

    let e = EnumDef {
        serde_content: Some("payload".to_string()),
        ..make_tagged_tuple_enum()
    };
    let result = gen_tagged_enum_as_struct(&e, &mapper("Wasm"), &[], &[]);

    assert!(
        !result.contains("fn set__0("),
        "must not emit `set__0` — double-underscore violates non_snake_case lint;\nactual:\n{result}"
    );

    assert!(
        result.contains("fn field_0("),
        "getter for positional `_0` field must be named `field_0`;\nactual:\n{result}"
    );

    assert!(
        result.contains("fn set_field_0("),
        "setter for positional `_0` field must be named `set_field_0`;\nactual:\n{result}"
    );

    assert!(
        result.contains("js_name = \"0\""),
        "js_name attribute must use the to_node_name result for `_0` field;\nactual:\n{result}"
    );

    assert!(
        result.contains("self._0"),
        "getter/setter body must access `self._0` (the struct field);\nactual:\n{result}"
    );
}

/// Regression test D4-WASM-A: tagged enum with unit variant emits { kind: 'bold' }
/// as a tagged-union type alias, not a numeric enum.
#[test]
fn gen_tagged_enum_unit_variant_emits_tagged_union() {
    use super::gen_tagged_enum_as_struct;

    let mut e = make_tagged_tuple_enum();
    e.variants[0].fields.clear();
    e.variants[0].is_tuple = false;

    let result = gen_tagged_enum_as_struct(&e, &mapper("Wasm"), &[], &[]);

    // Must emit a #[wasm_bindgen] struct with a discriminator field ("kind" or similar).
    assert!(
        result.contains("#[wasm_bindgen]") && result.contains("pub struct Wasm"),
        "WASM tagged enum must emit wasm_bindgen struct, not numeric enum;\nactual:\n{result}"
    );

    assert!(
        result.contains("pub(crate)") && (result.contains("kind") || result.contains("getter")),
        "WASM tagged enum struct must have a discriminator field for the tag;\nactual:\n{result}"
    );
}

/// Regression test D4-WASM-B: tagged enum variant tag values use camelCase.
/// E.g., `"fontSize"` not `"font_size"`.
#[test]
fn gen_tagged_enum_binding_to_core_matches_camel_case_tags() {
    use super::gen_tagged_enum_binding_to_core;

    let e = make_tagged_tuple_enum();
    let result = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");

    assert!(
        result.contains("match val.") && result.contains("as_str()"),
        "binding→core must dispatch on tag field string value;\nactual:\n{result}"
    );
}

/// Build a serde-tagged enum with two *named-field* (struct-like) variants that share a field
/// name at different `Named` types (`model`), alongside a field whose type is identical in both
/// (`meta`). `mixed_type_fields` degrades only `model` to `Option<JsValue>`, so one generated
/// enum carries both representations at once — the exact shape that exposes an emitter deciding
/// a conversion from the `TypeRef` instead of the binding type.
fn make_tagged_struct_enum_with_mixed_field() -> EnumDef {
    let make_variant = |variant_name: &str, tag: &str, model_ty: &str| EnumVariant {
        serde_untagged: false,
        name: variant_name.to_string(),
        sensitive: false,
        fields: vec![
            FieldDef {
                name: "model".to_string(),
                ty: TypeRef::Named(model_ty.to_string()),
                ..Default::default()
            },
            FieldDef {
                name: "meta".to_string(),
                ty: TypeRef::Named("MetaInfo".to_string()),
                ..Default::default()
            },
        ],
        is_tuple: false,
        doc: String::new(),
        is_default: false,
        serde_rename: Some(tag.to_string()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        originally_had_data_fields: true,
        cfg: None,
        version: Default::default(),
    };

    EnumDef {
        name: "Retrieval".to_string(),
        rust_path: "test_lib::Retrieval".to_string(),
        variants: vec![
            make_variant("Sparse", "sparse", "SparseModelType"),
            make_variant("Dense", "dense", "DenseModelType"),
        ],
        has_serde: true,
        serde_tag: Some("kind".to_string()),
        ..Default::default()
    }
}

/// The binding struct stores a mixed-type variant field as `Option<JsValue>` for *named*-field
/// variants exactly as it does for tuple variants — assert that first, because it is what makes
/// the two `From` impls below wrong if they choose `.into()`.
#[test]
fn gen_tagged_enum_as_struct_degrades_mixed_named_field_to_js_value() {
    use super::gen_tagged_enum_as_struct;

    let e = make_tagged_struct_enum_with_mixed_field();
    let result = gen_tagged_enum_as_struct(&e, &mapper("Wasm"), &[], &[]);

    assert!(
        result.contains("pub(crate) model: Option<JsValue>,"),
        "a field with a different Named type per variant must degrade to Option<JsValue>;\nactual:\n{result}"
    );
    assert!(
        result.contains("pub(crate) meta: Option<WasmMetaInfo>,"),
        "a field with one type across variants must keep its wrapper;\nactual:\n{result}"
    );
}

/// Regression test: `From<core::Enum> for Wasm{Enum}` must bridge a mixed named-variant field
/// through serde. `JsValue` implements no `From<CoreType>`, so `Some(model.into())` — what the
/// `TypeRef::Named` arm writes — is an `E0277` against the `Option<JsValue>` field the same
/// generator declared. The positive control is `meta`, which really does hold a wrapper and must
/// keep using `.into()`.
#[test]
fn gen_tagged_enum_core_to_binding_uses_serde_for_mixed_named_field() {
    use super::gen_tagged_enum_core_to_binding;

    let e = make_tagged_struct_enum_with_mixed_field();
    let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");

    assert!(
        result.contains("model: serde_wasm_bindgen::to_value(&model).ok()"),
        "mixed named-variant field must cross through serde;\nactual:\n{result}"
    );
    assert!(
        !result.contains("model: Some(model.into())"),
        "mixed named-variant field must not use .into() into a JsValue field;\nactual:\n{result}"
    );
    assert!(
        result.contains("meta: Some(meta.into())"),
        "a wrapper-typed field must still use .into();\nactual:\n{result}"
    );
}

/// Regression test: the reverse direction. `val.model.clone().map(Into::into)` asks for
/// `SparseModelType: From<JsValue>`, which does not exist either — the value must be
/// deserialized. `meta` is the positive control for the unchanged wrapper path.
#[test]
fn gen_tagged_enum_binding_to_core_uses_serde_for_mixed_named_field() {
    use super::gen_tagged_enum_binding_to_core;

    let e = make_tagged_struct_enum_with_mixed_field();
    let result = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");

    assert!(
        result.contains("serde_wasm_bindgen::from_value::<test_lib::SparseModelType>")
            && result.contains("serde_wasm_bindgen::from_value::<test_lib::DenseModelType>"),
        "mixed named-variant field must be deserialized per variant;\nactual:\n{result}"
    );
    assert!(
        !result.contains("model: val.model.clone().map(Into::into)"),
        "mixed named-variant field must not use Into::into from a JsValue field;\nactual:\n{result}"
    );
    assert!(
        result.contains("meta: val.meta.clone().map(Into::into).unwrap_or_else"),
        "a wrapper-typed field must still use Into::into;\nactual:\n{result}"
    );
}

/// Internal tagging merges a newtype variant's payload into the tag object, so the wire is
/// `{"role":"user","content":"hi"}` — there is no `"0"` property for a JS caller to read or set.
/// The struct field survives (both `From` impls read `val._0`); only the fabricated accessor goes.
#[test]
fn should_not_emit_a_positional_js_accessor_for_a_flattened_newtype_variant() {
    use super::gen_tagged_enum_as_struct;

    let result = gen_tagged_enum_as_struct(
        &make_tagged_tuple_enum(),
        &mapper("Wasm"),
        &make_tagged_tuple_enum_types(),
        &[],
    );

    assert!(
        !result.contains("js_name = \"0\""),
        "serde writes no `0` key for a flattened newtype payload;\nactual:\n{result}"
    );
    assert!(
        !result.contains("fn field_0(") && !result.contains("fn set_field_0("),
        "no getter/setter may advertise the synthetic positional field;\nactual:\n{result}"
    );
    assert!(
        result.contains("pub(crate) _0: Option<JsValue>,"),
        "the struct field itself must stay — the From impls read it;\nactual:\n{result}"
    );
    assert!(
        result.contains("js_name = \"role\""),
        "the discriminator accessor is unaffected;\nactual:\n{result}"
    );
}

/// The positional accessor is CORRECT wherever serde keeps a real key for the payload. Adjacent
/// tagging is the in-tree case (`DiffLine`); external tagging is the other (`EntityCategory`).
#[test]
fn should_keep_the_positional_js_accessor_for_representations_serde_does_not_flatten() {
    use super::gen_tagged_enum_as_struct;

    let adjacent = EnumDef {
        serde_content: Some("payload".to_string()),
        ..make_tagged_tuple_enum()
    };
    let external = EnumDef {
        serde_tag: None,
        ..make_tagged_tuple_enum()
    };

    for (case, enum_def) in [("adjacent", adjacent), ("external", external)] {
        let result = gen_tagged_enum_as_struct(&enum_def, &mapper("Wasm"), &[], &[]);
        assert!(
            result.contains("js_name = \"0\"") && result.contains("fn field_0("),
            "{case} tagging keeps a real key for the payload;\nactual:\n{result}"
        );
    }
}

/// `make_tagged_tuple_enum` (internal tag, every variant a flattened newtype) has no honest
/// field name for a discriminator struct to expose -- every variant's `_0` collapses onto one
/// shared, unusable slot (see `mixed_type_fields`). It must route through the JsValue bridge
/// (`is_json_passthrough_data_enum`), NOT the discriminator-struct emitter
/// (`is_tagged_data_enum`).
#[test]
fn fully_flattened_internal_enum_is_json_passthrough_not_a_tagged_data_enum() {
    let e = make_tagged_tuple_enum();
    let types = make_tagged_tuple_enum_types();
    assert!(super::is_fully_flattened_internal_enum(&e, &types));
    assert!(
        !super::is_tagged_data_enum(&e, &types),
        "a fully-flattened internal enum must not claim the discriminator-struct shape"
    );
    assert!(
        super::is_json_passthrough_data_enum(&e, &types),
        "a fully-flattened internal enum must claim the no-nominal-type JsValue bridge"
    );
}

/// Adjacent and external tagging never flatten a variant's payload beside the discriminator --
/// the payload keeps a real key (`content`, or the variant name itself) -- so both must keep
/// claiming the discriminator-struct shape, exactly as before this fix.
#[test]
fn adjacent_and_external_tagging_keep_the_tagged_data_enum_shape() {
    let adjacent = EnumDef {
        serde_content: Some("payload".to_string()),
        ..make_tagged_tuple_enum()
    };
    let external = EnumDef {
        serde_tag: None,
        ..make_tagged_tuple_enum()
    };

    for (case, enum_def) in [("adjacent", adjacent), ("external", external)] {
        assert!(
            !super::is_fully_flattened_internal_enum(&enum_def, &[]),
            "{case} tagging must not be classified as fully-flattened-internal"
        );
        assert!(
            super::is_tagged_data_enum(&enum_def, &[]),
            "{case} tagging must keep the discriminator-struct shape"
        );
        assert!(
            !super::is_json_passthrough_data_enum(&enum_def, &[]),
            "{case} tagging must not route through the JsValue bridge"
        );
    }
}

/// A MIXED enum -- one flattened newtype variant plus one struct-field variant with real field
/// names -- must keep its nominal type: only the struct-field variant needs a real accessor, and
/// `gen_tagged_enum_as_struct` already unions it in correctly (`flattened_only_field_names`).
#[test]
fn mixed_flattened_and_struct_variant_enum_keeps_the_tagged_data_enum_shape() {
    let mut e = make_tagged_tuple_enum();
    e.variants.push(EnumVariant {
        name: "Tool".to_string(),
        fields: vec![
            FieldDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            FieldDef {
                name: "arguments".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
        ],
        is_tuple: false,
        ..Default::default()
    });
    let types = make_tagged_tuple_enum_types();

    assert!(!super::is_fully_flattened_internal_enum(&e, &types));
    assert!(
        super::is_tagged_data_enum(&e, &types),
        "a mixed enum must keep the discriminator-struct shape"
    );
    assert!(!super::is_json_passthrough_data_enum(&e, &types));
}

/// A unit-only enum has no data variant to flatten at all, and must be unaffected by either
/// predicate -- it stays a plain `#[wasm_bindgen]` C-style enum.
#[test]
fn unit_only_enum_is_not_fully_flattened_internal() {
    let e = make_enum("Status", &["Active", "Inactive"]);
    assert!(!super::is_fully_flattened_internal_enum(&e, &[]));
    assert!(!super::is_tagged_data_enum(&e, &[]));
    assert!(!super::is_json_passthrough_data_enum(&e, &[]));
}

fn transparent_string_wrapper(containers: Vec<crate::core::ir::NewtypeContainer>) -> String {
    crate::core::ir::NewtypeWrapper::encode_explicit(&[crate::core::ir::NewtypeWrapperMetadata::transparent_string(
        "test_lib::SecretString",
        "from",
        "into_inner",
        containers,
    )])
}

#[test]
fn tagged_enum_transparent_string_payload_converts_both_directions_and_rejects_unknown_tag() {
    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    enum_def.variants[0].fields[0].ty = TypeRef::String;
    enum_def.variants[0].fields[0].newtype_wrapper = Some(transparent_string_wrapper(vec![]));

    let binding_to_core = gen_tagged_enum_binding_to_core(&enum_def, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains("test_lib::SecretString::from(val._0.clone().unwrap_or_else"),
        "{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("wasm_bindgen::throw_str(\"missing enum field _0\")"),
        "missing required wrapper payload must throw: {binding_to_core}"
    );
    assert!(
        binding_to_core.contains("wasm_bindgen::throw_str(\"unknown Message variant\")"),
        "malformed external tags must throw rather than synthesize a wrapper: {binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains("Some((field0).into_inner())"),
        "{core_to_binding}"
    );
}

#[test]
fn tagged_enum_required_named_payload_does_not_require_default() {
    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    let field = &mut enum_def.variants[0].fields[0];
    field.name = "authentication".to_string();
    field.ty = TypeRef::Named("Authentication".to_string());
    field.optional = false;

    let binding_to_core = gen_tagged_enum_binding_to_core(&enum_def, "test_lib", "Wasm");

    assert!(
        binding_to_core.contains("wasm_bindgen::throw_str(\"missing enum field authentication\")"),
        "a missing required payload must throw: {binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("authentication.clone().map(Into::into).unwrap_or_default()"),
        "required named payloads must not impose Default on the core enum: {binding_to_core}"
    );
}

#[test]
fn tagged_enum_nested_transparent_string_payload_uses_jsvalue_without_wrapper_serde() {
    use crate::core::ir::NewtypeContainer::MapValue;

    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    let field = &mut enum_def.variants[0].fields[0];
    field.ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    field.newtype_wrapper = Some(transparent_string_wrapper(vec![MapValue]));

    let declaration = super::gen_tagged_enum_as_struct(&enum_def, &mapper("Wasm"), &[], &[]);
    assert!(declaration.contains("pub(crate) _0: Option<JsValue>"), "{declaration}");

    let binding_to_core = gen_tagged_enum_binding_to_core(&enum_def, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains("serde_wasm_bindgen::from_value::<std::collections::HashMap<String, String>>"),
        "{binding_to_core}"
    );
    assert!(binding_to_core.contains("wasm_bindgen::throw_val"), "{binding_to_core}");
    assert!(
        binding_to_core.contains("test_lib::SecretString::from(value)"),
        "{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains("serde_wasm_bindgen::to_value"),
        "{core_to_binding}"
    );
    assert!(core_to_binding.contains("(value).into_inner()"), "{core_to_binding}");
}

#[test]
fn tagged_enum_required_boxed_transparent_string_payload_composes_boxing() {
    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    let field = &mut enum_def.variants[0].fields[0];
    field.ty = TypeRef::String;
    field.is_boxed = true;
    field.newtype_wrapper = Some(transparent_string_wrapper(vec![]));

    let binding_to_core = gen_tagged_enum_binding_to_core(&enum_def, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains("Box::new(test_lib::SecretString::from(val._0.clone().unwrap_or_else"),
        "{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains("Some((*field0).into_inner())"),
        "{core_to_binding}"
    );
}

#[test]
fn tagged_enum_optional_boxed_transparent_string_payload_composes_boxing() {
    use crate::core::ir::NewtypeContainer::Optional;

    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    let field = &mut enum_def.variants[0].fields[0];
    field.ty = TypeRef::String;
    field.optional = true;
    field.is_boxed = true;
    field.newtype_wrapper = Some(transparent_string_wrapper(vec![Optional]));

    let binding_to_core = gen_tagged_enum_binding_to_core(&enum_def, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains(".map(test_lib::SecretString::from)"),
        "{binding_to_core}"
    );
    assert!(!binding_to_core.contains("map(|value|"), "{binding_to_core}");
    assert!(binding_to_core.contains(".map(Box::new)"), "{binding_to_core}");

    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains("field0.map(|value| *value)"),
        "{core_to_binding}"
    );
    assert!(core_to_binding.contains("(value).into_inner()"), "{core_to_binding}");
}

#[test]
fn tagged_enum_root_optional_transparent_string_payload_uses_constructor_item() {
    use crate::core::ir::NewtypeContainer::Optional;

    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    enum_def.variants[0].is_tuple = false;
    let field = &mut enum_def.variants[0].fields[0];
    field.name = "value".to_string();
    field.ty = TypeRef::String;
    field.optional = true;
    field.newtype_wrapper = Some(transparent_string_wrapper(vec![Optional]));

    let binding_to_core = gen_tagged_enum_binding_to_core(&enum_def, "test_lib", "Wasm");
    assert!(
        binding_to_core.contains("value: (val.value.clone()).map(test_lib::SecretString::from)"),
        "{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("map(|value| test_lib::SecretString::from(value))"),
        "{binding_to_core}"
    );
}

#[test]
fn tagged_enum_optional_complex_wrapper_serializes_only_present_values() {
    use crate::core::ir::NewtypeContainer::{MapValue, Optional};

    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    let field = &mut enum_def.variants[0].fields[0];
    field.ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
    field.optional = true;
    field.newtype_wrapper = Some(transparent_string_wrapper(vec![Optional, MapValue]));

    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains(".and_then(|value| serde_wasm_bindgen::to_value(&value).ok())"),
        "{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("serde_wasm_bindgen::to_value(&(field0"),
        "the outer Option must not be serialized into a present JsValue: {core_to_binding}"
    );
}

#[test]
fn tagged_enum_intrinsic_optional_complex_wrapper_serializes_only_present_values() {
    use crate::core::ir::NewtypeContainer::{MapValue, Optional};

    let mut enum_def = make_tagged_tuple_enum();
    enum_def.variants.truncate(1);
    let field = &mut enum_def.variants[0].fields[0];
    field.ty = TypeRef::Optional(Box::new(TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::String),
    )));
    field.newtype_wrapper = Some(transparent_string_wrapper(vec![Optional, MapValue]));

    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "test_lib", "Wasm");
    assert!(
        core_to_binding.contains(".and_then(|value| serde_wasm_bindgen::to_value(&value).ok())"),
        "{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("serde_wasm_bindgen::to_value(&(field0"),
        "the intrinsic Option must not be serialized into a present JsValue: {core_to_binding}"
    );
}
