use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_shell, sanitize_filename, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::fmt::Write as FmtWrite;

/// Render a per-category `test_{category}.sh` file.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_category_file(
    category: &str,
    fixtures: &[&Fixture],
    binary_name: &str,
    default_subcommand: &str,
    static_cli_args: &[String],
    cli_flags: &std::collections::HashMap<String, String>,
    args: &[crate::e2e::config::ArgMapping],
    e2e_config: &E2eConfig,
) -> String {
    let safe_category = sanitize_filename(category);
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);

    for fixture in fixtures {
        render_test_function(
            &mut out,
            fixture,
            binary_name,
            default_subcommand,
            static_cli_args,
            cli_flags,
            args,
            e2e_config,
        );
        let _ = writeln!(out);
    }

    // Emit a runner function for this category.
    let _ = writeln!(out, "run_tests_{safe_category}() {{");
    for fixture in fixtures {
        let fn_name = sanitize_ident(&fixture.id);
        let _ = writeln!(out, "  run_test test_{fn_name}");
    }
    let _ = writeln!(out, "}}");
    out
}

/// Render a single `test_{id}()` function for a fixture.
#[allow(clippy::too_many_arguments)]
fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    binary_name: &str,
    default_subcommand: &str,
    static_cli_args: &[String],
    cli_flags: &std::collections::HashMap<String, String>,
    _args: &[crate::e2e::config::ArgMapping],
    e2e_config: &E2eConfig,
) {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let _ = writeln!(out, "test_{fn_name}() {{");
    let _ = writeln!(out, "  # {description}");

    // Resolve fixture-specific call config if provided, otherwise use defaults.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );

    // Check if this call is unsupported in brew. If so, skip the test with a documented reason.
    if let Some(reason) = call_config.unsupported_in.get("brew") {
        let _ = writeln!(out, "  # SKIP [brew unsupported]: {reason}");
        let _ = writeln!(out, "  return 0");
        let _ = writeln!(out, "}}");
        return;
    }
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    );
    let field_resolver = &call_field_resolver;

    // Determine subcommand based on fixture tags.
    // If "crawl" tag is present, use "crawl"; if "map" tag is present, use "map"; else use default.
    let subcommand = determine_subcommand(&fixture.tags, default_subcommand);

    // Build the CLI command using the resolved call config.
    let cmd_parts = build_cli_command(
        fixture,
        binary_name,
        &subcommand,
        static_cli_args,
        cli_flags,
        fixture.resolved_args(call_config),
    );

    if expects_error {
        let cmd = cmd_parts.join(" ");
        let _ = writeln!(out, "  if {cmd} >/dev/null 2>&1; then");
        let _ = writeln!(
            out,
            "    echo 'FAIL [error]: expected command to fail but it succeeded' >&2"
        );
        let _ = writeln!(out, "    return 1");
        let _ = writeln!(out, "  fi");
        let _ = writeln!(out, "}}");
        return;
    }

    // Check if any assertion will actually emit code (not be skipped).
    let has_active_assertions = fixture.assertions.iter().any(|a| {
        a.field
            .as_ref()
            .is_none_or(|f| f.is_empty() || field_resolver.is_valid_for_result(f))
    });

    // Capture output (only if there are active assertions that reference it).
    let cmd = cmd_parts.join(" ");
    if has_active_assertions {
        let _ = writeln!(out, "  local output");
        let _ = writeln!(out, "  output=$({cmd})");
    } else {
        let _ = writeln!(out, "  {cmd} >/dev/null");
    }
    let _ = writeln!(out);

    // Emit assertions.
    for assertion in &fixture.assertions {
        render_assertion(out, assertion, binary_name, field_resolver);
    }

    let _ = writeln!(out, "}}");
}

/// Determine the brew subcommand based on fixture tags.
///
/// If the fixture tags contain "crawl", returns "crawl".
/// If the fixture tags contain "map", returns "map".
/// Otherwise, returns the default subcommand.
fn determine_subcommand(tags: &[String], default: &str) -> String {
    for tag in tags {
        if tag == "crawl" {
            return "crawl".to_string();
        }
        if tag == "map" {
            return "map".to_string();
        }
    }
    default.to_string()
}

