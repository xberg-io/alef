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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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
        docs: None,
        requirements: Vec::new(),
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
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
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
        .generate(&groups, &e2e, &resolved, &type_defs, &[], &[], &[])
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
            skip: None,
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
            skip: None,
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
            skip: None,
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
        rendered.contains("XCTAssertEqual((result ?? \"\"), \"cpp\")"),
        "Optional<String> bare result must be coalesced with `?? \"\"` \
         before string operations. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("XCTAssertEqual(result,"),
        "must not compare the bare optional against a String literal. \
         Rendered:\n{rendered}"
    );
    // ~keep: 329d01a9b dropped `.trimmingCharacters(in:)` from equals — the fixture expected
    // literal is emitted verbatim, so trimming only the actual side hid trailing-whitespace
    // regressions and made newline-terminated expectations unsatisfiable.
    assert!(
        !rendered.contains("trimmingCharacters"),
        "equals compares both sides exactly; the actual side must not be trimmed. \
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
            skip: None,
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
            skip: None,
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
            skip: None,
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

/// Regression for the ci-e2e swift failure (`ContractTests.swift:129`).
///
/// This test previously asserted the opposite of what is checked below, on the theory that an
/// opaque parent's `Option<Vec<Named(struct)>>` field (e.g. `elements: Option<Vec<Element>>`) is
/// natively bridged by swift-bridge as `Optional<RustVec<Element>>`. That theory does not survive
/// reading the emitting backend: `field_needs_json_bridge` in
/// `src/backends/swift/gen_rust_crate/type_bridge.rs` is
/// `needs_json_bridge(ty) || (optional && matches!(ty, TypeRef::Vec(_)))` — for ANY optional
/// `Vec<_>` field this is `true` unconditionally, with no dependence on the parent `TypeDef`'s
/// `is_opaque` at all. Both call sites that decide the Swift-visible getter shape —
/// `wrappers::getters::emit_getters` (`getters.rs:100`, `:137`, gating which Rust wrapper method
/// body is emitted) and `extern_block::emit_extern_block_for_type` (`extern_block.rs:142`, gating
/// the swift-bridge extern signature Swift actually links against) — check this exact predicate
/// first in their if/else chain, before either one ever consults `parent_first_class`. The
/// opaque-only "native `RustVec`" branch inside `emit_vec_getter`
/// (`getter_vec_named_optional.jinja`) is therefore unreachable for `field.optional == true`: the
/// json-bridge check above always short-circuits it first. Running this exact fixture through
/// `render_with_config` confirms it: the field renders as `result.elements().toString().count`,
/// the same JSON-bridged `-> String` shape a first-class parent gets, not `elements()?.count ?? 0`
/// on a native handle. `count_min` must therefore treat an optional `Vec<Named(struct)>` field as
/// JSON-bridged regardless of the parent's opacity — the two shapes below (opaque and first-class
/// parent) must render identically. ~keep
///
/// ~keep What changed, and why these tests now pin a SKIP rather than `.toString().count`: the
/// analysis above is right about the getter's TYPE and wrong about the assertion's MEANING.
/// `.toString().count` is type-correct Swift, which is why it compiled and why this file pinned
/// it -- but it counts the characters of the JSON text, not the collection's elements. A
/// `count_min: 1` against an empty collection renders `"[]".count >= 1`, which is `2 >= 1`: a
/// PASS on exactly the input the assertion exists to reject. A consumer found the live instance,
/// a `count_equals: 2` reading `toolCalls().toString().count == 2`. The elements genuinely are
/// not reachable through a `-> String` getter, so there is no correct count to emit and the
/// honest answer is the recorded skip, not a comparison that cannot mean what it says. The
/// negative assertion against `elements()?.count` is kept: a native `RustVec` handle is still not
/// what this getter returns, so that shape would not compile either.
#[test]
fn count_min_on_optional_vec_of_named_struct_is_json_bridged_on_opaque_parent() {
    let rendered = render_optional_vec_of_named_count_min(true);

    assert!(
        rendered.contains("// skipped: field 'elements' has no countable Swift leaf"),
        "count_min on an opaque parent's Option<Vec<Named(struct)>> field must render the \
         registered skip: the getter returns `-> String`, so there is no element count to \
         emit at all. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains(".toString().count"),
        "must not count the CHARACTERS of the JSON text as if they were elements. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("elements()?.count"),
        "must not optional-chain `.count` onto a native RustVec handle that this field's \
         getter never returns. Rendered:\n{rendered}"
    );
}

/// Sibling of the opaque-parent case above, asserting the SAME json-bridged shape for a
/// first-class parent — the exact pairing this bug needs, because a fix for one parent shape
/// silently regressing the other is precisely how `2332b260a` shipped red through two releases.
/// See `optional_vec_of_serde_struct_on_first_class_parent_is_json_bridged` in
/// `src/e2e/codegen/swift/values.rs` for the classifier-level counterpart of this end-to-end
/// check. ~keep
#[test]
fn count_min_on_optional_vec_of_named_struct_is_json_bridged_on_first_class_parent() {
    let rendered = render_optional_vec_of_named_count_min(false);

    assert!(
        rendered.contains("// skipped: field 'elements' has no countable Swift leaf"),
        "count_min on a first-class parent's Option<Vec<Named(struct)>> field must render the \
         same registered skip the opaque-parent case does. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains(".toString().count"),
        "must not count the CHARACTERS of the JSON text as if they were elements. Rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("elements()?.count"),
        "must not optional-chain `.count` onto a native RustVec handle that this field's \
         getter never returns. Rendered:\n{rendered}"
    );
}

/// Shared fixture builder for the two `count_min` json-bridge tests above. `parent_is_opaque`
/// is the only variable between the two calls — everything else about the shape (an optional
/// `Vec<Named(struct)>` field on a `has_serde` parent) is held constant so the two assertions are
/// a true opaque-vs-first-class pair, not two differently-shaped fixtures that happen to agree.
fn render_optional_vec_of_named_count_min(parent_is_opaque: bool) -> String {
    let toml = r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample_pack"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_optional = ["elements"]

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
        "extract_text_optional_vec_count_min",
        Assertion {
            skip: None,
            assertion_type: "count_min".to_string(),
            field: Some("elements".to_string()),
            value: Some(serde_json::json!(1)),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        },
    );
    let mut elements_field = make_field(
        "elements",
        TypeRef::Vec(Box::new(TypeRef::Named("Element".to_string()))),
    );
    elements_field.optional = true;
    let mut parent = make_type("TextResult", vec![elements_field]);
    parent.is_opaque = parent_is_opaque;
    let result_ir = vec![parent, make_type("Element", vec![make_field("text", TypeRef::String)])];
    render_with_config(toml, fixture, result_ir)
}
