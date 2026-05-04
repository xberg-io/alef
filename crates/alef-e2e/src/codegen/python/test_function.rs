//! Python test function body rendering (non-HTTP fixtures).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use heck::{ToShoutySnakeCase, ToSnakeCase};

use crate::codegen::resolve_field;
use crate::config::E2eConfig;
use crate::escape::{escape_python, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::Fixture;

use super::assertions::render_assertion;
use super::helpers::{
    BytesKind, classify_bytes_value, is_skipped, resolve_client_factory, resolve_function_name_for_call,
};
use super::json::json_to_python_literal;
use super::visitors::emit_python_visitor_method;

/// Render a pytest test function for a non-HTTP fixture.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
    field_resolver: &FieldResolver,
) {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_function_name_for_call(call_config);
    let result_var = &call_config.result_var;

    let python_override = call_config.overrides.get("python");
    let result_is_simple = python_override.is_some_and(|o| o.result_is_simple);
    let arg_name_map = python_override.map(|o| &o.arg_name_map);

    let desc_with_period = if description.ends_with('.') {
        description.to_string()
    } else {
        format!("{description}.")
    };

    if is_skipped(fixture, "python") {
        let reason = fixture
            .skip
            .as_ref()
            .and_then(|s| s.reason.as_deref())
            .unwrap_or("skipped for python");
        let escaped = escape_python(reason);
        let _ = writeln!(out, "@pytest.mark.skip(reason=\"{escaped}\")");
    }

    let is_async = call_config.r#async;
    if is_async {
        let _ = writeln!(out, "@pytest.mark.asyncio");
        let _ = writeln!(out, "async def test_{fn_name}() -> None:");
    } else {
        let _ = writeln!(out, "def test_{fn_name}() -> None:");
    }
    let _ = writeln!(out, "    \"\"\"{desc_with_period}\"\"\"");

    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (arg_bindings, kwarg_exprs) = build_args_and_setup(
        fixture,
        call_config,
        options_type,
        options_via,
        enum_fields,
        handle_nested_types,
        handle_dict_types,
        arg_name_map,
    );

    if let Some(visitor_spec) = &fixture.visitor {
        let _ = writeln!(out, "    class _TestVisitor:");
        for (method_name, action) in &visitor_spec.callbacks {
            emit_python_visitor_method(out, method_name, action);
        }
    }

    for binding in &arg_bindings {
        let _ = writeln!(out, "{binding}");
    }

    let call_args_str = {
        let mut exprs = kwarg_exprs.clone();
        if fixture.visitor.is_some() {
            exprs.push("visitor=_TestVisitor()".to_string());
        }
        exprs.join(", ")
    };
    let await_prefix = if is_async { "await " } else { "" };

    // Client factory: when configured, create a client and dispatch as a method.
    // Point the client at MOCK_SERVER_URL/fixtures/<id> so the mock server serves
    // the fixture response via prefix routing.
    let client_factory = resolve_client_factory(e2e_config);
    let call_expr = if let Some(ref factory) = client_factory {
        let fixture_id = &fixture.id;
        let _ = writeln!(
            out,
            "    client = {factory}(api_key=\"test-key\", base_url=os.environ[\"MOCK_SERVER_URL\"] + \"/fixtures/{fixture_id}\")"
        );
        format!("{await_prefix}client.{function_name}({call_args_str})")
    } else {
        format!("{await_prefix}{function_name}({call_args_str})")
    };

    if has_error_assertion {
        emit_error_assertion(out, fixture, &call_expr);
        return;
    }

    emit_result_and_assertions(
        out,
        fixture,
        e2e_config,
        call_config,
        &call_expr,
        result_var,
        field_resolver,
        result_is_simple,
    );
}

