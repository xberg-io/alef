//! Regression coverage for the PHP wildcard element accessor's getter-vs-property
//! classification, mirroring `python::assertions::tests::wildcard_typeddict_tests` for the PHP
//! `#[php(getter)]`-vs-`#[php(prop)]` split instead of Python's `TypedDict`-vs-attribute one.
//!
//! The defect (issue #401): the PHP wildcard branch rendered the element half through the
//! generic, cross-language `FieldResolver::element_accessor`, which shares `render_relative_to`
//! with the result-anchored `accessor()` and so starts `render_php_with_getters`'s owner cursor
//! at `php_getter_map.root_type` -- the call's RESULT type -- even for an element-anchored path.
//! A field that is a getter-only method on the collection's ELEMENT type but does not exist (or
//! is scalar) on the result type never classified as a getter, and rendered as a plain property
//! access (`$e->kind`) -- `Undefined property` once the generated binding only exposes `kind` as
//! `getKind()`. `php_element_owner_type` resolves the actual element owner type by walking the
//! array field's own path through `php_getter_map.field_types`, and `php_element_accessor`
//! starts the getter-lookup cursor there instead.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::assertions::render_assertion;
use super::enum_variant_access::PhpVariantAccess;
use crate::e2e::field_access::{FieldResolver, PhpGetterMap};
use crate::e2e::fixture::Assertion;

/// The call's declared result type. Declares its OWN scalar `kind` field alongside `structure` --
/// load-bearing, not incidental: `PhpGetterMap::needs_getter` only consults the owner-type
/// classification when the owner actually declares the field (`all_fields[owner].contains(..)`),
/// and otherwise falls back to a bare-name union across every type that would find `kind` via
/// `StructureItem` regardless of which cursor the caller started from. Only when `ProcessResult`
/// itself declares a same-named (but scalar) `kind` does the owner-cursor bug actually surface a
/// wrong answer instead of accidentally landing on the right one through that fallback -- exactly
/// the same collision shape `render_php_with_getters_distinguishes_same_field_name_on_different_types`
/// covers for the result-anchored renderer.
const ROOT_TYPE: &str = "ProcessResult";

/// The IR type of `structure`'s elements. `kind` is a getter-only field here: the generated PHP
/// binding exposes it as `getKind()`, not as a `->kind` property.
const ELEMENT_TYPE: &str = "StructureItem";

fn getter_map() -> PhpGetterMap {
    let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
    // ProcessResult.kind is scalar -- explicit empty set, so `needs_getter` trusts this
    // classification (owner declares the field) instead of falling back to the bare-name union.
    getters.insert(ROOT_TYPE.to_string(), HashSet::new());
    getters.insert(ELEMENT_TYPE.to_string(), ["kind".to_string()].into_iter().collect());

    let mut all_fields: HashMap<String, HashSet<String>> = HashMap::new();
    all_fields.insert(ELEMENT_TYPE.to_string(), ["kind".to_string()].into_iter().collect());
    all_fields.insert(
        ROOT_TYPE.to_string(),
        ["structure".to_string(), "kind".to_string()].into_iter().collect(),
    );

    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    field_types.insert(
        ROOT_TYPE.to_string(),
        [("structure".to_string(), ELEMENT_TYPE.to_string())]
            .into_iter()
            .collect(),
    );

    PhpGetterMap {
        getters,
        field_types,
        root_type: Some(ROOT_TYPE.to_string()),
        all_fields,
    }
}

fn resolver() -> FieldResolver {
    FieldResolver::new_with_php_getters(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        getter_map(),
    )
}

fn render(resolver: &FieldResolver) -> String {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some("structure[].kind".to_string()),
        value: Some(serde_json::json!("Function")),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        resolver,
        false,
        false,
        &BTreeMap::new(),
        false,
        &PhpVariantAccess::none(),
    );
    out
}

/// ACCEPTANCE: the element half must call the getter method declared on `StructureItem`, not
/// render a bare property access classified against `ProcessResult`.
///
/// Pre-fix this rendered `str_contains((string)$e->kind, "Function")` -- `Undefined property`
/// once the generated PHP binding only exposes `kind` as `getKind()`.
#[test]
fn wildcard_element_getter_is_resolved_against_the_element_type_not_the_result_type() {
    let rendered = render(&resolver());
    assert_eq!(
        rendered,
        "        $this->assertTrue((bool)array_filter($result->structure, fn($e) => str_contains((string)$e->getKind(), \"Function\")));\n"
    );
}

/// CONTROL against an over-broad fix: when the element type is NOT recorded as declaring `kind`
/// as a getter (e.g. it stayed a scalar `#[php(prop)]` there too), the property form is still
/// correct and must not be forced into a getter call regardless of owner.
#[test]
fn wildcard_element_stays_a_property_when_the_element_type_has_no_getter_for_it() {
    let mut map = getter_map();
    map.getters.insert(ELEMENT_TYPE.to_string(), HashSet::new());
    let resolver = FieldResolver::new_with_php_getters(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        map,
    );

    let rendered = render(&resolver);
    assert_eq!(
        rendered,
        "        $this->assertTrue((bool)array_filter($result->structure, fn($e) => str_contains((string)$e->kind, \"Function\")));\n"
    );
}
