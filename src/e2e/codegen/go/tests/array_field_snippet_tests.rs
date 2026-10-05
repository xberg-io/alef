//! Regression coverage for array-typed DTO fields in the Go documentation snippet.
//!
//! `go_struct_field_expression` had no arm for `TypeRef::Vec`, so a fixture value such as
//! `"messages": [{...}]` fell into its catch-all, the field was omitted from the composite
//! literal, and the published snippet compiled while silently dropping the request's payload.
//! Split into its own file because `snippet.rs` is over the 1000-line cap and may not grow. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::codegen::go::snippet::render_snippet_body;
use crate::e2e::config::{ArgMapping, CallOverride, E2eConfig};
use crate::e2e::fixture::Fixture;

pub(super) fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

fn request_type(fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "SampleRequest".into(),
        rust_path: "samplelib::SampleRequest".into(),
        fields,
        ..TypeDef::default()
    }
}

pub(super) fn target_type() -> TypeDef {
    TypeDef {
        name: "SampleTarget".into(),
        rust_path: "samplelib::SampleTarget".into(),
        fields: vec![field("name", TypeRef::String, false)],
        ..TypeDef::default()
    }
}

/// `type SampleTagged struct { Kind string; Explicit *SampleTarget }` -- the shape liter-llm's
/// `Message` takes (`#[serde(tag = "role")]` over tuple variants holding a struct).
fn tagged_enum() -> EnumDef {
    EnumDef {
        name: "SampleTagged".into(),
        rust_path: "samplelib::SampleTagged".into(),
        variants: vec![EnumVariant {
            name: "Explicit".into(),
            fields: vec![field("_0", TypeRef::Named("SampleTarget".into()), false)],
            ..EnumVariant::default()
        }],
        serde_rename_all: Some("snake_case".into()),
        serde_tag: Some("kind".into()),
        ..EnumDef::default()
    }
}

fn request_arg() -> ArgMapping {
    ArgMapping {
        name: "req".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("SampleRequest".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn e2e(client_factory: bool) -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.module = "example.com/sample".into();
    e2e.call.result_var = "result".into();
    e2e.call.returns_result = true;
    e2e.call.args = vec![request_arg()];
    if client_factory {
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
    }
    e2e
}

pub(super) fn render(
    input: serde_json::Value,
    fields: Vec<FieldDef>,
    extra_types: Vec<TypeDef>,
    enums: &[EnumDef],
    client_factory: bool,
) -> String {
    let fixture = Fixture {
        id: "send_request".into(),
        description: "Send a request".into(),
        input,
        ..Fixture::default()
    };
    let mut type_defs = vec![request_type(fields)];
    type_defs.extend(extra_types);
    render_snippet_body(
        &fixture,
        &e2e(client_factory),
        &ResolvedCrateConfig::default(),
        &type_defs,
        enums,
        &[],
    )
    .expect("snippet renders")
}

#[test]
fn a_string_array_field_is_kept() {
    let rendered = render(
        serde_json::json!({"model": "m", "tags": ["a", "b"]}),
        vec![
            field("model", TypeRef::String, false),
            field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
        ],
        Vec::new(),
        &[],
        false,
    );

    assert!(rendered.contains("Tags:  []string{`a`, `b`},"), "{rendered}");
}

#[test]
fn a_primitive_array_field_is_kept() {
    let rendered = render(
        serde_json::json!({"weights": [1, 2]}),
        vec![field(
            "weights",
            TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U32))),
            false,
        )],
        Vec::new(),
        &[],
        false,
    );

    assert!(rendered.contains("Weights: []uint32{1, 2},"), "{rendered}");
}

#[test]
fn a_struct_array_field_is_kept_element_by_element() {
    let rendered = render(
        serde_json::json!({"targets": [{"name": "one"}, {"name": "two"}]}),
        vec![field(
            "targets",
            TypeRef::Vec(Box::new(TypeRef::Named("SampleTarget".into()))),
            false,
        )],
        vec![target_type()],
        &[],
        false,
    );

    assert!(rendered.contains("Targets: []pkg.SampleTarget{"), "{rendered}");
    assert!(rendered.contains("Name: `one`"), "{rendered}");
    assert!(rendered.contains("Name: `two`"), "{rendered}");
}

