//! Regression tests for four Swift e2e codegen bugs surfaced by
//! sample-language-pack against alef HEAD:
//!
//! A. `init` is a Swift keyword. When a call's function name is `init` the
//!    codegen previously emitted `SampleLanguagePack.init(config:)`, which
//!    Swift rejects ("module has no member named 'init'"). The Swift binding
//!    renames the function to `init_` (mirroring the swift-bridge rename
//!    applied to the Rust side), so the codegen must apply the same keyword
//!    escape.
//!
//! B. When the call returns `Vec<String>` exposed to Swift as a native
//!    `[String]` (`result_element_is_string = true`), the codegen previously
//!    emitted `result.map { $0.asStr().toString() }.contains(val)` —
//!    `String` has no `asStr()` method. The codegen must emit
//!    `result.contains(val)` directly.
//!
//! C. When `result_is_simple + result_is_option` (the bare result is
//!    `Optional<String>`), the codegen emitted
//!    `result.trimmingCharacters(...)` against the optional. Swift requires
//!    unwrapping first — the codegen must coalesce to `""` so the macro
//!    receives a concrete `String`.
//!
//! D. When a `contains` assertion iterates an array of opaque structs whose
//!    "name string" accessor is not `as_str` (e.g. `Vec<StructureItem>` where
//!    `StructureItem` exposes `kind() -> String`), the codegen emitted
//!    `$0.asStr().toString()` — which fails to compile against the opaque
//!    type. The new `result_field_accessor` per-language override names the
//!    correct accessor (`structure → kind`).

use alef::core::config::NewAlefConfig;
use alef::core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};
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

fn process_result_ir() -> Vec<TypeDef> {
    let usize_field = |name: &str| make_field(name, TypeRef::Primitive(PrimitiveType::Usize));
    vec![
        make_type(
            "ProcessResult",
            vec![
                make_field("metrics", TypeRef::Named("FileMetrics".to_string())),
                make_field(
                    "structure",
                    TypeRef::Vec(Box::new(TypeRef::Named("StructureItem".to_string()))),
                ),
            ],
        ),
        make_type(
            "FileMetrics",
            vec![usize_field("total_lines"), usize_field("error_count")],
        ),
        make_type(
            "StructureItem",
            vec![make_field("kind", TypeRef::Named("StructureKind".to_string()))],
        ),
    ]
}

