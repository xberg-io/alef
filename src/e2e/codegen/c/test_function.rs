//! C e2e per-fixture test function rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::transform_json_keys_for_language;
use crate::e2e::escape::{escape_c, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::assertions::TargetParams;
use super::docs_input::render_c_docs_json;
use super::ffi_constructors::is_std_type_without_ffi_constructor;
use super::{
    FieldConfigSources, LeafFieldCheck, build_args_string_c, classify_nested_leaf, emit_nested_accessor,
    ensure_leaf_field_exists, infer_opaque_handle_type, ir_declares_a_json_string_param, is_primitive_c_type,
    is_skipped_c_field, json_to_c, render_assertion, render_bytes_test_function, render_c_diagnostic_skip,
    render_engine_factory_test_function, render_streaming_test_function, resolve_c_client_owner_type,
    resolve_c_streaming_adapter, resolve_optional_sentinel, try_emit_enum_accessor, validate_c_snippet_metadata,
};

/// Emit the C error-path epilogue every `expects_error` return site shares: the declared `error`
/// value comparison — or, when the value names a real error variant C cannot substantiate, the
/// registered skip instead — then a marker for each assertion this path does not render.
///
/// ~keep The C ABI carries a failure as `set_last_error(alef_ffi_error_code(&e), &e.to_string())`,
/// so `{prefix}_last_error_context()` is the failure's Display text and is the only textual
/// evidence C has. Before this, a fixture's declared `error` value was discarded outright and the
/// whole assertion collapsed into `assert(handle == 0)` — a check that cannot tell the expected
/// failure from any other. C can never satisfy the message-or-type-name disjunction's type-name
/// side (`declared_error_variant::classify` always answers `Unsubstantiable` for a known variant
/// here): the C ABI exposes a variant only as a numeric taxonomy code
/// (`{prefix}_last_error_code()`), never as a string the generated message check could compare.
pub(super) fn emit_c_error_epilogue(
    out: &mut String,
    prefix: &str,
    fixture: &Fixture,
    errors: &[crate::core::ir::ErrorDef],
    documentation_snippet: bool,
) {
    // ~keep A documentation snippet is published prose, not a test: a `// skipped:` comment reads
    // as a defect in the example and a message assert turns a runnable snippet into one that can
    // abort. Snippet output therefore stays byte-identical to before this change.
    if documentation_snippet {
        return;
    }
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    match classify("c", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => {}
        DeclaredErrorAssertion::Assert(declared) => {
            let expected = escape_c(declared);
            let _ = writeln!(out, "    {{");
            let _ = writeln!(out, "        const char* _err_message = {prefix}_last_error_context();");
            let _ = writeln!(
                out,
                "        assert(_err_message != NULL && \"expected an error message\");"
            );
            let _ = writeln!(
                out,
                "        assert(strstr(_err_message, \"{expected}\") != NULL && \"error message mismatch\");"
            );
            let _ = writeln!(out, "    }}");
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "{}", skip_line("    ", "//", variant, &fixture.id, "c"));
        }
    }
    crate::e2e::codegen::error_path_assertions::emit(out, fixture, "    // ", "c");
}

/// Snippet-local definition of the `ALEF_TEST_SKIP` guard macro.
///
/// The generated e2e runner declares this macro in its `test_runner.h`, but a
/// documentation snippet is a standalone translation unit that includes only the
/// crate's FFI header, so it must carry the definition itself. The body differs
/// from the runner's on purpose: a snippet's guard sits in `int main(void)`, where
/// the runner's bare `return;` — valid in its `void test_*(void)` functions — is
/// not. The `#ifndef` keeps an enclosing definition authoritative if a snippet is
/// ever embedded in a unit that already has one.
const SNIPPET_TEST_SKIP_MACRO: &str = concat!(
    "#ifndef ALEF_TEST_SKIP\n",
    "#define ALEF_TEST_SKIP(reason) do { \\\n",
    "    fprintf(stderr, \"skipped: %s\\n\", (reason)); \\\n",
    "    return EXIT_SUCCESS; \\\n",
    "} while (0)\n",
    "#endif",
);

const SNIPPET_TEST_PASS_MACRO: &str = concat!(
    "#ifndef ALEF_TEST_PASS\n",
    "#define ALEF_TEST_PASS() do { return EXIT_SUCCESS; } while (0)\n",
    "#endif",
);

/// File-scope declarations the emitted snippet needs so that every symbol its
/// body references resolves inside the emitted translation unit.
fn snippet_declarations(body: &str) -> String {
    let mut declarations = Vec::new();
    if body.contains("ALEF_TEST_SKIP(") {
        declarations.push(SNIPPET_TEST_SKIP_MACRO);
    }
    if body.contains("ALEF_TEST_PASS()") {
        declarations.push(SNIPPET_TEST_PASS_MACRO);
    }
    declarations.join("\n")
}

fn is_expected_result_assertion(line: &str, result_var: &str) -> bool {
    line.trim_start().starts_with("assert(") && line.contains(result_var) && line.contains("expected call to fail")
}

/// The variable a C declaration statement introduces, if the line is one.
///
/// Recognises the declarator shapes the C emitters produce — `TYPE name`, `TYPE* name`,
/// `const TYPE *name` — and rejects anything whose left-hand side is a condition, a call or a
/// bare re-assignment, so `assert(result == 0 …)` and `if (result != 0) …` never read as
/// declarations of `result`. ~keep
fn declared_variable(line: &str) -> Option<&str> {
    let (declarator, _) = line.split_once('=')?;
    let declarator = declarator.trim();
    if declarator.contains(['(', ')', '{', '}', '!', '<', '>', ',']) || declarator.ends_with(['*', '=']) {
        return None;
    }
    if declarator.split_whitespace().count() < 2 {
        return None;
    }
    declarator
        .rsplit(|character: char| character.is_whitespace() || character == '*')
        .next()
}

/// Assemble a documentation snippet body from the rendered e2e test function.
///
/// An `expects_error` fixture's `expected call to fail` assertion becomes a
/// `return EXIT_FAILURE` guard naming `result_var`. That guard is only legal C once the call
/// declaring `result_var` has been emitted, so the walk carries the declaration state and pairs
/// the rewrite with it: a matching assertion reached before the declaration is an upstream
/// emitter defect and is reported here rather than published as use-before-declaration C, which
/// no snippet-level test could catch because the translation unit would not compile at all. ~keep
///
/// `unexpected_success_condition` is the condition under which the call did NOT fail as the
/// fixture expects, and it is passed in rather than derived because it depends on the return
/// shape the emitter chose: a null-on-failure handle has not failed while it is non-zero, an
/// `i32` status export has not failed while it is zero. Hardcoding one polarity here published a
/// guard that returned `EXIT_FAILURE` on precisely the outcome the fixture was asserting. ~keep
fn assemble_snippet_body(
    function: &str,
    result_var: &str,
    expects_error: bool,
    fixture_id: &str,
    unexpected_success_condition: &str,
) -> anyhow::Result<String> {
    let failure_check = format!("if ({unexpected_success_condition}) {{ return EXIT_FAILURE; }}");
    let body_line_count = function.lines().count().saturating_sub(3);
    let mut result_declared = false;
    let mut lines: Vec<String> = Vec::new();
    for raw_line in function.lines().skip(2).take(body_line_count) {
        if !expects_error && raw_line.trim_start().starts_with("assert(") {
            continue;
        }
        let line = raw_line.strip_prefix("    ").unwrap_or(raw_line);
        if declared_variable(line) == Some(result_var) {
            result_declared = true;
        } else if expects_error && is_expected_result_assertion(line, result_var) {
            if !result_declared {
                anyhow::bail!(
                    "C e2e generator: fixture `{fixture_id}` would emit the `{result_var}` failure guard before \
                     `{result_var}` is declared; the call producing `{result_var}` must be emitted first"
                );
            }
            lines.push(failure_check.clone());
            continue;
        }
        lines.push(line.to_string());
    }
    Ok(lines.join("\n"))
}

pub(super) struct SnippetContext<'a> {
    pub fixture: &'a Fixture,
    pub e2e_config: &'a crate::e2e::config::E2eConfig,
    pub header: &'a str,
    pub prefix: &'a str,
    pub info: &'a super::ResolvedCallInfo,
    pub field_resolver: &'a FieldResolver,
    pub config: &'a ResolvedCrateConfig,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    /// The core IR the visitor arm re-resolves its own call info from. It cannot reuse
    /// `info`: `render_visitor_snippet` renders a whole file and resolves per-fixture. ~keep
    pub ir: super::CallIr<'a>,
}

