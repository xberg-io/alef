use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

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
    let lang = "elixir";
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
    let raw_module = override_config
        .and_then(|value| value.module.as_deref())
        .unwrap_or(&call.module);
    let module = if raw_module.contains('.') || raw_module.chars().next().is_some_and(char::is_uppercase) {
        raw_module.to_string()
    } else {
        super::values::elixir_module_name(raw_module)
    };
    let mut function = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_string();
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    if call.r#async && !function.ends_with("_async") && !is_streaming {
        function.push_str("_async");
    }
    let request_type = config
        .adapters
        .iter()
        .find(|value| value.name == call.function)
        .and_then(|value| value.request_type.as_deref())
        .and_then(|value| value.rsplit("::").next());
    let (mut setup_lines, mut args, _) = super::args::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &module,
        recipe.options_type,
        override_config.and_then(|value| value.options_via.as_deref()),
        &HashMap::new(),
        fixture,
        override_config.and_then(|value| value.handle_struct_type.as_deref()),
        &HashSet::new(),
        &e2e_config.test_documents_relative_from(0),
        request_type,
        enums,
        config,
        type_defs,
        false,
    );
    if let Some(visitor) = &fixture.visitor {
        let visitor_arg = super::visitor::build_elixir_visitor(&mut setup_lines, visitor);
        args = inject_visitor_into_options(&mut setup_lines, &args, &visitor_arg);
    }
    if !recipe.extra_args.is_empty() {
        args = [args, recipe.extra_args.join(", ")]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let call_args = apply_client_factory(
        &mut setup_lines,
        args,
        ClientFactoryContext {
            fixture,
            e2e_config,
            call,
            override_config,
            module: &module,
        },
    );
    let call_expr = format!("{module}.{function}({call_args})");
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let presentation =
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang, type_defs, enums, functions);
    Ok(crate::e2e::template_env::render(
        "elixir/snippet_body.jinja",
        minijinja::context! {
            setup_lines => setup_lines, call_expr => call_expr, result_var => call.effective_result_var(),
            returns_void => call.returns_void, is_streaming => is_streaming,
            expects_error => expects_error, presentation => presentation,
        },
    ))
}

struct ClientFactoryContext<'a> {
    fixture: &'a Fixture,
    e2e_config: &'a E2eConfig,
    call: &'a crate::e2e::config::CallConfig,
    override_config: Option<&'a crate::e2e::config::CallOverride>,
    module: &'a str,
}

/// Construct the client a `client_factory` binding's functions take as their first
/// argument, appending the construction to `setup_lines` and returning the argument
/// list for the call itself.
///
/// The Elixir binding is not a bare-module API when a `client_factory` is configured:
/// `elixir/test_case.rs` creates `{:ok, client} = <Module>.<factory>(...)` and prefixes
/// `client` to every call's arguments, so the exported function's arity includes it. A
/// snippet that rendered `<Module>.chat(request)` therefore documented a function that
/// does not exist. Unlike the e2e suite the reader gets no mock server: the credential
/// comes from the environment and the base URL is left at the binding default unless the
/// fixture's `docs.client` names one, matching `java/snippet_body.jinja` and
/// `csharp/snippet_body.jinja`.
fn apply_client_factory(setup_lines: &mut Vec<String>, args: String, context: ClientFactoryContext<'_>) -> String {
    let ClientFactoryContext {
        fixture,
        e2e_config,
        call,
        override_config,
        module,
    } = context;
    let Some(factory) = override_config
        .and_then(|value| value.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get("elixir")
                .and_then(|value| value.client_factory.as_deref())
        })
    else {
        return args;
    };
    let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    // `fetch_env!/1` is the Elixir spelling of "this credential is required": it raises
    // `System.EnvError` naming the variable, so the snippet needs no hand-written guard
    // clause the way the Java and C# templates do. ~keep
    setup_lines.push(format!("api_key = System.fetch_env!(\"{api_key_var}\")"));
    let docs_client = fixture.docs_client();
    let mut options: Vec<String> = crate::e2e::codegen::client_factory::docs_base_url(docs_client)
        .map(|url| format!("base_url: \"{}\"", crate::e2e::escape::escape_elixir(url)))
        .into_iter()
        .collect();
    options.extend(crate::e2e::codegen::client_factory::trailing_args(
        docs_client,
        e2e_config,
        call,
        "elixir",
        &[],
    ));
    let factory_args = if options.is_empty() {
        "api_key".to_string()
    } else {
        format!("api_key, {}", options.join(", "))
    };
    setup_lines.push(format!("{{:ok, client}} = {module}.{factory}({factory_args})"));
    if args.is_empty() {
        "client".to_string()
    } else {
        format!("client, {args}")
    }
}

fn inject_visitor_into_options(setup_lines: &mut Vec<String>, args: &str, visitor: &str) -> String {
    let parts = args.split(", ").collect::<Vec<_>>();
    match parts.as_slice() {
        [input, "nil"] => format!("{input}, %{{visitor: {visitor}}}"),
        [input, options] => {
            setup_lines.push(format!("{options} = Map.put({options}, :visitor, {visitor})"));
            format!("{input}, {options}")
        }
        [input] => format!("{input}, %{{visitor: {visitor}}}"),
        _ => args.to_string(),
    }
}

