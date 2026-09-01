use std::collections::HashMap;

use anyhow::{Result, bail};
use heck::{ToSnakeCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

/// Render a C# documentation snippet without any core IR to consult.
///
/// Kept as the five-argument entry point every existing caller and test already uses: with no
/// `functions` the seam resolves to `TargetParams::IrAbsent`, which is exactly the state this path
/// was always in, so its output is unchanged by the seam. ~keep
pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, &[])
}

pub(super) fn render_snippet_body_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("csharp", fixture, call, type_defs)
        .with_functions(functions);
    let target_params = recipe.target_params("csharp");
    let overrides = recipe.override_config;
    let class_name = crate::codegen::naming::csharp_wrapper_class_name(&config.name, "");
    let mut function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_upper_camel_case();
    let is_async = overrides.and_then(|value| value.r#async).unwrap_or(call.r#async);
    // Mirrors `csharp.rs`'s own dispatch (`resolve_is_streaming(fixture, call_config.
    // streaming_enabled())`) — the same shared seam `streaming_assertions::resolve_is_streaming`
    // every backend is documented to use. A streaming call's C# binding returns
    // `IAsyncEnumerable<T>` synchronously (see `csharp/streaming.rs`'s `await foreach`
    // emission for the full e2e test suite); it is the *iteration*, not the call, that is
    // async. Without this, the docs snippet path (which never consulted streaming
    // classification at all) fell through to the plain `var result = await
    // client.Method(...)` shape every other async call uses — `IAsyncEnumerable<T>` has no
    // `GetAwaiter`, so `await` on the bare call is CS1061. ~keep
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    if is_async && !function_name.ends_with("Async") {
        function_name.push_str("Async");
    }
    let options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get("csharp")
            .and_then(|value| value.options_type.as_deref())
    });
    let options_via = overrides
        .and_then(|value| value.options_via.as_deref())
        .filter(|value| *value != "from_json");
    // Resolve the adapter's declared request type exactly as `csharp/streaming.rs` does for the
    // generated e2e test, so a streaming/adapter-backed snippet shows the real request-DTO call
    // shape instead of a bare `string`/`List<string>` -- CS1503 against the binding. ~keep
    let adapter_lookup_name = call.core_lookup_name("csharp");
    let adapter_request_type: Option<String> = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|a| a.name == name))
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());
    let mut visitor_declarations = Vec::new();
    let mut teardown_lines = Vec::new();
    let (mut setup_lines, mut args) = super::setup::build_args_and_setup(
        &fixture.input,
        recipe.args,
        &class_name,
        options_type,
        options_via,
        &HashMap::new(),
        &HashMap::new(),
        fixture,
        adapter_request_type.as_deref(),
        config,
        type_defs,
        enums,
        target_params,
        &mut visitor_declarations,
        &mut teardown_lines,
    );
    if let Some(visitor_spec) = &fixture.visitor {
        // A fixture that declares a visitor with no options type to bind it to is a
        // configuration defect, not a legitimate shape: there is nowhere to attach the
        // visitor. Fail closed here — the snippet pipeline records this as an
        // undocumented coverage gap naming the fixture — rather than fabricating a type
        // name, which publishes a documentation example that does not compile. Matches
        // `php::snippet` and `go::snippet`. Intentional omissions belong in the
        // fixture's `docs.coverage_exceptions`, where the reason is visible. ~keep
        let Some(options_type) =
            options_type.or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
        else {
            bail!(
                "C# documentation snippet `{}` needs an options type for its visitor",
                fixture.id
            );
        };
        let visitor_config = super::visitor::resolve_csharp_visitor_config(config, overrides, type_defs, visitor_spec);
        let visitor = super::visitor::build_csharp_visitor(
            &mut setup_lines,
            &mut visitor_declarations,
            &fixture.id,
            visitor_spec,
            &visitor_config,
        );
        setup_lines.push(format!("var options = new {options_type} {{ Visitor = {visitor} }};"));
        args = replace_or_append_options(&args, options_type);
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
                .get("csharp")
                .and_then(|value| value.client_factory.as_deref())
        })
        .map(ToUpperCamelCase::to_upper_camel_case);
    let client_args = render_client_factory_args(fixture, e2e_config, call);
    let namespace = overrides
        .and_then(|value| value.module.clone())
        .or_else(|| config.csharp.as_ref().and_then(|value| value.namespace.clone()))
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    // Classify on the resolved name, snake-cased: the registry heuristic below is written in
    // Rust spelling, but a call whose base `function` is empty carries its only name in
    // `overrides.csharp.function`, spelled the C# way (`ClearValidators`). Reading the raw
    // base there yields `""`, which matches no prefix and misclassifies every registry call
    // as value-returning. ~keep
    let registry_name = call
        .effective_function("csharp")
        .map(|function| function.to_snake_case())
        .unwrap_or_default();
    let returns_void = call.returns_void
        || matches!(registry_name.as_str(), "initialize" | "shutdown")
        || ["register_", "unregister_", "clear_"]
            .iter()
            .any(|prefix| registry_name.starts_with(prefix));
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let needs_json = setup_lines.iter().any(|line| line.contains("JsonSerializer")) || args.contains("JsonSerializer");
    let needs_system = expects_error
        || !returns_void
        || client_factory.is_some()
        || setup_lines.iter().any(|line| line.contains("Environment."));
    let needs_collections = setup_lines
        .iter()
        .any(|line| line.contains("List<") || line.contains("Dictionary<"));
    let result_var = call.effective_result_var();
    // Supplying our own resolver rather than letting `presentation::resolve` build one, exactly
    // as php/snippet.rs does: `build_resolver` furnishes no per-language representation facts,
    // so a path that steps into a tagged-union variant had no way to render as anything but a
    // plain field chain -- `.Format!.Html!` where `Html` is the variant TYPE (CS0572). Mirrors
    // `build_resolver`'s own `new` + `with_ir_fields`, because `resolve_with` applies the
    // declared-result anchoring but not that, and adds the narrowing map on top. ~keep
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) =
        crate::e2e::field_access::FieldResolver::ir_field_sets(type_defs);
    let field_resolver = crate::e2e::field_access::FieldResolver::new(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_variant_accessors(super::variant_accessors::build_variant_accessor_map(enums));
    let presentation = disambiguate_presentation_items(
        dereference_optional_iterate_collections(crate::e2e::codegen::presentation::resolve_with(
            fixture,
            e2e_config,
            "csharp",
            &field_resolver,
            type_defs,
            enums,
            functions,
        )),
        result_var,
        client_factory.is_some(),
    );
    Ok(crate::e2e::template_env::render(
        "csharp/snippet_body.jinja",
        minijinja::context! {
            namespace => namespace,
            setup_lines => setup_lines,
            client_factory => client_factory,
            class_name => class_name,
            client_args => client_args,
            function_name => function_name,
            args => args,
            result_var => result_var,
            returns_void => returns_void,
            is_async => is_async,
            is_streaming => is_streaming,
            needs_json => needs_json,
            needs_system => needs_system,
            needs_collections => needs_collections,
            fixture_id => fixture.id,
            api_key_var => api_key_var,
            expects_error => expects_error,
            // `build_csharp_visitor` indents its class by four spaces so the e2e test file can nest
            // it inside the test class. A snippet is top-level statements followed by file-scope
            // declarations, where that indent is just wrong — and it was load-bearing wrong: the
            // batch validator's statement/declaration split keyed on column, so an indented class
            // stayed inside the wrapper method and failed to compile. ~keep
            visitor_declarations => visitor_declarations
                .iter()
                .map(|declaration| dedent_file_scope_declaration(declaration))
                .collect::<Vec<_>>(),
            presentation => presentation,
        },
    ))
}

