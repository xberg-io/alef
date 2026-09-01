use super::*;

use std::fmt::Write as _;

use crate::e2e::codegen::inert_example::{self, InertCause};

/// Escape a string so it matches itself literally when embedded as the body of a
/// JS/TS regex literal (`/…/`), rather than being interpreted as a regex pattern.
///
/// Escapes the standard JS regex metacharacters, the `/` delimiter (which would
/// otherwise terminate the literal early), and control characters that cannot
/// appear raw inside a regex literal.
pub(in crate::e2e::codegen::typescript::test_file) fn escape_js_regex_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '/' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    client_factory: Option<&str>,
    options_type: Option<&str>,
    e2e_config: &E2eConfig,
    lang: &str,
    nested_types: &std::collections::HashMap<String, String>,
    enum_fields: &std::collections::HashMap<String, String>,
    result_enum_fields: &std::collections::HashMap<String, String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    wasm_type_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    referenced_enums: &mut std::collections::BTreeSet<String>,
    errors: &[crate::core::ir::ErrorDef],
) {
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
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    // Anchor the IR-derived result-field oracle (`with_ir_result_fields`) at the call's declared
    // Rust return type, mirroring the rust/python/java/csharp/elixir/go e2e generators. Purely
    // additive: `result_field_oracle_knows` only ever REFUSES what it positively knows the root
    // type lacks; an unresolved root (e.g. no `functions` in scope, as in the WASM caller today)
    // leaves every anchored answer disabled and the pre-existing behaviour unchanged. ~keep
    let call_root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call_config,
        lang,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        // The consumer's own `fields_method_calls` is what declares a tagged-union crossing, and
        // `FieldResolver::tagged_union_split` scans exactly this set. Passing a fresh empty set
        // (as this generator did) makes that scan answer `None` for every path, so the node/wasm
        // suites rendered a raw dotted accessor across the boundary — `TS2339`, because NAPI
        // flattens a data enum into one object with no variant member. Every sibling generator
        // (gleam, kotlin, dart, python, elixir, rust, java, zig) already passes it. TypeScript's
        // accessor renderer takes no `method_calls` argument, so this only enables the crossing
        // detector and the `is_known_via_sibling_field_config` acceptance it feeds — it cannot
        // change an emitted accessor. ~keep
        e2e_config.effective_fields_method_calls(call_config),
    )
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, lang), call_root_type.clone())
    // Anchored at the same declared return type, so a crossing this generator refuses can be
    // named against the IR's real union type rather than re-derived from the path's shape.
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_wasm_enum_representations(enums)
    .with_napi_tagged_object_enums(enums)
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type)
    .with_collection_element_metadata(type_defs)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    // `with_ir_fields` only proves a BARE field name optional, by crate-wide unanimity — no
    // path context. The `_with_optionals` renderers key their per-segment `?.`/`?.[0]` check by
    // the FULL cumulative path walked so far, so a bare name never matches once the path crosses
    // more than one segment (an `Option<Vec<T>>` reached through e.g. `entries[0].sections`).
    // Anchors this fixture's own assertion paths via the IR's real (owner_type, field_name)
    // walk, mirroring `presentation.rs`'s existing use of `with_anchored_optional_paths`. ~keep
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()));
    let field_resolver = &call_field_resolver;
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call_config, type_defs)
        .with_functions(functions);
    let function_name = resolve_node_function_name(call_config);
    let core_lookup_name = call_config.core_lookup_name(lang);
    let adapter_lookup_names = core_lookup_name.as_deref().map_or_else(
        || vec![function_name.as_str()],
        |core| vec![function_name.as_str(), core],
    );
    let streaming_item_type =
        crate::e2e::codegen::recipe::streaming_item_type(call_config, &config.adapters, &adapter_lookup_names);
    let streaming_item_enum = streaming_item_type.and_then(|name| enums.iter().find(|enum_def| enum_def.name == name));
    let result_var = call_config.effective_result_var();
    // A per-language `async` override is an explicit, trusted answer -- honor it verbatim
    // even against a disagreeing IR, the same way every other per-language override in this
    // file wins over a derived default. Absent that, `call_config.r#async` is a plain `bool`
    // defaulting to `false`, so "never configured" and "explicitly not async" are the same
    // bit -- and a Rust function that became `async fn` after the fixture was authored left
    // that default stale. `ResolvedE2eCallRecipe::ir_is_async` is the IR's own authoritative
    // answer for this shape; OR it in only when there is no per-language override, so a call
    // the IR cannot resolve (a client_factory-only call, a trait method the seam does not
    // cover) keeps behaving exactly as configured. ~keep
    let call_is_async = match call_config.overrides.get(lang).and_then(|o| o.r#async) {
        Some(explicit) => explicit,
        None => call_config.r#async || recipe.ir_is_async(lang).unwrap_or(false),
    };
    let args = recipe.args;
    let result_is_simple =
        call_config.result_is_simple || call_config.overrides.get(lang).is_some_and(|o| o.result_is_simple);

    // Resolve per-fixture wasm/node override fields (options_type, bigint_fields,
    // nested_types, enum_fields). Per-call overrides win over the file-level
    // default; missing fields fall back to the file-level default. WASM/wasm-bindgen
    // is the primary consumer of `bigint_fields` (u64/i64 setters reject Number).
    let per_call_override = recipe.override_config;
    let effective_options_type: Option<String> = per_call_override
        .and_then(|o| o.options_type.clone())
        .or_else(|| options_type.map(|s| s.to_string()))
        .map(|type_name| canonical_ts_type_name(lang, &type_name, config));
    let mut effective_nested_types: std::collections::HashMap<String, String> = nested_types.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.nested_types {
            effective_nested_types.insert(k.clone(), v.clone());
        }
    }
    let mut effective_enum_fields: std::collections::HashMap<String, String> = enum_fields.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.enum_fields {
            effective_enum_fields.insert(k.clone(), v.clone());
        }
    }
    let mut effective_result_enum_fields: std::collections::HashMap<String, String> = result_enum_fields.clone();
    if let Some(o) = per_call_override {
        for (k, v) in &o.result_enum_fields {
            effective_result_enum_fields.insert(k.clone(), v.clone());
        }
    }
    // Per-language `extra_args` from call overrides — verbatim trailing
    // expressions appended after the configured args (e.g. `undefined` for an
    // optional trailing parameter the fixture cannot supply).
    let extra_args = recipe.extra_args;
    let global_bigint_fields: Vec<String> = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|o| o.bigint_fields.clone())
        .unwrap_or_default();
    let mut effective_bigint_fields: std::collections::BTreeSet<String> = global_bigint_fields.into_iter().collect();
    if let Some(o) = per_call_override {
        for f in &o.bigint_fields {
            effective_bigint_fields.insert(f.clone());
        }
    }

    // Force test to async if we need to read files for bytes args or have trait bridge tests
    let has_trait_bridge = has_trait_bridge_args(args);
    // ~keep `void_not_error` has to be known HERE, not only at its use site further down: the
    // branch it selects in `test_function.jinja` emits `await expect(..).resolves.not.toThrow()`
    // into the very `it(..)` callback whose `async` keyword is frozen into `async_kw` two lines
    // below. For a `returns_void` + `not_error` fixture over a *synchronous* call, `call_is_async`
    // is false, so without this term the callback renders without `async` while its body still
    // carries `await` — a hard TS/JS syntax error that aborts the whole formatting phase (and with
    // it every other language's formatting), not merely a bad test.
    let void_not_error = call_config.returns_void && fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    let test_is_async =
        call_is_async || has_bytes_file_reads(&fixture.input, args) || has_trait_bridge || void_not_error;
    // `await_kw` may still force `await` for trait bridge calls: `await` on a non-Promise value
    // is a legal no-op in JS/TS, so this cannot introduce a compile or runtime error even when
    // the underlying call is synchronous. `call_is_async` itself must NOT be forced this way: the
    // `void_not_error` branch of `test_function.jinja` uses it to pick between
    // `expect(...).resolves.not.toThrow()` (needs a real Promise) and `expect(() =>
    // ...).not.toThrow()` (works on any callable). Forcing it to `true` for every trait-bridge
    // call made a *synchronous* trait method emit `.resolves` on a non-Promise, which is a hard
    // TypeError at runtime and a type error under strict TypeScript. The IR/config-derived
    // `call_is_async` computed above is the single authority for that decision; only the local
    // `await_kw` gets the trait-bridge forcing. ~keep
    let await_kw_is_async = call_is_async || has_trait_bridge;

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('\\', "\\\\").replace('"', "\\\"");
    let async_kw = if test_is_async { "async " } else { "" };
    let await_kw = if await_kw_is_async { "await " } else { "" };

    let handle_config_type = per_call_override.and_then(|o| o.handle_config_type.clone());

    let (mut setup_lines, mut args_str) = build_args_and_setup(
        &fixture.input,
        args,
        effective_options_type.as_deref(),
        fixture,
        &effective_nested_types,
        lang,
        &effective_enum_fields,
        &effective_bigint_fields,
        handle_config_type.as_deref(),
        type_defs,
        enums,
        wasm_type_prefix,
        config,
        false,
        referenced_enums,
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
    );
    // The builders above recognise an undeclared fixture key many frames down, where nothing
    // knows which call or language is being served. This is the frame that does, and the only
    // one that can tell a per-call `options_type` from a file-level default this call merely
    // inherited -- the distinction that decides where the fix belongs. ~keep
    attribute_key_refusals(lang, fixture, e2e_config, call_config, per_call_override, options_type);

    if !extra_args.is_empty() {
        let extra_str = extra_args.join(", ");
        args_str = if args_str.is_empty() {
            extra_str
        } else {
            format!("{args_str}, {extra_str}")
        };
    }

    let mut visitor_arg = String::new();
    if let Some(visitor_spec) = &fixture.visitor {
        visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
    }

    let final_args = if visitor_arg.is_empty() {
        args_str
    } else if lang == "wasm" {
        if let Some(binding) = wasm_visitor_binding(config, effective_options_type.as_deref()) {
            apply_wasm_visitor_arg(&args_str, &visitor_arg, &binding)
        } else {
            args_str
        }
    } else if lang == "node" {
        // Node: visitor is read off `options.visitor` by the NAPI binding. Cast through
        // `any` so the plain visitor object satisfies the opaque `VisitorHandle` field type.
        node_visitor_args(&args_str, &visitor_arg)
    } else if args_str.is_empty() {
        format!("{{ visitor: {visitor_arg} }}")
    } else {
        format!("{args_str}, {{ visitor: {visitor_arg} }}")
    };

    let call_expr = if client_factory.is_some() {
        format!("client.{function_name}({final_args})")
    } else {
        format!("{function_name}({final_args})")
    };

    let base_url_expr = if fixture.has_host_root_route() {
        format!(
            "process.env.MOCK_SERVER_{} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{}`",
            fixture.id.to_uppercase(),
            fixture.id
        )
    } else {
        format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id)
    };

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // A declared error value is matched against EITHER the thrown error's message
    // OR its `name`/type — the same disjunction the Rust and Python generators use
    // (see `crate::e2e::codegen::declared_error_value` doc comment) — for the fixtures
    // `declared_error_variant::classify` recognises as message-style. Building a regex literal
    // from the raw value would let regex metacharacters in the fixture's declared string (e.g.
    // `.`, `(`, `)`) change what the pattern matches, so the value is escaped to match itself
    // literally. A value naming a real `ErrorVariant` this backend cannot substantiate renders
    // the registered skip instead (`error_skip_line`, spliced verbatim by the template): every
    // NAPI throw site is `napi::Error::new(Status::GenericFailure, e.to_string())` — generic
    // status, generic `.name`, message only.
    let (error_value_regex, error_skip_line) = if expects_error {
        match crate::e2e::codegen::declared_error_variant::classify(lang, fixture, errors) {
            crate::e2e::codegen::declared_error_variant::DeclaredErrorAssertion::Undeclared => (None, None),
            crate::e2e::codegen::declared_error_variant::DeclaredErrorAssertion::Assert(value) => {
                (Some(format!("/{}/", escape_js_regex_literal(value))), None)
            }
            crate::e2e::codegen::declared_error_variant::DeclaredErrorAssertion::Unsubstantiable(variant) => (
                None,
                Some(crate::e2e::codegen::declared_error_variant::skip_line(
                    "\t\t",
                    "//",
                    variant,
                    &fixture.id,
                    lang,
                )),
            ),
        }
    } else {
        (None, None)
    };

    // Build client setup
    let has_mock = fixture.mock_response.is_some() || fixture.http.is_some();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    let client_setup = if let Some(factory) = client_factory {
        if let Some(var) = api_key_var.filter(|_| has_mock) {
            let mock_url = format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{}`", fixture.id);
            format!(
                "const apiKey = process.env.{var};\n    \
                 const baseUrl = apiKey ? undefined : {mock_url};\n    \
                 console.log(`{id}: ${{apiKey ? 'using real API ({var} is set)' : 'using mock server ({var} not set)'}}`);\n    \
                 const client = {factory}(apiKey ?? 'test-key', baseUrl);",
                id = fixture.id
            )
        } else if has_mock {
            format!("const client = {factory}('test-key', {base_url_expr});")
        } else if let Some(var) = api_key_var {
            // Live-API tests: skip when the env var isn't set so the suite can run
            // without real credentials, matching the python codegen's pattern.
            format!(
                "const apiKey = process.env.{var};\n    \
                 if (!apiKey) {{\n        \
                     return;\n    \
                 }}\n    \
                 const client = {factory}(apiKey);"
            )
        } else {
            format!("const client = {factory}('test-key', {base_url_expr});")
        }
    } else {
        String::new()
    };

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());

    // Streaming-error detection: a fixture that calls a streaming function with an
    // error assertion (e.g. 401, 400 content-policy) — the upstream rejects before
    // any chunks arrive, but the NAPI / wasm binding returns the stream handle
    // synchronously. The HTTP error only surfaces when iterating, so we generate a
    // drain loop inside the `rejects.toThrow()` block so the error propagates
    // before the expect wrapper exits.
    //
    // Triggers in two cases:
    // - Declared streaming call (`call_config.streaming_enabled() = true`) + error fixture.
    // - Heuristic name-based detection (function name contains "stream") for
    //   fixtures that pre-date the explicit `streaming` flag.
    let is_streaming_error_call = expects_error && (is_streaming || function_name.to_lowercase().contains("stream"));

    // Build assertions body. Every assertion (including `not_error`) is passed
    // through — `render_assertion` decides what, if anything, to render for each
    // type. A prior `!call_config.returns_result` guard here skipped `not_error`
    // outright for some calls, but `render_assertion`'s own `not_error` arm was
    // *also* a no-op regardless of that condition, so the guard was a distinction
    // without a difference: both branches produced the same (vacuous) result.
    // WHETHER `not_error` may assert presence is decided once, centrally — see
    // `not_error_presence::may_assert_presence`'s doc for why a sibling assertion or an
    // `Option<T>` result both make an unconditional presence check unsafe. ~keep
    let not_error_result_is_option =
        call_config.result_is_option || call_config.overrides.get(lang).is_some_and(|o| o.result_is_option);
    let not_error_may_assert_presence =
        crate::e2e::codegen::not_error_presence::may_assert_presence(fixture, not_error_result_is_option);
    let mut assertions_body = String::new();
    for assertion in &fixture.assertions {
        render_assertion_with_streaming_item_type(
            &mut assertions_body,
            assertion,
            result_var,
            field_resolver,
            result_is_simple,
            &effective_result_enum_fields,
            lang,
            is_streaming,
            streaming_item_enum,
            call_config.returns_void,
            not_error_may_assert_presence,
        );
    }

    // A fixture that declared at least one assertion but every one of them resolved
    // to a "skipped" comment (all its fields are unavailable on the result type) is
    // otherwise indistinguishable from a fixture with zero declared assertions — an
    // entirely comment-only, vacuously-passing test body. `not_error` already emits
    // a real `expect(...).toBeDefined()` (see `render_assertion`'s `not_error` arm),
    // so this only fires on the remaining gap: real field assertions that all got
    // dropped. TypeScript was the one backend in this defect class with no fallback
    // of any kind for that case — mirror python/php's `apply_vacuous_assertion_fallback`.
    // A fixture with genuinely zero declared assertions is left untouched, matching
    // every other backend's deliberate "just call it" smoke-test contract. ~keep
    // ~keep The marker scan and the verdict both run BEFORE the fallback, not after: the
    // fallback's whole job is to put an executable line into an otherwise comment-only body, so a
    // verdict read after it would answer `None` every time and the refusal would be dead code.
    // `expects_error` is excluded because `test_function.jinja`'s error branch never references
    // `assertions_body` — the `rejects` assertion IS the expectation there.
    crate::e2e::codegen::fail_on_unavailable_field_markers(&assertions_body, lang, &fixture.id, &fixture.assertions);
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&assertions_body, lang, &fixture.id);
    let declares_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
    // A `returns_void` call binds no usable `result`, so a fixture whose only assertion is
    // `not_error` has nothing to assert on the way non-void calls do. `test_function.jinja`
    // wraps `call_expr` itself in `expect(...).resolves.not.toThrow()` instead, so the check is
    // a real, visible assertion rather than a bare `await call_expr;` relying only on an
    // unhandled rejection to fail the test. ~keep
    debug_assert_eq!(void_not_error, call_config.returns_void && declares_not_error);
    // ~keep `void_not_error` is excluded here for the same reason `expects_error` is: its real
    // assertion is rendered by the call-wrapping branch in `test_function.jinja`, not spliced
    // into `assertions_body` — `inert_verdict` only sees `assertions_body` and would otherwise
    // misread a correctly-empty body as vacuous and refuse it, discarding the real check that
    // already exists one branch over.
    let verdict = if expects_error || void_not_error {
        None
    } else {
        inert_example::inert_verdict(&assertions_body, lang, &fixture.id, &fixture.assertions)
    };
    // ~keep An unresolved field path is the consumer's to fix, so it stays a running `it(..)` and
    // gets an expectation that FAILS and names the fixture. A non-streaming example still has an
    // honest, FAILABLE fallback for every other cause — `expect(result).toBeDefined()`, which a
    // binding returning `undefined` really does trip — so refusing those would delete the "the
    // call worked" coverage that fallback carries. A streaming example has no such subject
    // (`chunks` is a freshly bound array), so its remaining causes become vitest's own `it.skip`,
    // which never reports a pass, unless the fixture declared `not_error` — there the drive itself
    // is the check and refusing would delete it.
    let mut refusal_body = String::new();
    let refuse = verdict.as_ref().is_some_and(|refusal| {
        refusal.cause == InertCause::UnresolvedFieldPath || (is_streaming && !declares_not_error)
    });
    match verdict.filter(|_| refuse) {
        Some(refusal) => {
            inert_example::record_refusal(&refusal);
            match refusal.cause {
                InertCause::UnresolvedFieldPath => {
                    let reason = escape_js(&refusal.reason());
                    assertions_body = inert_example::refusal_body(
                        &assertions_body,
                        &format!(
                            "    const unresolvedAssertion = \"{reason}\";\n    \
                             expect(unresolvedAssertion).toBeNull();\n"
                        ),
                    );
                }
                InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
                    for line in assertions_body.lines().filter(|line| !line.trim().is_empty()) {
                        let _ = writeln!(refusal_body, "\t\t{}", line.trim());
                    }
                    let _ = writeln!(refusal_body, "\t\t// {}", refusal.reason());
                }
            }
        }
        None => apply_vacuous_assertion_fallback(
            &mut assertions_body,
            !fixture.assertions.is_empty(),
            is_streaming,
            result_var,
            declares_not_error,
            call_config.returns_void,
        ),
    }

    // Whether the call's result is worth binding to `const result = ...` rather
    // than discarding with a bare `await callExpr();`. Derived from what
    // `assertions_body` actually contains (a real, non-comment line) instead of a
    // separately maintained predicate over `fixture.assertions` — the previous
    // predicate excluded `not_error` from ever counting as "usable" even after
    // this fix made `render_assertion` emit a real `expect(...).toBeDefined()`
    // for it, which would have silently reintroduced the same drift this
    // regression test guards against. ~keep
    let has_usable_assertion = assertions_body
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with("//"));

    // For streaming fixtures: capture the stream in `stream`, then collect into `chunks`.
    // Pass the actual `lang` (was hardcoded to "node") so wasm gets the
    // explicit-`next()` drain instead of the NAPI `for await` loop.
    let (ts_result_var, collect_snippet) = if is_streaming {
        let snip = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            lang, "stream", "chunks",
        )
        .unwrap_or_default();
        ("stream".to_string(), snip)
    } else {
        (result_var.to_string(), String::new())
    };

    // Extract skip reason if the fixture has a skip directive for this language
    let skip_reason = fixture.skip.as_ref().and_then(|skip| {
        if skip.should_skip(lang) {
            skip.reason.clone()
        } else {
            None
        }
    });

    // Long-running fixtures opt in explicitly through tags, or use slow-grammar timeouts.
    let timeout_ms = if fixture.tags.contains(&"embeddings".to_string()) {
        "600000"
    } else if is_slow_grammar(&fixture.input) {
        "90000"
    } else {
        "30000"
    };

    // For NAPI (Node.js) trait bridge tests, generate cleanup to dispose bridges
    let bridge_cleanup = if lang == "node" && has_trait_bridge {
        extract_bridge_cleanup(&setup_lines)
    } else {
        String::new()
    };

    // ~keep The `expects_error` branch of `typescript/test_function.jinja` renders the
    // `rejects` assertion and nothing else, so every other assertion on an error fixture — most
    // often an `equals` against `error.status_code` — used to leave no trace at all in the
    // generated test.
    let unrenderable_error_assertions = crate::e2e::codegen::error_path_assertions::render(fixture, "\t\t// ", lang);

    let ctx = minijinja::context! {
        test_name => test_name,
        description => description,
        async_kw => async_kw,
        client_setup => client_setup,
        setup_lines => setup_lines,
        call_expr => call_expr,
        has_usable_assertion => has_usable_assertion || is_streaming,
        void_not_error => void_not_error,
        call_is_async => call_is_async,
        result_var => ts_result_var,
        await_kw => await_kw,
        collect_snippet => collect_snippet,
        assertions_body => assertions_body,
        expects_error => expects_error,
        unrenderable_error_assertions => unrenderable_error_assertions.trim_end(),
        error_value_regex => error_value_regex,
        error_skip_line => error_skip_line,
        is_streaming_error_call => is_streaming_error_call,
        lang => lang,
        skip_reason => skip_reason,
        refusal_body => refusal_body.trim_end(),
        timeout_ms => timeout_ms,
        bridge_cleanup => bridge_cleanup,
    };
    let rendered = crate::e2e::template_env::render("typescript/test_function.jinja", ctx);
    out.push_str(&rendered);
}

/// When a fixture declares at least one assertion but the rendered body has no
/// executable statement — every field assertion resolved to a "skipped" comment —
/// inject a real assertion instead of leaving the test vacuous. `not_error`
/// already renders a real `expect(...).toBeDefined()` on its own (see
/// `render_assertion`'s `not_error` arm), so this only fires on the remaining
/// gap: declared field assertions that all turned out unavailable. Fixtures that
/// declare NO assertions at all are left untouched — a deliberate "just call it"
/// smoke test, matching every other backend in this defect class. ~keep
fn apply_vacuous_assertion_fallback(
    assertions_body: &mut String,
    has_declared_assertions: bool,
    is_streaming: bool,
    result_var: &str,
    streaming_drive_is_the_check: bool,
    returns_void: bool,
) {
    let has_real_assertion = assertions_body
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with("//"));
    if !has_declared_assertions || has_real_assertion {
        return;
    }
    if is_streaming {
        // ~keep `chunks` is bound to a freshly drained array immediately above, so
        // `expect(chunks).toBeDefined()` cannot fail — it is a vacuous guard, not a check, and it
        // is what kept a streaming example that asserts nothing looking green. It is kept only
        // where the drive itself IS the declared check (`not_error`). Everywhere else the body is
        // left comment-only so the refusal in `render_test_case` can see it.
        if streaming_drive_is_the_check {
            assertions_body.push_str("    expect(chunks).toBeDefined();\n");
        }
    } else if returns_void {
        // ~keep A void call's binding return is napi-rs's mapping of Rust `()` to JS
        // `undefined`, so `expect(result).toBeDefined()` would fail every successful call, not
        // just an unsuccessful one. `render_test_case`'s `void_not_error` flag already wraps the
        // call itself in `expect(...).resolves.not.toThrow()` when `not_error` is declared;
        // there is nothing else to assert here for the void case.
    } else {
        assertions_body.push_str(&format!("    expect({result_var}).toBeDefined();\n"));
    }
}

/// Check if a grammar has slow load times and needs extended timeout.
/// Tree-sitter grammars with complex scanner.c or large parser.c files
/// may take significantly longer to load and parse on first invocation.
fn is_slow_grammar(input: &serde_json::Value) -> bool {
    // Extract language from nested input.config.language
    let language = input
        .get("config")
        .and_then(|config| config.get("language"))
        .and_then(|lang| lang.as_str());

    // Grammars with slow parse times: known slow compilation or heavy scanner logic
    const SLOW_GRAMMARS: &[&str] = &["earthfile", "perl", "vb"];

    language.is_some_and(|lang| SLOW_GRAMMARS.contains(&lang))
}

#[cfg(test)]
mod void_not_error_tests {
    use super::*;
    use crate::e2e::config::CallConfig;
    use crate::e2e::fixture::Assertion;

    /// Regression coverage for the void `not_error` defect: before this fix, a fixture whose
    /// only assertion was `not_error` on a `returns_void` call rendered `expect(result)
    /// .toBeDefined()` — but napi-rs maps a void call's `Ok(())` to JS `undefined`, so that
    /// assertion FAILED every successful call, not just an unsuccessful one. Worse than the
    /// vacuous body it replaced.
    fn void_fixture(id: &str, assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    fn render_void_call(assertions: Vec<Assertion>) -> String {
        let fixture = void_fixture("prefetch_languages", assertions);
        let call = CallConfig {
            function: "prefetchLanguages".to_string(),
            module: "myLib".to_string(),
            result_var: "result".to_string(),
            returns_void: true,
            ..Default::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..Default::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<TypeDef> = Vec::new();
        let enums: Vec<EnumDef> = Vec::new();
        let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
        let mut referenced_enums = std::collections::BTreeSet::new();

        let mut out = String::new();
        render_test_case(
            &mut out,
            &fixture,
            None,
            None,
            &e2e_config,
            "node",
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &type_defs,
            &enums,
            &[],
            "",
            &config,
            &mut referenced_enums,
            &errors,
        );
        out
    }

    /// The regression this test exists for: before this earlier fix, a void `not_error`-only
    /// fixture rendered `const result = await prefetchLanguages(); expect(result).toBeDefined();`
    /// — an assertion that fails on every successful call, since a void call resolves `undefined`.
    /// `CallConfig::default()` is synchronous, so the wrapper shape asserted here is the sync one
    /// (`expect(() => ...)`, no Promise); see `void_not_error_call_tests.rs` for the sibling
    /// async-shape coverage and the sync/async selection defect this split guards against.
    #[test]
    fn void_not_error_wraps_the_call_without_asserting_tobedefined() {
        let out = render_void_call(vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }]);

        assert!(
            out.contains("expect(() => prefetchLanguages()).not.toThrow();"),
            "expected the sync void call wrapped in expect(() => ...).not.toThrow(), got:\n{out}"
        );
        assert!(
            !out.contains("toBeDefined()"),
            "must not assert toBeDefined() on a void call's always-undefined result, got:\n{out}"
        );
    }

    /// A void `not_error` fixture over a *synchronous* call renders its wrapper expression
    /// (`expect(() => ...).not.toThrow()`, see `void_not_error_call_tests.rs`) inside the `it(..)`
    /// callback whose `async` keyword is frozen into `async_kw` by `test_is_async` two lines above
    /// `void_not_error`'s use site. `test_is_async` forces `async` whenever `void_not_error` is set
    /// (regardless of `call_is_async`) so that other backends' analogous async-body content, or a
    /// future template change, can never reintroduce an `await` stranded in a non-async callback —
    /// that would not be a weak assertion, it would be a TS/JS syntax error, and it aborted the
    /// entire formatting phase of `alef all` (taking every other language's formatting with it)
    /// rather than failing one test. `CallConfig::default()` is synchronous, which is exactly the
    /// shape that reproduces it.
    #[test]
    fn sync_void_not_error_marks_the_test_callback_async() {
        let out = render_void_call(vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }]);

        assert!(
            out.contains("async () => {"),
            "a body containing `await` must live in an async callback, got:\n{out}"
        );
        let awaits_in_body = out.contains("await ");
        let callback_is_async = out.contains("async () => {");
        assert!(
            !awaits_in_body || callback_is_async,
            "`await` outside an async function is a syntax error, got:\n{out}"
        );
    }

    /// A void fixture with no `not_error` assertion at all must keep emitting a bare call —
    /// wrapping every void call regardless of what it asserts would be a different, unrequested
    /// behavior change.
    #[test]
    fn void_call_without_not_error_stays_a_bare_statement() {
        let out = render_void_call(vec![]);

        assert!(
            out.contains("prefetchLanguages();"),
            "expected a bare call statement, got:\n{out}"
        );
        assert!(!out.contains("resolves.not.toThrow"), "got:\n{out}");
    }
}

