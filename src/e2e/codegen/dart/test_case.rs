//! Dart ordinary function-call e2e test rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::assertions::{render_assertion_dart, render_streaming_assertion_dart};
use super::values::{escape_dart, mime_from_extension, type_name_to_create_from_json_dart};

const COMPATIBLE_OPTIONS_TYPE_LANGS: &[&str] = &["csharp", "c", "go", "java", "php", "python", "r"];

mod args;

/// Build the `throwsA(...)` matcher expression for an `error`-asserting test.
///
/// ~keep flutter_rust_bridge 2.x decodes Rust errors as raw String values (see the
/// `throwsA(anything)` rationale above), so a declared expectation can't rely on a typed
/// exception hierarchy. Checking `toString()` OR `runtimeType.toString()` mirrors the
/// message-or-type disjunction other backends use (`declared_error_value`'s contract):
/// config-validation fixtures name text that only appears in the message, API-error
/// fixtures name a type prefix that only appears in the runtime type. With no declared
/// value this returns the original `throwsA(anything)` unchanged. Which of those two
/// conventions applies, and whether Dart can ever satisfy the second, is decided once by
/// `declared_error_variant::classify` — see its doc for why Dart lands on "never" today.
///
/// ~keep When the declared value names a real variant Dart cannot substantiate, this still
/// returns a matcher expression (a Dart matcher is spliced inline into a statement, with no
/// place of its own for a comment) — `throwsA(anything)`, same as the undeclared case, since
/// that is still an honest "the call must fail" check. The registered skip is written into
/// `out` as its own comment line immediately above the statement that consumes the matcher, so
/// the skip is visible in the generated file AND counted on the shared ledger, rather than being
/// silently downgraded to `throwsA(anything)` with no trace.
fn dart_error_matcher(
    out: &mut String,
    indent: &str,
    fixture: &Fixture,
    errors: &[crate::core::ir::ErrorDef],
) -> String {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    match classify("dart", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => "throwsA(anything)".to_string(),
        DeclaredErrorAssertion::Assert(declared) => {
            let escaped = escape_dart(declared);
            format!(
                "throwsA(predicate((e) => e.toString().contains('{escaped}') || e.runtimeType.toString().contains('{escaped}')))"
            )
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "{}", skip_line(indent, "//", variant, &fixture.id, "dart"));
            "throwsA(anything)".to_string()
        }
    }
}

/// True when `body` contains at least one line that is not blank and not a
/// `//`-prefixed comment — i.e. a real `expect(...)` statement. A body made
/// up only of "// skipped: ..." lines is not executable.
fn has_real_dart_assertion(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with("//")
    })
}

/// snake_case -> lowerCamelCase, matching Dart naming conventions. Shared by the call's
/// `function_name` and `client_factory` lowering, which previously duplicated this identical
/// closure inline. ~keep
fn snake_to_lower_camel_case(name: &str) -> String {
    name.split('_')
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Fixture-driven visitor handle. When `fixture.visitor` is set this builds a `visitor` via the
/// generated visitor factory (emitted by `alef-backend-dart`'s trait-bridge generator in the
/// `type_alias` mode) and threads it into the options blob via the
/// `create<OptionsType>FromJsonWithVisitor(json, visitor)` helper.
///
/// The visitor setup line is INSERTED at the front of `setup_lines` so `visitor` is defined
/// before any `options` line that references it. Fixtures without an `options` json_object in
/// input still need an options blob to carry the visitor through to the configured call -- an
/// empty options call with the configured options type is synthesised here when no `options`
/// arg was emitted by the arg-building loop. Extracted verbatim from `render_test_case`; no
/// emitted string, whitespace, or branch condition changed. ~keep
struct VisitorArgContext<'a> {
    config: &'a ResolvedCrateConfig,
    call_overrides: Option<&'a crate::e2e::config::CallOverride>,
    type_defs: &'a [crate::core::ir::TypeDef],
    options_type: Option<&'a str>,
}

