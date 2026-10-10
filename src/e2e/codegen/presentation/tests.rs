//! Unit tests for `presentation.rs`, split out for the file-modularization cap.

use super::*;
use crate::e2e::config::{ArgMapping, CallConfig};
use crate::e2e::fixture::{FixtureDocs, FixtureDocsPresentation, SideEffectClass};
use std::collections::BTreeMap;

fn fixture() -> Fixture {
    Fixture {
        id: "present_items".into(),
        description: "Present returned items".into(),
        input: serde_json::json!({"old_source": "test.txt"}),
        docs: Some(FixtureDocs {
            sample_url: None,
            topic: "configuration".into(),
            stem: None,
            paths: BTreeMap::new(),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: Some(FixtureDocsPresentation {
                call: None,
                input: Some(serde_json::json!({"source": "guide.txt"})),
                args: Some(vec![ArgMapping {
                    name: "source".into(),
                    field: "source".into(),
                    arg_type: "string".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                }]),
                files: Vec::new(),
                operations: vec![FixtureDocsOperation::Iterate {
                    path: "items".into(),
                    item: "item".into(),
                    fields: vec!["text".into(), "metadata.heading".into()],
                    display: true,
                    optional: true,
                }],
            }),
            client: None,
            side_effects: SideEffectClass::Safe,
            coverage_exceptions: BTreeMap::new(),
            sample_url_vars: Default::default(),
            body_file: None,
        }),
        ..Fixture::default()
    }
}

fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "process".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        fields_optional: ["items".to_string()].into_iter().collect(),
        ..E2eConfig::default()
    }
}

#[test]
fn docs_call_overrides_reuse_typed_fixture_arguments() {
    let fixture = fixture().docs_call_fixture();
    assert_eq!(fixture.input, serde_json::json!({"source": "guide.txt"}));
    assert_eq!(fixture.args[0].arg_type, "string");
    assert_eq!(fixture.args[0].field, "source");
}

#[test]
fn docs_call_fixture_removes_mock_harness_and_uses_an_illustrative_url() {
    let mut fixture = fixture();
    fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .input = None;
    fixture.input = serde_json::json!({
        "mock_responses": [{"path": "/guide.txt", "status_code": 200}],
        "extract_input": {"kind": "uri", "uri": "$mock_url/guide.txt"}
    });
    fixture.mock_response = Some(crate::e2e::fixture::MockResponse {
        status: 200,
        body: None,
        stream_chunks: None,
        headers: Default::default(),
    });

    let docs_fixture = fixture.docs_call_fixture();

    assert!(docs_fixture.mock_response.is_none());
    assert!(docs_fixture.input.get("mock_responses").is_none());
    assert_eq!(
        docs_fixture
            .input
            .pointer("/extract_input/uri")
            .and_then(serde_json::Value::as_str),
        Some("https://example.com/guide.txt")
    );
    assert!(!docs_fixture.needs_mock_server());
}

#[test]
fn show_display_flag_selects_the_human_readable_rust_formatter() {
    let mut display_fixture = fixture();
    display_fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .operations = vec![FixtureDocsOperation::Show {
        path: "text".into(),
        display: true,
    }];
    let mut debug_fixture = fixture();
    debug_fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .operations = vec![FixtureDocsOperation::Show {
        path: "text".into(),
        display: false,
    }];
    let config = config();

    let render = |operations| {
        crate::e2e::template_env::render(
            "rust/snippet_body.rs.jinja",
            crate::alef_context! { imports => Vec::<String>::new(), body => vec!["let result = process();"],
            is_async => false, presentation => operations },
        )
    };
    let displayed = render(resolve(&display_fixture, &config, "rust", &[], &[], &[]));
    let debugged = render(resolve(&debug_fixture, &config, "rust", &[], &[], &[]));

    assert!(displayed.contains("println!(\"{}\", result.text);"), "{displayed}");
    assert!(debugged.contains("println!(\"{:?}\", result.text);"), "{debugged}");
}

