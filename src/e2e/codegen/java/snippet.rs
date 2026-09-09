use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

use super::args::{JavaArgsContext, build_args_and_setup};

/// Render a Java documentation snippet without any core IR to consult.
///
/// Kept as the four-argument entry point every existing caller and test already uses: with no
/// `functions` the seam resolves to `TargetParams::IrAbsent`, which is exactly the state this
/// path was always in, so its output is unchanged by the seam. ~keep
pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
) -> String {
    render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, &[], &[])
}

pub(super) fn render_snippet_body_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> String {
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    // A Java enhanced-for declares its binding in the enclosing method's scope, so a loop named
    // after the result it iterates is "variable <name> is already defined". Decided before
    // anything renders — the per-item field accessors below are rooted at this name. ~keep
    let unshadowed =
        crate::e2e::codegen::loop_binding::without_shadowed_loop_bindings(fixture, &[call.effective_result_var()]);
    let fixture = unshadowed.as_ref();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("java", fixture, call, type_defs)
        .with_functions(functions);
    let target_params = recipe.target_params("java");
    let overrides = recipe.override_config;
    let class_name = overrides
        .and_then(|value| value.class.clone())
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "csharp", "c", "go", "python"]));

    // Resolve the same adapter metadata `java/test_method.rs` resolves for the generated e2e
    // test, so a docs snippet for a streaming (or other adapter-backed) call shows the real
    // request-DTO call shape the facade actually declares, instead of the flat scalar-argument
    // shape a plain call takes. Without this the snippet always rendered `Facade.method(engine,
    // url)` even when the facade signature is `Facade.method(engine, SomeRequest req)` --
    // `String` is not assignable to a request DTO. ~keep
    let adapter_lookup_name = call.core_lookup_name("java");
    let adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|a| a.name == name));
    let is_streaming_adapter =
        adapter.is_some_and(|a| matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming));
    let adapter_request_type: Option<String> = adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
    // Mirrors `java/test_method.rs`'s `filtered_args`: a non-streaming owner_type adapter
    // encapsulates the handle inside the facade call, so its handle `ArgMapping` must not also
    // be passed positionally. A streaming owner_type adapter keeps it -- it becomes the
    // instance-method receiver below, not a positional argument.
    let filtered_args: Vec<crate::e2e::config::ArgMapping> =
        if adapter.is_some_and(|a| a.owner_type.is_some()) && !is_streaming_adapter {
            recipe
                .args
                .iter()
                .filter(|arg| arg.arg_type != "handle")
                .cloned()
                .collect()
        } else {
            recipe.args.to_vec()
        };
    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the owner handle
    // (`engine.someStream(req)`), not as static facade methods -- see `test_method.rs`'s
    // identical `streaming_owner_handle`.
    let streaming_owner_handle: Option<String> =
        if is_streaming_adapter && adapter.is_some_and(|a| a.owner_type.is_some()) {
            filtered_args
                .iter()
                .find(|a| a.arg_type == "handle")
                .map(|a| a.name.clone())
        } else {
            None
        };

    let mut teardown = String::new();
    let (mut setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        &filtered_args,
        JavaArgsContext {
            class_name: &class_name,
            options_type,
            fixture,
            adapter_request_type: adapter_request_type.as_deref(),
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
            config,
            type_defs,
            enums,
            target_params,
            teardown_block: &mut teardown,
        },
    );
    setup_lines.splice(0..0, render_json_object_setup(fixture, recipe.args, options_type));
    if let Some(visitor_spec) = &fixture.visitor
        && let Some(binding) = super::visitor::java_visitor_binding(config, type_defs, Some(visitor_spec), options_type)
    {
        let visitor = super::visitor::build_java_visitor(&mut setup_lines, visitor_spec, &class_name, &binding);
        args = super::visitor::apply_java_visitor_arg(&mut setup_lines, &args, recipe.args, &visitor, &binding);
    }
    if !recipe.extra_args.is_empty() {
        args = if args.is_empty() {
            recipe.extra_args.join(", ")
        } else {
            format!("{args}, {}", recipe.extra_args.join(", "))
        };
    }
    let client_factory = overrides
        .and_then(|value| value.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get("java")
                .and_then(|value| value.client_factory.as_deref())
        })
        .map(ToLowerCamelCase::to_lower_camel_case);
    let client_args = render_client_factory_args(fixture, e2e_config, call);
    let package_name = overrides
        .and_then(|value| value.module.clone())
        .unwrap_or_else(|| config.java_package());
    // The snippet already emits `import <package>.*;`, so spelling the package again at the call
    // site renders `io.example.pkg.Facade.convert(...)` under an import that made `Facade` alone
    // sufficient. Consumers configure `[e2e.call.overrides.java] class` fully qualified because
    // that is how the binding names it; the docs example is where that stops being useful. Only
    // the exact configured package is stripped — a class from anywhere else stays qualified,
    // because no import covers it. ~keep
    let simple_class_name = class_name
        .strip_prefix(&format!("{package_name}."))
        .filter(|rest| !rest.contains('.'))
        .unwrap_or(&class_name)
        .to_string();
    let needs_mapper = setup_lines.iter().any(|line| line.contains("MAPPER"));
    // The template renders `{{ class_name }}.{{ function_name }}(...)` unconditionally.
    // Substitute the owner handle's variable name for a streaming owner_type adapter's
    // flat-call shape, so the snippet shows the real instance-method invocation
    // (`engine.someStream(req)`) instead of a nonexistent static overload. `client_factory`
    // fixtures already dispatch on `client`, not `class_name`, so this substitution is scoped
    // to the flat-call path where it actually changes the rendered call target -- mirrors
    // `kotlin/snippet.rs`'s identical `call_target_class_name`. ~keep
    let call_target_class_name = if client_factory.is_none() {
        streaming_owner_handle.unwrap_or_else(|| simple_class_name.clone())
    } else {
        simple_class_name.clone()
    };
    let presentation =
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "java", type_defs, enums, functions);
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    // The checked-exception class the Java backend actually declares is named by
    // `backends::java::naming::exception_class_name`, not by suffixing the facade class:
    // `simple_class_name` has its `Rs` marker stripped, but the exception class keeps it, so
    // suffixing the facade names a class that does not exist. Every docs site
    // (`docs/signatures.rs`, `docs/formatting.rs`) already calls this function for the same
    // reason -- both sides must name the class the backend really emits, not re-derive a
    // spelling of their own. ~keep
    let exception_class = crate::backends::java::naming::exception_class_name(&config.name);
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());

    crate::e2e::template_env::render(
        "java/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            class_name => call_target_class_name,
            setup_lines => setup_lines,
            client_factory => client_factory,
            client_args => client_args,
            function_name => function_name,
            args => args,
            result_var => call.effective_result_var(),
            returns_void => call.returns_void,
            needs_mapper => needs_mapper,
            fixture_id => fixture.id,
            stream_item => crate::e2e::codegen::presentation::stream_item_binding(fixture, e2e_config, "java"),
            presentation => presentation,
            expects_error => expects_error,
            exception_class => exception_class,
            api_key_var => api_key_var,
        },
    )
}

