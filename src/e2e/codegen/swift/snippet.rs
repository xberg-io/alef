use super::{test_method, values};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};
use heck::ToUpperCamelCase;

/// Render a Swift documentation snippet without any core IR to consult.
///
/// Kept as the five-argument entry point every existing caller and test already uses: with no
/// `functions` the seam resolves to `TargetParams::IrAbsent`, which is exactly the state this path
/// was always in, so its output is unchanged by the seam. ~keep
pub(super) fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Result<String> {
    render_with_ir(fixture, e2e_config, config, type_defs, enums, &[])
}

pub(super) fn render_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    let mut snippet_fixture = fixture.clone();
    // Naming a credential variable is what selects `test_method`'s environment-reading
    // client constructor (`let _apiKey = ProcessInfo…` + `let _baseUrl: String? = …`),
    // which the post-processing below already rewrites into the shape a reader would
    // write. Without one, `test_method` falls through to its mock-server constructor —
    // `Factory(apiKey: "test-key", baseUrl: AlefE2EMockServer.baseURL + "/fixtures/<id>")`
    // — and nothing downstream can undo that, so a snippet for any fixture that declares
    // no `env.api_key_var` pointed the reader at the e2e harness. Defaulting the variable
    // here rather than adding a second rewrite rule keeps one client-construction shape
    // to maintain. ~keep
    snippet_fixture.env = Some(crate::e2e::fixture::FixtureEnv {
        api_key_var: Some(crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref()).to_string()),
    });
    snippet_fixture.mock_response = None;
    let fixture = &snippet_fixture;
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if call.args.iter().any(|argument| argument.arg_type == "test_backend") {
        bail!(
            "swift snippet `{}` requires test-backend lifecycle teardown",
            fixture.id
        );
    }

    let package = e2e_config
        .resolve_package("swift")
        .and_then(|package| package.name)
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    let module = package.to_upper_camel_case();
    let first_class_map = values::build_swift_first_class_map(type_defs, enums, e2e_config, call);
    let override_config = e2e_config.call.overrides.get("swift");
    let result_var = call.effective_result_var();
    let mut call_fixture = fixture.clone();
    call_fixture.assertions.clear();
    let mut method = String::new();
    test_method::render_test_method(
        &mut method,
        &call_fixture,
        e2e_config,
        "",
        "",
        &[],
        false,
        override_config.and_then(|value| value.client_factory.as_deref()),
        &first_class_map,
        &module,
        config,
        type_defs,
        enums,
        functions,
        &[],
    );
    let body_line_count = method.lines().count().saturating_sub(3);
    let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    // A `configuration/custom-base-url`-style topic documents `docs.client.base_url` so the
    // reader sees the setting the topic is about, mirroring the Java/Rust/Elixir/Python
    // generators' `docs_client` handling. The client constructor's `baseUrl:` slot always
    // reduces to the bare token `_baseUrl` by this point in the pipeline (its declaration
    // line was already filtered out below), so substituting a literal here — rather than
    // threading `docs_client` through `test_method::render_test_method`, which the
    // executable e2e suite also calls — keeps this docs-only concern out of the shared
    // client-construction path entirely. ~keep
    let base_url_expr = match crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client()) {
        Some(base_url) => format!("\"{}\"", values::escape_swift(base_url)),
        None => "nil".to_string(),
    };
    let mut body = method
        .lines()
        .skip(2)
        .take(body_line_count)
        .map(|line| line.strip_prefix("        ").unwrap_or(line))
        .map(|line| {
            // `render_test_method` binds either `let <result_var> =` or `_ =`, never a blank
            // `let  =`: the name comes from `CallConfig::effective_result_var`, which has no
            // empty case. The two repairs that used to rewrite `let  =` here were a second
            // derivation of the same blank-name rule, and they disagreed with the emitter
            // about which of `_ =` and `let =` a blank meant. ~keep
            if !expects_error && !call.returns_void {
                line.replacen("_ =", &format!("let {result_var} ="), 1)
            } else {
                line.to_string()
            }
        })
        .filter(|line| !line.trim_start().starts_with("let _baseUrl: String? ="))
        .map(|line| {
            if line.trim_start().starts_with("let _apiKey =") {
                return format!(
                    "guard let _apiKey = ProcessInfo.processInfo.environment[\"{api_key_var}\"] else {{ fatalError(\"{api_key_var} must be set\") }}"
                );
            }
            line.replace("_apiKey ?? \"test-key\"", "_apiKey")
                .replace("_baseUrl", &base_url_expr)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if expects_error {
        let indented = body
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        body = format!("do {{\n{indented}\n}} catch {{\n    print(\"\\(type(of: error)): \\(error)\")\n}}");
    }
    // The presentation accessors are rooted on `result_var`, which only exists on the
    // success path of a non-void call — the error path binds nothing and the void path
    // has nothing to read. ~keep
    let presentation = if expects_error || call.returns_void {
        Vec::new()
    } else {
        let field_resolver = crate::e2e::field_access::FieldResolver::new_with_swift_first_class(
            e2e_config.effective_fields(call),
            e2e_config.effective_fields_optional(call),
            e2e_config.effective_result_fields(call),
            e2e_config.effective_fields_array(call),
            e2e_config.effective_fields_method_calls(call),
            &std::collections::HashMap::new(),
            first_class_map.clone(),
        );
        crate::e2e::codegen::presentation::resolve_with(
            fixture,
            e2e_config,
            "swift",
            &field_resolver,
            type_defs,
            enums,
            functions,
        )
    };
    if !expects_error && !call.returns_void && presentation.is_empty() {
        body.push_str(&format!("\nprint({result_var})"));
    }
    let stream_item = if presentation.is_empty() {
        None
    } else {
        crate::e2e::codegen::presentation::stream_item_binding(fixture, e2e_config, "swift")
    };
    let needs_foundation = swift_body_references_type(&body, "Data")
        || swift_body_references_type(&body, "URL")
        || swift_body_references_type(&body, "ProcessInfo")
        || body.contains("JSONDecoder")
        || body.contains("JSONEncoder");
    Ok(crate::e2e::template_env::render(
        "swift/snippet_body.jinja",
        minijinja::context! { module => module, body => body, needs_foundation => needs_foundation,
        presentation => presentation, stream_item => stream_item },
    ))
}

/// Whether `body` references `type_name` as a standalone Swift identifier -- a type
/// annotation (`: Data`), a bare return type (`-> Data`), or a constructor call
/// (`Data(...)`) -- rather than merely as a substring of some other identifier
/// (`WrappedData`).
///
/// A plain `body.contains("Data(")` only catches constructor calls. A trait-bridge test
/// stub's parameter list writes the bare type annotation `imageBytes: Data,` with no
/// following `(`, so that substring check silently dropped `import Foundation` and `swiftc`
/// failed with "cannot find type 'Data' in scope". ~keep
fn swift_body_references_type(body: &str, type_name: &str) -> bool {
    let is_word_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let bytes = body.as_bytes();
    body.match_indices(type_name).any(|(start, matched)| {
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let end = start + matched.len();
        let after_ok = end == bytes.len() || !is_word_byte(bytes[end]);
        before_ok && after_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit coverage for the word-boundary helper itself: a bare type annotation and a
    /// return-type arrow must count as a reference, but the substring appearing inside a
    /// longer identifier must not.
    #[test]
    fn body_references_type_finds_bare_annotations_but_not_substrings_of_other_identifiers() {
        assert!(swift_body_references_type(
            "func f(bytes: Data, config: String)",
            "Data"
        ));
        assert!(swift_body_references_type("func f() -> Data { Data() }", "Data"));
        assert!(!swift_body_references_type(
            "let x: WrappedDataHolder = WrappedDataHolder()",
            "Data"
        ));
    }

    /// End-to-end regression through the real snippet entry point (`render`): a trait-bridge
    /// stub whose parameter is `Bytes` maps to the bare Swift type annotation `imageBytes:
    /// Data,` (no constructor call). Before this fix, `needs_foundation`'s `body.contains("Data(")`
    /// check missed that annotation entirely and the snippet omitted `import Foundation`,
    /// so `swiftc` failed with "cannot find type 'Data' in scope".
    ///
    /// `e2e.call.args` is deliberately left empty: the fixture's own `args` (not the
    /// call-level list `render`'s test-backend guard inspects) is what
    /// `Fixture::resolved_args` prefers, mirroring how the real crate's `[[e2e.calls]]`
    /// entry has no `test_backend` arg of its own even though the fixture does.
    #[test]
    fn trait_bridge_snippet_imports_foundation_for_a_bare_data_typed_stub_param() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
        use crate::e2e::config::ArgMapping;

        let trait_type = TypeDef {
            name: "SampleBackend".into(),
            rust_path: "sample_crate::SampleBackend".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "process_image".into(),
                params: vec![ParamDef {
                    name: "image_bytes".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("anyhow::Error".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                register_fn: Some("register_sample_backend".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let fixture = Fixture {
            id: "register_sample_backend_trait_bridge".into(),
            description: "register_sample_backend: trait bridge".into(),
            args: vec![ArgMapping {
                name: "backend".into(),
                field: "backend".into(),
                arg_type: "test_backend".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: Some("SampleBackend".into()),
            }],
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "registerSampleBackend".into();

        let rendered =
            render(&fixture, &e2e, &config, std::slice::from_ref(&trait_type), &[]).expect("snippet renders");

        assert!(
            rendered.contains("imageBytes: Data"),
            "stub param must be typed Data, got:\n{rendered}"
        );
        assert!(
            rendered.contains("import Foundation"),
            "a bare Data type annotation must still trigger the Foundation import, got:\n{rendered}"
        );
    }

    #[test]
    fn snippet_reuses_typed_call_without_xctest_harness() {
        let fixture = Fixture {
            id: "count".into(),
            description: "Count".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "count_items".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let rendered = render(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");
        assert!(!rendered.contains("import RustBridge"));
        assert!(!rendered.contains("import Foundation"));
        assert!(rendered.contains("let result = try "));
        assert!(rendered.contains(".countItems()"));
        assert!(rendered.contains("print(result)"));
        assert!(!rendered.contains("XCTest"));
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = render(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

        assert!(rendered.contains("let result = try "), "{rendered}");
        assert!(rendered.contains("print(result.summary())"), "{rendered}");
        assert!(rendered.contains("for item in result.items() {"), "{rendered}");
        assert!(rendered.contains("debugPrint(item.label())"), "{rendered}");
        assert!(
            !rendered.contains("print(result)\n"),
            "the whole-result fallback must give way to the documented presentation:\n{rendered}"
        );
    }

    #[test]
    fn expected_error_snippet_uses_native_do_catch() {
        let mut fixture = Fixture {
            id: "invalid".into(),
            description: "Invalid".into(),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        let rendered = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet renders");
        assert!(rendered.contains("do {"));
        assert!(rendered.contains("catch {"));
        assert!(rendered.contains("type(of: error)"));
        assert!(!rendered.contains("fatalError"));
        assert!(!rendered.contains("XCTFail"));
    }

    #[test]
    fn visitor_snippet_reuses_native_bridge_setup() {
        let mut fixture = Fixture {
            id: "custom_text".into(),
            description: "Custom text".into(),
            input: serde_json::json!({ "html": "<p>Hello</p>" }),
            ..Fixture::default()
        };
        fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
            callbacks: [("visit_text".into(), crate::e2e::fixture::CallbackAction::Continue)].into(),
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "render_document".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "DocumentVisitor".into(),
                type_alias: Some("VisitorHandle".into()),
                options_type: Some("RenderOptions".into()),
                options_field: Some("visitor".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let rendered = render(&fixture, &e2e, &config, &[], &[]).expect("visitor snippet renders");
        assert!(rendered.contains("class LocalVisitor_CustomText"));
        assert!(rendered.contains("renderDocument"));
        assert!(!rendered.contains("XCTest"));
    }

    fn client_factory_fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "rate_limit_429",
            "description": "Rate limited",
            "input": null,
            "mock_response": {"status": 429, "body": {}}
        }))
        .expect("fixture")
    }

    fn client_factory_e2e() -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "swift".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("SampleClient".into()),
                ..Default::default()
            },
        );
        e2e
    }

    #[test]
    fn named_call_snippet_inherits_global_client_factory() {
        let mut e2e = client_factory_e2e();
        let mut named_call = e2e.call.clone();
        named_call.overrides.clear();
        e2e.calls.insert("named_chat".into(), named_call);
        let mut fixture = client_factory_fixture();
        fixture.call = Some("named_chat".into());
        let rendered =
            render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("named client snippet renders");

        assert!(rendered.contains("let _client = try SampleClient("), "{rendered}");
        assert!(rendered.contains("_client.chat("), "{rendered}");
        assert!(rendered.contains("import Foundation"), "{rendered}");
        assert!(!rendered.contains("XCT"), "{rendered}");
    }

    #[test]
    fn named_call_snippet_keeps_specific_client_factory() {
        let mut e2e = client_factory_e2e();
        let mut named_call = e2e.call.clone();
        named_call
            .overrides
            .get_mut("swift")
            .expect("Swift override")
            .client_factory = Some("SpecificClient".into());
        e2e.calls.insert("named_chat".into(), named_call);
        let mut fixture = client_factory_fixture();
        fixture.call = Some("named_chat".into());
        let rendered =
            render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("specific client snippet renders");

        assert!(rendered.contains("let _client = try SpecificClient("), "{rendered}");
        assert!(!rendered.contains("try SampleClient("), "{rendered}");
    }

    #[test]
    fn declared_error_snippets_use_native_catch_without_test_assertions() {
        for is_async in [false, true] {
            let fixture: Fixture = serde_json::from_value(serde_json::json!({
                "id": "invalid_request", "input": null,
                "assertions": [{"type": "error", "value": "BadRequest"}]
            }))
            .expect("declared error fixture");
            let mut e2e = client_factory_e2e();
            e2e.call.r#async = is_async;
            let rendered =
                render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("error snippet renders");

            assert!(rendered.contains("do {"), "{rendered}");
            assert!(rendered.contains("} catch {"), "{rendered}");
            assert!(
                rendered.contains("print(\"\\(type(of: error)): \\(error)\")"),
                "{rendered}"
            );
            assert_eq!(rendered.contains("await _client.chat("), is_async, "{rendered}");
            assert!(!rendered.contains("XCT"), "{rendered}");
            assert!(!rendered.contains("_errorMessage"), "{rendered}");
        }
    }

    /// `test_method` only emits its environment-reading client constructor for a fixture
    /// that names an `env.api_key_var`; every other fixture fell through to
    /// `Factory(apiKey: "test-key", baseUrl: AlefE2EMockServer.baseURL + "/fixtures/<id>")`,
    /// and this file's post-processing had no rule for `AlefE2EMockServer`. ~keep
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let rendered = render(
            &client_factory_fixture(),
            &client_factory_e2e(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            !rendered.contains("MOCK_SERVER"),
            "mock-server env var leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("AlefE2EMockServer"),
            "e2e mock-server harness type leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("\"test-key\""),
            "literal credential leaked:\n{rendered}"
        );
        assert!(
            rendered.contains("ProcessInfo.processInfo.environment[\"API_KEY\"]"),
            "credential is not read from the environment:\n{rendered}"
        );
        assert!(
            rendered.contains("let _client = try SampleClient(apiKey: _apiKey, baseUrl: nil)"),
            "client is not constructed the way a reader would:\n{rendered}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its Swift
    /// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling
    /// (`python/mod.rs::client_factory_snippet_renders_the_base_url_the_fixture_documents`).
    /// Paired with `client_factory_snippet_never_points_the_reader_at_the_mock_server` above
    /// (whose fixture declares no `docs.client` and must keep rendering `baseUrl: nil`) as the
    /// negative control: an indiscriminate "always add base_url" change would fail that test. ~keep
    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url",
            "description": "Custom base URL",
            "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture must parse");

        let rendered = render(
            &fixture,
            &client_factory_e2e(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains(
                "let _client = try SampleClient(apiKey: _apiKey, baseUrl: \"https://llm.internal.example.com/v1\")"
            ),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{rendered}"
        );
    }

    /// Companion pin: the e2e suite runs against the mock server, so `test_method`'s own
    /// output for the same fixture must keep pointing at it. Only the snippet renderer
    /// substitutes a reader-facing client. ~keep
    #[test]
    fn e2e_test_method_still_points_the_client_at_the_mock_server() {
        let fixture = client_factory_fixture();
        let e2e = client_factory_e2e();
        let mut rendered = String::new();
        test_method::render_test_method(
            &mut rendered,
            &fixture,
            &e2e,
            "",
            "",
            &[],
            false,
            Some("SampleClient"),
            &crate::e2e::field_access::SwiftFirstClassMap::default(),
            "Sample",
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        );

        assert!(
            rendered.contains("AlefE2EMockServer.baseURL + \"/fixtures/rate_limit_429\""),
            "{rendered}"
        );
        assert!(rendered.contains("apiKey: \"test-key\""), "{rendered}");
    }

    #[test]
    fn streaming_snippet_reuses_async_call_preparation() {
        let fixture = Fixture {
            id: "stream_items".into(),
            description: "Stream items".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "stream_items".into();
        e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            adapters: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "stream_items",
                    "pattern": "streaming",
                    "core_path": "sample::stream_items",
                    "item_type": "StreamItem"
                }))
                .expect("streaming adapter config"),
            ],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render(&fixture, &e2e, &config, &[], &[]).expect("streaming snippet renders");

        assert!(rendered.contains("streamItems"), "{rendered}");
        assert!(rendered.contains("for try await"), "{rendered}");
        assert!(!rendered.contains("XCTest"));
    }
}

#[cfg(test)]
#[path = "snippet_presentation_tests.rs"]
mod presentation_tests;
