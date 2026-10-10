use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::Result;
use std::collections::HashMap;

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, &[])
}

/// [`render_snippet_body`], with the free-function registry it cannot see.
///
/// `functions` lets the presentation resolver anchor the snippet's field facts at the call's
/// own declared result type instead of matching field names across the whole crate IR; without
/// it the resolver falls back to the flat, name-keyed answers. Mirrors `java/snippet.rs`'s
/// split for the same reason. ~keep
pub(super) fn render_snippet_body_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    if fixture.is_http_test() {
        return render_http_snippet(fixture);
    }
    let lang = "ruby";
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs);
    let override_config = recipe.override_config;
    let module = override_config
        .and_then(|value| value.module.as_deref())
        .unwrap_or(&call.module);
    let receiver = super::values::ruby_module_name(module);
    let function = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function);
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    let client_factory = crate::e2e::codegen::client_factory::plain_factory(e2e_config, call, lang);
    // ~keep The executable suite (`spec_file.rs`) calls an async function on a client through its
    // `_async` twin, because that is the method the native extension defines; the snippet has to
    // name the same one or it documents a method that does not exist.
    let function = if call.r#async && !is_streaming && client_factory.is_some() && !function.ends_with("_async") {
        format!("{function}_async")
    } else {
        function.to_string()
    };
    let adapter_lookup_name = call.core_lookup_name(lang);
    let request_type = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|value| value.name == name))
        .and_then(|value| value.request_type.as_deref())
        .and_then(|value| value.rsplit("::").next());
    let (mut setup_lines, mut args, _) = super::args::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &receiver,
        module,
        recipe.options_type,
        &HashMap::new(),
        call.result_is_simple,
        fixture,
        request_type,
        config,
        type_defs,
    );
    if let Some(visitor) = &fixture.visitor {
        let visitor_arg = super::visitor::build_ruby_visitor(&mut setup_lines, visitor);
        args = [args, visitor_arg]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    if !recipe.extra_args.is_empty() {
        args = [args, recipe.extra_args.join(", ")]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let config_call = client_factory
        .and_then(|_| crate::e2e::codegen::client_factory::client_config_call(fixture, e2e_config, call, lang));
    let call_receiver = if client_factory.is_some() { "client" } else { &receiver };
    let package = e2e_config
        .resolve_package(lang)
        .and_then(|value| value.name)
        .unwrap_or_else(|| config.name.replace('-', "_"));
    let require_path = package.replace('-', "_");
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let presentation =
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang, type_defs, enums, functions);
    let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let body = crate::e2e::template_env::render(
        "ruby/snippet_body.jinja",
        crate::alef_context! {
            require_path => require_path, receiver => receiver, setup_lines => setup_lines,
            client_factory => config_call.as_ref().map(|(from_json_factory, _)| *from_json_factory).or(client_factory),
            client_args => config_call
                .as_ref()
                .map_or_else(|| "api_key".to_string(), |(_, json)| crate::e2e::escape::ruby_string_literal(json)),
            reads_api_key => config_call.is_none(),
            call_receiver => call_receiver, function => function, args => args,
            result_var => call.effective_result_var(),
            returns_void => call.returns_void, is_streaming => is_streaming,
            expects_error => expects_error, presentation => presentation, api_key_var => api_key_var,
        },
    );
    // The template always emits a single-argument `client = receiver.factory(api_key)` call;
    // a `configuration/custom-base-url` topic's `docs.client.base_url` (java/elixir/rust/python
    // already wire this) adds a second positional argument matching the executable e2e suite's
    // own two-argument `factory(key, base_url)` convention (see ruby/examples.rs). Substituting
    // after render, rather than threading `docs_client` through the template, keeps this
    // docs-only concern out of the shared snippet path. ~keep
    let body = match client_factory.zip(crate::e2e::codegen::client_factory::docs_base_url(
        fixture.docs_client(),
    )) {
        Some((factory, base_url)) => {
            let bare_call = format!("client = {receiver}.{factory}(api_key)");
            let with_base_url = format!(
                "client = {receiver}.{factory}(api_key, \"{}\")",
                crate::e2e::escape::escape_ruby(base_url)
            );
            body.replace(&bare_call, &with_base_url)
        }
        None => body,
    };
    Ok(body)
}