pub(super) fn render_snippet_body(context: SnippetContext<'_>) -> anyhow::Result<String> {
    let SnippetContext {
        fixture,
        e2e_config,
        header,
        prefix,
        info,
        field_resolver,
        config,
        type_defs,
        ir,
    } = context;
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if fixture.visitor.is_some() {
        return super::visitor::render_visitor_snippet(fixture, header, prefix, e2e_config, config, ir);
    }
    if fixture
        .resolved_args(call)
        .iter()
        .any(|argument| argument.arg_type == "test_backend")
    {
        return super::trait_bridge_snippet::render(fixture, header, prefix, config, type_defs);
    }
    // What the core IR says about `call`'s target parameters, if anything -- shared by
    // both the void-call fallback below and `render_test_function_impl`'s legacy-call
    // path, since both eventually ask `build_args_string_c` to render an `args` list
    // that might be empty. `CallIr::signature` matches the same `core_lookup_name` key
    // `resolve_call_info` uses to resolve `info.result_type_name`, so this asks the
    // identical question the result-type resolution already asks of the IR -- just for
    // parameters instead of a return type. This is only possible here because
    // `render_snippet_body` has a `CallIr` in scope; the main e2e test-file emitter in
    // `c.rs` calls the `render_test_function` back-compat shim directly and has no such
    // IR to resolve, so it always renders `TargetParams::IrAbsent`. ~keep
    // A trait-bridge registry call (`register_fn` / `unregister_fn` / `clear_fn`) is a generated
    // FFI export, never a core IR function, so it is permanently unresolvable against the IR and
    // would refuse forever on the argument axis exactly as it did on the result-type axis. Its
    // shared `[crates.e2e.calls.*].args` list is legitimately empty: the only parameter its C
    // export takes beyond the configured ones is the trailing `out_error`, which `extra_args`
    // appends because the language-agnostic config cannot express it. So the shared arg list is
    // genuinely zero-length, which is `Known(&[])` -- not an authoring gap. ~keep
    let target_params = if crate::e2e::codegen::recipe::trait_bridge_derived_c_identity(config, fixture).is_some() {
        TargetParams::Known(&[])
    } else {
        TargetParams::resolve(call, "c", ir)
    };
    // Computed before the `returns_void` branch (not just the non-void path below it) so a
    // void-configured fixture's `error`/`not_error` declaration is not silently discarded --
    // see `void_call_status`'s module doc for why a `returns_void` call can still carry a
    // real status to check. ~keep
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if info.returns_void {
        // `{prefix_upper}AlefHandle` must be spelled the way the header declares it -- see
        // `render_test_function_impl`'s identical lookup for why this can't be a bare
        // uppercase. Needed here (not just there) because a `json_object` arg onto a
        // handle parameter constructs that handle via `from_json` before this call, same as
        // every other C call path. ~keep
        let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);
        // Build the handle(s) any `json_object` arg needs before the call that consumes
        // them -- exactly what the free-function path in `render_test_function_impl` does.
        // Before this, this branch passed `build_args_string_c` an empty handle map, so a
        // `json_object` arg matching a handle-typed parameter fell through to a raw JSON
        // string literal (`json_to_c`'s fallback) instead of a constructed `AlefHandle`,
        // which does not compile against the exported header. ~keep
        let mut setup = String::new();
        let (typed_arg_handles, typed_arg_cleanup) = build_json_object_arg_handles(
            &mut setup,
            fixture,
            prefix,
            &prefix_upper,
            &info.args,
            &info.options_type_name,
            true,
            target_params,
        );
        // The shared, language-agnostic `[crates.e2e.calls.*]` args config has no
        // concept of the C-only trailing `out_error` out-param that trait-bridge
        // `unregister`/`clear` exports always take; `extra_args` is where the C
        // codegen appends parameters the shared config can't express (see its
        // definition in `c.rs`), so it must be joined in after the configured args
        // rather than dropped on this void-call path.
        let mut arg_parts: Vec<String> = if info.args.is_empty() {
            Vec::new()
        } else {
            vec![build_args_string_c(
                &fixture.input,
                &info.args,
                &typed_arg_handles,
                config,
                type_defs,
                fixture,
                &info.function_name,
                target_params,
            )?]
        };
        arg_parts.extend(info.extra_args.iter().cloned());
        let args = arg_parts.join(", ");
        // `call`'s Rust-side signature, not `info.function_name` (already rewritten to the ABI
        // export name above): a `Result<(), E>` free function/method is fallible at the C ABI
        // even though this fixture is configured `returns_void`, and discarding that status is
        // the defect `void_call_status` exists to close. ~keep
        let is_fallible_void = super::void_call_status::is_fallible(call, ir);
        let call_line = super::void_call_status::render_call_line(
            &info.function_name,
            &args,
            call.effective_result_var(),
            expects_error,
            is_fallible_void,
        );
        let mut cleanup = String::new();
        render_typed_arg_cleanup(&mut cleanup, prefix, &typed_arg_cleanup);
        // `build_json_object_arg_handles`/`render_typed_arg_cleanup` pre-indent their lines
        // four spaces for the `void test_{fn}(void) { ... }` wrapper the free-function path
        // writes them into; `c/snippet_body.jinja` applies that same four-space indent once
        // more via `indent(4, true)`, so strip the baked-in indent here rather than stacking
        // the two. ~keep
        let dedent = |text: &str| -> String {
            text.lines()
                .map(|line| line.strip_prefix("    ").unwrap_or(line))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut body = dedent(&setup);
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(call_line.trim_end());
        let cleanup = dedent(&cleanup);
        if !cleanup.is_empty() {
            body.push('\n');
            body.push_str(&cleanup);
        }
        return Ok(crate::e2e::template_env::render(
            "c/snippet_body.jinja",
            minijinja::context! { header => header, declarations => "", body => body.trim_end() },
        ));
    }
    validate_c_snippet_metadata(
        config,
        type_defs,
        fixture,
        &info.function_name,
        info.client_factory.as_deref(),
        info.c_engine_factory.as_deref(),
        info.streaming,
    )?;
    let result_var = call.effective_result_var();
    let mut call_fixture = fixture.clone();
    if !expects_error {
        call_fixture.assertions.clear();
    }
    // Same shadowing rule as the main test-file emitter (`render_test_file` in `c.rs`):
    // a nested-field diagnostic raised while rendering this snippet must name whichever
    // `result_fields`/`fields` key actually governs `call`, not always the global
    // default. ~keep
    let config_sources = FieldConfigSources::resolve(e2e_config, call);
    let mut function = String::new();
    render_test_function_impl(
        &mut function,
        &call_fixture,
        prefix,
        &info.function_name,
        result_var,
        &info.args,
        field_resolver,
        e2e_config.effective_fields_c_types(call),
        e2e_config.effective_fields_enum(call),
        &info.result_type_name,
        &info.options_type_name,
        info.client_factory.as_deref(),
        info.raw_c_result_type.as_deref(),
        info.c_free_fn.as_deref(),
        info.c_engine_factory.as_deref(),
        info.result_is_option,
        info.result_is_bytes,
        info.streaming,
        &info.extra_args,
        config,
        type_defs,
        &[],
        true,
        &config_sources,
        target_params,
    )?;
    // A status-code export reports failure as a NON-zero return, so the outcome that
    // contradicts an `expects_error` fixture is `== 0`; every handle/pointer shape reports
    // failure as null, so theirs is `!= 0`. ~keep
    let unexpected_success_condition = if info.result_type_name.returns_status_code() {
        format!("{result_var} == 0")
    } else {
        format!("{result_var} != 0")
    };
    let body = assemble_snippet_body(
        &function,
        result_var,
        expects_error,
        &fixture.id,
        &unexpected_success_condition,
    )?;
    Ok(crate::e2e::template_env::render(
        "c/snippet_body.jinja",
        minijinja::context! { header => header, declarations => snippet_declarations(&body), body => body },
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_function_impl(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    field_resolver: &FieldResolver,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    // `result_type_name` is the *outcome* of resolution, not a name: the branches below that
    // never spell a result type into a symbol (streaming adapters, the `raw_c_result_type`
    // scalar path) must keep rendering for a call the IR cannot name, while every branch that
    // does spell one asks through `ResultTypeName::require` and fails there rather than
    // inventing it. ~keep
    result_type_name: &super::ResultTypeName,
    options_type_name: &str,
    client_factory: Option<&str>,
    raw_c_result_type: Option<&str>,
    c_free_fn: Option<&str>,
    c_engine_factory: Option<&str>,
    result_is_option: bool,
    result_is_bytes: bool,
    streaming: Option<bool>,
    extra_args: &[String],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    errors: &[crate::core::ir::ErrorDef],
    documentation_snippet: bool,
    config_sources: &FieldConfigSources,
    // What the core IR says about the target's parameters, for the status-code and legacy
    // (non-client) call paths below -- see `TargetParams` and `build_args_string_c`'s doc
    // comment. Every other branch in this function (`c_engine_factory`, streaming, bytes,
    // `client_factory`) builds its own argument list without going through
    // `build_args_string_c` at all, so this is inert on those paths. ~keep
    target_params: TargetParams<'_>,
) -> anyhow::Result<()> {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let _ = writeln!(out, "void test_{fn_name}(void) {{");
    let _ = writeln!(out, "    /* {description} */");

    // Smoke/live fixtures gated on a required env var (e.g. OPENAI_API_KEY).
    // When the var is missing, treat as a successful skip — mirrors Python's
    // `pytest.skip("OPENAI_API_KEY not set")` and Java's `Assumptions.assumeTrue(...)`
    // so CI runs without provider credentials don't fail every smoke test.
    //
    // When the fixture also has a mock_response/http block, we support an env+mock
    // fallback: if the API key is set, use the real API; otherwise fall back to the
    // mock server. This lets the same fixture exercise both paths.
    let has_mock = fixture.needs_mock_server() && !documentation_snippet;
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    if documentation_snippet && client_factory.is_some() {
        let variable = crate::e2e::fixture::FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
        let _ = writeln!(out, "    const char* api_key = getenv(\"{variable}\");");
        let _ = writeln!(out, "    assert(api_key != NULL && \"{variable} must be set\");");
    }
    if let Some(env) = &fixture.env
        && let Some(var) = &env.api_key_var
    {
        let fixture_id = &fixture.id;
        if has_mock {
            let _ = writeln!(out, "    const char* api_key = getenv(\"{var}\");");
            let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
            let _ = writeln!(out, "    char base_url_buf[512];");
            let _ = writeln!(out, "    int use_mock = !(api_key && api_key[0] != '\\0');");
            let _ = writeln!(out, "    if (!use_mock) {{");
            let _ = writeln!(
                out,
                "        fprintf(stderr, \"{fixture_id}: using real API ({var} is set)\\n\");"
            );
            let _ = writeln!(out, "    }} else {{");
            let _ = writeln!(
                out,
                "        fprintf(stderr, \"{fixture_id}: using mock server ({var} not set)\\n\");"
            );
            let _ = writeln!(
                out,
                "        snprintf(base_url_buf, sizeof(base_url_buf), \"%s/fixtures/{fixture_id}\", mock_base ? mock_base : \"\");"
            );
            let _ = writeln!(out, "        api_key = \"test-key\";");
            let _ = writeln!(out, "    }}");
        } else {
            out.push_str(&crate::e2e::template_env::render(
                "c/test_skip_if_env_missing.jinja",
                minijinja::context! { env_var => var },
            ));
        }
    }

    // `{prefix_upper}AlefHandle` must be spelled the way the header declares it, and cbindgen's
    // `[export] prefix` is shouty-snake rather than a bare uppercase (they differ for any prefix
    // with an internal word boundary). Route through the helper the header producer uses. ~keep
    let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);

    // Status-code shape, decided BEFORE every other branch because every other branch
    // presupposes a return shape rather than establishing one. A trait-bridge registry export
    // (`register_fn` / `unregister_fn` / `clear_fn`) is emitted by alef's own FFI templates as
    // `pub unsafe extern "C" fn ...(..) -> i32`, `0` for success -- so there is no result to
    // own, nothing to free, and the value that reaches the caller is a status, not a pointer.
    //
    // Ordering this first is the whole fix. Reached later, a call that inherited a default
    // `client_factory` would already have been rewritten into a client method, and the
    // fallthrough would have bound an `i32` to `{prefix_upper}AlefHandle` and passed it to
    // `{prefix}_{result}_free`, which frees an alef `Box`: freeing a status integer as a heap
    // handle, in code alef publishes as a documentation example. ~keep
    if result_type_name.returns_status_code() {
        // An `i32` has no fields, so a field assertion on one cannot be emitted -- and a branch
        // that quietly dropped it would render a test that passes without checking the thing the
        // fixture names, which is worse than the crash it replaced. Refuse instead. ~keep
        if let Some(field) = fixture
            .assertions
            .iter()
            .filter_map(|assertion| assertion.field.as_deref())
            .find(|field| !field.is_empty())
        {
            anyhow::bail!(
                "C e2e generator: fixture `{}` asserts on field `{field}` of `{function_name}`, a \
                 trait-bridge registry export returning an `i32` status code. Assert on the status \
                 instead (`not_error` / `error`), or point the fixture at the call that returns the \
                 value being checked.",
                fixture.id
            );
        }
        // No registry export (`register_fn` / `unregister_fn` / `clear_fn`) currently declares a
        // `json_object`-typed parameter -- their shared `args` config is a plain `name` string
        // plus the C-only `out_error` `extra_args` appends -- but nothing about this branch
        // enforces that, and it shares `build_args_string_c` with every path that does take one.
        // Building any handle a `json_object` arg needs before the call keeps this branch from
        // being the next place a raw JSON literal reaches an `AlefHandle` parameter, exactly the
        // `returns_void` path above until it did the same. ~keep
        let (typed_arg_handles, typed_arg_cleanup) = build_json_object_arg_handles(
            out,
            fixture,
            prefix,
            &prefix_upper,
            args,
            options_type_name,
            documentation_snippet,
            target_params,
        );
        // Argument construction mirrors the void-call path in `render_snippet_body`, for the
        // same reason: with no configured `args` the call's argument list is genuinely empty
        // (`TargetParams::Known(&[])`), so `extra_args` -- which carries the mandatory trailing
        // `out_error` these exports declare -- is the whole list. Routing an empty list through
        // `build_args_string_c` would instead splice the fixture `input` JSON in as an argument
        // on any `TargetParams::IrAbsent` caller. ~keep
        let mut arg_parts: Vec<String> = if args.is_empty() {
            Vec::new()
        } else {
            vec![build_args_string_c(
                &fixture.input,
                args,
                &typed_arg_handles,
                config,
                type_defs,
                fixture,
                function_name,
                target_params,
            )?]
        };
        arg_parts.extend(extra_args.iter().cloned());
        let args_str = arg_parts.join(", ");
        let _ = writeln!(out, "    int32_t {result_var} = {function_name}({args_str});");
        if expects_error {
            let _ = writeln!(out, "    assert({result_var} != 0 && \"expected call to fail\");");
            emit_c_error_epilogue(out, prefix, fixture, errors, documentation_snippet);
        } else {
            let _ = writeln!(out, "    assert({result_var} == 0 && \"expected call to succeed\");");
        }
        render_typed_arg_cleanup(out, prefix, &typed_arg_cleanup);
        let _ = writeln!(out, "}}");
        return Ok(());
    }

    // Engine-factory pattern: used when c_engine_factory is configured.
    // Creates a config handle from JSON, builds an engine, calls {prefix}_{function}(engine, url),
    // frees result and engine.
    if let Some(config_type) = c_engine_factory {
        render_engine_factory_test_function(
            out,
            fixture,
            prefix,
            function_name,
            result_var,
            field_resolver,
            fields_c_types,
            fields_enum,
            // `require_owned_handle`, not `require`: this pattern ends in
            // `{prefix}_{result_snake}_free({result_var})` (`call_patterns.rs`), so it takes
            // ownership of whatever the call returned. ~keep
            result_type_name.require_owned_handle()?,
            config_type,
            expects_error,
            raw_c_result_type,
            type_defs,
            config_sources,
        )?;
        return Ok(());
    }

    // Streaming adapters use an FFI iterator handle instead of a single
    // response. Emit start/next/free loop and aggregate per-chunk data
    // into local vars (chunks_count, stream_content, stream_complete) so fixture
    // assertions on pseudo-fields resolve to those locals rather than to
    // non-existent accessor functions on a single chunk handle.
    if client_factory.is_some() && crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, streaming) {
        let Some(streaming) = resolve_c_streaming_adapter(config, function_name) else {
            render_c_diagnostic_skip(
                out,
                "streaming fixture requires matching [[crates.adapters]] metadata for C e2e codegen",
            );
            return Ok(());
        };
        render_streaming_test_function(
            out,
            fixture,
            prefix,
            result_var,
            args,
            client_factory.unwrap_or(""),
            &streaming,
            expects_error,
            api_key_var,
            documentation_snippet,
        );
        return Ok(());
    }

    // Byte-buffer pattern: methods like `speech` and `file_content` return raw
    // bytes via the out-pointer FFI shape:
    //   `int32_t fn(this, req, uint8_t** out_ptr, uintptr_t* out_len, uintptr_t* out_cap)`
    // rather than as an opaque `*Response` handle. The C codegen must declare
    // the out-params, check the int32_t status code, and free with
    // `<prefix>_free_bytes` rather than emitting non-existent
    // `<prefix>_<response>_audio` / `_content` accessors.
    if let Some(factory) = client_factory
        && result_is_bytes
    {
        let Some(client_owner_type) = resolve_c_client_owner_type(config, type_defs, function_name) else {
            render_c_diagnostic_skip(
                out,
                "client_factory is configured but C e2e could not resolve the client owner type",
            );
            return Ok(());
        };
        render_bytes_test_function(
            out,
            fixture,
            prefix,
            function_name,
            result_var,
            args,
            options_type_name,
            result_type_name.require()?,
            factory,
            &client_owner_type,
            expects_error,
            errors,
            documentation_snippet,
        );
        return Ok(());
    }

    // Client pattern: used when client_factory is configured.
    // Builds typed request handles from json_object args, creates a client via the
    // factory function, calls {prefix}_default_client_{function_name}(client, req),
    // then frees result, request handles, and client.
    if let Some(factory) = client_factory {
        let Some(_client_owner_type) = resolve_c_client_owner_type(config, type_defs, function_name) else {
            render_c_diagnostic_skip(
                out,
                "client_factory is configured but C e2e could not resolve the client owner type",
            );
            return Ok(());
        };
        // This branch spells the result type into the request-type derivation, every field
        // accessor, and the trailing `_free`; resolve it once, up front, so an unnameable
        // result stops the branch before any of the three is written. `require_owned_handle`
        // rather than `require` because of that trailing `_free`: a call whose config declares
        // its result is a bare scalar or a raw byte buffer has said the result is not a handle,
        // and freeing it as one is a heap bug, not a naming slip. ~keep
        let result_type_name = result_type_name.require_owned_handle()?;
        let mut request_handle_vars: Vec<(String, String)> = Vec::new(); // (arg_name, var_name)
        // Inline argument expressions appended after request handles in the
        // method call (e.g. literal C strings for `string` args, `NULL` for
        // optional pointer args). Order matches the position in `args`.
        let mut inline_method_args: Vec<String> = Vec::new();
        // ~keep IR-checked, not just `arg_type` -- see `c::optional_arg`'s module doc.
        let sentinel_for = |arg: &crate::e2e::config::ArgMapping, index: usize| {
            resolve_optional_sentinel(target_params, &arg.name, index, &arg.arg_type).to_string()
        };

        for (index, arg) in args.iter().enumerate() {
            if arg.arg_type == "json_object" {
                // Prefer options_type from the C override when set, since the result
                // type isn't always a clean strip-Response/append-Request transform
                // (e.g. transcribe -> Create**Transcription**Request, not TranscriptionRequest).
                // Fall back to deriving from result_type for backward-compat cases.
                let request_type_pascal = if !options_type_name.is_empty() {
                    options_type_name.to_string()
                } else if let Some(stripped) = result_type_name.strip_suffix("Response") {
                    format!("{}Request", stripped)
                } else {
                    format!("{result_type_name}Request")
                };
                let request_type_snake = request_type_pascal.to_snake_case();
                let var_name = format!("{request_type_snake}_handle");

                let json_val = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);

                if !json_val.is_null() {
                    let val = json_val;
                    let normalized = transform_json_keys_for_language(val, "snake_case");
                    let (docs_setup, json_expr, docs_cleanup) = render_c_docs_json(
                        &arg.name,
                        &normalized,
                        &fixture.docs_files_for_arg(&arg.field),
                        documentation_snippet,
                    );
                    out.push_str(&docs_setup);
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}AlefHandle {var_name} = \
                             {prefix}_{request_type_snake}_from_json({json_expr});"
                    );
                    out.push_str(&docs_cleanup);
                    if expects_error {
                        // For error fixtures (e.g. invalid enum value rejected by
                        // serde), `_from_json` may legitimately return NULL — that
                        // counts as the expected failure. Mirror Java's pattern of
                        // wrapping setup + call inside `assertThrows(...)` so error
                        // fixtures pass at *any* failure step. The test returns
                        // before attempting to create a client, leaving no
                        // resources to free.
                        out.push_str(&crate::e2e::template_env::render(
                            "c/test_pass_if_null.jinja",
                            minijinja::context! { variable => var_name },
                        ));
                    } else {
                        let _ = writeln!(out, "    assert({var_name} != 0 && \"failed to build request\");");
                    }
                    request_handle_vars.push((arg.name.clone(), var_name));
                }
            // ~keep A `mock_url` arg joins the string branch ONLY when the fixture opted
            // into verbatim URLs and a string actually resolved. Admitting every
            // `mock_url` here would change the default path, which previously never
            // reached this branch at all — the fallthrough emitted a mock-server
            // identifier that is not declared on every code path through this emitter.
            } else if arg.arg_type == "string"
                || (arg.arg_type == "mock_url"
                    && crate::e2e::codegen::preserved_url_literal(
                        fixture.preserve_input_urls,
                        crate::e2e::codegen::resolve_field(&fixture.input, &arg.field),
                    )
                    .is_some())
            {
                // String arg: read fixture input, emit as a C string literal inline.
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = fixture.input.get(field);
                match val {
                    Some(v) if v.is_string() => {
                        let s = v.as_str().unwrap_or_default();
                        let escaped = escape_c(s);
                        inline_method_args.push(format!("\"{escaped}\""));
                    }
                    Some(serde_json::Value::Null) | None if arg.optional => {
                        inline_method_args.push(sentinel_for(arg, index));
                    }
                    None => {
                        inline_method_args.push("\"\"".to_string());
                    }
                    Some(other) => {
                        let s = serde_json::to_string(other).unwrap_or_default();
                        let escaped = escape_c(&s);
                        inline_method_args.push(format!("\"{escaped}\""));
                    }
                }
            } else if arg.optional {
                // ~keep Optional non-string, non-json_object arg: pass the type-appropriate
                // "none" sentinel — `0` for a scalar `AlefHandle`, `NULL` for a real pointer.
                inline_method_args.push(sentinel_for(arg, index));
            }
        }

        let fixture_id = &fixture.id;
        // Pass UINT64_MAX/UINT32_MAX (≡ -1ULL/-1U) as the FFI's None sentinel for
        // optional numeric primitives — passing literal 0 makes the binding see
        // Some(0), which Rust core treats as `Duration::from_secs(0)` (immediate
        // request deadline) and breaks every HTTP fixture.
        if has_mock && api_key_var.is_some() {
            // api_key and base_url_buf are already declared in the env-fallback block above.
            // use_mock was captured before api_key was potentially reassigned to "test-key",
            // so it correctly reflects the original env state.
            let _ = writeln!(out, "    const char* _base_url_arg = use_mock ? base_url_buf : NULL;");
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle client = {prefix}_{factory}(api_key, _base_url_arg, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        } else if has_mock {
            let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
            let _ = writeln!(out, "    assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");");
            let _ = writeln!(out, "    char base_url[1024];");
            let _ = writeln!(
                out,
                "    snprintf(base_url, sizeof(base_url), \"%s/fixtures/{fixture_id}\", mock_base);"
            );
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        } else if documentation_snippet {
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle client = {prefix}_{factory}(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        } else {
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        }
        let _ = writeln!(out, "    assert(client != 0 && \"failed to create client\");");

        let method_args = if request_handle_vars.is_empty() && inline_method_args.is_empty() && extra_args.is_empty() {
            String::new()
        } else {
            let handles: Vec<String> = request_handle_vars.iter().map(|(_, v)| v.clone()).collect();
            let parts: Vec<String> = handles
                .into_iter()
                .chain(inline_method_args.iter().cloned())
                .chain(extra_args.iter().cloned())
                .collect();
            format!(", {}", parts.join(", "))
        };

        let call_fn = format!("{prefix}_default_client_{function_name}");

        if expects_error {
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle {result_var} = {call_fn}(client{method_args});"
            );
            // ~keep The failure assert and the epilogue both run BEFORE the frees:
            // `{prefix}_last_error_context()` borrows thread-local storage that the very next FFI
            // call clears (`catch_ffi_panic` starts with `clear_last_error()`), so any `_free` in
            // between would leave the epilogue reading a wiped — and freed — buffer.
            let _ = writeln!(out, "    assert({result_var} == 0 && \"expected call to fail\");");
            emit_c_error_epilogue(out, prefix, fixture, errors, documentation_snippet);
            for (_, var_name) in &request_handle_vars {
                let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
                let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
            }
            let _ = writeln!(out, "    {prefix}_default_client_free(client);");
            let _ = writeln!(out, "}}");
            return Ok(());
        }

        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle {result_var} = {call_fn}(client{method_args});"
        );
        let _ = writeln!(out, "    assert({result_var} != 0 && \"expected call to succeed\");");

        let mut intermediate_handles: Vec<(String, String)> = Vec::new();
        let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
        // Locals declared as primitive C scalars (uint64_t, double, bool, ...).
        // Locals not present here default to char* (heap-allocated accessor result).
        let mut primitive_locals: HashMap<String, String> = HashMap::new();
        // Locals declared as opaque struct handles (e.g. SAMPLELLMUsage*).
        // Keyed by local_var, value is the snake_case type name used for free().
        let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();
        // `field[].key` wildcard leaves: local_var -> (array json var, key to extract per
        // element). See `collection_wildcard.rs`.
        let mut wildcard_locals: HashMap<String, (String, String)> = HashMap::new();

        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field
                && !f.is_empty()
                && !accessed_fields.iter().any(|(k, _, _)| k == f)
            {
                // Strips virtual namespace prefixes (e.g. "interaction.action_results[0].x"
                // → "action_results[0].x") before building the accessor chain.
                let resolved = field_resolver.result_relative_path(f);
                let resolved = resolved.as_ref();
                let local_var = f.replace(['.', '['], "_").replace(']', "");
                let has_map_access = resolved.contains('[');
                if resolved.contains('.') {
                    let leaf_result = emit_nested_accessor(
                        out,
                        prefix,
                        resolved,
                        &local_var,
                        result_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                        result_type_name,
                        f,
                        type_defs,
                        config_sources,
                    )?;
                    if let Some(outcome) = leaf_result {
                        classify_nested_leaf(
                            outcome,
                            &local_var,
                            &mut primitive_locals,
                            &mut opaque_handle_locals,
                            &mut wildcard_locals,
                        );
                    }
                } else {
                    let result_type_snake = result_type_name.to_snake_case();
                    let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                    let lookup_key = format!("{result_type_snake}.{resolved}");
                    if is_skipped_c_field(fields_c_types, &result_type_snake, resolved) {
                        // Field marked "skip" — record sentinel so render_assertion skips it.
                        primitive_locals.insert(local_var.clone(), "__skip__".to_string());
                    } else if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                        let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({result_var});");
                        primitive_locals.insert(local_var.clone(), t.clone());
                    } else if try_emit_enum_accessor(
                        out,
                        prefix,
                        &prefix_upper,
                        f,
                        resolved,
                        &result_type_snake,
                        &accessor_fn,
                        result_var,
                        &local_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                    ) {
                        // accessor emitted with enum-to-string conversion
                    } else if let Some(handle_pascal) =
                        infer_opaque_handle_type(fields_c_types, &result_type_snake, resolved)
                    {
                        // Opaque struct handle: cannot be read as char*.
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}AlefHandle {local_var} = {accessor_fn}({result_var});"
                        );
                        opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                    } else {
                        // A single-segment `resolved` is not proof the field is flat: it is
                        // also what namespace stripping produces from `data.kind` when `data`
                        // is absent from `result_fields`. That guess must be checked, or the
                        // walk emits `{prefix}_{result_type}_kind()` — an accessor that only
                        // exists on the *nested* type — and generation reports success. ~keep
                        ensure_leaf_field_exists(LeafFieldCheck {
                            prefix,
                            accessor_fn: &accessor_fn,
                            resolved,
                            raw_field: f,
                            segment: resolved,
                            parent_snake_type: &result_type_snake,
                            parent_is_ir_type: type_defs.iter().any(|type_def| type_def.name == result_type_name),
                            declared_in_fields_c_types: fields_c_types.contains_key(&lookup_key),
                            result_type_name,
                            type_defs,
                            result_fields_source: &config_sources.result_fields,
                            fields_source: &config_sources.fields,
                        })?;
                        let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                    }
                }
                accessed_fields.push((f.clone(), local_var, has_map_access));
            }
        }

        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                prefix,
                field_resolver,
                &accessed_fields,
                &primitive_locals,
                &opaque_handle_locals,
                &wildcard_locals,
            );
        }

        for (_f, local_var, from_json) in &accessed_fields {
            if primitive_locals.contains_key(local_var) {
                continue;
            }
            // No scalar local was declared for a wildcard leaf — the array json var it
            // reads is freed separately, below, via `intermediate_handles`.
            if wildcard_locals.contains_key(local_var) {
                continue;
            }
            if let Some(snake_type) = opaque_handle_locals.get(local_var) {
                let _ = writeln!(out, "    {prefix}_{snake_type}_free({local_var});");
                continue;
            }
            if *from_json {
                let _ = writeln!(out, "    free({local_var});");
            } else {
                let _ = writeln!(out, "    {prefix}_free_string({local_var});");
            }
        }
        for (handle_var, snake_type) in intermediate_handles.iter().rev() {
            if snake_type == "free_string" {
                let _ = writeln!(out, "    {prefix}_free_string({handle_var});");
            } else if snake_type == "free" {
                // Intermediate JSON-key extraction (alef_json_get_string) — heap
                // char* allocated by malloc-class helper; freed via plain free().
                let _ = writeln!(out, "    free({handle_var});");
            } else {
                let _ = writeln!(out, "    {prefix}_{snake_type}_free({handle_var});");
            }
        }
        let result_type_snake = result_type_name.to_snake_case();
        let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
        for (_, var_name) in &request_handle_vars {
            let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
        let _ = writeln!(out, "}}");
        return Ok(());
    }

    // Raw C result type path: functions returning a primitive C type (char*, int32_t,
    // uintptr_t) rather than an opaque handle pointer.
    if let Some(raw_type) = raw_c_result_type {
        // Build argument string. Void-arg functions pass nothing.
        let args_str = if args.is_empty() {
            String::new()
        } else {
            let optional_sentinel = |index: usize, arg: &crate::e2e::config::ArgMapping| {
                resolve_optional_sentinel(target_params, &arg.name, index, &arg.arg_type).to_string()
            };
            let parts: Vec<String> = args
                .iter()
                .enumerate()
                .filter_map(|(index, arg)| {
                    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                    let val = fixture.input.get(field);
                    match val {
                        None if arg.optional => Some(optional_sentinel(index, arg)),
                        None => None,
                        Some(v) if v.is_null() && arg.optional => Some(optional_sentinel(index, arg)),
                        Some(v) => Some(json_to_c(v)),
                    }
                })
                .collect();
            parts.join(", ")
        };

        // Declare result variable.
        let _ = writeln!(out, "    {raw_type} {result_var} = {function_name}({args_str});");

        // ~keep: early-return mirrors the client-factory/legacy opaque-handle
        // paths' expects_error handling so success-path assertions/cleanup below
        // never run for an error fixture.
        if expects_error {
            match raw_type {
                "char*" => {
                    let _ = writeln!(out, "    assert({result_var} == NULL && \"expected call to fail\");");
                }
                "int32_t" => {
                    let _ = writeln!(out, "    assert({result_var} < 0 && \"expected call to fail\");");
                }
                // ~keep uintptr_t and any other configured raw_c_result_type (bool,
                // uint64_t, size_t, ...): raw_c_result_type is free-form config, not a
                // closed set, so fall back to the always-present last_error_code symbol.
                // A no-op arm here would be a silently-passing error test — the exact
                // bug this block exists to fix.
                _ => {
                    let _ = writeln!(
                        out,
                        "    assert({prefix}_last_error_code() != 0 && \"expected call to fail\");"
                    );
                }
            }
            emit_c_error_epilogue(out, prefix, fixture, errors, documentation_snippet);
            let _ = writeln!(out, "}}");
            return Ok(());
        }

        // not_error assertion.
        let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
        if has_not_error {
            match raw_type {
                "char*" if !result_is_option => {
                    let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");
                }
                "int32_t" => {
                    let _ = writeln!(out, "    assert({result_var} >= 0 && \"expected call to succeed\");");
                }
                "uintptr_t" => {
                    let _ = writeln!(
                        out,
                        "    assert({prefix}_last_error_code() == 0 && \"expected call to succeed\");"
                    );
                }
                _ => {}
            }
        }

        // Other assertions.
        for assertion in &fixture.assertions {
            match assertion.assertion_type.as_str() {
                "not_error" | "error" => {} // handled above / not applicable
                "not_empty" => {
                    let _ = writeln!(
                        out,
                        "    assert({result_var} != NULL && strlen({result_var}) > 0 && \"expected non-empty value\");"
                    );
                }
                "is_empty" => {
                    if result_is_option && raw_type == "char*" {
                        let _ = writeln!(
                            out,
                            "    assert({result_var} == NULL && \"expected empty/null value\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert(strlen({result_var}) == 0 && \"expected empty value\");"
                        );
                    }
                }
                "count_min" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        match raw_type {
                            "char*" => {
                                let _ = writeln!(out, "    {{");
                                let _ = writeln!(
                                    out,
                                    "        assert({result_var} != NULL && \"expected non-null JSON array\");"
                                );
                                let _ = writeln!(out, "        int elem_count = alef_json_array_count({result_var});");
                                let _ = writeln!(
                                    out,
                                    "        assert(elem_count >= {n} && \"expected at least {n} elements\");"
                                );
                                let _ = writeln!(out, "    }}");
                            }
                            _ => {
                                let _ = writeln!(
                                    out,
                                    "    assert((size_t){result_var} >= {n} && \"expected at least {n} elements\");"
                                );
                            }
                        }
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert({result_var} >= {c_val} && \"expected greater than or equal\");"
                        );
                    }
                }
                "contains" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert(strstr({result_var}, {c_val}) != NULL && \"expected to contain substring\");"
                        );
                    }
                }
                "contains_all" => {
                    if let Some(values) = &assertion.values {
                        for val in values {
                            let c_val = json_to_c(val);
                            let _ = writeln!(
                                out,
                                "    assert(strstr({result_var}, {c_val}) != NULL && \"expected to contain substring\");"
                            );
                        }
                    }
                }
                "equals" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        if val.is_string() {
                            let _ = writeln!(
                                out,
                                "    assert({result_var} != NULL && strcmp({result_var}, {c_val}) == 0 && \"equals assertion failed\");"
                            );
                        } else {
                            let _ = writeln!(
                                out,
                                "    assert({result_var} == {c_val} && \"equals assertion failed\");"
                            );
                        }
                    }
                }
                "not_contains" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert(strstr({result_var}, {c_val}) == NULL && \"expected NOT to contain substring\");"
                        );
                    }
                }
                "starts_with" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert(strncmp({result_var}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
                        );
                    }
                }
                "is_true" => {
                    let _ = writeln!(out, "    assert({result_var});");
                }
                "is_false" => {
                    let _ = writeln!(out, "    assert(!{result_var});");
                }
                other => {
                    panic!("C e2e raw-result generator: unsupported assertion type: {other}");
                }
            }
        }

        // Free char* results.
        if raw_type == "char*" {
            let free_fn = c_free_fn
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{prefix}_free_string"));
            if result_is_option {
                let _ = writeln!(out, "    if ({result_var} != NULL) {{ {free_fn}({result_var}); }}");
            } else {
                let _ = writeln!(out, "    {free_fn}({result_var});");
            }
        }

        let _ = writeln!(out, "}}");
        return Ok(());
    }

    // Legacy (non-client) path: call the function directly.
    // Used for libraries that expose standalone FFI functions.

    // The capsule lookup below keys on the result type name without taking ownership of
    // anything -- a capsule is a borrowed, host-owned pointer that is deliberately never freed
    // -- so it asks the weaker `require`. The opaque-handle emission after it asks
    // `require_owned_handle` instead, at the point ownership is actually claimed. Resolving
    // once for both would have to pick one of the two questions, and picking the weaker one is
    // what let a non-handle reach `{prefix}_{result_snake}_free`. ~keep
    let result_type_label = result_type_name.require()?;

    // Use the function name directly — the override already includes the prefix
    // (e.g. "htm_convert"), so we must NOT prepend it again.
    let prefixed_fn = function_name.to_string();

    // For json_object args, emit a from_json call to construct the options handle.
    let (typed_arg_handles, typed_arg_cleanup) = build_json_object_arg_handles(
        out,
        fixture,
        prefix,
        &prefix_upper,
        args,
        options_type_name,
        documentation_snippet,
        target_params,
    );

    let configured_args = build_args_string_c(
        &fixture.input,
        args,
        &typed_arg_handles,
        config,
        type_defs,
        fixture,
        function_name,
        target_params,
    )?;
    // `extra_args` carries C-only trailing parameters the shared, language-agnostic
    // `[crates.e2e.calls.*]` args config cannot express; for a trait-bridge `unregister`/`clear`
    // export that is the mandatory `out_error` out-param (`clear_fn.jinja`, `unregister_fn.jinja`).
    // The void-call path in `render_snippet_body` already joins it in, but this free-function path
    // never did, and the omission was masked: a fixture with no `input` fell through to
    // `json_to_c(Value::Null)`, which renders the literal `NULL` and happened to land in exactly
    // the out_error slot. The emitted call therefore looked correct while nothing was appending
    // out_error at all -- a coincidence that only surfaced once `TargetParams::Known(&[])` began
    // rendering a genuinely empty argument list for these calls. ~keep
    let extra = extra_args.join(", ");
    let args_str = match (configured_args.is_empty(), extra.is_empty()) {
        (_, true) => configured_args,
        (true, false) => extra,
        (false, false) => format!("{configured_args}, {extra}"),
    };

    // Host-capsule passthrough: a free function whose result type is a configured
    // capsule (e.g. `get_language` → `const TSLanguage *`) returns a borrowed,
    // host-owned pointer — NOT an alef opaque handle. The exported symbol's C
    // return type is `const {c_return_type} *`, and the pointer must never be passed
    // to `{prefix}_{type}_free` (that frees an alef Box; the capsule points at a
    // static grammar / registry-owned object, so freeing it corrupts the heap).
    // Emit a minimal declare + null-check with no free, mirroring the borrowed
    // semantics the Go/Zig bindings get for free via GC / borrow checking.
    if let Some(capsule) = config.ffi.as_ref().and_then(|f| f.capsule_types.get(result_type_label)) {
        let c_return_type = &capsule.c_return_type;
        let _ = writeln!(
            out,
            "    const {c_return_type} *{result_var} = {prefixed_fn}({args_str});"
        );
        if expects_error {
            // ~keep Cleanup moves after the epilogue: see `emit_c_error_epilogue` — the borrowed
            // `last_error_context()` buffer does not survive the next FFI call.
            let _ = writeln!(out, "    assert({result_var} == NULL && \"expected call to fail\");");
            emit_c_error_epilogue(out, prefix, fixture, errors, documentation_snippet);
            render_typed_arg_cleanup(out, prefix, &typed_arg_cleanup);
        } else {
            render_typed_arg_cleanup(out, prefix, &typed_arg_cleanup);
            let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");
        }
        let _ = writeln!(out, "}}");
        return Ok(());
    }

    // Everything from here on binds the result to `{prefix_upper}AlefHandle` and ends in
    // `{prefix}_{result_snake}_free({result_var})` — it claims ownership of an alef `Box`. The
    // name therefore has to be backed by something that says the result IS such a handle; the
    // mere absence of a resolved type is not that, and reading it as one is how an `i32` status
    // and a declared-scalar result both ended up being freed. This is also the path that
    // emitted `{prefix}_list_ocr_backends_free` for a family whose header declares no `_free`
    // at all. ~keep
    let result_type_name = result_type_name.require_owned_handle()?;

    if expects_error {
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle {result_var} = {prefixed_fn}({args_str});"
        );
        // ~keep Cleanup moves after the epilogue: see `emit_c_error_epilogue` — the borrowed
        // `last_error_context()` buffer does not survive the next FFI call.
        let _ = writeln!(out, "    assert({result_var} == 0 && \"expected call to fail\");");
        emit_c_error_epilogue(out, prefix, fixture, errors, documentation_snippet);
        render_typed_arg_cleanup(out, prefix, &typed_arg_cleanup);
        let _ = writeln!(out, "}}");
        return Ok(());
    }

    // The FFI returns an opaque handle; extract the content string from it.
    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle {result_var} = {prefixed_fn}({args_str});"
    );
    let _ = writeln!(out, "    assert({result_var} != 0 && \"expected call to succeed\");");

    // Collect fields accessed by assertions so we can emit accessor calls.
    // C FFI uses the opaque handle pattern: {prefix}_conversion_result_{field}(handle).
    // For nested paths we generate chained FFI accessor calls using the type
    // chain from `fields_c_types`.
    // Each entry: (fixture_field, local_var, from_json_extract).
    // `from_json_extract` is true when the variable was extracted from a JSON
    // map via alef_json_get_string and needs free() instead of {prefix}_free_string().
    let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
    // Track intermediate handles emitted so we can free them and avoid duplicates.
    // Each entry: (handle_var_name, snake_type_name) — freed in reverse order.
    let mut intermediate_handles: Vec<(String, String)> = Vec::new();
    // Locals declared as primitive C scalars (uint64_t, double, bool, ...).
    let mut primitive_locals: HashMap<String, String> = HashMap::new();
    // Locals declared as opaque struct handles (e.g. SAMPLELLMUsage*).
    let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();
    // `field[].key` wildcard leaves: local_var -> (array json var, key to extract per
    // element). See `collection_wildcard.rs`.
    let mut wildcard_locals: HashMap<String, (String, String)> = HashMap::new();

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field
            && !f.is_empty()
            && !accessed_fields.iter().any(|(k, _, _)| k == f)
        {
            // Strips virtual namespace prefixes (e.g. "interaction.action_results[0].x"
            // → "action_results[0].x") before building the accessor chain.
            let resolved = field_resolver.result_relative_path(f);
            let resolved = resolved.as_ref();
            let local_var = f.replace(['.', '['], "_").replace(']', "");
            let has_map_access = resolved.contains('[');

            if resolved.contains('.') {
                let leaf_result = emit_nested_accessor(
                    out,
                    prefix,
                    resolved,
                    &local_var,
                    result_var,
                    fields_c_types,
                    fields_enum,
                    &mut intermediate_handles,
                    result_type_name,
                    f,
                    type_defs,
                    config_sources,
                )?;
                if let Some(outcome) = leaf_result {
                    classify_nested_leaf(
                        outcome,
                        &local_var,
                        &mut primitive_locals,
                        &mut opaque_handle_locals,
                        &mut wildcard_locals,
                    );
                }
            } else {
                let result_type_snake = result_type_name.to_snake_case();
                let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                let lookup_key = format!("{result_type_snake}.{resolved}");
                if is_skipped_c_field(fields_c_types, &result_type_snake, resolved) {
                    // Field marked "skip" — record sentinel so render_assertion skips it.
                    primitive_locals.insert(local_var.clone(), "__skip__".to_string());
                } else if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                    let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({result_var});");
                    primitive_locals.insert(local_var.clone(), t.clone());
                } else if try_emit_enum_accessor(
                    out,
                    prefix,
                    &prefix_upper,
                    f,
                    resolved,
                    &result_type_snake,
                    &accessor_fn,
                    result_var,
                    &local_var,
                    fields_c_types,
                    fields_enum,
                    &mut intermediate_handles,
                ) {
                    // accessor emitted with enum-to-string conversion
                } else if let Some(handle_pascal) =
                    infer_opaque_handle_type(fields_c_types, &result_type_snake, resolved)
                {
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}AlefHandle {local_var} = {accessor_fn}({result_var});"
                    );
                    opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                } else {
                    // See the identical guard in the client-call path above: a single-segment
                    // `resolved` may be the residue of namespace stripping, not a flat field. ~keep
                    ensure_leaf_field_exists(LeafFieldCheck {
                        prefix,
                        accessor_fn: &accessor_fn,
                        resolved,
                        raw_field: f,
                        segment: resolved,
                        parent_snake_type: &result_type_snake,
                        parent_is_ir_type: type_defs.iter().any(|type_def| type_def.name == result_type_name),
                        declared_in_fields_c_types: fields_c_types.contains_key(&lookup_key),
                        result_type_name,
                        type_defs,
                        result_fields_source: &config_sources.result_fields,
                        fields_source: &config_sources.fields,
                    })?;
                    let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                }
            }
            accessed_fields.push((f.clone(), local_var.clone(), has_map_access));
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            prefix,
            field_resolver,
            &accessed_fields,
            &primitive_locals,
            &opaque_handle_locals,
            &wildcard_locals,
        );
    }

    // Free extracted leaf strings.
    for (_f, local_var, from_json) in &accessed_fields {
        if primitive_locals.contains_key(local_var) {
            continue;
        }
        // No scalar local was declared for a wildcard leaf — the array json var it reads
        // is freed separately, below, via `intermediate_handles`.
        if wildcard_locals.contains_key(local_var) {
            continue;
        }
        if let Some(snake_type) = opaque_handle_locals.get(local_var) {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({local_var});");
            continue;
        }
        if *from_json {
            let _ = writeln!(out, "    free({local_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_free_string({local_var});");
        }
    }
    // Free intermediate handles in reverse order.
    for (handle_var, snake_type) in intermediate_handles.iter().rev() {
        if snake_type == "free_string" {
            // free_string handles are freed with the free_string function directly.
            let _ = writeln!(out, "    {prefix}_free_string({handle_var});");
        } else if snake_type == "free" {
            // Intermediate JSON-key extraction (e.g. alef_json_array_get_index) — freed via plain free().
            let _ = writeln!(out, "    free({handle_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({handle_var});");
        }
    }
    render_typed_arg_cleanup(out, prefix, &typed_arg_cleanup);
    let result_type_snake = result_type_name.to_snake_case();
    let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
    let _ = writeln!(out, "}}");
    Ok(())
}