/// Build the shell CLI invocation as a list of tokens.
///
/// Tokens are returned unquoted where safe (flag names) or single-quoted
/// (string values from the fixture).
fn build_cli_command(
    fixture: &Fixture,
    binary_name: &str,
    subcommand: &str,
    static_cli_args: &[String],
    cli_flags: &std::collections::HashMap<String, String>,
    args: &[crate::e2e::config::ArgMapping],
) -> Vec<String> {
    let mut parts: Vec<String> = vec![binary_name.to_string(), subcommand.to_string()];

    for arg in args {
        match arg.arg_type.as_str() {
            "mock_url" => {
                // Positional URL argument.
                //
                // Prefer the per-fixture `MOCK_SERVER_<FIXTURE_ID>` env var when set —
                // host-root fixtures (robots.txt, sitemap.xml) need their own listener
                // so the path lives at `/robots.txt`, not `/fixtures/<id>/robots.txt`.
                // Fall back to `MOCK_SERVER_URL/fixtures/<id>` for the common case.
                let upper_id = fixture.id.to_uppercase();
                parts.push(format!(
                    "\"${{MOCK_SERVER_{upper_id}:-${{MOCK_SERVER_URL}}/fixtures/{}}}\"",
                    fixture.id
                ));
            }
            "handle" => {
                // CLI manages its own engine; skip handle args.
            }
            _ => {
                // Check if there is a cli_flags mapping for this field.
                if let Some(flag) = cli_flags.get(&arg.field) {
                    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                    if let Some(val) = fixture.input.get(field) {
                        if !val.is_null() {
                            let val_str = json_value_to_shell_arg(val);
                            parts.push(flag.clone());
                            parts.push(val_str);
                        }
                    }
                }
            }
        }
    }

    // Check if fixture has input.config and emit it as --config flag.
    if let Some(config_val) = fixture.input.get("config") {
        if !config_val.is_null() {
            // Minify the JSON config object to a single line for shell argument.
            let config_json = serde_json::to_string(config_val).unwrap_or_default();
            parts.push("--config".to_string());
            parts.push(format!("'{}'", escape_shell(&config_json)));
        }
    }

    // Append static CLI args last.
    for static_arg in static_cli_args {
        parts.push(static_arg.clone());
    }

    parts
}

