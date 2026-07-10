//! Regression tests for three Swift e2e codegen bugs surfaced by
//! sample-language-pack against alef 0.17.15:
//!
//! 1. Scalar field accessors on a chain that crosses an opaque (`type X;` in
//!    swift-bridge, emitted as `typealias X = RustBridge.X`) parent must keep
//!    method-call syntax all the way down — even when the *child* type is
//!    independently eligible for first-class emission. Previously the codegen
//!    emitted `result.metrics().totalLines` (missing trailing `()`), which
//!    triggers `add () to forward '@autoclosure' parameter` errors in
//!    `XCTAssertEqual`/`XCTAssertGreaterThanOrEqual`.
//!
//! 2. `swift_array_contains_expr` unconditionally chained `?.map { … } ?? []`,
//!    producing `(result.foo()?.map { … } ?? []).contains(…)` even when
//!    `foo()` returns a non-optional `RustVec<T>`. Swift rejects optional
//!    chaining on non-optional values with "cannot use optional chaining on
//!    non-optional value of type 'RustVec<…>'".
//!
//! 3. The contains-via-map closure emitted snake_case `as_str()` — but
//!    swift-bridge renames the Rust `fn as_str(&self) -> &str` method to
//!    lowerCamelCase `asStr()` on the Swift side, so the original closure
//!    fails to compile: "value of type 'XRef' has no member 'as_str'".
//!
//! The fixes apply in `crates/alef-e2e/src/codegen/swift.rs`
//! (`swift_build_accessor`, `swift_array_contains_expr`) and
//! `crates/alef-e2e/src/field_access.rs`
//! (`render_swift_with_first_class_map`).

use alef::core::config::NewAlefConfig;
use alef::core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeDef, TypeRef};
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::swift::SwiftE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup, MockResponse};

const CONFIG_TOML: &str = r#"
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
module = "sample_pack"
result_var = "result"

# `result_type` on the (otherwise unused) `c` override gives the renderer a
# stable IR root for type-aware dispatch — matches what sample_language_pack does in alef.toml.
[crates.e2e.call.overrides.c]
result_type = "ProcessResult"

[[crates.e2e.call.args]]
name = "source"
field = "source_code"
type = "string"
"#;

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

/// Build IR mirroring sample_language_pack's opaque `ProcessResult` whose `metrics` field
/// holds a first-class-eligible `FileMetrics` (all `usize` primitives).
///
/// `ProcessResult` itself is forced opaque-by-emission because its
/// `structure: Vec<StructureItem>` field references a still-opaque
/// `StructureItem` (via a non-unit enum field, mimicked here with a Named
/// reference to a type that never enters `known_dto_names`).
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

fn make_fixture(id: &str, assertion_type: &str, field: &str, value: serde_json::Value) -> Fixture {
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
        assertions: vec![Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: Some(value),
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

fn render(fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }];
    let type_defs = process_result_ir();
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

/// Bug 1: scalar accessor through an opaque parent keeps `()` on the leaf.
#[test]
fn scalar_field_through_opaque_parent_keeps_trailing_parens() {
    let rendered = render(make_fixture(
        "smoke_metrics_total_lines",
        "greater_than_or_equal",
        "metrics.total_lines",
        serde_json::json!(5),
    ));
    assert!(
        rendered.contains("result.metrics().totalLines()"),
        "scalar leaf through an opaque parent must keep trailing `()`. \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains(".totalLines,"),
        "must not emit `totalLines,` (missing `()`) which triggers Swift's \
         @autoclosure error. Rendered:\n{rendered}"
    );
}

/// Bug 2: contains over a non-optional vec accessor must NOT use `?.map`.
#[test]
fn contains_over_non_optional_vec_does_not_use_optional_chain() {
    let rendered = render(make_fixture(
        "smoke_structure_contains_function",
        "contains",
        "structure",
        serde_json::json!("Function"),
    ));
    assert!(
        !rendered.contains("structure()?.map"),
        "non-optional RustVec accessor must not chain `?.map`. \
         Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("structure().map"),
        "non-optional RustVec accessor must chain plain `.map`. \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains(".map { $0.asStr().toString() } ?? []"),
        "must not coalesce a non-optional `[String]` with `?? []`. \
         Rendered:\n{rendered}"
    );
}

/// Bug 3: vec contains closure uses camelCase `asStr()` (swift-bridge rename),
/// never snake_case `as_str()`.
#[test]
fn contains_via_map_uses_camelcase_as_str_accessor() {
    let rendered = render(make_fixture(
        "smoke_structure_contains_class",
        "contains",
        "structure",
        serde_json::json!("Class"),
    ));
    assert!(
        rendered.contains("$0.asStr().toString()"),
        "vec-contains closure must call `asStr()` (camelCase). \
         Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("$0.as_str()"),
        "vec-contains closure must not emit snake_case `as_str()` — \
         swift-bridge renames the method to `asStr`. Rendered:\n{rendered}"
    );
}