fn render_http_snippet(fixture: &Fixture) -> Result<String> {
    let http = fixture.http.as_ref().expect("HTTP fixture checked by caller");
    let plan = crate::e2e::codegen::client::http_call::plan_request(http);
    let mut headers = plan.headers;
    if let Some(content_type) = &plan.content_type
        && !headers.keys().any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("content-type".into(), content_type.clone());
    }
    if !http.request.cookies.is_empty() {
        headers.insert(
            "cookie".into(),
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
        "elixir/http_snippet.jinja",
        minijinja::context! {
            method => http.request.method.to_lowercase(),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| minijinja::context! {
                key => crate::e2e::escape::escape_elixir(key), value => crate::e2e::escape::escape_elixir(value),
            }).collect::<Vec<_>>(),
            body => plan.body.as_ref().map(super::values::json_to_elixir),
            raw_body => raw_body,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::StreamingConfig;
    use crate::e2e::config::{ArgMapping, CallOverride};

    #[test]
    fn nests_visitor_in_options_argument() {
        let mut setup = Vec::new();
        assert_eq!(
            inject_visitor_into_options(&mut setup, "html, nil", "visitor"),
            "html, %{visitor: visitor}"
        );
        assert_eq!(
            inject_visitor_into_options(&mut setup, "html, options", "visitor"),
            "html, options"
        );
        assert_eq!(setup, ["options = Map.put(options, :visitor, visitor)"]);
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
        assert!(body.contains("IO.puts(result.summary)"), "{body}");
        assert!(body.contains("Enum.each(result.items, fn item ->"), "{body}");
        assert!(body.contains("IO.inspect(item.label)"), "{body}");
        assert!(
            !body.contains("IO.inspect(result)"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
    }

    #[test]
    fn renders_native_call_without_exunit() {
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
        assert!(!body.contains("assert"));
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

        assert!(
            body.contains("stream_result = Sample.stream_items() |> Enum.to_list()"),
            "{body}"
        );
        assert!(body.contains("IO.inspect(stream_result)"), "{body}");
        assert!(!body.contains("chunks ="), "{body}");
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
        e2e.call.module = "sample".into();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "elixir".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        e2e
    }

    /// The Elixir binding is a client API when `client_factory` is configured — every
    /// generated call takes `client` as its first argument (see `elixir/test_case.rs`) —
    /// so a snippet that emitted a bare `Sample.chat()` documented a function that does
    /// not exist, on top of never showing the reader how to authenticate. ~keep
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let body = render_snippet_body(
            &client_factory_fixture(),
            &client_factory_e2e(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("api_key = System.fetch_env!(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
        assert!(
            body.contains("{:ok, client} = Sample.create_client(api_key)"),
            "snippet does not construct a client at all:\n{body}"
        );
        assert!(
            body.contains("result = Sample.chat(client)"),
            "the call must take the client the binding requires:\n{body}"
        );
    }

    #[test]
    fn a_documented_base_url_reaches_the_client_options() {
        let mut fixture = client_factory_fixture();
        fixture.docs = Some(
            serde_json::from_value(serde_json::json!({
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }))
            .expect("fixture docs"),
        );

        let body = render_snippet_body(
            &fixture,
            &client_factory_e2e(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        let expected = "{:ok, client} = Sample.create_client(api_key, \
                        base_url: \"https://llm.internal.example.com/v1\")";
        assert!(body.contains(expected), "{body}");
    }

    /// Companion pin: the e2e suite runs against the mock server, so `test_case`'s own
    /// output for the same fixture must keep pointing at it. Only the snippet renderer
    /// substitutes a reader-facing client. ~keep
    #[test]
    fn e2e_test_case_still_points_the_client_at_the_mock_server() {
        let fixture = client_factory_fixture();
        let e2e = client_factory_e2e();
        let mut rendered = String::new();
        super::super::test_case::render_test_case(
            &mut rendered,
            &fixture,
            &e2e,
            "Sample",
            "chat",
            "result",
            &[],
            None,
            None,
            &HashMap::new(),
            None,
            &HashSet::new(),
            &[],
            &[],
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
        );

        let expected = concat!(
            "{:ok, client} = Sample.create_client(\"test-key\", base_url: ",
            "(System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/rate_limit_429\")"
        );
        assert!(rendered.contains(expected), "{rendered}");
        assert!(rendered.contains("Sample.chat(client)"), "{rendered}");
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

        assert!(body.contains("rescue\n  error ->"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
    }

    #[test]
    fn renders_http_request_without_exunit_assertions() {
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
        assert!(body.contains("Req.request"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("json: %{\"name\" => \"sample\"}"));
        assert!(!body.contains("assert"));
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
        e2e.call.module = "Sample".into();
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
            "elixir".into(),
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

        // The doc snippet shows the byte-integer-list form, not a bare `File.read!`, because the
        // documented call path really does JSON-encode: `Xberg.extract/1` sends a struct through
        // `Jason.encode!` (packages/elixir/lib/xberg.ex), and a raw binary of non-UTF-8 file bytes
        // raises `Jason.EncodeError` there. A snippet reading `File.read!` documented code that
        // crashed on any real PDF. See alef#308. ~keep
        assert!(
            body.contains("%Sample.DocumentRequest{content: :binary.bin_to_list(File.read!(\"document.pdf\"))}"),
            "{body}"
        );
    }
}
