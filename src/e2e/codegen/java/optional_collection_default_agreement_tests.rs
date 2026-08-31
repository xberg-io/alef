//! Pinning coverage: the Java e2e *test* emitter and the Java *binding* emitter must agree on
//! what an absent collection field defaults to.
//!
//! Two independent call sites materialize a Java DTO's absent collection field: Jackson's
//! `@JsonPOJOBuilder` deserializer, and any hand-written `TypeName.builder()...build()` call
//! that never sets the field -- both read the *same* generated `Builder` class
//! (`backends::java::gen_bindings::types::builders::gen_builder_nested_class`), whose
//! `field_is_optional_in_binding` branch defaults to `null` only when the IR's `field.optional`
//! is true. A non-optional `Vec<T>` field carrying `#[serde(default, skip_serializing_if =
//! "Vec::is_empty")]` -- never actually `Option` in Rust -- always defaults to `List.of()`
//! there instead, by design (see that function's own `~keep` comment: defaulting such a field
//! to `null` regressed to a `NullPointerException` on `.isEmpty()` elsewhere).
//!
//! Before this fix, the e2e *test* emitter did not ask that same question: an `equals`
//! assertion carrying a literal JSON `null` against such a field rendered a doomed
//! `assertEquals(null, result.imports())` -- a comparison the real binding can never satisfy,
//! because it never returns `null` for that field. `render_assertion` now recognizes an
//! `equals`-vs-`null` assertion on a field the IR proves is both a collection
//! (`is_collection_root`) and non-optional (`!is_optional`) and asserts the same emptiness the
//! binding's default actually produces instead.
//!
//! These tests drive both real entry points -- `render_test_method` (the test emitter) and
//! `gen_record_type` (the binding emitter, re-exported for tests as
//! `test_only_gen_record_type`) -- from ONE shared `TypeDef` fixture, so a future change that
//! moves either side out of step is caught here rather than by a live consumer's Maven build.

use super::test_method::render_test_method;
use crate::backends::java::gen_bindings::test_only_gen_record_type;
use crate::core::config::{JavaBuilderMode, ResolvedCrateConfig};
use crate::core::ir::{DefaultValue, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use ahash::AHashSet;
use std::collections::HashSet;

/// `data: Option<String>` -- the task's control case: a genuinely optional, non-collection
/// field. Its absent-value binding default really is `null`, so the test emitter's `equals:
/// null` handling must stay untouched for it.
fn data_field() -> FieldDef {
    FieldDef {
        name: "data".to_string(),
        ty: TypeRef::String,
        optional: true,
        ..FieldDef::default()
    }
}

/// `imports: Vec<String>` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]` --
/// never `Option` in Rust. This is the shape the defect is about: the wire key can be absent,
/// but the Rust type can never be `None`.
fn required_collection_with_skip_if_field() -> FieldDef {
    FieldDef {
        name: "imports".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        optional: false,
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::Empty),
        serde_skip_serializing_if: true,
        serde_skip: false,
        ..FieldDef::default()
    }
}

/// `imports: Option<Vec<String>>` -- genuinely optional collection. Negative control: the
/// binding's own default for this shape already is `null` (no `~keep` override applies), so an
/// `equals: null` assertion against it is not doomed and must keep rendering the literal
/// null-equality check.
fn genuinely_optional_collection_field() -> FieldDef {
    FieldDef {
        name: "imports".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        optional: true,
        ..FieldDef::default()
    }
}

fn process_result_type(imports_field: FieldDef) -> TypeDef {
    TypeDef {
        name: "ProcessResult".to_string(),
        has_serde: true,
        fields: vec![data_field(), imports_field],
        ..TypeDef::default()
    }
}

/// Render the real Java binding for `typ` through the real entry point, `gen_record_type` --
/// the exact function `alef build` calls for every record type.
fn render_binding(typ: &TypeDef) -> String {
    test_only_gen_record_type(
        "dev.sample.bindings",
        typ,
        &AHashSet::new(),
        &AHashSet::new(),
        "",
        &[],
        "",
        JavaBuilderMode::Always,
        &ahash::AHashMap::new(),
        &AHashSet::new(),
        &HashSet::new(),
    )
}