#[test]
fn presentation_templates_emit_idiomatic_python_rust_and_typescript() {
    let fixture = fixture();
    let config = config();
    let python = resolve(&fixture, &config, "python", &[], &[], &[]);
    let rust = resolve(&fixture, &config, "rust", &[], &[], &[]);
    let mut typescript_fixture = fixture.clone();
    typescript_fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .operations = vec![FixtureDocsOperation::Iterate {
        path: "results[0].chunks".into(),
        item: "chunk".into(),
        fields: vec!["content".into()],
        display: true,
        optional: true,
    }];
    let typescript = resolve(&typescript_fixture, &config, "node", &[], &[], &[]);

    let python_output = crate::e2e::template_env::render(
        "python/snippet_body.py.jinja",
        crate::alef_context! { imports => Vec::<String>::new(), body => vec!["result = process()"],
        is_async => false, presentation => python },
    );
    let rust_output = crate::e2e::template_env::render(
        "rust/snippet_body.rs.jinja",
        crate::alef_context! { imports => Vec::<String>::new(), body => vec!["let result = process();"],
        is_async => false, presentation => rust },
    );
    let typescript_output = crate::e2e::template_env::render(
        "typescript/snippet_body.jinja",
        crate::alef_context! { imports => vec!["process"], module => "@example/library",
        setup_lines => Vec::<String>::new(), client_setup => "", call_expr => "process()",
        result_var => "result", is_async => false, expects_error => false,
        presentation => typescript },
    );

    assert!(
        python_output.contains("for item in result.items or []:"),
        "{python_output}"
    );
    assert!(
        python_output.contains("print(item.metadata.heading)"),
        "{python_output}"
    );
    assert!(
        rust_output.contains("for item in result.items.iter().flatten()"),
        "{rust_output}"
    );
    // ~keep No `type_defs`/`functions` are passed to `resolve()` in this test, so
    // `collection_element_type` cannot resolve `item`'s own type and the per-item-field
    // `Display` allowlist has nothing to vouch for -- it falls back to `{:?}`, the same "cannot
    // determine the type, so don't guess `{}`" default as an unresolved field on any other
    // per-item field. This is not a loosened assertion: `println!("{:?}", ...)` always compiles,
    // where `{}` does not, so the fallback direction is deliberate.
    assert!(
        rust_output.contains("println!(\"{:?}\", item.metadata.heading);"),
        "{rust_output}"
    );
    assert!(
        typescript_output.contains("const [first] = result.results ?? [];"),
        "{typescript_output}"
    );
    assert!(
        typescript_output.contains("for (const chunk of first?.chunks ?? [])"),
        "{typescript_output}"
    );
    assert!(
        typescript_output.contains("console.log(chunk.content);"),
        "{typescript_output}"
    );
}

/// A fixture's own `optional: false` on an `Iterate` operation must not
/// override field-optionality the resolver already knows about (from the
/// e2e config's `fields_optional`). `config_element_types.json` hit this:
/// `results[0].elements` is a genuinely optional field (registered in
/// `fields_optional`), but the fixture's `Iterate` operation didn't set
/// `"optional": true`, so the generated node/wasm snippet rendered
/// `for (const element of first?.elements)` with no `?? []` guard --
/// `first?.elements` is `Element[] | undefined`, and iterating it directly
/// is a `tsc` TS18048 in strict mode.
#[test]
fn resolve_iterate_treats_path_optional_when_fixture_flag_is_stale() {
    let mut stale_fixture = fixture();
    stale_fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .operations = vec![FixtureDocsOperation::Iterate {
        path: "results[0].elements".into(),
        item: "element".into(),
        fields: vec!["element_type".into()],
        display: true,
        optional: false,
    }];
    let mut stale_config = config();
    stale_config.fields_optional = ["results[0].elements".to_string()].into_iter().collect();

    let operations = resolve(&stale_fixture, &stale_config, "node", &[], &[], &[]);
    let iterate = operations.first().expect("one iterate operation");
    assert!(
        iterate.optional,
        "resolver-known optionality for 'results[0].elements' must win over the fixture's stale `optional: false`"
    );

    let typescript_output = crate::e2e::template_env::render(
        "typescript/snippet_body.jinja",
        crate::alef_context! { imports => vec!["process"], module => "@example/library",
        setup_lines => Vec::<String>::new(), client_setup => "", call_expr => "process()",
        result_var => "result", is_async => false, expects_error => false,
        presentation => operations },
    );
    assert!(
        typescript_output.contains("for (const element of first?.elements ?? [])"),
        "{typescript_output}"
    );
}