/// The liter-llm shape: a `Vec` of an internally tagged union. Dropping it sent an empty
/// `messages` list to the API, so the documented request could not have worked.
#[test]
fn a_tagged_union_array_field_keeps_every_element_with_its_discriminator() {
    let rendered = render(
        serde_json::json!({"model": "m", "items": [{"kind": "explicit", "name": "search"}]}),
        vec![
            field("model", TypeRef::String, false),
            field(
                "items",
                TypeRef::Vec(Box::new(TypeRef::Named("SampleTagged".into()))),
                false,
            ),
        ],
        vec![target_type()],
        &[tagged_enum()],
        false,
    );

    assert!(rendered.contains("Items: []pkg.SampleTagged{"), "{rendered}");
    assert!(rendered.contains("Kind: `explicit`"), "{rendered}");
    assert!(rendered.contains("Name: `search`"), "{rendered}");
}

/// Go emits `Vec<T>` as `[]T` whether or not the field is optional (`go_optional_type` leaves a
/// slice alone), so wrapping it in `ptr(..)` would not type-check against the field.
#[test]
fn an_optional_array_field_is_not_pointer_wrapped() {
    let rendered = render(
        serde_json::json!({"tags": ["a"]}),
        vec![field("tags", TypeRef::Vec(Box::new(TypeRef::String)), true)],
        Vec::new(),
        &[],
        false,
    );

    assert!(rendered.contains("Tags: []string{`a`},"), "{rendered}");
    assert!(!rendered.contains("ptr("), "{rendered}");
}

/// Negative control: a value that is not an array has no slice expression and keeps the
/// pre-existing behaviour of being left out, rather than rendering a malformed literal.
#[test]
fn a_non_array_value_for_an_array_field_is_still_omitted() {
    let rendered = render(
        serde_json::json!({"model": "m", "tags": "not-an-array"}),
        vec![
            field("model", TypeRef::String, false),
            field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
        ],
        Vec::new(),
        &[],
        false,
    );

    assert!(rendered.contains("Model: `m`"), "{rendered}");
    assert!(!rendered.contains("Tags"), "{rendered}");
}

/// An element with no Go expression cannot be omitted individually without changing the
/// request's meaning, so the whole field is left out as before.
#[test]
fn an_array_with_an_unrenderable_element_is_omitted_whole() {
    let rendered = render(
        serde_json::json!({"model": "m", "tags": ["a", 7]}),
        vec![
            field("model", TypeRef::String, false),
            field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
        ],
        Vec::new(),
        &[],
        false,
    );

    assert!(!rendered.contains("Tags"), "{rendered}");
}

/// The docs pipeline never runs the published snippet through `gofmt`, so the generator itself
/// has to emit gofmt-stable text. A scalar field followed by a multi-line array is the shape that
/// used to pad the scalar's key out to the array key's width, which `gofmt` undoes because a
/// multi-line value is a section of its own.
fn assert_gofmt_stable(rendered: &str) {
    let Ok(mut child) = std::process::Command::new("gofmt")
        .arg("-e")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    use std::io::Write as _;
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(rendered.as_bytes())
        .expect("write Go snippet");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8(output.stdout)
            .expect("gofmt output is UTF-8")
            .trim_end(),
        rendered,
        "gofmt rewrote the snippet"
    );
}

#[test]
fn a_scalar_followed_by_a_multi_line_array_is_gofmt_stable() {
    let rendered = render(
        serde_json::json!({"model": "m", "messages": [{"kind": "explicit", "name": "search"}]}),
        vec![
            field("model", TypeRef::String, false),
            field(
                "messages",
                TypeRef::Vec(Box::new(TypeRef::Named("SampleTagged".into()))),
                false,
            ),
        ],
        vec![target_type()],
        &[tagged_enum()],
        false,
    );

    assert!(rendered.contains("Model: `m`,"), "{rendered}");
    assert_gofmt_stable(&rendered);
}

/// The liter-llm fixture shape end to end: `{model, messages}` through a client factory. The
/// snippet must keep the array and place `client, clientErr :=` at the same single tab as every
/// other statement in `main` -- the template already contributes that tab, so the factory line
/// carried a second one.
#[test]
fn a_client_factory_snippet_keeps_the_array_and_indents_the_client_line() {
    let rendered = render(
        serde_json::json!({"model": "m", "messages": [{"kind": "explicit", "name": "search"}]}),
        vec![
            field("model", TypeRef::String, false),
            field(
                "messages",
                TypeRef::Vec(Box::new(TypeRef::Named("SampleTagged".into()))),
                false,
            ),
        ],
        vec![target_type()],
        &[tagged_enum()],
        true,
    );

    assert!(rendered.contains("Messages: []pkg.SampleTagged{"), "{rendered}");
    assert!(
        rendered.contains("\n\tclient, clientErr := pkg.CreateClient("),
        "the client construction must sit at one tab:\n{rendered}"
    );
    assert!(
        !rendered.contains("\t\tclient, clientErr"),
        "the client construction must not be double-indented:\n{rendered}"
    );
    assert_gofmt_stable(&rendered);
}
