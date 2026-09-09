use crate::core::config::e2e::StreamingConfig;
use crate::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn render(call: CallConfig, assertions: Vec<Assertion>, types: &[TypeDef], config: ResolvedCrateConfig) -> String {
    let root = super::values::swift_call_result_type(&call).unwrap_or_else(|| "Response".into());
    let function = FunctionDef {
        name: call
            .core_lookup_name("swift")
            .expect("fixture call must resolve")
            .into_owned(),
        return_type: TypeRef::Named(root.clone()),
        ..Default::default()
    };
    let e2e = E2eConfig {
        call: call.clone(),
        result_fields: [if root == "Envelope" { "response" } else { "segments" }.into()].into(),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "runtime_compile_contract".into(),
        assertions,
        ..Default::default()
    };
    let map = super::values::build_swift_first_class_map(types, &[], &e2e, &call);
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        &fixture,
        &e2e,
        "",
        "",
        &[],
        false,
        None,
        &map,
        "Sample",
        &config,
        types,
        &[],
        &[function],
        &[],
    );
    out
}

fn render_stream(assertions: Vec<Assertion>) -> String {
    let call = CallConfig {
        function: "stream".into(),
        r#async: true,
        returns_result: true,
        streaming: Some(StreamingConfig::Enabled(true)),
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![AdapterConfig {
            name: "stream".into(),
            pattern: AdapterPattern::Streaming,
            item_type: Some("Chunk".into()),
            core_path: "sample::stream".into(),
            params: vec![],
            returns: None,
            error_type: None,
            owner_type: None,
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: vec![],
        }],
        ..Default::default()
    };
    render(call, assertions, &[], config)
}

#[test]
fn streaming_not_error_binds_result_before_draining() {
    let out = render_stream(vec![Assertion {
        assertion_type: "not_error".into(),
        ..Default::default()
    }]);
    let binding = out.find("let result = try await Sample.stream()").expect(&out);
    let drain = out.find("for try await _chunk in result").expect(&out);
    assert!(binding < drain, "stream must be bound before it is consumed: {out}");
    assert_eq!(out.matches("Sample.stream()").count(), 1, "{out}");
    assert!(!out.contains("XCTSkip"), "{out}");
}

fn render_optional_array(opaque: bool) -> String {
    render_nested_optional_array(opaque, false)
}

fn render_nested_optional_array(opaque: bool, opaque_parent: bool) -> String {
    let mut types = vec![
        TypeDef {
            name: "Segment".into(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "id".into(),
                ty: TypeRef::Primitive(PrimitiveType::I32),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "Response".into(),
            has_serde: true,
            is_opaque: opaque,
            fields: vec![FieldDef {
                name: "segments".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Segment".into()))),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    if opaque_parent {
        types.push(TypeDef {
            name: "Envelope".into(),
            has_serde: true,
            is_opaque: true,
            fields: vec![FieldDef {
                name: "response".into(),
                ty: TypeRef::Named("Response".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    let mut call = CallConfig {
        function: "transcribe".into(),
        returns_result: true,
        ..Default::default()
    };
    call.overrides.entry("c".into()).or_default().result_type =
        Some(if opaque_parent { "Envelope" } else { "Response" }.into());
    let assertion = Assertion {
        assertion_type: "equals".into(),
        field: Some(
            if opaque_parent {
                "response.segments[0].id"
            } else {
                "segments[0].id"
            }
            .into(),
        ),
        value: Some(serde_json::json!(0)),
        ..Default::default()
    };
    render(
        call,
        vec![assertion],
        &types,
        ResolvedCrateConfig {
            name: "sample".into(),
            ..Default::default()
        },
    )
}

#[test]
fn first_class_optional_array_navigation_uses_typed_properties() {
    let out = render_optional_array(false);
    assert!(
        !out.contains("toString()"),
        "native optional arrays are not RustString JSON getters: {out}"
    );
    assert!(out.contains("XCTAssertEqual"), "{out}");
    assert!(out.contains("result.segments?[0].id"), "{out}");
    assert!(!out.contains("skipped:"), "{out}");
}

#[test]
fn opaque_optional_array_navigation_keeps_json_decoding() {
    let out = render_optional_array(true);
    assert!(out.contains("toString()"), "{out}");
    assert!(out.contains("JSONSerialization.jsonObject"), "{out}");
    assert!(out.contains("XCTAssertEqual"), "{out}");
    assert!(!out.contains("skipped:"), "{out}");
}

#[test]
fn streaming_smoke_call_binds_result_before_draining() {
    let out = render_stream(vec![]);
    let binding = out.find("let result = try await Sample.stream()").expect(&out);
    let drain = out.find("for try await _chunk in result").expect(&out);
    assert!(binding < drain, "stream must be bound before it is consumed: {out}");
    assert_eq!(out.matches("Sample.stream()").count(), 1, "{out}");
    assert!(!out.contains("XCTSkip"), "{out}");
}

#[test]
fn first_class_type_reached_through_opaque_parent_keeps_json_getter() {
    let out = render_nested_optional_array(false, true);
    assert!(out.contains("result.response().segments().toString()"), "{out}");
    assert!(out.contains("JSONSerialization.jsonObject"), "{out}");
    assert!(out.contains("XCTAssertEqual"), "{out}");
    assert!(!out.contains("skipped:"), "{out}");
}