fn render_http_snippet(fixture: &Fixture) -> Result<String> {
    let http = fixture.http.as_ref().expect("HTTP fixture checked by caller");
    let plan = crate::e2e::codegen::client::http_call::plan_request(http);
    let mut headers = plan.headers;
    if let Some(content_type) = &plan.content_type
        && !headers.keys().any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("Content-Type".into(), content_type.clone());
    }
    if !http.request.cookies.is_empty() {
        headers.insert(
            "Cookie".into(),
            http.request
                .cookies
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    let raw_body = plan.body.as_ref().is_some_and(|body| {
        matches!(body, serde_json::Value::String(_))
            && plan
                .content_type
                .as_deref()
                .is_some_and(crate::e2e::codegen::client::is_raw_text_content_type)
    });
    Ok(crate::e2e::template_env::render(
        "ruby/http_snippet.jinja",
        crate::alef_context! {
            method_class => super::http::http_method_class(&http.request.method.to_uppercase()),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| crate::alef_context! {
                key => crate::e2e::escape::ruby_string_literal(key),
                value => crate::e2e::escape::ruby_string_literal(value),
            }).collect::<Vec<_>>(),
            body => plan.body.as_ref().map(super::values::json_to_ruby),
            raw_body => raw_body,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::StreamingConfig;
    use crate::e2e::config::{ArgMapping, CallOverride};

    /// Render the Ruby snippet for a streaming call whose adapter declares a `request_type`,
    /// with the call's name placed either at the base `function` or only in its Ruby override.
    fn streaming_snippet(base: &str, ruby_override: Option<&str>) -> String {
        let fixture = Fixture {
            id: "stream_items".into(),
            description: "Stream items".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = base.into();
        e2e.call.module = "sample".into();
        e2e.call.args = vec![
            serde_json::from_value(serde_json::json!({ "name": "url", "field": "url", "type": "mock_url" }))
                .expect("arg mapping"),
        ];
        if let Some(function) = ruby_override {
            e2e.call.overrides.insert(
                "ruby".into(),
                CallOverride {
                    function: Some(function.to_string()),
                    ..CallOverride::default()
                },
            );
        }
        let config = ResolvedCrateConfig {
            adapters: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "stream_items",
                    "pattern": "streaming",
                    "core_path": "sample::stream_items",
                    "returns": null,
                    "error_type": null,
                    "owner_type": null,
                    "item_type": null,
                    "request_type": "sample::StreamItemsRequest",
                }))
                .expect("adapter config"),
            ],
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders")
    }

    /// Ruby was the one client-constructing backend with no credential-pinning test at all,
    /// so nothing prevented its snippet from either leaking harness scaffolding or inlining
    /// a literal. It now follows java/csharp/kotlin/php and reads `env.api_key_var`.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.module = "sample".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "ruby".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("'test-key'"), "literal credential leaked:\n{body}");
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            !body.contains("your-api-key"),
            "credential is inlined rather than read from the environment:\n{body}"
        );
        assert!(
            body.contains("api_key = ENV.fetch(\"API_KEY\")"),
            "snippet does not read the credential from the environment:\n{body}"
        );
        assert!(
            body.contains("client = Sample.create_client(api_key)"),
            "client is not constructed from the environment-read credential:\n{body}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its Ruby
    /// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling
    /// (`java/snippet.rs::a_snippet_renders_the_base_url_the_fixture_documents`). Paired with
    /// `client_factory_snippet_never_points_the_reader_at_the_mock_server` above (whose fixture
    /// declares no `docs.client` and must keep rendering the bare, no-`base_url` call) as the
    /// negative control: an indiscriminate "always add base_url" change would fail that test.
    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url", "description": "Custom base URL", "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture must parse");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.module = "sample".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "ruby".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

        assert!(
            body.contains("client = Sample.create_client(api_key, \"https://llm.internal.example.com/v1\")"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    #[test]
    fn the_adapter_request_type_is_found_for_a_call_named_only_by_its_ruby_override() {
        // Adapters are keyed by the Rust name. With `function = ""` the lookup key was `""`,
        // no adapter matched, and the snippet passed the bare URL where the binding wants a
        // request object.
        let body = streaming_snippet("", Some("stream_items"));

        assert!(
            body.contains("url_req = Sample::StreamItemsRequest.new(url: url)"),
            "{body}"
        );
        assert!(body.contains("Sample.stream_items(url_req)"), "{body}");
    }

    #[test]
    fn a_call_named_by_its_base_resolves_the_same_adapter_as_before() {
        let body = streaming_snippet("stream_items", None);

        assert!(
            body.contains("url_req = Sample::StreamItemsRequest.new(url: url)"),
            "{body}"
        );
        assert!(body.contains("Sample.stream_items(url_req)"), "{body}");
    }

    #[test]
    fn renders_native_call_without_rspec() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.call.module = "sample".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();
        assert!(body.contains("Sample.load_document()"));
        assert!(!body.contains("expect("));
    }

    #[test]
    fn streaming_snippet_inspects_the_variable_it_bound() {
        let fixture = Fixture {
            id: "sample_stream".into(),
            description: "Sample stream".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "stream_items".into();
        e2e.call.module = "sample".into();
        e2e.call.result_var = "stream_result".into();
        e2e.call.streaming = Some(StreamingConfig::Enabled(true));

        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();

        assert!(body.contains("stream_result = Sample.stream_items().to_a"), "{body}");
        assert!(body.contains("puts stream_result.inspect"), "{body}");
        assert!(!body.contains("chunks ="), "{body}");
    }

    #[test]
    fn renders_ruby_require_path_instead_of_hyphenated_gem_name() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.call.module = "sample".into();
        e2e.packages.insert(
            "ruby".into(),
            crate::e2e::config::PackageRef {
                name: Some("sample-tools".into()),
                ..Default::default()
            },
        );

        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();
        assert!(body.contains("require \"sample_tools\""), "{body}");
        assert!(!body.contains("require \"sample-tools\""), "{body}");
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
        e2e.call.module = "sample".into();
        e2e.call.result_var = "result".into();
        e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();

        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(body.contains("result = Sample.process()"), "{body}");
        assert!(body.contains("puts result.summary\n"), "{body}");
        assert!(body.contains("result.items.each do |item|"), "{body}");
        assert!(body.contains("puts item.label.inspect"), "{body}");
        assert!(
            !body.contains("puts result.inspect"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
    }

    #[test]
    fn renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet");

        assert!(body.contains("rescue StandardError => error"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
    }

    #[test]
    fn renders_http_request_without_rspec_assertions() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "create_item", "description": "Create item", "input": null,
            "http": {
                "handler": {"route": "/items", "method": "POST"},
                "request": {"method": "POST", "path": "/items", "body": {"name": "sample"}},
                "expected_response": {"status_code": 201}
            }
        }))
        .unwrap();
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .unwrap();
        assert!(body.contains("Net::HTTP::Post"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("request.body = { 'name' => 'sample' }.to_json"), "{body}");
        assert!(body.contains(".to_json\n\nresponse ="), "{body}");
        assert!(!body.contains("expect("));
    }

    #[test]
    fn reads_nested_typed_dto_files() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "document_input", "description": "Read a document",
            "input": {"request": {"content": "ignored"}}, "assertions": [],
            "docs": {"topic": "documents", "presentation": {
                "files": [{"field": "/request/content", "path": "document.pdf"}]
            }}
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.call.module = "sample".into();
        e2e.call.args = vec![ArgMapping {
            name: "request".into(),
            field: "request".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "ruby".into(),
            CallOverride {
                options_type: Some("DocumentRequest".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture.docs_call_fixture(),
            &e2e,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet");

        assert!(
            body.contains("DocumentRequest.new(content: File.binread('document.pdf').bytes)"),
            "{body}"
        );
    }
}
