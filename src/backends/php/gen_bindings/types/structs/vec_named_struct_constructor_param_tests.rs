//! `php_field_can_be_constructor_param`'s `Vec` arm: `Vec<Named>` of a plain (non-opaque,
//! non-enum, non-untagged-enum) struct is now a real constructor parameter too, decoded from a
//! `&ext_php_rs::types::ZendHashTable` element-by-element (`gen_php_function_params` in
//! `helpers/params.rs`, and the `php_vec_named_struct_let_binding.jinja` let-binding
//! `gen_struct_methods_impl` already emits for it). That decode is fallible per element, so a
//! constructor accepting such a field must return `PhpResult<Self>` instead of the ordinarily
//! infallible bare `Self` -- these tests pin both the predicate itself and that return-type
//! wrapping end to end. Every positive case is paired with a negative control using the exact
//! element type the predicate still excludes (an untagged data enum), and every new-machinery
//! assertion is paired with a control proving the OLD machinery (bare `Self`, no fallible
//! conversion) still applies when nothing forces `PhpResult`.

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

fn names(values: &[&str]) -> AHashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn mapper() -> PhpMapper {
    PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    }
}

/// The genuinely new-behaviour case: before this widening, `Vec<Named>` was representable only
/// when the element was opaque or an enum (`opaque_types.contains(name) ||
/// enum_names.contains(name)`), so a plain struct element answered `false`. This is the one
/// assertion in this module where the boolean itself flips old-code-vs-new-code -- every other
/// `Vec` case here (opaque, enum, untagged) already answered the same before and after, so THIS
/// is the case that actually proves the widened arm exists.
#[test]
fn vec_of_plain_struct_is_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Vec(Box::new(TypeRef::Named("Definition".to_string()))),
        &AHashSet::new(),
        &AHashSet::new(),
        &AHashSet::new(),
    ));
}

/// Negative control: an untagged data enum element has no `#[php_class]` mirror to decode a
/// `ZendHashTable` entry into (it lowers to `serde_json::Value`), so `Vec<UntaggedEnum>` must
/// stay excluded even though a plain-struct element is now accepted.
#[test]
fn vec_of_untagged_data_enum_stays_unrepresentable() {
    assert!(!php_field_can_be_constructor_param(
        &TypeRef::Vec(Box::new(TypeRef::Named("Payload".to_string()))),
        &AHashSet::new(),
        &AHashSet::new(),
        &names(&["Payload"]),
    ));
}

/// Regression controls: `Vec<opaque>` and `Vec<enum>` were ALREADY representable before this
/// widening and must remain so -- these do not distinguish old code from new code (both answer
/// `true`), they only guard against the widening accidentally narrowing what already worked.
#[test]
fn vec_of_opaque_type_stays_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Vec(Box::new(TypeRef::Named("Client".to_string()))),
        &AHashSet::new(),
        &names(&["Client"]),
        &AHashSet::new(),
    ));
}

#[test]
fn vec_of_enum_type_stays_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Vec(Box::new(TypeRef::Named("Mode".to_string()))),
        &names(&["Mode"]),
        &AHashSet::new(),
        &AHashSet::new(),
    ));
}