/// A docs snippet that shows a field reached through an `Option<T>` in a non-leaf
/// position must unwrap, even when the consumer's `alef.toml` never lists that field
/// under `fields_optional` -- the IR alone (`FieldDef.optional`) must be enough. This
/// is the snippet-surface half of the same bug the e2e assertion resolver had: passing
/// real `type_defs` changes the rendered accessor, and `&[]` (no IR) reproduces the old
/// (broken) behavior -- proving the merge in `resolve` actually takes effect rather
/// than every new-parameter call site silently passing an empty set. ~keep
#[test]
fn resolve_show_unwraps_ir_only_optional_field_in_non_leaf_position() {
    use crate::core::ir::{FieldDef, TypeDef};

    let mut show_fixture = fixture();
    show_fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .operations = vec![FixtureDocsOperation::Show {
        path: "data.kind".into(),
        display: false,
    }];
    // No `fields_optional` entry for `data` anywhere in this config -- optionality
    // must come entirely from the IR passed to `resolve`.
    let config = config();
    assert!(!config.fields_optional.contains("data"));

    let process_result = TypeDef {
        name: "ProcessResult".to_string(),
        fields: vec![FieldDef {
            name: "data".to_string(),
            optional: true,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };

    let without_ir = resolve(&show_fixture, &config, "rust", &[], &[], &[]);
    let with_ir = resolve(&show_fixture, &config, "rust", &[process_result], &[], &[]);

    assert_eq!(
        without_ir[0].expression, "result.data.kind",
        "with no IR in scope, resolve falls back to the pre-fix (non-compiling) accessor"
    );
    assert_eq!(
        with_ir[0].expression, "result.data.as_ref().unwrap().kind",
        "with IR in scope, resolve must unwrap the Option before the nested field access"
    );
}

/// `display: true` on a `Show` path whose IR-resolved type is a struct/enum this crate
/// defines must be downgraded to the debug formatter -- `extract` never records `Display`
/// impls (`STD_TRAITS` discards them), so `println!("{}", result.data)` against a `Data`
/// struct with no hand-written `Display` is a snippet that does not compile. A sibling
/// `Show` on a plain `String` field must keep `display: true` unchanged -- the whole point
/// of the flag.
#[test]
fn resolve_downgrades_display_true_against_an_ir_struct_field_but_keeps_it_for_a_scalar() {
    use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

    let mut fixture = fixture();
    fixture
        .docs
        .as_mut()
        .and_then(|docs| docs.presentation.as_mut())
        .expect("presentation")
        .operations = vec![
        FixtureDocsOperation::Show {
            path: "data".into(),
            display: true,
        },
        FixtureDocsOperation::Show {
            path: "text".into(),
            display: true,
        },
    ];
    let config = config();

    let process_result = TypeDef {
        name: "ProcessResult".to_string(),
        fields: vec![
            FieldDef {
                name: "data".to_string(),
                ty: TypeRef::Named("Data".to_string()),
                ..FieldDef::default()
            },
            FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    };
    let data = TypeDef {
        name: "Data".to_string(),
        ..TypeDef::default()
    };
    let process_fn = FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("ProcessResult".to_string()),
        ..FunctionDef::default()
    };

    let operations = resolve(
        &fixture,
        &config,
        "rust",
        &[process_result, data],
        &[],
        std::slice::from_ref(&process_fn),
    );
    let by_path = |path: &str| operations.iter().find(|op| op.expression.ends_with(path)).unwrap();

    assert!(
        !by_path("data").display,
        "a struct-typed field must be downgraded to the debug formatter"
    );
    assert!(
        by_path("text").display,
        "a scalar field must keep its authored display: true"
    );

    let rust_output = crate::e2e::template_env::render(
        "rust/snippet_body.rs.jinja",
        crate::alef_context! { imports => Vec::<String>::new(), body => vec!["let result = process();"],
        is_async => false, presentation => operations },
    );
    assert!(
        rust_output.contains("println!(\"{:?}\", result.data);"),
        "{rust_output}"
    );
    assert!(rust_output.contains("println!(\"{}\", result.text);"), "{rust_output}");
}

/// The shape every fixture-driven (non-hand-authored) docs fixture takes: `docs` is
/// present so the fixture DOES get a snippet, but nobody hand-annotated `shows` or
/// `presentation` -- the only field knowledge lives in `assertions`. Before this fell
/// back to reading `assertions`, `resolve` returned an empty operations list here and
/// every generated snippet in every language bottomed out at a bare
/// `print(result)`/`println!("{:?}", result)`, never showing how to consume the return
/// value. Two assertions on the same field (`equals` and `not_empty`, both on
/// `"content"`) must collapse to one `show`, not print the field twice.
#[test]
fn resolve_derives_show_operations_from_assertion_fields_when_docs_names_none() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "smoke_simple_paragraph",
        "description": "Simple paragraph converts correctly",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": [
            {"type": "equals", "field": "content", "value": "Hello World\n"},
            {"type": "not_empty", "field": "content"}
        ],
        "docs": {"topic": "smoke", "stem": "smoke_simple_paragraph"}
    }))
    .expect("fixture must parse");
    let config = E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let python = resolve(&fixture, &config, "python", &[], &[], &[]);
    assert_eq!(python.len(), 1, "the duplicate 'content' field must not be shown twice");
    assert_eq!(python[0].kind, "show");
    assert_eq!(python[0].expression, "result.content");

    let rust = resolve(&fixture, &config, "rust", &[], &[], &[]);
    assert_eq!(rust[0].expression, "result.content");
}

/// An `error`-typed assertion names no `field` and must not be mistaken for one -- it
/// documents a failure mode, not a field to print on the success path.
#[test]
fn resolve_ignores_assertions_with_no_field_when_deriving_show_operations() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "auth_error",
        "description": "Authentication failure",
        "input": {"token": "bad"},
        "assertions": [{"type": "error"}],
        "docs": {"topic": "errors", "stem": "auth_error"}
    }))
    .expect("fixture must parse");
    let config = config();

    assert!(resolve(&fixture, &config, "python", &[], &[], &[]).is_empty());
}

/// A void call has no result to access; even a fixture whose assertions happen to name a
/// field (e.g. a side-effect check) must not gain a fabricated `print(result.<field>)`.
#[test]
fn resolve_derives_no_show_operations_for_a_void_returning_call() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "configure_logging",
        "description": "Configure logging",
        "input": {"level": "debug"},
        "assertions": [{"type": "equals", "field": "level", "value": "debug"}],
        "docs": {"topic": "configuration", "stem": "configure_logging"}
    }))
    .expect("fixture must parse");
    let mut config = config();
    config.call.returns_void = true;

    assert!(resolve(&fixture, &config, "python", &[], &[], &[]).is_empty());
}