/// Argument list appended to a `client_factory` call when the project configures no
/// `[e2e.call.overrides.java] client_factory_trailing_args`.
///
/// These were hardcoded into `java/snippet_body.jinja` before the override was wired
/// up and remain the default, so a project that has not adopted the key keeps the
/// argument list it renders today.
const JAVA_CLIENT_FACTORY_FALLBACK_ARGS: [&str; 3] = ["null", "null", "null"];

/// The full argument list for a snippet's `client_factory` call: the credential, the
/// base URL, and whatever trails them.
///
/// The credential is always the `apiKey` local the template declares just above the
/// call — a snippet must read it from the environment rather than inline a literal.
fn render_client_factory_args(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call: &crate::e2e::config::CallConfig,
) -> String {
    let docs_client = fixture.docs_client();
    let base_url = match crate::e2e::codegen::client_factory::docs_base_url(docs_client) {
        Some(url) => format!("\"{}\"", crate::e2e::escape::escape_java(url)),
        None => "null".to_string(),
    };
    let trailing = crate::e2e::codegen::client_factory::trailing_args(
        docs_client,
        e2e_config,
        call,
        "java",
        &JAVA_CLIENT_FACTORY_FALLBACK_ARGS,
    );
    let mut args = vec!["apiKey".to_string(), base_url];
    args.extend(trailing);
    args.join(", ")
}