fn emit_error_assertion(out: &mut String, fixture: &Fixture, call_expr: &str) {
    let error_assertion = fixture.assertions.iter().find(|a| a.assertion_type == "error");
    let has_message = error_assertion
        .and_then(|a| a.value.as_ref())
        .and_then(|v| v.as_str())
        .is_some();

    if has_message {
        let _ = writeln!(out, "    with pytest.raises(Exception) as exc_info:  # noqa: B017");
        let _ = writeln!(out, "        {call_expr}");
        if let Some(msg) = error_assertion.and_then(|a| a.value.as_ref()).and_then(|v| v.as_str()) {
            let escaped = escape_python(msg);
            let _ = writeln!(out, "    assert \"{escaped}\" in str(exc_info.value)  # noqa: S101");
        }
    } else {
        let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
        let _ = writeln!(out, "        {call_expr}");
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_result_and_assertions(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call_config: &crate::config::CallConfig,
    call_expr: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
) {
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        if result_is_simple {
            if let Some(f) = &a.field {
                let f_lower = f.to_lowercase();
                if !f.is_empty()
                    && f_lower != "content"
                    && f_lower != "result"
                    && (f_lower.starts_with("metadata")
                        || f_lower.starts_with("document")
                        || f_lower.starts_with("structure")
                        || f_lower.starts_with("pages")
                        || f_lower.starts_with("chunks")
                        || f_lower.starts_with("tables")
                        || f_lower.starts_with("images")
                        || f_lower.starts_with("mime_type")
                        || f_lower.starts_with("is_")
                        || f_lower == "byte_length"
                        || f_lower == "page_count"
                        || f_lower == "output_format"
                        || f_lower == "extraction_method")
                {
                    return false;
                }
            }
            return true;
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    let py_result_var = if has_usable_assertion {
        result_var.to_string()
    } else {
        "_".to_string()
    };
    let _ = writeln!(out, "    {py_result_var} = {call_expr}");

    let fields_enum = &e2e_config.fields_enum;
    for assertion in &fixture.assertions {
        if assertion.assertion_type == "not_error" {
            if !call_config.returns_result {
                continue;
            }
            continue;
        }
        render_assertion(
            out,
            assertion,
            result_var,
            field_resolver,
            fields_enum,
            result_is_simple,
        );
    }
}

/// Build arg binding lines and kwarg expressions for a fixture call.
#[allow(clippy::too_many_arguments)]
fn build_args_and_setup(
    fixture: &Fixture,
    call_config: &crate::config::CallConfig,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
    arg_name_map: Option<&HashMap<String, String>>,
) -> (Vec<String>, Vec<String>) {
    let mut arg_bindings = Vec::new();
    let mut kwarg_exprs = Vec::new();

    for arg in &call_config.args {
        let var_name = &arg.name;
        let kwarg_name = arg_name_map
            .and_then(|m| m.get(var_name.as_str()))
            .map(|s| s.as_str())
            .unwrap_or(var_name.as_str());

        if arg.arg_type == "handle" {
            emit_handle_arg(
                &mut arg_bindings,
                &mut kwarg_exprs,
                fixture,
                arg,
                var_name,
                kwarg_name,
                options_type,
                handle_nested_types,
                handle_dict_types,
            );
            continue;
        }

        if arg.arg_type == "mock_url" {
            let fixture_id = &fixture.id;
            arg_bindings.push(format!(
                "    {var_name} = os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'"
            ));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        let value = resolve_field(&fixture.input, &arg.field);

        if value.is_null() && arg.optional {
            continue;
        }

        if arg.arg_type == "json_object"
            && !value.is_null()
            && emit_json_object_arg(
                &mut arg_bindings,
                &mut kwarg_exprs,
                value,
                var_name,
                options_type,
                options_via,
                enum_fields,
            )
        {
            continue;
        }

        if arg.optional && value.is_null() {
            continue;
        }

        if value.is_null() && !arg.optional {
            let default_val = match arg.arg_type.as_str() {
                "string" => "\"\"".to_string(),
                "int" | "integer" => "0".to_string(),
                "float" | "number" => "0.0".to_string(),
                "bool" | "boolean" => "False".to_string(),
                _ => "None".to_string(),
            };
            arg_bindings.push(format!("    {var_name} = {default_val}"));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        if arg.arg_type == "bytes" {
            emit_bytes_arg(&mut arg_bindings, &mut kwarg_exprs, value, var_name);
            continue;
        }

        let literal = json_to_python_literal(value);
        let noqa = if literal.contains("/tmp/") {
            "  # noqa: S108"
        } else {
            ""
        };
        arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
        kwarg_exprs.push(var_name.to_string());
    }

    (arg_bindings, kwarg_exprs)
}

#[allow(clippy::too_many_arguments)]
fn emit_handle_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    fixture: &Fixture,
    arg: &crate::config::ArgMapping,
    var_name: &str,
    _kwarg_name: &str,
    options_type: Option<&str>,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
) {
    let constructor_name = format!("create_{}", arg.name.to_snake_case());
    let config_value = resolve_field(&fixture.input, &arg.field);
    if config_value.is_null() || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty()) {
        arg_bindings.push(format!("    {var_name} = {constructor_name}(None)"));
    } else if let Some(obj) = config_value.as_object() {
        let kwargs: Vec<String> = obj
            .iter()
            .map(|(k, v)| {
                let snake_key = k.to_snake_case();
                let py_val = build_handle_kwarg_value(k, v, handle_nested_types, handle_dict_types);
                format!("{snake_key}={py_val}")
            })
            .collect();
        let config_class = options_type.unwrap_or("CrawlConfig");
        let single_line = format!("    {var_name}_config = {config_class}({})", kwargs.join(", "));
        if single_line.len() <= 120 {
            arg_bindings.push(single_line);
        } else {
            let mut lines = format!("    {var_name}_config = {config_class}(\n");
            for kw in &kwargs {
                lines.push_str(&format!("        {kw},\n"));
            }
            lines.push_str("    )");
            arg_bindings.push(lines);
        }
        arg_bindings.push(format!("    {var_name} = {constructor_name}({var_name}_config)"));
    } else {
        let literal = json_to_python_literal(config_value);
        arg_bindings.push(format!("    {var_name} = {constructor_name}({literal})"));
    }
    kwarg_exprs.push(var_name.to_string());
}

fn build_handle_kwarg_value(
    k: &str,
    v: &serde_json::Value,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
) -> String {
    if let Some(type_name) = handle_nested_types.get(k) {
        if let Some(nested_obj) = v.as_object() {
            if nested_obj.is_empty() {
                return format!("{type_name}()");
            }
            if handle_dict_types.contains(k) {
                return json_to_python_literal(v);
            }
            let nested_kwargs: Vec<String> = nested_obj
                .iter()
                .map(|(nk, nv)| {
                    let nested_snake_key = nk.to_snake_case();
                    format!("{nested_snake_key}={}", json_to_python_literal(nv))
                })
                .collect();
            return format!("{type_name}({})", nested_kwargs.join(", "));
        }
    }
    if k == "request_timeout" {
        if let Some(ms) = v.as_u64() {
            return format!("{}", ms / 1000);
        }
    }
    json_to_python_literal(v)
}

/// Returns `true` if the arg was fully emitted (caller should `continue`).
#[allow(clippy::too_many_arguments)]
fn emit_json_object_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
) -> bool {
    match options_via {
        "dict" => {
            let literal = json_to_python_literal(value);
            let noqa = if literal.contains("/tmp/") {
                "  # noqa: S108"
            } else {
                ""
            };
            arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "json" => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            let escaped = escape_python(&json_str);
            arg_bindings.push(format!("    {var_name} = json.loads(\"{escaped}\")"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        _ => {
            // "kwargs" mode
            if let (Some(opts_type), Some(obj)) = (options_type, value.as_object()) {
                let kwargs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let snake_key = k.to_snake_case();
                        let py_val = if let Some(enum_type) = enum_fields.get(k) {
                            if let Some(s) = v.as_str() {
                                let upper_val = s.to_shouty_snake_case();
                                format!("{enum_type}.{upper_val}")
                            } else {
                                json_to_python_literal(v)
                            }
                        } else {
                            json_to_python_literal(v)
                        };
                        format!("{snake_key}={py_val}")
                    })
                    .collect();
                let constructor = format!("{opts_type}({})", kwargs.join(", "));
                arg_bindings.push(format!("    {var_name} = {constructor}"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
    }
}

fn emit_bytes_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
) {
    if let Some(raw) = value.as_str() {
        match classify_bytes_value(raw) {
            BytesKind::FilePath => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = Path(\"{escaped}\").read_bytes()"));
            }
            BytesKind::InlineText => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = b\"{escaped}\""));
            }
            BytesKind::Base64 => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = base64.b64decode(\"{escaped}\")"));
            }
        }
    } else {
        arg_bindings.push(format!("    {var_name} = None"));
    }
    kwarg_exprs.push(var_name.to_string());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &HashSet::new(), &HashSet::new())
    }

    #[test]
    fn build_args_and_setup_empty_args_returns_empty_vecs() {
        use crate::fixture::Fixture;
        let fixture = Fixture {
            id: "t".to_string(),
            description: "d".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: None,
            visitor: None,
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let call_config = crate::config::CallConfig::default();
        let (bindings, exprs) = build_args_and_setup(
            &fixture,
            &call_config,
            None,
            "kwargs",
            &HashMap::new(),
            &HashMap::new(),
            &HashSet::new(),
            None,
        );
        assert!(bindings.is_empty());
        assert!(exprs.is_empty());
    }

    #[test]
    fn emit_bytes_arg_file_path_uses_path_read_bytes() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::Value::String("pdf/memo.pdf".to_string());
        emit_bytes_arg(&mut bindings, &mut exprs, &value, "content");
        assert!(bindings[0].contains("Path("), "got: {:?}", bindings[0]);
        assert!(bindings[0].contains("read_bytes"), "got: {:?}", bindings[0]);
    }

    #[test]
    fn emit_bytes_arg_base64_uses_b64decode() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::Value::String("/9j/4AAQ".to_string());
        emit_bytes_arg(&mut bindings, &mut exprs, &value, "data");
        assert!(bindings[0].contains("b64decode"), "got: {:?}", bindings[0]);
    }

    #[test]
    fn emit_json_object_arg_dict_mode_emits_literal() {
        let mut bindings = Vec::new();
        let mut exprs = Vec::new();
        let value = serde_json::json!({"key": "val"});
        let done = emit_json_object_arg(&mut bindings, &mut exprs, &value, "opts", None, "dict", &HashMap::new());
        assert!(done);
        assert!(bindings[0].contains("\"key\""), "got: {:?}", bindings[0]);
    }

    #[test]
    fn render_test_function_skipped_fixture_emits_skip_decorator() {
        use crate::fixture::{Fixture, SkipDirective};
        let fixture = Fixture {
            id: "skipped_test".to_string(),
            description: "A skipped test".to_string(),
            input: serde_json::Value::Null,
            http: None,
            assertions: Vec::new(),
            call: None,
            skip: Some(SkipDirective {
                languages: vec!["python".to_string()],
                reason: Some("not supported".to_string()),
            }),
            visitor: None,
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let e2e_config = crate::config::E2eConfig::default();
        let resolver = empty_resolver();
        let mut out = String::new();
        render_test_function(
            &mut out,
            &fixture,
            &e2e_config,
            None,
            "kwargs",
            &HashMap::new(),
            &HashMap::new(),
            &HashSet::new(),
            &resolver,
        );
        assert!(out.contains("pytest.mark.skip"), "got: {out}");
        assert!(out.contains("not supported"), "got: {out}");
    }
}
