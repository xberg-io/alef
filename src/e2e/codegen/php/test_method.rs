use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_php_single, php_pcre_literal};
use crate::e2e::fixture::Fixture;
use heck::ToLowerCamelCase;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use super::{args, assertions, stubs, types, visitor};
use crate::e2e::codegen::inert_example::{self, InertCause, InertExample};

fn uses_unsupported_php_callback(args: &[crate::e2e::config::ArgMapping], config: &ResolvedCrateConfig) -> bool {
    args.iter().any(|arg| {
        arg.arg_type == "test_backend"
            && arg.trait_name.as_deref().is_some_and(|trait_name| {
                config
                    .trait_bridges_for(crate::core::config::Language::Php)
                    .any(|bridge| bridge.trait_name == trait_name && bridge.php_callbacks_unsupported())
            })
    })
}

fn render_unsupported_php_callback_method(method_name: &str, description: &str) -> String {
    crate::e2e::template_env::render(
        "php/test_method.jinja",
        crate::alef_context! {
            method_name => method_name,
            description => description,
            skip_reason => "PHP callback bridge is unavailable for this lifecycle-only trait",
        },
    )
}

#[cfg(test)]
mod php_callback_capability_tests;

/// True when `body` contains at least one line that is not blank and not a
/// `//`-prefixed comment — i.e. an executable PHPUnit assertion statement.
/// A body made up only of "// skipped: ..." lines is not executable and
/// must not be treated as if the fixture asserted something.
fn has_executable_assertion(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with("//")
    })
}

/// When a fixture's rendered `assertions_body` has no executable statement —
/// its only assertion was `not_error`, or every field assertion resolved to a
/// "skipped" comment — inject a real assertion instead of leaving the
/// PHPUnit test vacuous. Never falls back to `expectNotToPerformAssertions()`:
/// that only quiets PHPUnit's own risky-test detector without asserting
/// anything, certifying untested behaviour as green. ~keep
fn apply_vacuous_assertion_fallback(
    assertions_body: &mut String,
    is_streaming: bool,
    expects_error: bool,
    result_var: &str,
    streaming_drive_is_the_check: bool,
    returns_void: bool,
) {
    if expects_error || has_executable_assertion(assertions_body) {
        return;
    }
    if is_streaming {
        // ~keep `$chunks` is bound to a freshly drained array immediately above, so
        // `is_array($chunks)` cannot fail — it is a vacuous guard, not a check, and it is exactly
        // what kept a streaming example that asserts nothing looking green. It is kept only where
        // the drive itself IS the declared check (`not_error`): there the real expectation is that
        // the call did not throw, and this line only stops PHPUnit filing the test as risky.
        // Everywhere else the body is left comment-only so the refusal below can see it.
        if streaming_drive_is_the_check {
            assertions_body.push_str("        $this->assertTrue(is_array($chunks), 'expected drained chunks list');\n");
        }
    } else if returns_void {
        // ~keep A void call's return value is PHP `null` on success — PHP's binding backend
        // declares these functions `: void` (see `src/backends/php/gen_bindings/public_api.rs`),
        // and a void function's return is always `null`. `assertNotNull($result)` would therefore
        // fail on every successful call, not just an unsuccessful one: a guaranteed-red test, worse
        // than the vacuous body it replaced. There is no result to assert on, so assert the one
        // thing that is true and meaningful here — the call ran this far without throwing.
        assertions_body.push_str("        $this->assertTrue(true, 'expected the call not to throw');\n");
    } else {
        let _ = writeln!(assertions_body, "        $this->assertNotNull(${result_var});");
    }
}

/// The body emitted in place of one whose declared assertions all funnelled into skip markers.
///
/// ~keep Which refusal is emitted follows who can fix it, exactly as in `ruby/examples.rs`. An
/// unresolved field path is the consumer's to repair, so it gets `$this->fail(..)` — a spelling
/// this backend already emits for an error fixture that did not throw. Everything else is alef's
/// generator debt or a PHP-extension limit no consumer edit clears, so it gets PHPUnit's own
/// `markTestSkipped`, which reports the test as skipped and never as a pass.
fn render_php_refusal(markers: &str, refusal: &InertExample) -> String {
    let reason = escape_php_single(&refusal.reason());
    let statement = match refusal.cause {
        InertCause::UnresolvedFieldPath => format!("        $this->fail('{reason}');\n"),
        InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
            format!("        $this->markTestSkipped('{reason}');\n")
        }
    };
    inert_example::refusal_body(markers, &statement)
}