/// Construct a typed `AlefHandle` via `{prefix}_{type}_from_json(...)` for every `json_object`
/// arg with a non-null value whose declared parameter actually takes a handle
/// ([`ir_declares_a_json_string_param`]), ahead of the call that consumes it, appending the
/// construction (and any documentation setup/cleanup) to `out`.
///
/// Returns the arg-name -> handle-variable map [`build_args_string_c`] needs to splice a real
/// handle expression instead of a JSON literal, and the `(handle, type_snake)` pairs the caller
/// frees afterward via [`render_typed_arg_cleanup`].
///
/// The free-function path and the `returns_void` snippet path in [`render_snippet_body`] both
/// need this: a `json_object` arg lowers to a C literal unless something builds the handle first,
/// and the two call sites independently doing so is exactly how the `returns_void` path went
/// without it — the free-function path built handles, the void path never did. One function
/// both paths call is what keeps that from happening again, the same shape the Go-enum sibling
/// of this defect was fixed with. ~keep
///
/// Lines are written pre-indented four spaces, matching the `void test_{fn}(void) { ... }`
/// wrapper the free-function path (and [`render_test_function_impl`]'s other `out`-writing
/// branches) builds directly. [`render_snippet_body`]'s `returns_void` branch has no such
/// wrapper — it hands its whole body to `c/snippet_body.jinja`, which applies the same
/// four-space indent once via its `indent(4, true)` filter — so that caller strips this
/// baked-in indent before combining it with the call line, rather than this function knowing
/// two different indent conventions. ~keep
#[allow(clippy::too_many_arguments)]
fn build_json_object_arg_handles(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    prefix_upper: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type_name: &str,
    documentation_snippet: bool,
    target_params: TargetParams<'_>,
) -> (HashMap<String, String>, Vec<(String, String)>) {
    let mut typed_arg_handles = HashMap::new();
    let mut typed_arg_cleanup = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if arg.arg_type != "json_object" {
            continue;
        }
        let val = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
        if val.is_null() {
            continue;
        }
        if ir_declares_a_json_string_param(target_params, &arg.name, index) {
            continue;
        }
        // Fixture keys are camelCase; generated FFI from_json helpers
        // deserialize into Rust types using serde's configured casing.
        // Normalize keys before serializing.
        let normalized = transform_json_keys_for_language(val, "snake_case");
        let (docs_setup, json_expr, docs_cleanup) = render_c_docs_json(
            &arg.name,
            &normalized,
            &fixture.docs_files_for_arg(&arg.field),
            documentation_snippet,
        );
        out.push_str(&docs_setup);
        // A `json_object` arg needs a type name to call the FFI `from_json` helper
        // and produce a typed handle below. Silently skipping here used to leave
        // `arg.name` out of `typed_arg_handles`, so `build_args_string_c` fell
        // through to splicing the raw JSON literal as the argument expression
        // (`c/assertions.rs`'s `parts.push(json_to_c(v))` fallback) — exactly the
        // untyped-literal bug the `element_type` backfill exists to prevent. Fail
        // generation loudly instead, matching the other "cannot render this" panics
        // in this backend (see `build_args_string_c`'s `test_backend` arm). ~keep
        let Some(type_name) = arg
            .element_type
            .as_deref()
            .or_else(|| (!options_type_name.is_empty()).then_some(options_type_name))
        else {
            panic!(
                "C e2e generator: fixture `{}` declares a `json_object` arg `{}` with no resolvable type — \
                 `element_type` is unset and no `options_type_name` fallback is configured; cannot construct \
                 a typed `from_json` handle without knowing the target type",
                fixture.id, arg.name
            );
        };
        // ~keep Skipping leaves the arg to `build_args_string_c`'s JSON-literal path, which is
        // the wrong answer for a real DTO (hence the `element_type` backfill above) and the right
        // one here -- see `is_std_type_without_ffi_constructor`.
        if is_std_type_without_ffi_constructor(type_name) {
            continue;
        }
        let type_snake = type_name.to_snake_case();
        let handle = format!("{}_handle", sanitize_ident(&arg.name));
        out.push_str(&crate::e2e::template_env::render(
            "c/typed_handle.jinja",
            minijinja::context! {
                prefix_upper => prefix_upper,
                type_name => type_name,
                handle => handle,
                prefix => prefix,
                type_snake => type_snake,
                json_expression => json_expr,
            },
        ));
        out.push_str(&docs_cleanup);
        typed_arg_handles.insert(arg.name.clone(), handle.clone());
        typed_arg_cleanup.push((handle, type_snake));
    }
    (typed_arg_handles, typed_arg_cleanup)
}

