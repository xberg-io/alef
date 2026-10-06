//! Zig doc-snippet body rendering, split out of test_file.rs.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::zig) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    module_name: &str,
    ffi_prefix: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> anyhow::Result<String> {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let result_var = call.effective_result_var();
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    if call.args.iter().any(|argument| argument.arg_type == "test_backend") {
        anyhow::bail!("zig snippet `{}` requires test-backend lifecycle teardown", fixture.id);
    }
    let mut call_fixture = fixture.clone();
    if !expects_error {
        call_fixture.assertions.clear();
    }
    let mut test = String::new();
    render_test_fn(
        &mut test,
        &call_fixture,
        e2e_config,
        "",
        "",
        &[],
        module_name,
        ffi_prefix,
        config,
        type_defs,
        &[],
        false,
        true,
        // Snippet rendering has no free-function IR to consult — only `type_defs`. This still
        // lets `ir_says_json_struct` resolve through a method on an IR type (client_factory
        // calls), the same degraded-but-not-absent state `CallIr::is_absent` already models.
        crate::e2e::codegen::call_ir::CallIr {
            functions: &[],
            type_defs,
        },
        &[],
    );
    // The test-mode error path captures the failure with a discarded `else |_|` arm
    // (nothing to report inside `test { ... }`). The snippet is a runnable `main`,
    // so swap in a named capture that prints the caught error instead. ~keep
    // Rebinding the discarded call result is a ONE-LINE, ONCE-PER-BODY edit, and both halves of
    // that had to be stated explicitly. Applied per line with no guard, `"_ = "` also matched the
    // allocator teardown every snippet emits -- `defer _ = gpa.deinit();` became
    // `defer const result = gpa.deinit();`, which is not Zig at all and failed 54 of one consumer's
    // snippets on `expected block or expression`. Requiring the discard to open the statement is
    // what separates the call from `defer`/`errdefer`-prefixed ones; `bound` stops a second,
    // later discard from being rebound to the same name. ~keep
    let mut bound = false;
    let mut body = test
        .lines()
        .map(|line| {
            let line = line.replace(
                "else |_| {}",
                "else |err| { std.debug.print(\"call failed as expected: {s}\\n\", .{@errorName(err)}); }",
            );
            if expects_error || call.returns_void || bound {
                return line;
            }
            let Some(discarded) = discarded_call_statement(&line) else {
                return line;
            };
            bound = true;
            let indent = &line[..line.len() - line.trim_start().len()];
            format!("{indent}const {result_var} = {discarded}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A `result_is_json_struct` call binds `_result_json` — a `[]u8` payload — instead of a
    // typed `result`, so no `docs.shows` field path has a struct to read from. Those
    // fixtures keep the whole-payload print below. ~keep
    let displays_stream = body.contains("const _stream_handle =");
    let binds_typed_result =
        !expects_error && !call.returns_void && !displays_stream && !body.contains("const _result_json =");
    let presentation = if binds_typed_result {
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "zig", type_defs, enums, functions)
    } else {
        Vec::new()
    };
    if !expects_error && !call.returns_void && !displays_stream && presentation.is_empty() {
        let displayed_result = if body.contains("const _result_json =") {
            "_result_json"
        } else {
            result_var
        };
        let format = if displayed_result == "_result_json" {
            "{s}"
        } else {
            "{any}"
        };
        // `join("\n")` above drops the trailing newline `render_test_fn` wrote, so appending
        // without restoring it splices the print onto the end of the last statement line —
        // legal Zig, but a published snippet a reader has to un-run-on. ~keep
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!(
            "    std.debug.print(\"{format}\\n\", .{{{displayed_result}}});\n"
        ));
    }
    Ok(crate::e2e::template_env::render(
        "zig/snippet_body.jinja",
        minijinja::context! { module => module_name, body => body, body_is_indented => true,
        presentation => presentation },
    ))
}