fn render_json_object_setup(
    fixture: &Fixture,
    args: &[crate::e2e::config::ArgMapping],
    options_type: Option<&str>,
) -> Vec<String> {
    args.iter()
        .filter_map(|arg| {
            if arg.arg_type != "json_object" {
                return None;
            }
            let value = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
            let type_name = crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, value)?;
            if value.is_null() || value.is_array() {
                return None;
            }
            let mut normalized = crate::e2e::codegen::transform_json_keys_for_language(value, "snake_case");
            let files = fixture.docs_files_for_arg(&arg.field);
            let file_reads = files
                .iter()
                .enumerate()
                .filter_map(|(index, file)| {
                    let marker = format!("__ALEF_DOC_FILE_{index}__");
                    let target = if file.field.is_empty() {
                        Some(&mut normalized)
                    } else {
                        normalized.pointer_mut(&file.field)
                    }?;
                    *target = serde_json::Value::String(marker.clone());
                    Some((index, marker, file.path.clone()))
                })
                .collect::<Vec<_>>();
            let json = serde_json::to_string(&normalized).unwrap_or_default();
            Some(crate::e2e::template_env::render(
                "java/snippet_json_object_setup.jinja",
                minijinja::context! {
                    variable => arg.name,
                    // A full Java string-literal expression (quoted, or `+`-chunked when the
                    // fixture body is long enough to threaten the JVM's 65535-byte constant
                    // cap) -- not just escaped content -- so the template can splice it in
                    // without assuming a single `"..."` literal is safe. See
                    // `values::java_string_literal`. ~keep
                    json_literal => super::values::java_string_literal(&json),
                    type_name => type_name,
                    file_reads => file_reads,
                },
            ))
        })
        .flat_map(|block| split_rendered_lines(&block))
        .collect()
}

/// ~keep `java/snippet_body.jinja` indents `setup_lines` by prepending the method
/// body's indent to each *entry* once (`        {{ line }}`), not to every physical
/// line inside it. A producer that renders a multi-statement block (this file's own
/// `snippet_json_object_setup.jinja` emits a `varJson = "...";` line and a separate
/// `var = JsonUtil.fromJson(...)` line, plus a base64-file-read block spanning three
/// lines) and pushes it as one `String` therefore gets the indent on only the first
/// physical line — every line after it renders flush left. Regression: every
/// generated Java snippet with a `json_object` arg had its `JsonUtil.fromJson(...)`
/// line at column 0 instead of the surrounding method's indent. Splitting the
/// rendered block here, so each physical line becomes its own `setup_lines` entry,
/// lets the template's per-entry indent apply uniformly while preserving each
/// producer's own relative indentation between lines (e.g. the base64 read's
/// continuation line and closing `);`).
pub(super) fn split_rendered_lines(block: &str) -> Vec<String> {
    block.trim_end().lines().map(str::to_string).collect()
}

#[cfg(test)]
#[path = "snippet/oversized_literal_javac_tests.rs"]
mod oversized_literal_javac_tests;