fn render_typed_arg_cleanup(out: &mut String, prefix: &str, handles: &[(String, String)]) {
    for (handle, type_snake) in handles {
        out.push_str(&crate::e2e::template_env::render(
            "c/typed_handle_free.jinja",
            minijinja::context! { prefix => prefix, type_snake => type_snake, handle => handle },
        ));
    }
}

#[cfg(test)]
mod declared_error_variant_tests {
    use super::emit_c_error_epilogue;
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn fixture_with_declared_error(value: &str) -> Fixture {
        Fixture {
            id: "declares_error".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    fn error_def_with(variant_name: &str, error_code: Option<u32>) -> Vec<ErrorDef> {
        vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                error_code,
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    /// A message-style value (not a known variant name) is unaffected by this fix.
    #[test]
    fn message_style_value_still_asserts() {
        let mut out = String::new();
        let fixture = fixture_with_declared_error("rate limit");

        emit_c_error_epilogue(&mut out, "sample", &fixture, &[], false);

        assert!(out.contains("strstr(_err_message, \"rate limit\")"), "got: {out}");
    }

    /// The defect this fix closes: C can NEVER substantiate a known variant, even a CODED one —
    /// the C ABI exposes the variant only as a numeric taxonomy code, never as a string the
    /// generated message check could compare. Must render the registered skip, not an assertion
    /// that can never pass.
    #[test]
    fn known_variant_renders_the_skip_even_when_coded() {
        let mut out = String::new();
        let fixture = fixture_with_declared_error("Authentication");
        let errors = error_def_with("Authentication", Some(100));

        emit_c_error_epilogue(&mut out, "sample", &fixture, &errors, false);

        assert_eq!(
            out,
            "    // skipped: declared error variant 'Authentication' not substantiated by this backend's \
             generated error type\n"
        );
        assert!(!out.contains("strstr"), "must not render an assertion, got: {out}");
    }

    /// A documentation snippet renders nothing regardless of substantiability — published prose
    /// must never carry an aborting assertion or a `// skipped:` comment that reads as a defect.
    #[test]
    fn documentation_snippet_renders_nothing_for_an_unsubstantiable_variant() {
        let mut out = String::new();
        let fixture = fixture_with_declared_error("Authentication");
        let errors = error_def_with("Authentication", Some(100));

        emit_c_error_epilogue(&mut out, "sample", &fixture, &errors, true);

        assert_eq!(
            out, "",
            "documentation snippets must stay byte-identical to before this fix"
        );
    }
}

#[cfg(test)]
mod snippet_tests {
    use super::{assemble_snippet_body, declared_variable, is_expected_result_assertion, snippet_declarations};

    /// Wrap body lines the way `render_test_function` does, so `assemble_snippet_body`'s
    /// two-line header / one-line footer trim lands on the same rows it does in production. ~keep
    fn rendered_function(body_lines: &[&str]) -> String {
        let mut function = String::from("void test_fixture(void) {\n    /* Fixture */\n");
        for line in body_lines {
            function.push_str("    ");
            function.push_str(line);
            function.push('\n');
        }
        function.push_str("}\n");
        function
    }

    #[test]
    fn standalone_snippet_declares_success_guard() {
        let declarations = snippet_declarations("if (request == 0) { ALEF_TEST_PASS(); }");

        assert!(declarations.contains("return EXIT_SUCCESS"));
    }

    #[test]
    fn error_rewrite_only_matches_declared_call_result_assertion() {
        assert!(!is_expected_result_assertion(
            "assert(client != 0 && \"failed to create client\");",
            "result",
        ));
        assert!(is_expected_result_assertion(
            "assert(result == 0 && \"expected call to fail\");",
            "result",
        ));
    }

    #[test]
    fn declared_variable_reads_declarations_and_not_conditions() {
        assert_eq!(
            declared_variable("SAMPLEAlefHandle result = sample_chat(client);"),
            Some("result")
        );
        assert_eq!(
            declared_variable("char* result = sample_list_formats();"),
            Some("result")
        );
        assert_eq!(
            declared_variable("const TSLanguage *result = sample_language();"),
            Some("result")
        );
        assert_eq!(
            declared_variable("const char* api_key = getenv(\"API_KEY\");"),
            Some("api_key")
        );
        assert_eq!(
            declared_variable("assert(result == 0 && \"expected call to fail\");"),
            None
        );
        assert_eq!(declared_variable("if (result != 0) { return EXIT_FAILURE; }"), None);
        assert_eq!(declared_variable("result = sample_chat(client);"), None);
    }

    #[test]
    fn failure_guard_is_emitted_with_the_declaration_it_names() {
        let function = rendered_function(&[
            "const char* api_key = getenv(\"API_KEY\");",
            "assert(api_key != NULL && \"API_KEY must be set\");",
            "SAMPLEAlefHandle client = sample_create_client(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL);",
            "assert(client != 0 && \"failed to create client\");",
            "SAMPLEAlefHandle result = sample_default_client_chat(client);",
            "sample_default_client_free(client);",
            "assert(result == 0 && \"expected call to fail\");",
        ]);

        let body =
            assemble_snippet_body(&function, "result", true, "chat_auth_401", "result != 0").expect("body assembles");

        let guard = body
            .lines()
            .position(|line| line.contains("if (result != 0) { return EXIT_FAILURE; }"))
            .expect("failure guard is emitted");
        let declaration = body
            .lines()
            .position(|line| line.contains("SAMPLEAlefHandle result ="))
            .expect("result is declared");
        assert!(declaration < guard, "guard precedes its declaration:\n{body}");
        assert!(
            body.contains("assert(client != 0 && \"failed to create client\");"),
            "the client assertion must survive untouched:\n{body}"
        );
    }

    /// Regression control for the 0.60.0 shape: back then every `assert(` line in an
    /// error fixture became the `result` guard, so the client assertion turned into a guard
    /// naming a variable declared on the next line. Feeding that ordering back in must be
    /// rejected outright rather than published as C that cannot compile. ~keep
    #[test]
    fn failure_guard_before_its_declaration_is_rejected() {
        let function = rendered_function(&[
            "SAMPLEAlefHandle client = sample_create_client(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL);",
            "assert(result == 0 && \"expected call to fail\");",
            "SAMPLEAlefHandle result = sample_default_client_chat(client);",
        ]);

        let error = assemble_snippet_body(&function, "result", true, "chat_auth_401", "result != 0")
            .expect_err("a guard preceding its declaration must be rejected");

        assert!(error.to_string().contains("before `result` is declared"), "{error}");
    }

    #[test]
    fn success_snippets_drop_assertions_without_touching_declarations() {
        let function = rendered_function(&[
            "SAMPLEAlefHandle result = sample_default_client_chat(client);",
            "assert(result != 0 && \"expected call to succeed\");",
            "sample_chat_response_free(result);",
        ]);

        let body =
            assemble_snippet_body(&function, "result", false, "chat_basic", "result != 0").expect("body assembles");

        assert!(!body.contains("assert("), "{body}");
        assert!(
            body.contains("SAMPLEAlefHandle result = sample_default_client_chat(client);"),
            "{body}"
        );
        assert!(body.contains("sample_chat_response_free(result);"), "{body}");
    }
}
