//! The wasm docs-snippet emitter must import a type under the same, prefixed name its body
//! references.
//!
//! Split out of `tests.rs` (a remediation target well over the file-size cap) because this is a
//! self-contained concern with its own fixture: the snippet path is a separate emitter from
//! `render_test_file`, and every assertion here is about the wasm type prefix agreeing across
//! the import line and the body. ~keep

use super::snippet::{SnippetContext, render_snippet_body};
use super::tests::{make_field, make_type};
use crate::core::ir::{EnumDef, TypeRef};
use crate::e2e::fixture::Fixture;

/// The docs-snippet emitter is a separate code path from `render_test_file`
/// and used to skip the wasm prefix on its import statements while the body
/// (via `build_args_and_setup`) applied it -- so every generated wasm snippet
/// imported `ExtractInput` and then referenced `WasmExtractInput`, failing
/// typecheck twice over: TS2724 on the import and TS2304 on the reference.
#[test]
fn wasm_snippet_imports_the_same_prefixed_names_its_body_references() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "extract".to_string();
    e2e_config.call.module = "@test/wasm".to_string();
    e2e_config.call.args = vec![crate::e2e::config::ArgMapping {
        name: "input".to_string(),
        field: "input".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some("ExtractInput".to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    e2e_config.call.overrides.insert(
        "wasm".into(),
        crate::e2e::config::CallOverride {
            enum_fields: [("kind".into(), "ExtractInputKind".into())].into_iter().collect(),
            ..Default::default()
        },
    );

    let fixture = Fixture {
        id: "extract_input_kind_uri".to_string(),
        category: Some("extract".to_string()),
        description: "extract from a uri".to_string(),
        input: serde_json::json!({ "kind": "uri", "uri": "https://example.com/a.pdf" }),
        ..Default::default()
    };
    let extract_input = make_type(
        "ExtractInput",
        vec![
            make_field("kind", TypeRef::Named("ExtractInputKind".to_string())),
            make_field("uri", TypeRef::String),
        ],
    );
    let enums = [EnumDef {
        name: "ExtractInputKind".into(),
        variants: vec![crate::core::ir::EnumVariant {
            name: "Uri".into(),
            serde_rename: Some("uri".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let body = render_snippet_body(SnippetContext {
        lang: "wasm",
        fixture: &fixture,
        module: "@test/wasm",
        client_factory: None,
        e2e_config: &e2e_config,
        type_defs: &[extract_input],
        enums: &enums,
        functions: &[],
        wasm_type_prefix: "Wasm",
        config: &config,
    });

    let import_line = body
        .lines()
        .find(|line| line.starts_with("import") && line.contains("@test/wasm"))
        .expect("wasm snippet must have a binding import line");

    // Every name the body references must be imported under that same name.
    assert!(
        import_line.contains("WasmExtractInput"),
        "snippet import must reference the prefixed input class;\n{import_line}"
    );
    assert!(
        !import_line
            .split([',', '{', '}', ' '])
            .any(|token| token == "ExtractInput"),
        "snippet import must NOT reference the bare, unprefixed input class;\n{import_line}"
    );
    assert!(
        !import_line
            .split([',', '{', '}', ' '])
            .any(|token| token == "ExtractInputKind"),
        "snippet import must NOT reference the bare, unprefixed enum class;\n{import_line}"
    );
    assert!(
        body.contains("WasmExtractInput.default()"),
        "snippet body must construct the prefixed input class;\n{body}"
    );
    assert!(
        body.contains("WasmExtractInputKind."),
        "snippet body must reference the prefixed enum class;\n{body}"
    );
}
