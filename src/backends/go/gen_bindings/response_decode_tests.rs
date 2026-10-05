//! A response decoded from JSON must surface a decode failure as an error, never as `nil, nil`
//! (liter-llm#246). ~keep

use super::functions::gen_function_wrapper;
use super::methods::gen_method_wrapper;
use crate::core::ir::{FunctionDef, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use std::collections::HashSet;

fn client_type() -> TypeDef {
    TypeDef {
        name: "Client".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        cfg: None,
        fields: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn fallible_method(return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: "fetch".to_string(),
        doc: String::new(),
        params: vec![],
        return_type,
        is_static: false,
        is_async: false,
        error_type: Some("SampleError".to_string()),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn fallible_function(return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: "fetch".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
        return_type,
        is_async: false,
        error_type: Some("SampleError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn gen_method(return_type: TypeRef) -> String {
    gen_method_wrapper(
        &client_type(),
        &fallible_method(return_type),
        "sample",
        &HashSet::from(["Client", "Handle"]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn gen_function(return_type: TypeRef) -> String {
    gen_function_wrapper(
        &fallible_function(return_type),
        "sample",
        &HashSet::from(["Client"]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn named(name: &str) -> TypeRef {
    TypeRef::Named(name.to_string())
}

fn string_to_u32_map() -> TypeRef {
    TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Primitive(PrimitiveType::U32)),
    )
}

fn json_decoded_shapes() -> Vec<(&'static str, TypeRef)> {
    vec![
        ("named", named("Response")),
        ("vec_named", TypeRef::Vec(Box::new(named("Item")))),
        ("vec_string", TypeRef::Vec(Box::new(TypeRef::String))),
        ("map", string_to_u32_map()),
        ("optional_named", TypeRef::Optional(Box::new(named("Response")))),
        (
            "optional_vec",
            TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(named("Item"))))),
        ),
        ("optional_map", TypeRef::Optional(Box::new(string_to_u32_map()))),
    ]
}

fn assert_go_parses(generated: &str) {
    use std::io::Write as _;

    let Ok(mut child) = crate::test_support::spawn_from_stable_dir("gofmt")
        .arg("-e")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    let source = format!("package sample\n\ntype Client struct {{}}\n\n{generated}");
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(source.as_bytes())
        .expect("write generated Go source");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(
        output.status.success(),
        "generated Go does not parse: {}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn named_response_decode_failure_returns_an_error() {
    let out = gen_method(named("Response"));
    assert!(
        out.contains(
            "return func() (*Response, error) {\n\
             \tjsonPtr := C.sample_response_to_json(ptr)\n"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "\t\tif err := lastError(); err != nil { return nil, err }\n\
             \t\treturn nil, fmt.Errorf(\"failed to serialise Response response: native returned null JSON\")\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("return nil, fmt.Errorf(\"failed to serialise Response response: %w\", err)"),
        "{out}"
    );
    assert!(out.contains("return &result, nil\n"), "{out}");
    assert_go_parses(&out);
}

#[test]
fn function_named_response_decode_failure_returns_an_error() {
    let out = gen_function(named("Response"));
    assert!(out.contains("jsonPtr == nil"), "{out}");
    assert!(out.contains("failed to unmarshal: %w"), "{out}");
    assert!(!out.contains("}(), nil"), "{out}");
    assert_go_parses(&out);
}

#[test]
fn vec_and_map_responses_report_null_and_malformed_json() {
    let vec_out = gen_method(TypeRef::Vec(Box::new(named("Item"))));
    assert!(vec_out.contains("return func() ([]Item, error) {"), "{vec_out}");
    assert!(
        vec_out.contains("failed to serialise []Item response: native returned null JSON"),
        "{vec_out}"
    );
    assert!(vec_out.contains("failed to serialise []Item response: %w"), "{vec_out}");

    let map_out = gen_method(string_to_u32_map());
    assert!(
        map_out.contains("failed to serialise map[string]uint32 response: native returned null JSON"),
        "{map_out}"
    );
}

#[test]
fn optional_response_keeps_null_as_none_but_errors_on_malformed_json() {
    let out = gen_method(TypeRef::Optional(Box::new(named("Response"))));
    assert!(out.contains("return func() (*Response, error) {"), "{out}");
    assert!(
        out.contains("\tif jsonPtr == nil {\n\t\treturn nil, nil\n\t}\n"),
        "{out}"
    );
    assert!(
        out.contains("return nil, fmt.Errorf(\"failed to serialise Response response: %w\", err)"),
        "{out}"
    );
    assert!(!out.contains("native returned null JSON"), "{out}");
}

#[test]
fn no_json_decoded_response_collapses_a_failure_into_nil_nil() {
    for (label, ty) in json_decoded_shapes() {
        for (kind, out) in [("method", gen_method(ty.clone())), ("function", gen_function(ty))] {
            assert!(
                !out.contains("}(), nil"),
                "{kind} `{label}` appends `, nil` to a decode closure:\n{out}"
            );
            assert!(
                !out.contains("{ return nil }"),
                "{kind} `{label}` swallows a failure in a one-line `return nil`:\n{out}"
            );
            assert!(
                out.contains("failed to serialise") || out.contains("failed to unmarshal"),
                "{kind} `{label}` has no decode error path:\n{out}"
            );
            assert_go_parses(&out);
        }
    }
}

#[test]
fn non_json_responses_are_unchanged() {
    let out = gen_method(TypeRef::Primitive(PrimitiveType::U32));
    assert!(out.contains("return uint32(ptr), nil\n"), "{out}");
    let out = gen_method(named("Handle"));
    assert!(out.contains("return &Handle{ptr: ptr}, nil\n"), "{out}");
}