/// Render the PHP test-method body for an `error`-asserting fixture.
///
/// ~keep With no declared value this returns output byte-identical to the
/// pre-existing `expectException` idiom. PHPUnit's `expectException*` family
/// only ever inspects the thrown exception's message, so matching the
/// message-or-class-name disjunction other backends use (see
/// `declared_error_value`'s doc comment) requires a manual try/catch instead
/// of `expectExceptionMessageMatches`. Which of those two conventions applies, and whether PHP
/// can ever satisfy the second, is decided once by `declared_error_variant::classify` — see its
/// doc for why PHP lands on "not yet" today (one exception class for the whole extension).
fn render_error_test_body(
    setup_lines: &[String],
    call_expr: &str,
    fixture: &Fixture,
    errors: &[crate::core::ir::ErrorDef],
) -> String {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    let mut out = String::new();
    match classify("php", fixture, errors) {
        DeclaredErrorAssertion::Assert(declared) => {
            let pattern = php_pcre_literal(declared);
            let message_value = escape_php_single(declared);
            // ~keep PHPUnit assertion failures are Exceptions too; assert outside the application's catch.
            out.push_str("        $caughtException = null;\n        try {\n");
            for line in setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            let _ = writeln!(out, "            {call_expr};");
            out.push_str("        } catch (\\Exception $e) {\n            $caughtException = $e;\n        }\n");
            out.push_str("        $this->assertInstanceOf(\\Exception::class, $caughtException);\n");
            let _ = write!(
                out,
                "        $this->assertTrue(preg_match({pattern}, $caughtException->getMessage()) === 1 || preg_match({pattern}, get_class($caughtException)) === 1, 'expected exception message or class name to match {message_value}');"
            );
        }
        declared => {
            if let DeclaredErrorAssertion::Unsubstantiable(variant) = declared {
                let _ = writeln!(out, "{}", skip_line("        ", "//", variant, &fixture.id, "php"));
            }
            out.push_str("        $this->expectException(\\Exception::class);\n");
            for line in setup_lines {
                let _ = writeln!(out, "        {line}");
            }
            let _ = write!(out, "        {call_expr};");
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    lang: &str,
    namespace: &str,
    class_name: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    php_enum_lowering: &super::enum_variant_access::PhpEnumLowering,
    enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    php_client_factory: Option<&str>,
    options_via: &str,
    adapters: &[crate::core::config::extras::AdapterConfig],
    php_lang_rename_all: &str,
    config: &ResolvedCrateConfig,
    errors: &[crate::core::ir::ErrorDef],
    trait_bridge_imports: &mut Vec<String>,
) {
    // Resolve per-fixture call config: supports named calls via fixture.call field.
    let mut call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Fallback: if the resolved call has required args missing from input,
    // try to find a better-matching call from the named calls.
    call_config = crate::e2e::codegen::select_best_matching_call(call_config, e2e_config, fixture);
    let per_call_getter_map = types::build_php_getter_map(
        type_defs,
        enums,
        call_config,
        e2e_config.effective_result_fields(call_config),
    );
    let variant_access = super::enum_variant_access::PhpVariantAccess::new(&per_call_getter_map, php_enum_lowering);
    // Extracted to `call_field_resolver.rs` (this file is at the file-size ratchet's frozen
    // ceiling).
    let call_field_resolver =
        super::call_field_resolver::build_call_field_resolver(super::call_field_resolver::CallFieldResolverInputs {
            e2e_config,
            call_config,
            fixture,
            lang,
            type_defs,
            functions,
            per_call_getter_map: &per_call_getter_map,
        });
    let field_resolver = &call_field_resolver;
    let call_overrides = call_config.overrides.get(lang);
    let has_override = call_overrides.is_some_and(|o| o.function.is_some());
    // `result_is_simple` is a Rust-side property of the call's return type and
    // applies identically to every binding. Read it from the call-level field
    // first (preferred), and fall back to the per-call language override or the
    // file-level language default for backwards compatibility.
    let result_is_simple =
        call_config.result_is_simple || call_overrides.is_some_and(|o| o.result_is_simple) || result_is_simple;
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // The PHP facade exposes async Rust methods under their bare name (no `_async`
    // suffix) — PHP has no surface-level async, so the facade picks the async
    // implementation as the default and delegates to `*Async` on the native class.
    // The `*_sync` variants stay explicit (e.g. `extract_bytes_sync` → `extractBytesSync`).
    if !has_override {
        function_name = function_name.to_lower_camel_case();
    }
    let result_var = call_config.effective_result_var();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    let args = recipe.args;

    let method_name = crate::e2e::escape::sanitize_filename(&fixture.id);
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    if uses_unsupported_php_callback(args, config) {
        out.push_str(&render_unsupported_php_callback_method(&method_name, description));
        return;
    }

    // Resolve options_type for this call. Precedence: per-language call override,
    // then the call-level `options_type` (the binding-agnostic config parameter type,
    // a call-specific options type), then the global per-language call override (fallback default).
    let call_options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.options_type.as_deref())
    });

    let adapter_lookup_name = call_config.core_lookup_name(lang);
    let call_adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| adapters.iter().find(|a| a.name == name));
    let adapter_request_type: Option<String> = call_adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on the
    // owner handle (`$engine->streamItems($req)`), not as static facade methods.
    // Capture the owner handle variable so the call is rendered as an
    // instance-method invocation and the handle is omitted from the argument list.
    let streaming_owner_handle: Option<String> = if call_adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    }) {
        args.iter().find(|a| a.arg_type == "handle").map(|a| a.name.clone())
    } else {
        None
    };

    let (mut setup_lines, args_str, teardown_block) = args::build_args_and_setup(
        &fixture.input,
        args,
        class_name,
        enum_fields,
        fixture,
        options_via,
        call_options_type,
        adapter_request_type.as_deref(),
        namespace,
        streaming_owner_handle.is_some(),
        type_defs,
        php_lang_rename_all,
        config,
        trait_bridge_imports,
        false,
    );

    // Check for skip_languages early
    let skip_test = call_config.skip_languages.iter().any(|l| l == "php");
    if skip_test {
        let rendered = crate::e2e::template_env::render(
            "php/test_method.jinja",
            crate::alef_context! {
                method_name => method_name,
                description => description,
                client_factory => String::new(),
                setup_lines => Vec::<String>::new(),
                expects_error => false,
                skip_test => true,
                call_expr => String::new(),
                result_var => result_var,
                assertions_body => String::new(),
            },
        );
        out.push_str(&rendered);
        return;
    }

    // Build visitor if present and add to setup
    let mut options_already_created = !args_str.is_empty() && args_str == "$options";
    if let Some(visitor_spec) = &fixture.visitor {
        visitor::build_php_visitor(&mut setup_lines, visitor_spec);
        if !options_already_created {
            // A visitor has no standalone PHP parameter — it is only reachable by
            // assignment onto an options object, so without a resolvable options type
            // there is nowhere to attach it. This used to re-render the template with
            // `skip_test => true` — the *same* output the sanctioned `skip_languages`
            // branch above emits — so a config failure was indistinguishable from an
            // author-declared skip, and the suite reported green either way. That
            // sanctioned branch has already returned by here, so reaching this point
            // means the fixture is genuinely required for PHP. Fail generation instead
            // of emitting a test that silently drops the visitor. ~keep
            let Some(options_type) = call_options_type.or_else(|| stubs::trait_bridge_options_type(config)) else {
                panic!(
                    "PHP e2e generator: fixture `{}` declares a `visitor`, but neither its `[e2e.call]` config nor any `[[crates.trait_bridges]]` entry provides an `options_type` to attach it to; cannot generate a PHP visitor test without a resolvable trait bridge options type",
                    fixture.id
                );
            };
            if options_via == "from_json" {
                // When options_via is "from_json", create options from JSON first,
                // then attach the visitor using with_visitor() since PHP closures can't be JSON-encoded.
                setup_lines.push(format!("$options = \\{namespace}\\{options_type}::from_json('{{}}');"));
                setup_lines.push(format!(
                    "$visitorHandle = \\{namespace}\\VisitorHandle::from_php_object($visitor);"
                ));
                // ext-php-rs camel-cases snake_case method names; the generated PHP class
                // exposes the wither as `withVisitor`, not `with_visitor`.
                setup_lines.push("$options = $options->withVisitor($visitorHandle);".to_string());
            } else {
                // Default builder pattern for other options_via modes
                setup_lines.push(format!("$builder = \\{namespace}\\{options_type}::builder();"));
                setup_lines.push("$options = $builder->visitor($visitor)->build();".to_string());
            }
            options_already_created = true;
        }
    }

    let final_args = if options_already_created {
        if args_str.is_empty() || args_str == "$options" {
            "$options".to_string()
        } else {
            format!("{args_str}, $options")
        }
    } else {
        args_str
    };

    let call_expr = if php_client_factory.is_some() {
        format!("$client->{function_name}({final_args})")
    } else if let Some(ref handle_var) = streaming_owner_handle {
        // Instance-method invocation on the owner handle.
        format!("${handle_var}->{function_name}({final_args})")
    } else {
        format!("{class_name}::{function_name}({final_args})")
    };

    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_factory = if let Some(factory) = php_client_factory {
        let fixture_id = &fixture.id;
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            format!(
                "$apiKey = getenv('{var}');\n        $baseUrl = ($apiKey !== false && $apiKey !== '') ? null : getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}';\n        fwrite(STDERR, \"{fixture_id}: \" . ($baseUrl === null ? 'using real API ({var} is set)' : 'using mock server ({var} not set)') . \"\\n\");\n        $client = \\{namespace}\\{class_name}::{factory}($baseUrl === null ? $apiKey : 'test-key', $baseUrl);"
            )
        } else if has_mock {
            let base_url_expr = if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                format!("(getenv('{env_key}') ?: getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}')")
            } else {
                format!("getenv('MOCK_SERVER_URL') . '/fixtures/{fixture_id}'")
            };
            format!("$client = \\{namespace}\\{class_name}::{factory}('test-key', {base_url_expr});")
        } else if let Some(var) = api_key_var {
            format!(
                "$apiKey = getenv('{var}');\n        if (!$apiKey) {{ $this->markTestSkipped('{var} not set'); return; }}\n        $client = \\{namespace}\\{class_name}::{factory}($apiKey);"
            )
        } else {
            format!("$client = \\{namespace}\\{class_name}::{factory}('test-key');")
        }
    } else {
        String::new()
    };

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());

    let collect_snippet = if is_streaming {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet("php", result_var, "chunks")
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Collect fields_array fields that are referenced in assertions
    // so we can emit bindings for them (e.g., $chunks = $result->getChunks();).
    //
    // Use a BTreeMap (sorted by key) so the emitted accessor extraction lines
    // appear in a stable order across regens. A HashMap here previously leaked
    // its randomized iteration order into the generated PHP source, causing
    // e.g. parser-pack's `e2e/php/tests/ProcessTest.php` to flip the relative order
    // of `$imports` vs `$structure` bindings between back-to-back
    // `alef e2e generate` invocations.
    let mut fields_array_bindings: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::new();
    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field
            && !f.is_empty()
            && !variant_access.is_unavailable(f)
            && field_resolver.is_array(f)
            // Only collect bindings for fields that are valid on the result type
            && field_resolver.is_valid_for_result(f)
            && !fields_array_bindings.contains_key(f.as_str())
        {
            let accessor = field_resolver.accessor(f, "php", &format!("${result_var}"));
            let var_name = f.to_lower_camel_case();
            fields_array_bindings.insert(f.clone(), (var_name, accessor));
        }
    }

    // Generate field binding lines (e.g., $chunks = $result->getChunks();)
    // Every collected array-binding accessor needs its $var emitted; the prior
    // hardcoded allowlist ("chunks"/"imports"/"structure") silently dropped
    // bindings like $choices0MessageToolCalls and $segments, leaving
    // assertions that reference them to fail with "Undefined variable".
    // BTreeMap iteration is sorted-by-key, so this loop is deterministic.
    let mut field_bindings = String::new();
    for (var_name, accessor) in fields_array_bindings.values() {
        field_bindings.push_str(&format!("        ${} = {};\n", var_name, accessor));
    }

    // Render assertions_body
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        assertions::render_assertion(
            &mut assertions_body,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            call_config.result_is_array,
            &fields_array_bindings,
            is_streaming,
            &variant_access,
        );
    }

    // A fixture whose only assertion is `not_error` (streaming or not), or
    // whose resolvable field assertions all rendered as "skipped" comments,
    // leaves assertions_body with no executable statement. Fall back to a
    // real assertion instead of a vacuous test body.
    crate::e2e::codegen::fail_on_unavailable_field_markers(&assertions_body, "php", &fixture.id, &fixture.assertions);
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, "php", &fixture.id);

    // ~keep The verdict is taken BEFORE the fallback, not after: the fallback's whole job is to
    // put an executable line into an otherwise comment-only body, so reading the verdict after it
    // would answer `None` every time and the refusal would be dead code. `expects_error` is
    // excluded because `test_method.jinja` splices `error_test_body` instead of `assertions_body`
    // for those.
    let verdict = if expects_error {
        None
    } else {
        inert_example::inert_verdict(&assertions_body, "php", &fixture.id, &fixture.assertions)
    };
    let declares_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    // ~keep A non-streaming example still has an honest, FAILABLE fallback available to it —
    // `$this->assertNotNull($result)`, which a binding returning null really does fail — so only
    // the consumer-fixable unresolved path is refused there; refusing the rest would delete the
    // "the call worked" coverage that fallback carries. A streaming example has no such subject,
    // so every cause is refused unless the fixture declared `not_error`, where the drive itself is
    // the check.
    let refuse = verdict.as_ref().is_some_and(|refusal| {
        refusal.cause == InertCause::UnresolvedFieldPath || (is_streaming && !declares_not_error)
    });
    if let Some(refusal) = verdict.filter(|_| refuse) {
        inert_example::record_refusal(&refusal);
        assertions_body = render_php_refusal(&assertions_body, &refusal);
    } else {
        apply_vacuous_assertion_fallback(
            &mut assertions_body,
            is_streaming,
            expects_error,
            result_var,
            declares_not_error,
            call_config.returns_void,
        );
    }

    let error_test_body = if expects_error {
        let mut body = render_error_test_body(&setup_lines, &call_expr, fixture, errors);
        // ~keep `render_error_test_body` ends without a trailing newline on the `None` arm, so the
        // markers have to open their own line rather than being appended to the last statement.
        let markers = crate::e2e::codegen::error_path_assertions::render(fixture, "        // ", "php");
        if !markers.is_empty() {
            body.push('\n');
            body.push_str(markers.trim_end());
        }
        body
    } else {
        String::new()
    };

    let rendered = crate::e2e::template_env::render(
        "php/test_method.jinja",
        crate::alef_context! {
            method_name => method_name,
            description => description,
            client_factory => client_factory,
            setup_lines => setup_lines,
            expects_error => expects_error,
            error_test_body => error_test_body,
            skip_test => fixture.assertions.is_empty(),
            returns_void => call_config.returns_void,
            call_expr => call_expr,
            result_var => result_var,
            collect_snippet => collect_snippet,
            field_bindings => field_bindings,
            assertions_body => assertions_body,
            teardown_block => teardown_block,
        },
    );
    out.push_str(&rendered);
}

#[cfg(test)]
mod vacuous_assertion_fallback_tests;

#[cfg(test)]
mod error_test_body_tests;

#[cfg(test)]
mod visitor_options_type_tests;

#[cfg(test)]
#[path = "test_method/inert_example_refusal_tests.rs"]
mod inert_example_refusal_tests;