fn apply_fixture_visitor(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    fixture: &Fixture,
    ctx: &VisitorArgContext<'_>,
) {
    let options_type = ctx.options_type;
    if let Some(visitor_spec) = &fixture.visitor {
        let mut visitor_setup: Vec<String> = Vec::new();
        let visitor_config = crate::e2e::codegen::dart_visitors::resolve_dart_visitor_config(
            ctx.config,
            ctx.call_overrides,
            ctx.type_defs,
            visitor_spec,
        );
        let _ =
            crate::e2e::codegen::dart_visitors::build_dart_visitor(&mut visitor_setup, visitor_spec, &visitor_config);
        // Prepend the visitor block so `visitor` is in scope by the time the
        // options call (which may reference it) runs.
        for line in visitor_setup.into_iter().rev() {
            setup_lines.insert(0, line);
        }

        // If no `options` arg was emitted by the loop above (the fixture has no
        // input.options block), build an empty options-with-visitor and add it as
        // an `options:` named arg so the visitor reaches the convert call.
        let already_has_options = args.iter().any(|a| a.starts_with("options:") || a == "options");
        if !already_has_options {
            if let Some(opts_type) = options_type {
                let dart_fn = type_name_to_create_from_json_dart(opts_type);
                setup_lines.push(format!(
                    "final options = await {dart_fn}WithVisitor(json: '{{}}', visitor: visitor);"
                ));
                args.push("options: options".to_string());
            }
        } else if let Some(opts_type) = options_type {
            // The args loop already emitted a non-WithVisitor options call (e.g.
            // for `options: {}` or `options: {some: value}`). Without the visitor
            // attached the convert call ignores `visitor` — rewrite the
            // emitted call to its `WithVisitor` sibling so the visitor reaches
            // the converter.
            let dart_fn = type_name_to_create_from_json_dart(opts_type);
            let needle = format!("await {dart_fn}(json:");
            let replacement = format!("await {dart_fn}WithVisitor(visitor: visitor, json:");
            for line in setup_lines.iter_mut() {
                if line.contains(&needle) {
                    *line = line.replace(&needle, &replacement);
                }
            }
        }
    }
}

/// Resolve the receiver expression a call is made on, and any setup statement that must
/// precede it. When `client_factory` is set, tests create a client instance and call methods
/// on it rather than using static bridge-class calls (mirroring the go/python/zig pattern for
/// stateful clients). The mock URL derivation follows the same has_host_root_route /
/// plain-fixture split used by the mock_url arg handler in `args.rs`. Extracted verbatim from
/// `render_test_case`; no emitted string, whitespace, or branch condition changed. ~keep
fn resolve_dart_receiver(
    client_factory_camel: Option<&str>,
    is_snippet: bool,
    fixture: &Fixture,
    receiver_class: &str,
    call_config: &crate::e2e::config::CallConfig,
    fixture_id: &str,
) -> (String, Option<String>) {
    if let Some(factory) = client_factory_camel {
        if is_snippet {
            // Doc snippets are standalone: there is no mock server and no `_fixtureUrl`
            // helper (only the full e2e test-file emitter defines one), so the harness
            // is stripped entirely — matching the PHP/Ruby/Go/TypeScript emitters, which
            // all omit baseUrl from their snippet client construction.
            let api_key_var = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
            // ~keep `docs.client.base_url` is the mechanism a configuration/custom-base-url
            // topic uses to show its client constructed against the endpoint the prose is
            // about, mirroring the Java/Elixir/Rust/Python `docs_client` handling. `baseUrl:`
            // is already this call's named slot for the full e2e suite's mock URL (see the
            // else branch below), so it is an unambiguous, pre-existing slot here too.
            let base_url_arg = crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client())
                .map(|base_url| format!(", baseUrl: '{}'", escape_dart(base_url)))
                .unwrap_or_default();
            let create_line = format!(
                "final apiKey = Platform.environment['{api_key_var}'];\n  if (apiKey == null || apiKey.isEmpty) {{ throw StateError('{api_key_var} must be set'); }}\n  final client = await {receiver_class}.{factory}(apiKey{base_url_arg});"
            );
            ("client".to_string(), Some(create_line))
        } else {
            let has_mock_url = fixture
                .resolved_args(call_config)
                .iter()
                .any(|a| a.arg_type == "mock_url");
            let mock_url_setup = if !has_mock_url {
                // No explicit mock_url arg — derive the URL inline.
                Some(format!(r#"final mockUrl = _fixtureUrl("{fixture_id}");"#))
            } else {
                None
            };
            let url_expr = if has_mock_url {
                // A mock_url arg was emitted into setup_lines already — reuse the variable name
                // from the first mock_url arg definition so we don't duplicate the URL.
                call_config
                    .args
                    .iter()
                    .find(|a| a.arg_type == "mock_url")
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "mockUrl".to_string())
            } else {
                "mockUrl".to_string()
            };
            let create_line =
                format!("final client = await {receiver_class}.{factory}('test-key', baseUrl: {url_expr});");
            let full_setup = if let Some(url_line) = mock_url_setup {
                Some(format!("{url_line}\n    {create_line}"))
            } else {
                Some(create_line)
            };
            ("client".to_string(), full_setup)
        }
    } else {
        (receiver_class.to_string(), None)
    }
}