fn make_fixture(id: &str, assertion: Assertion) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "regression fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "source_code": "fn main() {}" }),
        mock_response: Some(MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions: vec![assertion],
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn render_with_config(config_toml: &str, fixture: Fixture, type_defs: Vec<TypeDef>) -> String {
    let cfg: NewAlefConfig = toml::from_str(config_toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }];
    let files = SwiftE2eCodegen
        .generate(&groups, &e2e, &resolved, &type_defs, &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SmokeTests.swift"))
        .expect("SmokeTests.swift is emitted")
        .content
        .clone()
}

#[test]
fn function_named_init_is_escaped_to_init_underscore() {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "init"
module = "SampleLanguagePack"
result_var = "result"

[[crates.e2e.call.args]]
name = "config"
field = "config"
type = "json_object"
"#;
    let fixture = make_fixture(
        "init_default",
        Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let rendered = render_with_config(toml, fixture, process_result_ir());

    assert!(
        rendered.contains(".init_(config:"),
        "Swift keyword `init` must be escaped to `init_` to match the \
         swift-bridge rename. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("try SamplePack.init("),
        "must not emit the bare keyword `init` as a module member. \
         Rendered:\n{rendered}"
    );
}

#[test]
fn vec_string_result_uses_native_contains_without_as_str_coercion() {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "manifest_languages"
module = "SampleLanguagePack"
result_var = "result"
result_is_simple = true
result_is_array = true
result_element_is_string = true
args = []
"#;
    let fixture = make_fixture(
        "manifest_languages",
        Assertion {
            assertion_type: "contains".to_string(),
            field: None,
            value: Some(serde_json::json!("python")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let rendered = render_with_config(toml, fixture, vec![]);

    assert!(
        rendered.contains("XCTAssertTrue(result.contains(\"python\")"),
        "Vec<String> result must use `result.contains(val)` directly. \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("$0.asStr()"),
        "must not emit `.asStr()` on a plain Swift String. Rendered:\n{rendered}"
    );
}

#[test]
fn simple_optional_result_coalesces_before_string_assertions() {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "detect_language_from_extension"
module = "SampleLanguagePack"
result_var = "result"
result_is_simple = true
result_is_option = true

[[crates.e2e.call.args]]
name = "ext"
field = "extension"
type = "string"
"#;
    let fixture = make_fixture(
        "detect_ext_cpp",
        Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(serde_json::json!("cpp")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let rendered = render_with_config(toml, fixture, vec![]);

    assert!(
        rendered.contains("(result ?? \"\").trimmingCharacters("),
        "Optional<String> bare result must be coalesced with `?? \"\"` \
         before string operations. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("        XCTAssertEqual(result.trimmingCharacters"),
        "must not call `.trimmingCharacters` on the optional directly. \
         Rendered:\n{rendered}"
    );
}

#[test]
fn contains_over_opaque_vec_uses_configured_element_accessor() {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_array = ["structure"]

[crates.e2e.call]
function = "process"
module = "SampleLanguagePack"
result_var = "result"

[crates.e2e.call.overrides.c]
result_type = "ProcessResult"

[crates.e2e.call.overrides.swift]
result_field_accessor = { structure = "kind" }

[[crates.e2e.call.args]]
name = "source"
field = "source_code"
type = "string"
"#;
    let fixture = make_fixture(
        "process_structure_contains_function",
        Assertion {
            assertion_type: "contains".to_string(),
            field: Some("structure".to_string()),
            value: Some(serde_json::json!("Function")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let rendered = render_with_config(toml, fixture, process_result_ir());

    assert!(
        rendered.contains("$0.kind().toString()"),
        "opaque element accessor override must produce `.kind()` in the \
         contains-closure. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("$0.asStr().toString()"),
        "must not fall back to the default `.asStr()` accessor when an \
         override is configured. Rendered:\n{rendered}"
    );
}

#[test]
fn count_min_on_opaque_method_call_wraps_with_tostring() {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_text"
module = "SampleLanguagePack"
result_var = "result"

[[crates.e2e.call.args]]
name = "document"
field = "input"
type = "string"
"#;
    let fixture = make_fixture(
        "extract_text_min_length",
        Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("text".to_string()),
            value: Some(serde_json::json!(5)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let result_ir = vec![make_type(
        "TextResult",
        vec![make_field("text", TypeRef::Named("String".to_string()))],
    )];
    let rendered = render_with_config(toml, fixture, result_ir);

    assert!(
        rendered.contains("result.text().toString().count"),
        "count_min on opaque String field must wrap with `.toString()` \
         to convert RustString to Swift String. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("result.text().count"),
        "must not call `.count` directly on RustString. Rendered:\n{rendered}"
    );
}

/// Regression for the ci-e2e swift failure (`ContractTests.swift:129`): a scalar-string
/// leaf reached with `has_optional = true` rendered `...elements().toString()?.count`,
/// which Swift rejects with "cannot use optional chaining on non-optional value of type
/// 'String'" because `.toString()` returns a non-optional `String`. Such a target must
/// take `.count` directly.
#[test]
fn count_min_on_optional_scalar_field_does_not_optional_chain_count() {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_text"
module = "SampleLanguagePack"
result_var = "result"

[[crates.e2e.call.args]]
name = "document"
field = "input"
type = "string"
"#;
    let fixture = make_fixture(
        "extract_text_optional_count_min",
        Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("text".to_string()),
            value: Some(serde_json::json!(1)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let mut text_field = make_field("text", TypeRef::Named("String".to_string()));
    text_field.optional = true;
    let result_ir = vec![make_type("TextResult", vec![text_field])];
    let rendered = render_with_config(toml, fixture, result_ir);

    assert!(
        !rendered.contains(".toString()?.count"),
        "must not optional-chain `.count` onto a non-optional `.toString()` String. \
         Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains(".toString().count"),
        "count_min on an optional scalar String must take `.count` on the Swift String directly. \
         Rendered:\n{rendered}"
    );
}
