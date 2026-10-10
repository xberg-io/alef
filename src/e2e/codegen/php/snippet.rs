use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
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
    let lang = "php";
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
    let extension = config.php_extension_name();
    let default_namespace = crate::backends::php::naming::php_autoload_namespace(config);
    let namespace = override_config
        .and_then(|value| value.module.as_deref())
        .unwrap_or(&default_namespace);
    let class_name = override_config
        .and_then(|value| value.class.as_deref())
        .and_then(|value| value.split('\\').next_back())
        .map(str::to_string)
        .unwrap_or_else(|| extension.to_upper_camel_case());
    let mut function_name = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_string();
    if override_config.and_then(|value| value.function.as_ref()).is_none() {
        function_name = function_name.to_lower_camel_case();
    }
    let adapter_lookup_name = call.core_lookup_name(lang);
    let adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|value| value.name == name));
    let request_type = adapter
        .and_then(|value| value.request_type.as_deref())
        .and_then(|value| value.rsplit("::").next());
    let owner_handle = adapter
        .is_some_and(|value| value.owner_type.is_some())
        .then(|| {
            recipe
                .args
                .iter()
                .find(|arg| arg.arg_type == "handle")
                .map(|arg| arg.name.clone())
        })
        .flatten();
    let options_via = override_config
        .and_then(|value| value.options_via.as_deref())
        .unwrap_or("array");
    let mut imports = Vec::new();
    let (mut setup_lines, mut args, _) = super::args::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &class_name,
        &HashMap::new(),
        fixture,
        options_via,
        recipe.options_type,
        request_type,
        namespace,
        owner_handle.is_some(),
        type_defs,
        &config.serde_rename_all_for_language(crate::core::config::Language::Php),
        config,
        &mut imports,
        true,
    );
    if let Some(visitor) = &fixture.visitor {
        super::visitor::build_php_visitor(&mut setup_lines, visitor);
        let Some(options_type) = recipe
            .options_type
            .or_else(|| super::stubs::trait_bridge_options_type(config))
        else {
            bail!(
                "PHP documentation snippet `{}` needs an options type for its visitor",
                fixture.id
            );
        };
        if options_via == "from_json" {
            setup_lines.push(format!("$options = \\{namespace}\\{options_type}::from_json('{{}}');"));
            setup_lines.push(format!(
                "$visitorHandle = \\{namespace}\\VisitorHandle::from_php_object($visitor);"
            ));
            setup_lines.push("$options = $options->withVisitor($visitorHandle);".to_string());
        } else {
            setup_lines.push(format!("$builder = \\{namespace}\\{options_type}::builder();"));
            setup_lines.push("$options = $builder->visitor($visitor)->build();".to_string());
        }
        if args != "$options" {
            args = [args, "$options".to_string()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
        }
    }
    if !recipe.extra_args.is_empty() {
        args = [args, recipe.extra_args.join(", ")]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
    }
    // ~keep PHP projects name their factory `php_client_factory`; `client_factory` is not read by
    // the PHP e2e suite, so reading it here left every PHP snippet on the static facade.
    let client_factory = crate::e2e::codegen::client_factory::php_factory(e2e_config, call);
    let receiver = owner_handle.map(|value| format!("${value}")).unwrap_or_else(|| {
        client_factory
            .map(|_| "$client".to_string())
            .unwrap_or_else(|| class_name.clone())
    });
    let operator = if receiver.starts_with('$') { "->" } else { "::" };
    let call_expr = format!("{receiver}{operator}{function_name}({args})");
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    let config_call = client_factory
        .and_then(|_| crate::e2e::codegen::client_factory::client_config_call(fixture, e2e_config, call, lang));
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let mut imported_types = type_defs
        .iter()
        .filter(|type_def| {
            let name = &type_def.name;
            name != &class_name && (setup_lines.iter().any(|line| line.contains(name)) || args.contains(name))
        })
        .map(|type_def| type_def.name.as_str())
        .collect::<Vec<_>>();
    imported_types.sort_unstable();
    imported_types.dedup();
    let field_resolver = crate::e2e::field_access::FieldResolver::new_with_error_aliases(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
        &HashMap::new(),
    )
    .with_php_getter_map(super::types::build_php_getter_map(
        type_defs,
        enums,
        call,
        e2e_config.effective_result_fields(call),
    ));
    let presentation = crate::e2e::codegen::presentation::resolve_with(
        fixture,
        e2e_config,
        lang,
        &field_resolver,
        type_defs,
        enums,
        functions,
    );
    let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let body = crate::e2e::template_env::render(
        "php/snippet_body.jinja",
        crate::alef_context! {
            namespace => namespace, class_name => class_name, setup_lines => setup_lines,
            client_factory => config_call.as_ref().map(|(from_json_factory, _)| *from_json_factory).or(client_factory),
            client_args => config_call.as_ref().map_or_else(
                || "$apiKey".to_string(),
                |(_, json)| format!("'{}'", crate::e2e::escape::escape_php_single(json)),
            ),
            reads_api_key => config_call.is_none(),
            call_expr => call_expr, result_var => call.effective_result_var(),
            returns_void => call.returns_void, is_streaming => is_streaming, imported_types => imported_types,
            expects_error => expects_error, presentation => presentation, api_key_var => api_key_var,
        },
    );
    // A `configuration/custom-base-url`-style topic documents `docs.client.base_url` so the
    // reader sees the setting the topic is about, mirroring the Java/Rust/Elixir/Python
    // generators' `docs_client` handling. `snippet_body.jinja` always renders the client
    // construction as `::{factory}($apiKey);` immediately after the credential read above, so
    // targeting that exact substring here (rather than threading `docs_client` through the
    // shared executable-suite construction in `test_method.rs`) keeps this docs-only concern
    // out of the path the executable e2e suite also uses. ~keep
    let body = match client_factory.zip(crate::e2e::codegen::client_factory::docs_base_url(
        fixture.docs_client(),
    )) {
        Some((factory, base_url)) => {
            let bare_call = format!("::{factory}($apiKey);");
            let with_base_url = format!(
                "::{factory}($apiKey, \"{}\");",
                crate::e2e::escape::escape_php(base_url)
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
        "php/http_snippet.jinja",
        crate::alef_context! {
            method => http.request.method.to_uppercase(),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| crate::alef_context! {
                key => crate::e2e::escape::escape_php(key), value => crate::e2e::escape::escape_php(value),
            }).collect::<Vec<_>>(),
            body => plan.body.as_ref().map(super::values::json_to_php),
            raw_body => raw_body,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::CallOverride;

    /// Pins that a `client_factory` docs snippet never points the reader at the e2e
    /// mock server: no `MOCK_SERVER` env var, no `/fixtures/<id>` route, and no
    /// inlined `'test-key'` credential (the mock-mode branches the generated
    /// PHPUnit test takes live in `test_method.rs` around lines 299-314).
    ///
    /// PHP now follows java/csharp/kotlin/zig/elixir and reads the credential from the
    /// environment via `FixtureEnv::api_key_var_or_default`. An inlined placeholder is not
    /// a harness leak, but it is worse documentation — it shows a reader the one thing the
    /// snippet should never teach — and it is not configurable per project, whereas
    /// `env.api_key_var` is.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "php".into(),
            CallOverride {
                php_client_factory: Some("createClient".into()),
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
        assert!(
            !body.contains("'your-api-key'"),
            "credential is inlined rather than read from the environment:\n{body}"
        );
        assert!(
            body.contains("$apiKey = getenv('API_KEY');"),
            "snippet does not read the credential from the environment:\n{body}"
        );
        assert!(
            body.contains("::createClient($apiKey);"),
            "client is not constructed from the environment-read credential:\n{body}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its PHP
    /// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling
    /// (`java/snippet.rs::a_snippet_renders_the_base_url_the_fixture_documents`,
    /// `python/mod.rs::client_factory_snippet_renders_the_base_url_the_fixture_documents`).
    /// Paired with `client_factory_snippet_never_points_the_reader_at_the_mock_server` above
    /// (whose fixture declares no `docs.client` and must keep rendering the bare,
    /// no-`base_url` call) as the negative control: an indiscriminate "always add base_url"
    /// change would fail that test. ~keep
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
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "php".into(),
            CallOverride {
                php_client_factory: Some("createClient".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

        assert!(
            body.contains("::createClient($apiKey, \"https://llm.internal.example.com/v1\");"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
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
        e2e.call.module = "Sample".into();
        e2e.call.result_var = "result".into();
        e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet");

        assert!(body.contains("$result = Sample::process();"), "{body}");
        assert!(body.contains("echo $result->summary, PHP_EOL;"), "{body}");
        assert!(body.contains("foreach ($result->items as $item) {"), "{body}");
        assert!(body.contains("var_dump($item->label);"), "{body}");
        assert!(
            !body.contains("var_dump($result);"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
    }

    #[test]
    fn renders_native_call_without_phpunit() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.call.module = "Sample".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).unwrap();
        assert!(body.contains("Sample::loadDocument()"));
        assert!(!body.contains("assert"));
    }

    #[test]
    fn renders_strict_executable_with_composer_autoload() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();

        assert!(body.contains("declare(strict_types=1);"), "{body}");
        assert!(
            body.contains("require_once __DIR__ . '/vendor/autoload.php';"),
            "{body}"
        );
    }

    #[test]
    fn expected_error_snippet_handles_the_exception() {
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
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");
        assert!(body.contains("catch (Throwable $error)"));
        assert!(!body.contains("expected call to fail"));
    }

    #[test]
    fn renders_http_request_without_phpunit_assertions() {
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
        assert!(body.contains("new Client"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("'json' => [\"name\" => \"sample\"]"), "{body}");
        assert!(!body.contains("assert"));
    }

    fn visitor_fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "visitor_link_rewrite",
            "description": "Visitor rewrites links",
            "input": {"html": "<a href=\"a\">a</a>"},
            "visitor": {"callbacks": {"visit_link": {"action": "skip"}}}
        }))
        .expect("fixture")
    }

    fn visitor_e2e() -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "html".into(),
            field: "input.html".into(),
            arg_type: "string".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e
    }

    fn bridge_config(options_type: Option<&str>) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "HtmlVisitor".into(),
                type_alias: Some("VisitorHandle".into()),
                param_name: Some("visitor".into()),
                options_type: options_type.map(str::to_string),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    /// A visitor fixture with no resolvable options type is a configuration defect —
    /// there is nowhere to attach the declared visitor — so the snippet must fail closed
    /// rather than fabricate a type name (C#) or drop the visitor (Go). This pins the
    /// message the snippet pipeline surfaces as an undocumented coverage gap. ~keep
    #[test]
    fn visitor_without_a_trait_bridge_options_type_fails_closed() {
        let error = render_snippet_body(&visitor_fixture(), &visitor_e2e(), &bridge_config(None), &[], &[])
            .expect_err("a visitor with no options type must not render");

        assert_eq!(
            format!("{error}"),
            "PHP documentation snippet `visitor_link_rewrite` needs an options type for its visitor"
        );
    }

    /// Positive control for the above: with the bridge's `options_type` configured, the
    /// ordinary visitor path is unchanged and builds against the real type. ~keep
    #[test]
    fn visitor_with_a_trait_bridge_options_type_builds_against_the_real_type() {
        let body = render_snippet_body(
            &visitor_fixture(),
            &visitor_e2e(),
            &bridge_config(Some("ConversionOptions")),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("ConversionOptions::builder();"), "{body}");
        assert!(
            body.contains("$options = $builder->visitor($visitor)->build();"),
            "{body}"
        );
    }
}