pub(super) struct DartTestCaseContext<'a> {
    pub(super) e2e_config: &'a E2eConfig,
    pub(super) lang: &'a str,
    pub(super) bridge_class: &'a str,
    pub(super) dart_first_class_map: &'a crate::e2e::field_access::DartFirstClassMap,
    pub(super) adapters: &'a [crate::core::config::extras::AdapterConfig],
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    /// `ApiSurface::functions`, supplied only by callers that have opted into type-aware
    /// argument lowering. An empty slice keeps the recipe's `IrAbsent` behaviour. ~keep
    pub(super) functions: &'a [crate::core::ir::FunctionDef],
    pub(super) errors: &'a [crate::core::ir::ErrorDef],
    pub(super) native_typed_dtos: bool,
    /// ~keep Standalone doc snippets have no mock server behind them, so `_fixtureUrl`
    /// — a helper only the full e2e test-file emitter defines — must never be
    /// referenced; the client_factory branch below strips the mock URL/baseUrl
    /// entirely instead of reusing the full-suite wiring.
    pub(super) is_snippet: bool,
}

pub(super) fn render_test_case(out: &mut String, fixture: &Fixture, context: DartTestCaseContext<'_>) {
    let DartTestCaseContext {
        e2e_config,
        lang,
        bridge_class,
        dart_first_class_map,
        adapters,
        config,
        type_defs,
        enums,
        functions,
        errors,
        native_typed_dtos,
        is_snippet,
    } = context;
    // HTTP fixtures: hit the mock server.
    if let Some(http) = &fixture.http {
        super::http::render_http_test_case(out, fixture, http);
        return;
    }

    // Non-HTTP fixtures: render a call-based test using the resolved call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let call_recipe =
        crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs)
            .with_functions(functions);
    let target_params = call_recipe.target_params(lang);
    let call_overrides = call_config.overrides.get(lang);

    // Build per-call field resolver using the effective field sets for this call. Extracted to
    // `call_field_resolver.rs` (this file is at the file-size ratchet's frozen ceiling).
    let call_field_resolver = super::call_field_resolver::build_call_field_resolver(
        e2e_config,
        call_config,
        fixture,
        lang,
        dart_first_class_map,
        type_defs,
        enums,
        functions,
    );
    let field_resolver = &call_field_resolver;
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // Convert snake_case function names to camelCase for Dart conventions.
    function_name = snake_to_lower_camel_case(&function_name);
    let result_var = call_config.effective_result_var();
    let description = escape_dart(&fixture.description);
    let fixture_id = &fixture.id;
    // `is_async` retained for future use (e.g. non-FRB backends); unused with FRB since
    // all wrappers return Future<T>.
    let _is_async = call_overrides.and_then(|o| o.r#async).unwrap_or(call_config.r#async);

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    // `result_is_simple = true` means the dart return is a scalar/bytes value
    // (e.g. `Uint8List` for speech/file_content), not a struct. Field-based
    // assertions like `audio.not_empty` collapse to whole-result checks so we
    // don't emit `result.audio` against a `Uint8List` receiver.
    let result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple) || call_config.result_is_simple;

    // Resolve options_type and options_via from per-fixture → per-call → default.
    // These drive how `json_object` args are constructed:
    //   options_via = "from_json" — call `createTypeNameFromJson(json: r'...')` bridge
    //                               helper and pass the result as a named parameter `req:`.
    //   All other values (or absent) — existing behaviour (batch arrays, config objects,
    //   generic JSON arrays, or nothing).
    let options_type: Option<&str> = call_recipe.compatible_options_type(COMPATIBLE_OPTIONS_TYPE_LANGS);
    let options_via: &str = call_recipe.options_via;

    // Build argument list from fixture.input and resolved args (fixture.args or call_config.args).
    // Use `resolve_field` (respects the `field` path like "input.data") rather than
    // looking up by `arg_def.name` directly — the name and the field key may differ.
    //
    // For `extract_file_sync` / `extract_file` fixtures that omit `mime_type`,
    // derive the MIME from the path extension so `extractBytesSync`/`extractBytes`
    // can be called (both require an explicit MIME type).
    let file_path_for_mime: Option<&str> = fixture
        .resolved_args(call_config)
        .iter()
        .find(|a| a.arg_type == "file_path")
        .and_then(|a| resolve_field(&fixture.input, &a.field).as_str());

    // Detect whether this call converts a file_path arg to bytes at test-run time.
    // Most dart fixtures take the bytes path historically (`extractFile` →
    // `extractBytes`), but source-code files need the path-based variant to reach
    // CodeExtractor's `extract_file` implementation (its `extract_bytes` path
    // requires a shebang line for language detection, so `code/hello.py` fed as
    // bytes errors out with "Cannot detect programming language from content").
    // We therefore keep the bytes remap for ordinary file types and skip it for
    // source-code extensions, letting the binding's own `extractFileSync` /
    // `extractFile` accept the path directly.
    let has_file_path_arg = fixture
        .resolved_args(call_config)
        .iter()
        .any(|a| a.arg_type == "file_path");
    let routes_to_source_code = file_path_for_mime
        .and_then(mime_from_extension)
        .map(|m| m == "text/x-source-code")
        .unwrap_or(false);
    // Apply the remap only when no per-fixture dart override has already specified the
    // function — if the fixture author set a dart-specific function name we trust it.
    let caller_supplied_override = call_overrides.and_then(|o| o.function.as_ref()).is_some();
    if has_file_path_arg && !caller_supplied_override && !routes_to_source_code {
        function_name = match function_name.as_str() {
            "extractFile" => "extractBytes".to_string(),
            "extractFileSync" => "extractBytesSync".to_string(),
            other => other.to_string(),
        };
    }

    // Resolve client_factory early so the per-arg builders below can pick the
    // calling convention. When `client_factory` is set the test calls methods on
    // an FRB-generated client instance, and FRB
    // emits every non-`config` parameter as a Dart named-required parameter. When
    // unset the call routes through a hand-written facade whose required args are
    // positional. See the `"string"` arg handler below.
    let client_factory_for_args: Option<&str> =
        call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|o| o.client_factory.as_deref())
        });

    // Dart e2e currently emits all args positionally; FRB-direct-call named-arg dispatch
    // is parked behind this flag until the codegen can distinguish facade vs bridge call
    // shapes precisely.
    let is_frb_bridge_call = false;

    // Resolve adapter request type for streaming methods.
    let adapter_lookup_name = call_config.core_lookup_name(lang);
    let adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| adapters.iter().find(|a| a.name == name));
    let adapter_request_type: Option<String> = adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // setup_lines holds per-test statements that must precede the main call:
    // engine construction (handle args) and URL building (mock_url args).
    let mut setup_lines: Vec<String> = Vec::new();
    let mut args = Vec::new();

    // Extracted to `test_case/args.rs` -- the per-argument lowering switch was this
    // function's dominant complexity driver; moving it out keeps `render_test_case`'s own
    // control flow (call setup, visitor wiring, receiver resolution, body emission) readable
    // on its own. Every branch, string, and emission condition is unchanged by the move. ~keep
    let arg_ctx = args::DartArgContext {
        fixture,
        is_snippet,
        bridge_class,
        config,
        type_defs,
        enums,
        call_recipe: &call_recipe,
        target_params,
        native_typed_dtos,
        options_type,
        options_via,
        is_frb_bridge_call,
        adapter_request_type: adapter_request_type.as_deref(),
        file_path_for_mime,
        routes_to_source_code,
        client_factory_for_args,
    };
    args::build_args_and_setup(&mut setup_lines, &mut args, &arg_ctx);

    // Fixture-driven visitor handle -- extracted to `apply_fixture_visitor` below (its own
    // self-contained responsibility: build the `visitor` var, then thread it into the
    // `options` blob emitted by the arg loop above, or synthesize one if none was emitted).
    let visitor_ctx = VisitorArgContext { config, call_overrides, type_defs, options_type };
    apply_fixture_visitor(&mut setup_lines, &mut args, fixture, &visitor_ctx);

    // Resolve client_factory: when set, tests create a client instance and call
    // methods on it rather than using static bridge-class calls. This mirrors the
    // go/python/zig pattern for stateful clients (e.g. demo-client).
    let client_factory: Option<&str> = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    // Convert factory name to camelCase (same rule as function_name above).
    let client_factory_camel: Option<String> = client_factory.map(snake_to_lower_camel_case);

    // All bridge methods return Future<T> because FRB v2 wraps every Rust
    // function as async in Dart — even "sync" Rust functions. Always emit an async
    // test body and await the call so the test framework waits for the future.
    let _ = writeln!(out, "  test('{description}', () async {{");

    let args_str = args.join(", ");
    let receiver_class = call_overrides
        .and_then(|o| o.class.as_ref())
        .cloned()
        .unwrap_or_else(|| bridge_class.to_string());

    // When client_factory is set, determine the mock URL and emit client instantiation.
    // Extracted to `resolve_dart_receiver` below; no emitted string, whitespace, or branch
    // condition changed by the move.
    let (receiver, extra_setup): (String, Option<String>) = resolve_dart_receiver(
        client_factory_camel.as_deref(),
        is_snippet,
        fixture,
        &receiver_class,
        call_config,
        fixture_id,
    );

    // Extracted to `emit_call_and_assertions` below; no emitted string, whitespace, or branch
    // condition changed by the move.
    let emission_ctx = CallEmissionContext {
        setup_lines: &setup_lines,
        extra_setup: &extra_setup,
        is_streaming,
        receiver: &receiver,
        function_name: &function_name,
        args_str: &args_str,
        fixture,
        errors,
        call_config,
        result_binding: result_var,
        field_resolver,
        result_is_simple,
        expects_error,
    };
    emit_call_and_assertions(out, &emission_ctx);

    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);
}