/// A minimal, fully-populated fixture (`Fixture` has no `Default` impl) asserting `equals:
/// null` on `field`.
fn equals_null_fixture(id: &str, field: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::Null),
            ..Assertion::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Render `fixture`'s assertions through the real entry point, `render_test_method`, against a
/// call whose declared return type is `ProcessResult` (`type_defs`'s single entry).
fn render_test(typ: &TypeDef, fixture: &Fixture) -> String {
    let type_defs = vec![typ.clone()];
    let functions = vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("ProcessResult".to_string()),
        ..FunctionDef::default()
    }];
    let call = CallConfig {
        function: "process".to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = ResolvedCrateConfig::default();

    let mut out = String::new();
    render_test_method(
        &mut out,
        fixture,
        "SampleClass",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        false,
        &[],
        &config,
        &type_defs,
        &[],
        &functions,
        &[],
    );
    out
}

/// The pinning test: for a non-optional `Vec<T>` field with `#[serde(default,
/// skip_serializing_if)]`, the real binding's default is `List.of()` (never `null`), and the
/// real test emitter must agree -- never assert the field equals `null`.
#[test]
fn required_collection_field_test_and_binding_agree_absent_means_empty_not_null() {
    let typ = process_result_type(required_collection_with_skip_if_field());

    let binding = render_binding(&typ);
    assert!(
        binding.contains("private List<String> imports = List.of();"),
        "expected the binding's Builder to default a non-optional, serde-defaulted Vec field \
         to List.of(), got:\n{binding}"
    );

    let fixture = equals_null_fixture("imports_equals_null", "imports");
    let test = render_test(&typ, &fixture);
    assert!(
        !test.contains("assertEquals(null,"),
        "the test emitter must not assert a field equals null when the binding it is testing \
         can never produce null for that field, got:\n{test}"
    );
    assert!(
        test.contains(".isEmpty()"),
        "expected the test to assert emptiness (matching the binding's List.of() default) \
         instead, got:\n{test}"
    );
}

/// Negative control: a genuinely optional `Option<Vec<T>>` field's binding default really is
/// `null` (unlike the serde-defaulted case above) -- the test emitter's `equals: null` handling
/// must NOT be redirected for it.
#[test]
fn genuinely_optional_collection_field_keeps_null_equality() {
    let typ = process_result_type(genuinely_optional_collection_field());

    let binding = render_binding(&typ);
    assert!(
        !binding.contains("private List<String> imports = List.of();"),
        "a genuinely optional (Option<Vec<T>>) field's Builder default must stay null (no \
         eager initializer), got:\n{binding}"
    );

    let fixture = equals_null_fixture("imports_equals_null_optional", "imports");
    let test = render_test(&typ, &fixture);
    assert!(
        test.contains("assertEquals(null,"),
        "a genuinely optional collection field's equals-null assertion must be left as a \
         literal null comparison -- the binding really can produce null here, got:\n{test}"
    );
}

/// Control case from the task: `data: Option<String>` is non-collection, so it must be
/// entirely unaffected by this fix regardless of which `imports` shape shares the type.
#[test]
fn non_collection_optional_field_is_unaffected() {
    let typ = process_result_type(required_collection_with_skip_if_field());
    let fixture = equals_null_fixture("data_equals_null", "data");
    let out = render_test(&typ, &fixture);

    assert!(
        out.contains("data"),
        "expected an assertion referencing the data field to be rendered at all, got:\n{out}"
    );
    assert!(
        !out.contains("binding never returns null for a non-optional collection"),
        "the new collection-defaulting branch must never fire for a non-collection field -- \
         `data` is `TypeRef::String`, not a Vec/Map, so `is_collection_root` must answer false \
         and this fixture must render through the pre-existing, untouched code path, got:\n{out}"
    );
}