/// Convert a JSON value to a shell argument string.
///
/// Strings are wrapped in single quotes with embedded single quotes escaped.
/// Numbers and booleans are emitted verbatim.
fn json_value_to_shell_arg(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("'{}'", escape_shell(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "''".to_string(),
        other => format!("'{}'", escape_shell(&other.to_string())),
    }
}

/// Convert a fixture field path to a jq expression.
///
/// A path like `metadata.title` becomes `.metadata.title`.
/// An array field like `links` becomes `.links`.
/// The pseudo-property `length` (also `count`, `size`) becomes `| length`
/// because jq uses pipe syntax for the `length` builtin.
fn field_to_jq_path(resolved: &str) -> String {
    // Check if the path ends with a length/count/size pseudo-property.
    // E.g., "pages.length" → ".pages | length"
    if let Some((prefix, suffix)) = resolved.rsplit_once('.') {
        if suffix == "length" || suffix == "count" || suffix == "size" {
            return format!(".{prefix} | length");
        }
    }
    // Handle bare "length" / "count" / "size" (top-level array).
    if resolved == "length" || resolved == "count" || resolved == "size" {
        return ". | length".to_string();
    }
    format!(".{resolved}")
}

/// Build a CLI command for a method_result assertion.
///
/// Uses generic dispatch: `{binary_name} {kebab-method} "$output" args...`.
/// The method name is converted from snake_case to kebab-case for the CLI subcommand.
/// Args from the fixture JSON object are emitted as positional shell arguments in
/// insertion order, using best-effort shell quoting.
fn build_brew_method_call(binary_name: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    let subcommand = method_name.replace('_', "-");
    if let Some(args_val) = args {
        let arg_str = args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => format!("'{}'", escape_shell(s)),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        if arg_str.is_empty() {
            format!("{binary_name} {subcommand} \"$output\"")
        } else {
            format!("{binary_name} {subcommand} \"$output\" {arg_str}")
        }
    } else {
        format!("{binary_name} {subcommand} \"$output\"")
    }
}

/// Render a single assertion as shell code.
fn render_assertion(out: &mut String, assertion: &Assertion, binary_name: &str, field_resolver: &FieldResolver) {
    // Skip assertions on fields not available on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "  # skipped: field '{f}' not available on result type");
            return;
        }
    }

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(field) = &assertion.field {
                if let Some(expected) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let expected_str = json_value_to_shell_string(expected);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(out, "  assert_equals \"$val_{safe_field}\" '{expected_str}' '{field}'");
                }
            }
        }
        "contains" => {
            if let Some(field) = &assertion.field {
                if let Some(expected) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let expected_str = json_value_to_shell_string(expected);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(
                        out,
                        "  assert_contains \"$val_{safe_field}\" '{expected_str}' '{field}'"
                    );
                }
            }
        }
        "not_empty" | "tree_not_null" => {
            if let Some(field) = &assertion.field {
                let resolved = field_resolver.resolve(field);
                let jq_path = field_to_jq_path(resolved);
                let safe_field = sanitize_ident(field);
                let _ = writeln!(out, "  local val_{safe_field}");
                let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                let _ = writeln!(out, "  assert_not_empty \"$val_{safe_field}\" '{field}'");
            }
        }
        "count_min" | "root_child_count_min" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    if let Some(min) = val.as_u64() {
                        let resolved = field_resolver.resolve(field);
                        let jq_path = field_to_jq_path(resolved);
                        let safe_field = sanitize_ident(field);
                        let _ = writeln!(out, "  local count_{safe_field}");
                        let _ = writeln!(
                            out,
                            "  count_{safe_field}=$(echo \"$output\" | jq '{jq_path} | length')"
                        );
                        let _ = writeln!(out, "  assert_count_min \"$count_{safe_field}\" {min} '{field}'");
                    }
                }
            }
        }
        "greater_than" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let threshold = json_value_to_shell_string(val);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(
                        out,
                        "  assert_greater_than \"$val_{safe_field}\" '{threshold}' '{field}'"
                    );
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let threshold = json_value_to_shell_string(val);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(
                        out,
                        "  assert_greater_than_or_equal \"$val_{safe_field}\" '{threshold}' '{field}'"
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(field) = &assertion.field {
                if let Some(serde_json::Value::Array(items)) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    for (index, item) in items.iter().enumerate() {
                        let item_str = json_value_to_shell_string(item);
                        let _ = writeln!(
                            out,
                            "  assert_contains \"$val_{safe_field}\" '{item_str}' '{field}[{index}]'"
                        );
                    }
                }
            }
        }
        "is_empty" => {
            if let Some(field) = &assertion.field {
                let resolved = field_resolver.resolve(field);
                let jq_path = field_to_jq_path(resolved);
                let safe_field = sanitize_ident(field);
                let _ = writeln!(out, "  local val_{safe_field}");
                // Use `// empty` so JSON null becomes an empty string rather than the literal "null".
                let _ = writeln!(
                    out,
                    "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path} // empty')"
                );
                let _ = writeln!(out, "  assert_is_empty \"$val_{safe_field}\" '{field}'");
            }
        }
        "less_than" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let threshold = json_value_to_shell_string(val);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(out, "  assert_less_than \"$val_{safe_field}\" '{threshold}' '{field}'");
                }
            }
        }
        "not_contains" => {
            if let Some(field) = &assertion.field {
                if let Some(expected) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let expected_str = json_value_to_shell_string(expected);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(
                        out,
                        "  assert_not_contains \"$val_{safe_field}\" '{expected_str}' '{field}'"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    if let Some(n) = val.as_u64() {
                        let resolved = field_resolver.resolve(field);
                        let jq_path = field_to_jq_path(resolved);
                        let safe_field = sanitize_ident(field);
                        let _ = writeln!(out, "  local count_{safe_field}");
                        let _ = writeln!(
                            out,
                            "  count_{safe_field}=$(echo \"$output\" | jq '{jq_path} | length')"
                        );
                        let _ = writeln!(out, "  [ \"$count_{safe_field}\" -eq {n} ] || exit 1");
                    }
                }
            }
        }
        "is_true" => {
            if let Some(field) = &assertion.field {
                let resolved = field_resolver.resolve(field);
                let jq_path = field_to_jq_path(resolved);
                let safe_field = sanitize_ident(field);
                let _ = writeln!(out, "  local val_{safe_field}");
                let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                let _ = writeln!(out, "  [ \"$val_{safe_field}\" = \"true\" ] || exit 1");
            }
        }
        "is_false" => {
            if let Some(field) = &assertion.field {
                let resolved = field_resolver.resolve(field);
                let jq_path = field_to_jq_path(resolved);
                let safe_field = sanitize_ident(field);
                let _ = writeln!(out, "  local val_{safe_field}");
                let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                let _ = writeln!(out, "  [ \"$val_{safe_field}\" = \"false\" ] || exit 1");
            }
        }
        "less_than_or_equal" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let threshold = json_value_to_shell_string(val);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(
                        out,
                        "  assert_less_than_or_equal \"$val_{safe_field}\" '{threshold}' '{field}'"
                    );
                }
            }
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let check = assertion.check.as_deref().unwrap_or("is_true");
                let cmd = build_brew_method_call(binary_name, method_name, assertion.args.as_ref());
                // For is_error, skip capturing the result — just run the command and check
                // the exit code so we don't execute the method twice.
                if check == "is_error" {
                    let _ = writeln!(out, "  if {cmd} >/dev/null 2>&1; then");
                    let _ = writeln!(
                        out,
                        "    echo 'FAIL [method_result]: expected method to raise error but it succeeded' >&2"
                    );
                    let _ = writeln!(out, "    return 1");
                    let _ = writeln!(out, "  fi");
                } else {
                    let method_var = format!("method_result_{}", sanitize_ident(method_name));
                    let _ = writeln!(out, "  local {method_var}");
                    let _ = writeln!(out, "  {method_var}=$({cmd})");
                    match check {
                        "equals" => {
                            if let Some(val) = &assertion.value {
                                let expected = json_value_to_shell_string(val);
                                let _ = writeln!(out, "  [ \"${method_var}\" = '{expected}' ] || exit 1");
                            }
                        }
                        "is_true" => {
                            let _ = writeln!(out, "  [ \"${method_var}\" = \"true\" ] || exit 1");
                        }
                        "is_false" => {
                            let _ = writeln!(out, "  [ \"${method_var}\" = \"false\" ] || exit 1");
                        }
                        "greater_than_or_equal" => {
                            if let Some(val) = &assertion.value {
                                if let Some(n) = val.as_u64() {
                                    let _ = writeln!(out, "  [ \"${method_var}\" -ge {n} ] || exit 1");
                                }
                            }
                        }
                        "count_min" => {
                            if let Some(val) = &assertion.value {
                                if let Some(n) = val.as_u64() {
                                    let _ = writeln!(
                                        out,
                                        "  local count_from_method_result=$(echo \"${method_var}\" | jq 'length')"
                                    );
                                    let _ = writeln!(out, "  [ \"$count_from_method_result\" -ge {n} ] || exit 1");
                                }
                            }
                        }
                        "contains" => {
                            if let Some(val) = &assertion.value {
                                let expected = json_value_to_shell_string(val);
                                let _ = writeln!(out, "  [[ \"${method_var}\" == *'{expected}'* ]] || exit 1");
                            }
                        }
                        other_check => {
                            panic!("Brew e2e generator: unsupported method_result check type: {other_check}");
                        }
                    }
                }
            } else {
                panic!("method_result assertion missing 'method' field");
            }
        }
        "min_length" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    if let Some(n) = val.as_u64() {
                        let resolved = field_resolver.resolve(field);
                        let jq_path = field_to_jq_path(resolved);
                        let safe_field = sanitize_ident(field);
                        let _ = writeln!(out, "  local val_{safe_field}");
                        let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                        let _ = writeln!(
                            out,
                            "  [ \"${{#val_{safe_field}}}\" -ge {n} ] || {{ echo \"FAIL [{field}]: expected length >= {n}\" >&2; return 1; }}"
                        );
                    }
                }
            }
        }
        "max_length" => {
            if let Some(field) = &assertion.field {
                if let Some(val) = &assertion.value {
                    if let Some(n) = val.as_u64() {
                        let resolved = field_resolver.resolve(field);
                        let jq_path = field_to_jq_path(resolved);
                        let safe_field = sanitize_ident(field);
                        let _ = writeln!(out, "  local val_{safe_field}");
                        let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                        let _ = writeln!(
                            out,
                            "  [ \"${{#val_{safe_field}}}\" -le {n} ] || {{ echo \"FAIL [{field}]: expected length <= {n}\" >&2; return 1; }}"
                        );
                    }
                }
            }
        }
        "ends_with" => {
            if let Some(field) = &assertion.field {
                if let Some(expected) = &assertion.value {
                    let resolved = field_resolver.resolve(field);
                    let jq_path = field_to_jq_path(resolved);
                    let expected_str = json_value_to_shell_string(expected);
                    let safe_field = sanitize_ident(field);
                    let _ = writeln!(out, "  local val_{safe_field}");
                    let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                    let _ = writeln!(
                        out,
                        "  [[ \"$val_{safe_field}\" == *'{expected_str}' ]] || {{ echo \"FAIL [{field}]: expected to end with '{expected_str}'\" >&2; return 1; }}"
                    );
                }
            }
        }
        "matches_regex" => {
            if let Some(field) = &assertion.field {
                if let Some(expected) = &assertion.value {
                    if let Some(pattern) = expected.as_str() {
                        let resolved = field_resolver.resolve(field);
                        let jq_path = field_to_jq_path(resolved);
                        let safe_field = sanitize_ident(field);
                        let _ = writeln!(out, "  local val_{safe_field}");
                        let _ = writeln!(out, "  val_{safe_field}=$(echo \"$output\" | jq -r '{jq_path}')");
                        let _ = writeln!(
                            out,
                            "  [[ \"$val_{safe_field}\" =~ {pattern} ]] || {{ echo \"FAIL [{field}]: expected to match /{pattern}/\" >&2; return 1; }}"
                        );
                    }
                }
            }
        }
        "not_error" => {
            // No-op: reaching this point means the call succeeded.
        }
        "error" => {
            // Handled at the function level (early return above).
        }
        other => {
            panic!("Brew e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a JSON value to a plain string suitable for use in shell assertions.
///
/// Returns the bare string content (no quotes) — callers wrap in single quotes.
fn json_value_to_shell_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => escape_shell(s),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => escape_shell(&other.to_string()),
    }
}