/// Emit the test body's call statement (wrapped for an expected error, or plain) and its
/// assertions. Extracted verbatim from `render_test_case`; no emitted string, whitespace, or
/// branch condition changed. ~keep
struct CallEmissionContext<'a> {
    setup_lines: &'a [String],
    extra_setup: &'a Option<String>,
    is_streaming: bool,
    receiver: &'a str,
    function_name: &'a str,
    args_str: &'a str,
    fixture: &'a Fixture,
    errors: &'a [crate::core::ir::ErrorDef],
    call_config: &'a crate::e2e::config::CallConfig,
    result_binding: &'a str,
    field_resolver: &'a crate::e2e::field_access::FieldResolver,
    result_is_simple: bool,
    expects_error: bool,
}

fn emit_call_and_assertions(out: &mut String, ctx: &CallEmissionContext<'_>) {
    let setup_lines = ctx.setup_lines;
    let extra_setup = ctx.extra_setup;
    let is_streaming = ctx.is_streaming;
    let receiver = ctx.receiver;
    let function_name = ctx.function_name;
    let args_str = ctx.args_str;
    let fixture = ctx.fixture;
    let errors = ctx.errors;
    let call_config = ctx.call_config;
    let result_var = ctx.result_binding;
    let field_resolver = ctx.field_resolver;
    let result_is_simple = ctx.result_is_simple;
    if ctx.expects_error && (!setup_lines.is_empty() || extra_setup.is_some()) {
        // Wrap setup + call in an async lambda so any exception at any step is caught.
        // flutter_rust_bridge 2.x decodes Rust errors as raw String values (not Exception
        // subtypes), so throwsException will not match. Use throwsA(anything) instead.
        let _ = writeln!(out, "    await expectLater(() async {{");
        for line in setup_lines {
            // Handle multi-line setup blocks (e.g., class definitions from emit_test_backend).
            // Each embedded newline in `line` needs proper indentation.
            for inner_line in line.lines() {
                let _ = writeln!(out, "      {inner_line}");
            }
        }
        if let Some(extra) = extra_setup {
            for line in extra.lines() {
                let _ = writeln!(out, "      {line}");
            }
        }
        if is_streaming {
            let _ = writeln!(out, "      return {receiver}.{function_name}({args_str}).toList();");
        } else {
            let _ = writeln!(out, "      return {receiver}.{function_name}({args_str});");
        }
        let matcher = dart_error_matcher(out, "      ", fixture, errors);
        let _ = writeln!(out, "    }}(), {matcher});");
    } else if ctx.expects_error {
        // No setup lines, direct call — same throwsA(anything) rationale as above.
        if let Some(extra) = extra_setup {
            for line in extra.lines() {
                let _ = writeln!(out, "    {line}");
            }
        }
        let matcher = dart_error_matcher(out, "    ", fixture, errors);
        if is_streaming {
            let _ = writeln!(
                out,
                "    await expectLater({receiver}.{function_name}({args_str}).toList(), {matcher});"
            );
        } else {
            let _ = writeln!(
                out,
                "    await expectLater({receiver}.{function_name}({args_str}), {matcher});"
            );
        }
    } else {
        for line in setup_lines {
            // Handle multi-line setup blocks (e.g., class definitions from emit_test_backend).
            // Each embedded newline in `line` needs proper indentation.
            for inner_line in line.lines() {
                let _ = writeln!(out, "    {inner_line}");
            }
        }
        if let Some(extra) = extra_setup {
            for line in extra.lines() {
                let _ = writeln!(out, "    {line}");
            }
        }
        // A `Future<void>`-returning call cannot be bound to a variable — `final result =
        // await voidCall();` is a Dart compile error ("This expression has a type of
        // 'void' so its value can't be used"), because the initializer's value is used
        // even though nothing ever reads `result` afterward. Trait-bridge registration
        // wrappers (registerValidator, registerOcrBackend, ...) are declared `returns_void`
        // in alef.toml precisely because the generated Dart bridge method returns
        // `Future<void>`, so honor that flag here instead of always binding a variable.
        // A `returns_void` call binds no `result` (see the compile-error rationale just
        // above), so a fixture whose only assertion is `not_error` has nothing to assert a
        // value against the way the fallback below does. `package:test` has no
        // `throwsNothing`/`completesNormally` boolean-style matcher, but `completes` IS a
        // real matcher: it fails the test if the `Future` rejects. Wrap the call in
        // `expectLater(..., completes)` instead of leaving it a bare `await` that relies
        // only on the implicit "an uncaught rejection fails the test" behavior every other
        // `returns_void` fixture still uses. ~keep
        let void_not_error = fixture
            .assertions
            .iter()
            .any(|assertion| assertion.assertion_type == "not_error");
        if call_config.returns_void && void_not_error {
            out.push_str(&crate::e2e::template_env::render(
                "dart/void_not_error_call.jinja",
                minijinja::context! { receiver => receiver, function_name => function_name, args => args_str },
            ));
        } else if call_config.returns_void {
            let _ = writeln!(out, "    await {receiver}.{function_name}({args_str});");
        } else if is_streaming {
            let _ = writeln!(
                out,
                "    final {result_var} = await {receiver}.{function_name}({args_str}).toList();"
            );
        } else {
            let _ = writeln!(
                out,
                "    final {result_var} = await {receiver}.{function_name}({args_str});"
            );
        }
        let assertions_start = out.len();
        for assertion in &fixture.assertions {
            if is_streaming {
                render_streaming_assertion_dart(out, assertion, result_var);
            } else {
                render_assertion_dart(out, assertion, result_var, result_is_simple, field_resolver);
            }
        }

        // A non-streaming fixture whose only assertion is `not_error` (which
        // intentionally renders nothing — a thrown error already fails the
        // `await`) or whose field assertions all resolved to "skipped"
        // comments leaves the body with no real `expect(...)` call, even
        // though `result` was already bound above. Mirror the streaming path's
        // existing `not_error` idiom (`expect(result, isNotNull)`) instead of
        // leaving the test vacuous. Skipped for `returns_void` calls, where
        // `result` was never bound and `expect(<void>, ...)` is a compile
        // error, and for fixtures that declare no assertions at all (an
        // intentional bare smoke test). ~keep
        if !is_streaming
            && !call_config.returns_void
            && !fixture.assertions.is_empty()
            && !has_real_dart_assertion(&out[assertions_start..])
        {
            let _ = writeln!(out, "    expect({result_var}, isNotNull);");
        }
        crate::e2e::codegen::fail_on_unavailable_field_markers(
            &out[assertions_start..],
            "dart",
            &fixture.id,
            &fixture.assertions,
        );
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[assertions_start..], "dart", &fixture.id);
    }
}

#[cfg(test)]
mod tests;