/// Argument list appended to a `client_factory` call when the project configures no
/// `[e2e.call.overrides.csharp] client_factory_trailing_args`.
///
/// These were hardcoded into `csharp/snippet_body.jinja` before the override was wired
/// up and remain the default, so a project that has not adopted the key keeps the
/// argument list it renders today.
const CSHARP_CLIENT_FACTORY_FALLBACK_ARGS: [&str; 3] = ["null", "null", "null"];

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
        Some(url) => format!("\"{}\"", crate::e2e::escape::escape_csharp(url)),
        None => "null".to_string(),
    };
    let trailing = crate::e2e::codegen::client_factory::trailing_args(
        docs_client,
        e2e_config,
        call,
        "csharp",
        &CSHARP_CLIENT_FACTORY_FALLBACK_ARGS,
    );
    let mut args = vec!["apiKey".to_string(), base_url];
    args.extend(trailing);
    args.join(", ")
}

fn replace_or_append_options(args: &str, options_type: &str) -> String {
    if let Some(prefix) = args.strip_suffix(", null") {
        return format!("{prefix}, options");
    }
    let default_options = format!("new {options_type}()");
    if args == default_options {
        return "options".to_string();
    }
    if let Some(prefix) = args.strip_suffix(&format!(", {default_options}")) {
        return format!("{prefix}, options");
    }
    if args.is_empty() {
        "options".to_string()
    } else {
        format!("{args}, options")
    }
}

