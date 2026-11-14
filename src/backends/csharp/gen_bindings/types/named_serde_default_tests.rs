//! SECURITY. `gen_record_type` decides whether a DTO property is initialized to a C# collection
//! zero (`[]`, `new Dictionary<..>()`) or left nullable so an unset value is dropped from the wire
//! and Rust's own serde default supplies it. A field carrying `#[serde(default = "path")]` reaches
//! the IR as `DefaultValue::FunctionCall` — alef records the function's *name*, never its return
//! value — so the zero is a claim about a value alef does not have, and the emitter declines it.
//!
//! That refusal was unreachable in practice. `extract::extractor::types` blanket-overwrote every
//! field's `typed_default` with `DefaultValue::Empty` whenever the container derived `Default`,
//! and the `Empty` arm renders the collection zero. A named allow-list or deny-list default
//! therefore shipped as `[]` — an allow-list that permits nothing, or a deny-list that fails open.
//! These tests pin the deferral against the IR shape the extractor now produces.
//!
//! Lives in its own module because `tests.rs` next door is within a handful of lines of the
//! repo's 1,000-line file-size cap. ~keep

use super::gen_record_type;
use super::tests::{field, record_type};
use crate::core::ir::{DefaultValue, PrimitiveType, TypeDef, TypeRef};
use std::collections::HashSet;

fn render(typ: &TypeDef) -> String {
    gen_record_type(
        typ,
        &[],
        "Demo",
        "demo",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        "snake_case",
        &HashSet::new(),
        &[],
        "DemoException",
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        false,
    )
}

fn render_one(name: &str, ty: TypeRef, typed_default: DefaultValue) -> String {
    let mut f = field(name, ty);
    f.typed_default = Some(typed_default);
    f.default = Some("serde(default = \"named_default\")".to_string());
    render(&record_type(vec![f]))
}

#[test]
fn a_named_serde_default_on_a_vec_defers_to_rust_instead_of_an_empty_collection_literal() {
    let code = render_one(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::FunctionCall("default_scheme_allowlist".to_string()),
    );

    assert!(
        !code.contains("SchemeAllowlist { get; init; } = [];"),
        "alef never evaluates default_scheme_allowlist(); `= []` ships an allow-list that permits \
         nothing in place of the real one:\n{code}"
    );
    assert!(
        code.contains("public List<string>? SchemeAllowlist { get; init; } = null;"),
        "the property must stay nullable so the key is omitted and Rust's serde default fires:\n{code}"
    );
}

#[test]
fn a_named_serde_default_on_a_map_defers_to_rust_instead_of_an_empty_dictionary() {
    let code = render_one(
        "header_overrides",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        DefaultValue::PublicFunctionCall("demo::Policy::header_overrides".to_string()),
    );

    assert!(
        !code.contains("HeaderOverrides { get; init; } = new Dictionary"),
        "a resolved function-call default is still a value alef has not read:\n{code}"
    );
    assert!(
        code.contains("HeaderOverrides { get; init; } = null;"),
        "the property must defer to Rust rather than materialize an empty dictionary:\n{code}"
    );
}

#[test]
fn a_named_serde_default_on_a_scalar_defers_to_rust_instead_of_a_zero() {
    let code = render_one(
        "redirect_limit",
        TypeRef::Primitive(PrimitiveType::U32),
        DefaultValue::FunctionCall("default_redirect_limit".to_string()),
    );

    assert!(
        !code.contains("RedirectLimit { get; init; } = 0;"),
        "`= 0` is a claim about a value alef does not have:\n{code}"
    );
}

/// Discrimination control for all three tests above. `Empty` genuinely IS `Default::default()`,
/// so the collection zero and the scalar zero are exact for it and must still be emitted. Without
/// this, a change that stopped initializing collection properties at all would satisfy every
/// assertion above while leaving ordinary `#[derive(Default)]` fields null in a property declared
/// non-nullable — the exact class the `Empty` initializers exist to close. ~keep
#[test]
fn an_empty_default_still_renders_the_csharp_zero_for_each_shape() {
    let vec_code = render_one(
        "scheme_allowlist",
        TypeRef::Vec(Box::new(TypeRef::String)),
        DefaultValue::Empty,
    );
    assert!(
        vec_code.contains("public List<string> SchemeAllowlist { get; init; } = [];"),
        "`Empty` is the type's own default and keeps the empty-collection initializer:\n{vec_code}"
    );

    let map_code = render_one(
        "header_overrides",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        DefaultValue::Empty,
    );
    assert!(
        map_code.contains("HeaderOverrides { get; init; } = new Dictionary<string, string>();"),
        "`Empty` keeps the empty-dictionary initializer:\n{map_code}"
    );

    let scalar_code = render_one(
        "redirect_limit",
        TypeRef::Primitive(PrimitiveType::U32),
        DefaultValue::Empty,
    );
    assert!(
        scalar_code.contains("RedirectLimit { get; init; } = 0;"),
        "`Empty` keeps the scalar zero initializer:\n{scalar_code}"
    );
}
