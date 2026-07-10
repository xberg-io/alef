//! Regression test for the Swift e2e accessor bug where `result.content` was
//! emitted as a method *reference* (`@Sendable () -> RustString`) rather than
//! a method *call* (`result.content()` → `RustString`).
//!
//! Root cause: `SwiftFirstClassMap::is_first_class(None)` returned `true`,
//! making the accessor generator use property syntax (no parens) when the
//! per-fixture root type was unknown (`None`). In practice the root type is
//! `None` whenever `swift_call_result_type` finds no language-specific
//! `result_type` override in the fixture call config. Opaque swift-bridge
//! types expose their fields as methods (with `()`), so property syntax
//! produces a method reference — not a value — causing Swift compile errors:
//!
//!   "value of type '@Sendable () -> RustString' has no member 'contains'"
//!
//! Fix: `is_first_class(None)` now returns `false` (method-call syntax is
//! the safe default for unknown root types). Only explicitly classified
//! first-class types (non-opaque, has_serde, all fields supported) use
//! property syntax.

use alef::core::config::NewAlefConfig;
use alef::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_pack::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// Build IR for a result type that contains a Json field, preventing it from
/// being classified as a first-class Swift struct. The `structured_output` field
/// (TypeRef::Json) makes the type fail the first-class eligibility check.
fn opaque_result_ir() -> Vec<TypeDef> {
    vec![make_type(
        "DocumentResult",
        vec![
            make_field("content", TypeRef::String),
            make_field("mime_type", TypeRef::String),
            make_field("structured_output", TypeRef::Json),
        ],
    )]
}

/// Config with NO result_type override — simulates an opaque result type fixture
/// where `swift_call_result_type` returns None.
const CONFIG_NO_RESULT_TYPE: &str = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_file_sync"
module = "SamplePack"
result_var = "result"

[[crates.e2e.call.args]]
name = "path"
field = "file_path"
type = "file_path"
"#;

fn make_contains_fixture(id: &str, field: &str, value: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "opaque root type regression fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "file_path": "test.txt" }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!(value)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn render_no_result_type(fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_NO_RESULT_TYPE).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }];
    let files = SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &opaque_result_ir(), &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SmokeTests.swift"))
        .expect("SmokeTests.swift is emitted")
        .content
        .clone()
}

/// Opaque root type with no result_type configured: field accessors must use
/// method-call syntax (`result.content()`) not property syntax (`result.content`).
///
/// The bug produced `result.content.contains("Flyers")` where `result.content`
/// resolved to `@Sendable () -> RustString` (a closure) rather than `RustString`.
/// Swift rejects `.contains` on a closure type.
#[test]
fn opaque_root_without_result_type_uses_method_call_syntax() {
    let rendered = render_no_result_type(make_contains_fixture(
        "extract_file_sync_opaque_content",
        "content",
        "Flyers",
    ));

    assert!(
        rendered.contains("result.content()"),
        "opaque root with unknown root type must emit `result.content()` (method call). \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("result.content.contains"),
        "must not emit `result.content.contains` — `content` without `()` is a closure reference \
         that Swift types as `@Sendable () -> RustString`, which has no `contains` member. \
         Rendered:\n{rendered}"
    );
}

/// Same fixture on `mime_type` — another field that triggered the bug.
#[test]
fn opaque_root_without_result_type_mime_type_uses_method_call_syntax() {
    let rendered = render_no_result_type(make_contains_fixture(
        "extract_file_sync_opaque_mime_type",
        "mime_type",
        "application/pdf",
    ));

    assert!(
        rendered.contains("result.mimeType()") || rendered.contains("result.mime_type()"),
        "opaque root with unknown root type must emit `result.mimeType()` (method call). \
         Rendered:\n{rendered}"
    );
}