/// Strip the uniform four-space indent `build_csharp_visitor` adds for nesting inside an e2e test
/// class, so the same declaration reads correctly at a snippet's file scope. Lines that do not
/// carry the indent (blank ones) are passed through unchanged.
fn dedent_file_scope_declaration(declaration: &str) -> String {
    declaration
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Append a null-forgiving `!` to an `iterate` operation's own collection `expression` when the
/// iterated value is itself optional -- e.g. `foreach (var keyword in result.Keywords!)`.
///
/// This is deliberately narrower than the general-purpose accessor every language and every
/// context shares (`FieldResolver::accessor` / `render_csharp_with_optionals`): reading an
/// optional SCALAR leaf bare (an assertion, a `Console.WriteLine`) is not a dereference and needs
/// no `!` -- `render_csharp_with_optionals` already gets that right by only marking non-leaf
/// segments, and every other backend's accessor contract (e.g. Rust's `.as_ref().unwrap()` on the
/// optional PARENT, never the leaf) agrees. `foreach` is different: it calls `GetEnumerator()` on
/// whatever `expression` evaluates to, so an optional COLLECTION consumed directly as a `foreach`
/// source needs the mark regardless of leaf/non-leaf position. `operation.optional` (`*optional ||
/// resolver.is_optional(path)`, the same signal the TypeScript backend already trusts for its own
/// `?? []` guard) answers exactly "is the thing this `expression` names itself absent-able" -- the
/// one question relevant here, asked only where a collection is about to be consumed, not baked
/// into the shared accessor every other reader of a field path also goes through. ~keep
fn dereference_optional_iterate_collections(
    mut operations: Vec<crate::e2e::codegen::presentation::PresentationOperation>,
) -> Vec<crate::e2e::codegen::presentation::PresentationOperation> {
    for operation in &mut operations {
        if operation.kind == "iterate" && operation.optional && !operation.expression.ends_with('!') {
            operation.expression.push('!');
        }
    }
    operations
}

/// Rename a fixture-authored `docs.presentation` `iterate` operation's `item` when it reuses a
/// name already bound in the snippet's own scope -- the call's own `result_var` binding, and
/// (when the fixture constructs one) the `client`/`apiKey` bindings just above it.
///
/// A batch/list call's presentation naturally spells its loop variable as the singular of the
/// collection it walks (`results` -> `result`), which collides with the outer `var result =
/// ...Call(...)` the template already emitted -- `foreach (var result in result.Items)`. C#'s
/// block scoping rejects that with CS0136 ("A local ... named 'result' cannot be declared in
/// this scope because that name is used in an enclosing scope to denote something else"); this
/// presentation layer is shared by every backend, and Python/Go/etc. accept the shadowing
/// without complaint, which is why no other backend needed this guard. Renaming here -- after
/// resolution, before the template ever sees it -- means the template stays a straight
/// `foreach (var {{ operation.item }} in ...)` with no scope awareness of its own. ~keep
fn disambiguate_presentation_items(
    mut operations: Vec<crate::e2e::codegen::presentation::PresentationOperation>,
    result_var: &str,
    constructs_client: bool,
) -> Vec<crate::e2e::codegen::presentation::PresentationOperation> {
    let reserved: std::collections::HashSet<&str> = if constructs_client {
        [result_var, "client", "apiKey"].into_iter().collect()
    } else {
        [result_var].into_iter().collect()
    };
    for operation in &mut operations {
        if operation.kind != "iterate" || !reserved.contains(operation.item.as_str()) {
            continue;
        }
        let candidate = format!("{}Item", operation.item);
        let old_prefix = format!("{}.", operation.item);
        for field in &mut operation.fields {
            if let Some(rest) = field.strip_prefix(old_prefix.as_str()) {
                *field = format!("{candidate}.{rest}");
            }
        }
        operation.item = candidate;
    }
    operations
}

#[cfg(test)]
mod tests {
    use super::dedent_file_scope_declaration;

    /// `build_csharp_visitor` indents its class for nesting inside an e2e test class. A snippet puts
    /// it at file scope after top-level statements, and the stray indent was not merely cosmetic:
    /// the batch validator's statement/declaration split keyed on column, so the indented class
    /// stayed inside the wrapper method and 54 of one consumer's snippets failed to compile. ~keep
    #[test]
    fn a_file_scope_declaration_loses_the_nesting_indent() {
        let nested = "    sealed class ExampleVisitor : IHtmlVisitor\n    {\n        public int Value => 1;\n    }";

        assert_eq!(
            dedent_file_scope_declaration(nested),
            "sealed class ExampleVisitor : IHtmlVisitor\n{\n    public int Value => 1;\n}"
        );
    }

    #[test]
    fn a_blank_line_survives_dedenting_unchanged() {
        assert_eq!(
            dedent_file_scope_declaration("    class A\n\n    {\n    }"),
            "class A\n\n{\n}"
        );
    }

    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    #[test]
    fn visitor_options_replace_the_placeholder_argument() {
        assert_eq!(
            replace_or_append_options("html, null", "ConversionOptions"),
            "html, options"
        );
        assert_eq!(
            replace_or_append_options("html, new ConversionOptions()", "ConversionOptions"),
            "html, options"
        );
    }

    #[test]
    fn snippet_keeps_async_native_call_without_xunit_harness() {
        let fixture = Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            r#async: true,
            ..CallConfig::default()
        };
        call.overrides.insert("csharp".into(), CallOverride::default());
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
            ..ResolvedCrateConfig::default()
        };
        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("await SampleCoreConverter.LoadDocumentAsync()"));
        assert!(!body.contains("using System.Collections.Generic;"));
        assert!(body.contains("using System;"));
        assert!(body.contains("Console.WriteLine(document);"));
        assert!(!body.contains("[Fact]"));
        assert!(!body.contains("Assert."));
    }

    /// Render the C# snippet for a `clear_*` registry call spelled `spelling`, placed either at
    /// the call's base `function` or only in its `overrides.csharp.function`.
    fn registry_snippet(base: &str, csharp_override: Option<&str>) -> String {
        let fixture = Fixture {
            id: "clear_validators".into(),
            description: "Clear all validators".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: base.into(),
            result_var: "result".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "csharp".into(),
            CallOverride {
                function: csharp_override.map(str::to_string),
                ..CallOverride::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
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
            &[],
        )
        .expect("snippet renders")
    }

    #[test]
    fn a_registry_call_named_only_by_its_csharp_override_still_reads_as_void_returning() {
        // `function = ""` plus one override per language is the shape a trait-bridge registry
        // call takes when the bindings disagree on spelling. Classifying `returns_void` from the
        // raw base saw `""`, matched no `clear_` prefix, and bound a result from a void method.
        let body = registry_snippet("", Some("ClearValidators"));

        assert!(body.contains("SampleCoreConverter.ClearValidators();"), "{body}");
        assert!(
            !body.contains("var result ="),
            "a void C# method must not have its return value bound:\n{body}"
        );
        assert!(!body.contains("Console.WriteLine(result);"), "{body}");
    }

    #[test]
    fn a_registry_call_named_by_its_base_is_classified_exactly_as_before() {
        let body = registry_snippet("clear_validators", None);

        assert!(body.contains("SampleCoreConverter.ClearValidators();"), "{body}");
        assert!(!body.contains("var result ="), "{body}");
    }

    #[test]
    fn a_value_returning_call_is_not_swept_up_by_the_registry_prefixes() {
        let body = registry_snippet("", Some("LoadDocument"));

        assert!(
            body.contains("var result = SampleCoreConverter.LoadDocument();"),
            "resolving the override must not turn every call into a void one:\n{body}"
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
        let e2e = E2eConfig {
            call: CallConfig {
                function: "process".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            result_fields: ["summary".to_string(), "items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

        assert!(body.contains("var result = SampleCoreConverter.Process();"), "{body}");
        assert!(body.contains("Console.WriteLine(result.Summary);"), "{body}");
        assert!(body.contains("foreach (var item in result.Items)"), "{body}");
        assert!(body.contains("Console.WriteLine(item.Label);"), "{body}");
        assert!(
            !body.contains("Console.WriteLine(result);"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
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
            "csharp".into(),
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
            body.contains("Environment.GetEnvironmentVariable(\"API_KEY\")"),
            "credential is not read from the environment:\n{body}"
        );
    }

    fn client_snippet(docs: Option<serde_json::Value>) -> String {
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
            "csharp".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
        )
        .expect("snippet renders")
    }

    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let body = client_snippet(Some(serde_json::json!({
            "topic": "configuration",
            "client": {"base_url": "https://llm.internal.example.com/v1"}
        })));

        assert!(
            body.contains("CreateClient(apiKey, \"https://llm.internal.example.com/v1\", null, null, null)"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    #[test]
    fn client_factory_snippet_without_a_docs_client_keeps_the_bare_call() {
        let body = client_snippet(None);

        assert!(
            body.contains("CreateClient(apiKey, null, null, null, null)"),
            "a fixture with no docs client must render the unconfigured argument list:\n{body}"
        );
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
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
        .expect("snippet renders");

        assert!(body.contains("catch (Exception error)"), "{body}");
        assert!(!body.contains("InvalidOperationException"), "{body}");
    }

    #[test]
    fn snippet_constructs_known_dto_without_json_round_trip() {
        let fixture = Fixture {
            id: "typed_input".into(),
            description: "Typed input".into(),
            input: serde_json::json!({"payload": {"label": "sample"}}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "payload".into(),
                field: "input.payload".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("SampleInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "csharp".into(),
            CallOverride {
                options_via: Some("from_json".into()),
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
            &[TypeDef {
                name: "SampleInput".into(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "label".into(),
                    ty: crate::core::ir::TypeRef::String,
                    ..Default::default()
                }],
                ..TypeDef::default()
            }],
            &[],
        )
        .expect("snippet renders");

        assert!(body.contains("new SampleInput { Label = \"sample\" }"), "{body}");
        assert!(!body.contains("FromJson"), "{body}");
        assert!(!body.contains("JsonSerializer"), "{body}");
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

    fn visitor_call() -> CallConfig {
        CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "html".into(),
                field: "input.html".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        }
    }

    fn bridge_config(options_type: Option<&str>) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "sample_core".into(),
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

    /// Regression: a visitor fixture with no resolvable options type used to fall back to
    /// the literal type name `Options`, publishing `new Options { Visitor = .. }` — a
    /// documentation example naming a type that does not exist. It must fail closed
    /// instead, matching PHP and Go. ~keep
    #[test]
    fn visitor_without_a_trait_bridge_options_type_fails_instead_of_fabricating_one() {
        let error = render_snippet_body(
            &visitor_fixture(),
            &E2eConfig {
                call: visitor_call(),
                ..E2eConfig::default()
            },
            &bridge_config(None),
            &[],
            &[],
        )
        .expect_err("a visitor with no options type must not render");

        assert_eq!(
            format!("{error}"),
            "C# documentation snippet `visitor_link_rewrite` needs an options type for its visitor"
        );
    }

    /// Positive control for the above: with the bridge's `options_type` configured, the
    /// ordinary visitor path is unchanged and names the real type. ~keep
    #[test]
    fn visitor_with_a_trait_bridge_options_type_still_names_the_real_type() {
        let body = render_snippet_body(
            &visitor_fixture(),
            &E2eConfig {
                call: visitor_call(),
                ..E2eConfig::default()
            },
            &bridge_config(Some("ConversionOptions")),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            body.contains("var options = new ConversionOptions { Visitor = _visitor_visitor_link_rewrite };"),
            "{body}"
        );
        assert!(!body.contains("new Options"), "{body}");
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
            "csharp".into(),
            CallOverride {
                client_factory: Some("create_client".into()),
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
            &[],
        )
        .expect("snippet renders")
    }

    /// Every C# opaque handle alef generates is `IDisposable` over an owning `SafeHandle`, so a
    /// snippet holding a bare `var client` defers release to finalization. A `using` declaration
    /// binds it to the enclosing scope instead, which the compiler lowers to a `try`/`finally` —
    /// the reason this is a declaration and not a trailing `client.Dispose();`. ~keep
    #[test]
    fn client_factory_snippet_scopes_the_client_to_a_using_declaration() {
        let body = client_release_snippet(false);

        assert!(
            body.contains("using var client = "),
            "a constructed client must be scoped to a using declaration:\n{body}"
        );
        assert!(
            !body.contains("\nvar client = ") && !body.contains("  var client = "),
            "the unscoped declaration must be replaced, not duplicated:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_scopes_the_client_to_a_using_declaration`.
    /// The `expects_error` arm wraps the body in `try`/`catch`, and the whole point of a `using`
    /// declaration over a trailing `Dispose()` is that the release still runs when the call
    /// throws — so pin that the declaration lands inside the `try` the failing call sits in. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_release_snippet(true);

        let try_block = body.find("try\n{").expect("expects-error snippet opens a try block");
        let declaration = body.find("using var client = ").expect("using declaration");
        let catch_block = body.find("catch (Exception error)").expect("catch block");
        assert!(
            try_block < declaration && declaration < catch_block,
            "the using declaration must sit inside the try the failing call runs in:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so its snippet must be untouched.
    /// `using System;` and the namespace import are using *directives*, not declarations, so this
    /// asserts on `using var` specifically — an unconditional change would fail here. ~keep
    #[test]
    fn snippet_without_a_client_factory_emits_no_using_declaration() {
        let body = render_snippet_body(
            &Fixture {
                id: "quick_start".into(),
                description: "Quick start".into(),
                input: serde_json::Value::Null,
                ..Fixture::default()
            },
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            !body.contains("using var"),
            "a snippet that constructs no client must emit no using declaration:\n{body}"
        );
        assert!(
            !body.contains("var client"),
            "a snippet that constructs no client must not declare one:\n{body}"
        );
    }

    /// Render the C# snippet for an async call, with streaming forced on or off.
    fn streaming_snippet(streaming: bool) -> String {
        let fixture = Fixture {
            id: "stream_document".into(),
            description: "Stream a document".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "stream_document".into(),
            result_var: "chunks".into(),
            r#async: true,
            streaming: Some(crate::core::config::e2e::StreamingConfig::Enabled(streaming)),
            ..CallConfig::default()
        };
        call.overrides.insert("csharp".into(), CallOverride::default());
        let config = ResolvedCrateConfig {
            name: "sample_core".into(),
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
            &[],
        )
        .expect("snippet renders")
    }

    /// A streaming C# binding returns `IAsyncEnumerable<T>` *synchronously* -- it is the
    /// iteration that is async, not the call. The docs-snippet path never consulted streaming
    /// classification at all, so it fell through to the plain `var x = await client.Method(...)`
    /// shape every other async call uses, and `IAsyncEnumerable<T>` has no `GetAwaiter`: CS1061.
    /// Classification comes from the shared `streaming_assertions::resolve_is_streaming` seam the
    /// e2e test path already uses, so snippet and test cannot disagree about what streams. ~keep
    #[test]
    fn a_streaming_call_is_iterated_with_await_foreach_not_awaited_directly() {
        let body = streaming_snippet(true);

        assert!(body.contains("await foreach"), "streaming must iterate:\n{body}");
        assert!(
            !body.contains("await SampleCoreConverter.StreamDocumentAsync()"),
            "awaiting the bare call is CS1061 on IAsyncEnumerable<T>:\n{body}"
        );
        assert!(
            !body.contains("var chunks ="),
            "a streaming call must not bind its result as a plain value:\n{body}"
        );
    }

    /// Negative control. Without it the assertions above would pass just as well if the template
    /// had been changed to emit `await foreach` unconditionally, which would break every ordinary
    /// async call in exactly the opposite direction. ~keep
    #[test]
    fn a_non_streaming_async_call_is_still_awaited_directly() {
        let body = streaming_snippet(false);

        assert!(
            !body.contains("await foreach"),
            "a plain async call must not iterate:\n{body}"
        );
        assert!(
            body.contains("await SampleCoreConverter.StreamDocumentAsync()"),
            "a plain async call is awaited directly:\n{body}"
        );
    }
}

/// Regression coverage for `disambiguate_presentation_items`, split into its own file per the
/// `file-modularization` rule: `snippet.rs` was already close to the 1,000-line cap.
#[cfg(test)]
#[path = "snippet/collision_tests.rs"]
mod collision_tests;

/// End-to-end coverage, through the full snippet pipeline, for the CS8602 fix in
/// `dereference_optional_iterate_collections`.
#[cfg(test)]
#[path = "snippet/nullable_presentation_tests.rs"]
mod nullable_presentation_tests;

/// Unit-level coverage for `dereference_optional_iterate_collections`, split into its own file
/// per the `file-modularization` rule.
#[cfg(test)]
#[path = "snippet/optional_iterate_tests.rs"]
mod optional_iterate_tests;

/// Regression coverage for the streaming-adapter request-DTO seam, split into its own file
/// per the `file-modularization` rule: `snippet.rs` was already close to the 1,000-line cap.
#[cfg(test)]
#[path = "snippet/streaming_request_tests.rs"]
mod streaming_request_tests;