#[cfg(test)]
mod ir_async_tests {
    use super::*;
    use crate::e2e::config::CallConfig;
    use crate::e2e::fixture::Assertion;

    /// Regression coverage: `alef.toml`'s `[call] async` is a plain `bool` defaulting to
    /// `false`, so a fixture whose config never set it is indistinguishable from one that
    /// explicitly opted out. A Rust function that became `async fn` after the fixture was
    /// authored left the config stale, and the generated test called it without `await` —
    /// a `Promise` object compared against the expected value instead of its resolved
    /// value. The core IR's own `is_async` (populated regardless of what `alef.toml` says)
    /// must be consulted when the config does not override the answer.
    #[test]
    fn ir_async_function_gets_await_even_when_config_omits_async() {
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "detect_widget_smoke".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Assertion::default()
            }],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };
        let call = CallConfig {
            function: "detect_widget".to_string(),
            module: "myLib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            // Deliberately NOT setting `r#async: true` -- the IR alone must supply it.
            ..Default::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..Default::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();
        let functions = vec![crate::core::ir::FunctionDef {
            name: "detect_widget".to_string(),
            is_async: true,
            ..Default::default()
        }];
        let type_defs: Vec<TypeDef> = Vec::new();
        let enums: Vec<EnumDef> = Vec::new();
        let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
        let mut referenced_enums = std::collections::BTreeSet::new();

        let mut out = String::new();
        render_test_case(
            &mut out,
            &fixture,
            None,
            None,
            &e2e_config,
            "node",
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &type_defs,
            &enums,
            &functions,
            "",
            &config,
            &mut referenced_enums,
            &errors,
        );

        assert!(
            out.contains("await detectWidget("),
            "the IR declares detect_widget async; the call must be awaited, got:\n{out}"
        );
    }

    /// A per-language `async` override is trusted verbatim, even against a disagreeing IR:
    /// this is the escape hatch for a call the IR seam does not (or should not) resolve.
    #[test]
    fn explicit_async_override_wins_over_a_sync_ir_signature() {
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "detect_widget_smoke".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Assertion::default()
            }],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "node".to_string(),
            crate::core::config::e2e::CallOverride {
                r#async: Some(true),
                ..Default::default()
            },
        );
        let call = CallConfig {
            function: "detect_widget".to_string(),
            module: "myLib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            overrides,
            ..Default::default()
        };
        let e2e_config = E2eConfig {
            call,
            ..Default::default()
        };
        let config = crate::core::config::ResolvedCrateConfig::default();
        // IR says sync; the override must still win.
        let functions = vec![crate::core::ir::FunctionDef {
            name: "detect_widget".to_string(),
            is_async: false,
            ..Default::default()
        }];
        let type_defs: Vec<TypeDef> = Vec::new();
        let enums: Vec<EnumDef> = Vec::new();
        let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
        let mut referenced_enums = std::collections::BTreeSet::new();

        let mut out = String::new();
        render_test_case(
            &mut out,
            &fixture,
            None,
            None,
            &e2e_config,
            "node",
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &type_defs,
            &enums,
            &functions,
            "",
            &config,
            &mut referenced_enums,
            &errors,
        );

        assert!(
            out.contains("await detectWidget("),
            "an explicit per-language async override must win over a sync IR signature, got:\n{out}"
        );
    }
}

/// Attach this fixture's call context to every refusal the argument builders just recorded.
///
/// `options_type` is the file-level `[e2e.call.overrides.<lang>].options_type` the caller
/// resolved once for the whole file; `per_call_override` is this call's own table. Which of the
/// two supplied the type is the actionable half of the diagnostic: a file-level default silently
/// applies to every call that does not override it, so "add a per-call override" and "change the
/// default" are opposite fixes and only one of them is right. ~keep
fn attribute_key_refusals(
    lang: &str,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call_config: &crate::core::config::e2e::CallConfig,
    per_call_override: Option<&crate::core::config::e2e::CallOverride>,
    options_type: Option<&str>,
) {
    use crate::e2e::codegen::fixture_refusal::{attribute, language_default_source, resolved_call_key};

    let source = language_default_source(
        per_call_override.and_then(|value| value.options_type.as_deref()),
        options_type,
    );
    attribute(lang, &fixture.id, resolved_call_key(e2e_config, call_config), source);
}