/// End-to-end regression pin, shaped after the real defect this widening fixes
/// (`<core>::ChunkClassificationConfig.definitions: Vec<ChunkClassificationDefinition>`): a
/// required `Vec<Named>` field of a plain struct must
///   - render its param as `&ext_php_rs::types::ZendHashTable` (not the bare struct's own type),
///   - decode through a `{param}_core` let-binding containing a fallible `return Err(...)`,
///   - wrap the constructor's return type in `PhpResult<Self>`, and
///   - build the final value through `Ok(Self { .. })`, referencing the `_core` local, not the
///     raw `&ZendHashTable` parameter directly (which is not the field's type and would not
///     compile).
#[test]
fn required_vec_of_plain_struct_field_wraps_constructor_in_php_result_and_uses_core_binding() {
    let typ = TypeDef {
        name: "ChunkClassificationConfig".to_string(),
        rust_path: "test_lib::ChunkClassificationConfig".to_string(),
        fields: vec![
            field("prompt_template", TypeRef::String, false),
            field(
                "definitions",
                TypeRef::Vec(Box::new(TypeRef::Named("Definition".to_string()))),
                false,
            ),
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
        ctor_only.contains("definitions: &ext_php_rs::types::ZendHashTable"),
        "a Vec<Named> of a plain struct must decode from a ZendHashTable reference, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains(") -> PhpResult<Self>"),
        "a fallible Vec<Named> element conversion must wrap the constructor in PhpResult, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains("let definitions_core"),
        "the ZendHashTable must be decoded through a `_core` let-binding, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains("return Err("),
        "an inconvertible array element must be refused at runtime, not silently dropped, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains("Ok(Self {") && ctor_only.contains("definitions: definitions_core"),
        "the final value must be built from the decoded `_core` local inside `Ok(..)`, got:\n{ctor_only}"
    );
    assert!(
        !ctor_only.contains("definitions: definitions }") && !ctor_only.contains("definitions: definitions,"),
        "the raw &ZendHashTable parameter must never be assigned directly into the Vec<Definition> field, got:\n{ctor_only}"
    );
}

/// Negative control for the `PhpResult` wrapping. A struct made ENTIRELY of prop-scalar fields
/// (including a `Vec<Named>` of an ENUM element, which round-trips through `String` and cannot
/// fail) never reaches the `needs_php_result`-computing branch at all -- `has_named_params`
/// (unaffected by this widening, still keyed on `is_php_prop_scalar_with_enums`) routes it
/// through the OLD all-fields-are-params "plain" constructor path instead, which hardcodes bare
/// `Self` and was never touched by this change. That path would trivially "pass" this control
/// even with `needs_php_result` miscomputed as unconditionally `true`, so it proves nothing about
/// the new code (confirmed by deliberately forcing `needs_php_result = true` in `structs.rs` and
/// observing this exact shape stay green). A required nested struct field (`profile`) is added so
/// `has_named_params` is true and generation routes through the SAME per-field-filtered branch
/// `required_vec_of_plain_struct_field_wraps_constructor_in_php_result_and_uses_core_binding`
/// exercises -- the only branch `needs_php_result` can affect -- while `tags` alone still
/// contributes no fallible conversion.
#[test]
fn vec_of_enum_field_does_not_force_php_result() {
    let typ = TypeDef {
        name: "TagHolder".to_string(),
        rust_path: "test_lib::TagHolder".to_string(),
        fields: vec![
            field("path", TypeRef::String, false),
            field("profile", TypeRef::Named("Outcome".to_string()), false),
            field(
                "tags",
                TypeRef::Vec(Box::new(TypeRef::Named("Mode".to_string()))),
                false,
            ),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let mapper = PhpMapper {
        enum_names: names(&["Mode"]),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper,
        true,
        "test_lib",
        &AHashSet::new(),
        &names(&["Mode"]),
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

    // Sanity check that this DID route through the per-field-filtered branch (the one
    // `needs_php_result` lives in), not the plain all-fields-are-params branch -- the nested
    // struct field is by-reference-and-cloned only in the former.
    assert!(
        ctor_only.contains("profile: &Outcome") && ctor_only.contains("profile: profile.clone()"),
        "test setup must route through the per-field-filtered branch, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains(") -> Self {") && !ctor_only.contains("PhpResult"),
        "a constructor with no fallible Vec<Named> conversion must keep the bare Self return \
         type even though it DID route through the branch that can add PhpResult, got:\n{ctor_only}"
    );
    assert!(
        !ctor_only.contains("return Err("),
        "must never contain a fallible return when nothing forces one, got:\n{ctor_only}"
    );
}

/// Same pin as the required case, for the `Option<Vec<Named>>` shape: `Option<&ZendHashTable>`
/// param, an `Option<Vec<..>>`-typed `_core` local, and the same `PhpResult` wrapping.
#[test]
fn optional_vec_of_plain_struct_field_also_wraps_constructor_in_php_result() {
    let typ = TypeDef {
        name: "ChunkClassificationConfig".to_string(),
        rust_path: "test_lib::ChunkClassificationConfig".to_string(),
        fields: vec![
            field("prompt_template", TypeRef::String, false),
            field(
                "definitions",
                TypeRef::Vec(Box::new(TypeRef::Named("Definition".to_string()))),
                true,
            ),
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
        ctor_only.contains("definitions: Option<&ext_php_rs::types::ZendHashTable>"),
        "an optional Vec<Named> of a plain struct must decode from Option<&ZendHashTable>, got:\n{ctor_only}"
    );
    assert!(ctor_only.contains(") -> PhpResult<Self>"), "got:\n{ctor_only}");
    assert!(
        ctor_only.contains("let definitions_core: Option<Vec<"),
        "got:\n{ctor_only}"
    );
}

/// A constructor's `Vec<Named>` let-binding must collect the **binding** element type, not the
/// core one.
///
/// `php_vec_named_struct_let_binding.jinja` is shared with the function-argument path in
/// `helpers/params.rs` and `functions/params.rs`, where the decoded vector is handed straight to a
/// core API call and so must be `Vec<{core_import}::T>` built with `parsed.clone().into()`. A
/// constructor is the opposite direction: it initializes a `#[php_class]` field, whose declared
/// type is the *binding* `T`. Emitting the core type here produced `expected T, found
/// {core_import}::T` for every such field -- 91 of them in one real consumer -- and for a
/// binding-only element type (no core counterpart at all) it produced `cannot find type` instead.
/// The template takes a `to_core` flag for exactly this split; this test pins the constructor
/// side of it. ~keep
#[test]
fn constructor_vec_of_plain_struct_collects_the_binding_element_type() {
    let typ = TypeDef {
        name: "Attributes".to_string(),
        rust_path: "test_lib::Attributes".to_string(),
        fields: vec![field(
            "key_values",
            TypeRef::Vec(Box::new(TypeRef::Named("KeyValueAttribute".to_string()))),
            false,
        )],
        has_serde: true,
        ..Default::default()
    };

    let out = super::gen_struct_methods_with_exclude(
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

    assert!(
        out.contains("keyValues_core_result: Vec<KeyValueAttribute>"),
        "the constructor initializes a binding-typed field, so its let-binding must collect \
         `Vec<KeyValueAttribute>`:\n{out}"
    );
    assert!(
        !out.contains("Vec<test_lib::KeyValueAttribute>"),
        "collecting the core element type does not type-check against the binding field:\n{out}"
    );
    assert!(
        !out.contains("parsed.clone().into()"),
        "a binding-typed element needs no conversion; `.into()` targets the core type:\n{out}"
    );
}