#[cfg(test)]
#[path = "snippet/streaming_tests.rs"]
mod streaming_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
        body.lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
    }

    fn leading_spaces(line: &str) -> usize {
        line.len() - line.trim_start().len()
    }

    #[test]
    fn snippet_keeps_native_call_without_junit_harness() {
        let fixture = Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                class: Some("DocumentApi".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(body.contains("public static void main(String[] args) throws Exception"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
        assert!(body.contains("System.out.println(document);"));
    }

    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(!body.contains("mockServer"), "mock-server property leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("System.getenv(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
        assert!(
            body.contains("createClient(apiKey, null, null, null, null)"),
            "an unconfigured project must keep the argument list it renders today:\n{body}"
        );
    }

    fn client_release_snippet(expects_error: bool) -> String {
        let mut fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        if expects_error {
            fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        }
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &config,
            &[],
        )
    }

    /// The generated Java facade class implements `AutoCloseable` (`service_close.jinja` /
    /// `opaque_handle_header.jinja` in the Java backend), so try-with-resources is the correct,
    /// idiomatic release for a client a docs snippet constructs. Mirrors the C# lane's `using var
    /// client`, which is the same construct for the same reason. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_in_a_try_with_resources_block() {
        let body = client_release_snippet(false);

        assert!(
            body.contains("try (var client = Sample.createClient(apiKey, null, null, null, null)) {"),
            "the client must be constructed inside a try-with-resources header:\n{body}"
        );
        assert!(
            body.contains("var result = client.chat();"),
            "the call moves onto the client the try-with-resources declares:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_releases_the_client_in_a_try_with_resources_block`.
    /// A `try (resource) { ... } catch (...) { ... }` releases the resource before the catch runs
    /// regardless of which statement inside the try threw — unlike the pre-existing Kotlin
    /// template, whose bare `client.close()` sat after the call and was skipped whenever the call
    /// itself threw. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_release_snippet(true);

        let try_with_resources = body
            .find("try (var client = Sample.createClient")
            .expect("expects-error snippet still constructs the client via try-with-resources");
        let catch_clause = body
            .find("catch (SampleRsException error)")
            .expect("catch clause present");
        assert!(
            try_with_resources < catch_clause,
            "the catch must follow the try-with-resources that declares the client:\n{body}"
        );
        assert!(
            !body.contains("} catch (SampleRsException error)"),
            "the try-with-resources closes on its own line before catch, not on the same brace:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so its snippet must be byte-for-byte
    /// what it was — no `try (`, no `AutoCloseable` resource, and the existing plain `try { ... }
    /// catch` shape for `expects_error`. A change that wraps every snippet's call in
    /// try-with-resources unconditionally would fail here. ~keep
    #[test]
    fn snippet_without_a_client_factory_is_unchanged() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &E2eConfig::default(), &config, &[]);

        assert!(
            !body.contains("try ("),
            "a snippet that constructs no client must emit no try-with-resources:\n{body}"
        );
        assert!(
            body.contains("        try {\n"),
            "the plain expects_error try block must be unchanged:\n{body}"
        );
        assert!(
            body.contains("        } catch (SampleRsException error) {\n"),
            "the plain expects_error catch must still close the try on the same line:\n{body}"
        );
    }

    fn client_snippet(docs: Option<serde_json::Value>, trailing_args: &[&str]) -> String {
        let mut fixture = Fixture {
            id: "custom_base_url".into(),
            description: "Custom base URL".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        fixture.docs = docs.map(|value| serde_json::from_value(value).expect("fixture docs"));
        let mut call = CallConfig {
            function: "chat".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
                client_factory_trailing_args: trailing_args.iter().map(|arg| (*arg).to_string()).collect(),
                ..CallOverride::default()
            },
        );
        render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        )
    }

    #[test]
    fn configured_trailing_args_replace_the_templates_hardcoded_nulls() {
        let body = client_snippet(None, &["java.time.Duration.ofSeconds(30)", "2", "null"]);
        assert!(
            body.contains("createClient(apiKey, null, java.time.Duration.ofSeconds(30), 2, null)"),
            "{body}"
        );
    }

    #[test]
    fn a_snippet_renders_the_base_url_the_fixture_documents() {
        let body = client_snippet(
            Some(serde_json::json!({
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            })),
            &[],
        );
        assert!(
            body.contains("createClient(apiKey, \"https://llm.internal.example.com/v1\", null, null, null)"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    #[test]
    fn a_fixture_scoped_argument_list_outranks_the_configured_one() {
        let body = client_snippet(
            Some(serde_json::json!({
                "topic": "configuration",
                "client": {"args": {"java": ["30", "null", "null"]}}
            })),
            &["1", "1", "1"],
        );
        assert!(body.contains("createClient(apiKey, null, 30, null, null)"), "{body}");
    }

    #[test]
    fn a_documentation_client_declared_for_another_language_does_not_reach_java() {
        let body = client_snippet(
            Some(serde_json::json!({
                "topic": "configuration",
                "client": {"args": {"rust": ["Some(30)", "None", "None"]}}
            })),
            &[],
        );
        assert!(body.contains("createClient(apiKey, null, null, null, null)"), "{body}");
        assert!(
            !body.contains("Some(30)"),
            "rust syntax leaked into a java snippet:\n{body}"
        );
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &E2eConfig::default(), &config, &[]);

        assert!(body.contains("catch (SampleRsException error)"), "{body}");
        assert!(!body.contains("AssertionError"), "{body}");
    }

    #[test]
    fn snippet_presents_selected_result_fields() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "list_items",
            "description": "List items",
            "input": null,
            "assertions": [],
            "docs": {
                "topic": "items",
                "presentation": {
                    "operations": [{"op": "iterate", "path": "items", "item": "item", "fields": ["name"]}]
                }
            }
        }))
        .expect("fixture");
        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call: CallConfig {
                    function: "list_items".into(),
                    result_var: "result".into(),
                    ..CallConfig::default()
                },
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(body.contains("for (var item : result.items())"), "{body}");
        assert!(body.contains("System.out.println(item.name());"), "{body}");
        assert!(!body.contains("System.out.println(result);"), "{body}");
    }

    #[test]
    fn snippet_declares_typed_json_arguments_and_preserves_qualified_service_name() {
        let fixture = Fixture {
            id: "configured_call".into(),
            description: "Configured call".into(),
            input: serde_json::json!({"source": "example", "options": {"mode": "fast"}}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![
                crate::e2e::config::ArgMapping {
                    name: "source".into(),
                    field: "source".into(),
                    arg_type: "string".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
                crate::e2e::config::ArgMapping {
                    name: "options".into(),
                    field: "options".into(),
                    arg_type: "json_object".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
            ],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                class: Some("dev.example.SampleService".into()),
                options_type: Some("ProcessOptions".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(body.contains("var optionsJson = \"{\\\"mode\\\":\\\"fast\\\"}\";"));
        assert!(body.contains("var options = JsonUtil.fromJson(optionsJson, ProcessOptions.class);"));
        assert!(body.contains("dev.example.SampleService.process(\"example\", options)"));
        assert!(!body.contains("DevExampleSampleService"));

        // ~keep Regression: `snippet_json_object_setup.jinja` renders a two-statement
        // block (the JSON-literal line, then the `JsonUtil.fromJson(...)` line) as one
        // string; `java/snippet_body.jinja` only prepends the method body's indent to
        // the first physical line of each `setup_lines` entry, so an un-split block put
        // the second statement at column 0 in every generated Java snippet with a
        // `json_object` arg.
        let json_line = line_containing(&body, "var optionsJson =");
        let deserialize_line = line_containing(&body, "JsonUtil.fromJson(optionsJson");
        assert_eq!(
            leading_spaces(json_line),
            leading_spaces(deserialize_line),
            "json literal and its deserialize call must share the method body's indent:\n{body}"
        );
        assert_eq!(
            leading_spaces(deserialize_line),
            8,
            "expected the `public static void main` body's 8-space indent:\n{body}"
        );
    }

    /// The positive counterpart to the test above: when the configured class DOES live in the
    /// package the snippet wildcard-imports, spelling the package again renders
    /// `io.example.pkg.Facade.convert(...)` under an `import io.example.pkg.*;` that made `Facade`
    /// alone sufficient. Only the exact configured package is stripped, and only when what remains
    /// is a bare class name — a nested or foreign class stays qualified, since no import covers it.
    /// ~keep
    #[test]
    fn a_class_inside_the_imported_package_is_called_by_its_simple_name() {
        let fixture = Fixture {
            id: "configured_call".into(),
            description: "Configured call".into(),
            input: serde_json::json!({"source": "example"}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "source".into(),
                field: "source".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        let package = ResolvedCrateConfig::default().java_package();
        call.overrides.insert(
            "java".into(),
            CallOverride {
                class: Some(format!("{package}.SampleService")),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(body.contains(&format!("import {package}.*;")), "{body}");
        assert!(body.contains("SampleService.process(\"example\")"), "{body}");
        assert!(
            !body.contains(&format!("{package}.SampleService.process")),
            "the wildcard import already covers the class: {body}"
        );
    }

    #[test]
    fn snippet_reads_nested_typed_dto_files() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "document_input",
            "description": "Read a document",
            "input": {"request": {"content": "ignored"}},
            "assertions": [],
            "docs": {
                "topic": "documents",
                "presentation": {"files": [{"field": "/request/content", "path": "document.pdf"}]}
            }
        }))
        .expect("fixture");
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "request".into(),
                field: "request".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                options_type: Some("DocumentRequest".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture.docs_call_fixture(),
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(
            body.contains("Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))"),
            "{body}"
        );
        assert!(body.contains("Base64.getEncoder().encodeToString"), "{body}");
        assert!(body.contains("DocumentRequest.class"), "{body}");

        // ~keep Regression: the base64 file-read is a three-line block (opening call,
        // an indented continuation line, a closing `);`). Every physical line must
        // land at the method body's indent, with the continuation line one level
        // deeper — the same defect class as the two-statement json_object setup above.
        let open_line = line_containing(&body, "Base64.getEncoder().encodeToString(");
        let continuation_line = line_containing(&body, "Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))");
        assert_eq!(leading_spaces(open_line), 8, "{body}");
        assert_eq!(leading_spaces(continuation_line), 12, "{body}");
    }

    /// Regression for alef task #180: a fixture whose `json_object` field carries a value long
    /// enough to threaten the JVM's 65535-byte `CONSTANT_Utf8` cap must never render as a
    /// single Java string literal above that limit -- no amount of escaping can raise the cap,
    /// so the doc snippet generator has to stop emitting one literal once a value is long
    /// enough. Neutral synthetic payload (`project-agnostic-codegen`): not any real consumer's
    /// fixture, just something unambiguously larger than the JVM cap. ~keep
    #[test]
    fn a_doc_snippet_never_emits_a_single_java_literal_over_the_jvm_constant_cap() {
        let oversized_payload = "abcdefghij".repeat(10_000); // 100,000 bytes
        let fixture = Fixture {
            id: "large_payload".into(),
            description: "Process a large payload".into(),
            input: serde_json::json!({"content": oversized_payload}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "content".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "java".into(),
            CallOverride {
                options_type: Some("PayloadOptions".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
        );

        assert!(
            body.contains(&oversized_payload[..100]),
            "the snippet must still contain the fixture content, just not as one literal:\n{}",
            &body[..body.len().min(500)]
        );
        for segment in body.split('"') {
            assert!(
                segment.len() <= 65_535,
                "a generated Java doc snippet must never contain a single quoted-literal \
                 segment above the JVM's 65535-byte CONSTANT_Utf8 cap: got {} bytes",
                segment.len()
            );
        }
    }

    /// Regression: the snippet generator used to build the checked-exception class name by
    /// suffixing `simple_class_name` (the public facade, which has its `Rs` marker stripped)
    /// with `Exception` -- a re-derivation that drifts from
    /// `backends::java::naming::exception_class_name`, the function that actually names the
    /// class the Java backend declares (`<MainClass>Exception`, where `MainClass` keeps the `Rs`
    /// suffix). For any crate name that does not already end in `Rs` -- i.e. nearly every crate --
    /// the two spellings disagree and the snippet's `catch` clause names a class that does not
    /// exist. Deriving `expected` from the canonical function (instead of a hardcoded literal)
    /// means a future change to the naming policy keeps this test in sync automatically. ~keep
    #[test]
    fn exception_class_matches_the_backends_canonical_derivation() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let config = ResolvedCrateConfig {
            name: "sample-multi-word".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(&fixture, &E2eConfig::default(), &config, &[]);

        let expected = crate::backends::java::naming::exception_class_name(&config.name);
        assert!(
            body.contains(&format!("catch ({expected} error)")),
            "the snippet's catch clause must name the exception class the Java backend actually \
             declares (via `backends::java::naming::exception_class_name`), not a spelling \
             re-derived from the facade class name:\n{body}"
        );
    }
}
