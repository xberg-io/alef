use crate::e2e::escape::escape_elixir;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::values::json_to_elixir;

/// Returns true if the field expression is a numeric/integer expression
/// (e.g., a `length(...)` call) rather than a string.
pub(super) fn is_numeric_expr(field_expr: &str) -> bool {
    field_expr.starts_with("length(")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    module_path: &str,
    fields_enum: &HashSet<String>,
    per_call_enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
    is_streaming: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct field accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred =
                    format!("Enum.all?({result_var}.chunks || [], fn c -> c.content != nil and c.content != \"\" end)");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "Enum.all?({result_var}.chunks || [], fn c -> c.embedding != nil and c.embedding != [] end)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "chunks_have_heading_context" => {
                let pred = format!(
                    "Enum.all?({result_var}.chunks || [], fn c -> c.metadata != nil and c.metadata.heading_context != nil end)"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "first_chunk_starts_with_heading" => {
                let expr = format!(
                    "case List.first({result_var}.chunks || []) do
        c when is_map(c) -> String.trim_leading(c.content || \"\") |> String.starts_with?(\"#\")
        _ -> false
      end"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert ({expr})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute ({expr})");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert length({result_var}) == {ex_val}");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert length({result_var}) >= {ex_val}");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "      assert {result_var} != []");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "      assert {result_var} == []");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("(if {result_var} == [], do: 0, else: length(hd({result_var})))");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert {expr} == {ex_val}");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let ex_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert {expr} > {ex_val}");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("Enum.all?({result_var}, fn e -> e != [] end)")
                    }
                    "embeddings_finite" => {
                        format!("Enum.all?({result_var}, fn e -> Enum.all?(e, fn v -> is_float(v) and v == v end) end)")
                    }
                    "embeddings_non_zero" => {
                        format!("Enum.all?({result_var}, fn e -> Enum.any?(e, fn v -> v != 0.0 end) end)")
                    }
                    "embeddings_normalized" => {
                        format!(
                            "Enum.all?({result_var}, fn e -> n = Enum.reduce(e, 0.0, fn v, acc -> acc + v * v end); abs(n - 1.0) < 1.0e-3 end)"
                        )
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "      # skipped: unsupported assertion type on synthetic field '{f}'"
                        );
                    }
                }
                return;
            }
            "keywords" | "keywords_count" => {
                let _ = writeln!(out, "      # skipped: field '{f}' not available on Elixir result type");
                return;
            }
            _ => {}
        }
    }

    if is_streaming {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                if let Some(expr) =
                    crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "elixir", result_var)
                {
                    match assertion.assertion_type.as_str() {
                        "count_min" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert length({expr}) >= {n}");
                            }
                        }
                        "count_equals" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert length({expr}) == {n}");
                            }
                        }
                        "equals" => {
                            if let Some(serde_json::Value::String(s)) = &assertion.value {
                                let escaped = escape_elixir(s);
                                let _ = writeln!(out, "      assert {expr} == \"{escaped}\"");
                            } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert {expr} == {n}");
                            }
                        }
                        "not_empty" => {
                            let _ = writeln!(out, "      assert {expr} != []");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "      assert {expr} == []");
                        }
                        "is_true" => {
                            let _ = writeln!(out, "      assert {expr}");
                        }
                        "is_false" => {
                            let _ = writeln!(out, "      refute {expr}");
                        }
                        "greater_than" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert {expr} > {n}");
                            }
                        }
                        "greater_than_or_equal" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "      assert {expr} >= {n}");
                            }
                        }
                        "contains" => {
                            if let Some(serde_json::Value::String(s)) = &assertion.value {
                                let escaped = escape_elixir(s);
                                let _ = writeln!(out, "      assert String.contains?({expr}, \"{escaped}\")");
                            }
                        }
                        _ => {
                            let _ = writeln!(
                                out,
                                "      # streaming field '{f}': assertion type '{}' not rendered",
                                assertion.assertion_type
                            );
                        }
                    }
                }
                return;
            }
        }
    }

    if !result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
                let _ = writeln!(out, "      # skipped: field '{f}' not available on result type");
                return;
            }
        }
    }

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "elixir", result_var),
            _ => result_var.to_string(),
        }
    };

    let is_numeric = is_numeric_expr(&field_expr);
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f)
            || fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
    });
    let field_is_format_metadata = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| f == "metadata.format" || f.ends_with(".metadata.format"));
    let coerced_field_expr = if field_is_format_metadata {
        format!("alef_e2e_format_to_string({field_expr})")
    } else if field_is_enum {
        format!("to_string({field_expr})")
    } else {
        field_expr.clone()
    };
    let trimmed_field_expr = if is_numeric {
        field_expr.clone()
    } else {
        format!("String.trim({coerced_field_expr})")
    };

    let field_is_array = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let is_string_expected = expected.is_string();
                if is_string_expected && !is_numeric {
                    let _ = writeln!(out, "      assert {trimmed_field_expr} == {elixir_val}");
                } else if field_is_enum {
                    let _ = writeln!(out, "      assert {coerced_field_expr} == {elixir_val}");
                } else {
                    let _ = writeln!(out, "      assert {field_expr} == {elixir_val}");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                if field_is_array && expected.is_string() {
                    let _ = writeln!(
                        out,
                        "      assert Enum.any?({field_expr}, fn item -> Enum.any?(alef_e2e_item_texts(item), &String.contains?(&1, {elixir_val})) end)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      assert String.contains?(to_string({field_expr}), {elixir_val})"
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let elixir_val = json_to_elixir(val);
                    if field_is_array && val.is_string() {
                        let _ = writeln!(
                            out,
                            "      assert Enum.any?({field_expr}, fn item -> Enum.any?(alef_e2e_item_texts(item), &String.contains?(&1, {elixir_val})) end)"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "      assert String.contains?(to_string({field_expr}), {elixir_val})"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                if field_is_array && expected.is_string() {
                    let _ = writeln!(
                        out,
                        "      refute Enum.any?({field_expr}, fn item -> Enum.any?(alef_e2e_item_texts(item), &String.contains?(&1, {elixir_val})) end)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      refute String.contains?(to_string({field_expr}), {elixir_val})"
                    );
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "      assert {field_expr} != \"\"");
        }
        "is_empty" => {
            if is_numeric {
                let _ = writeln!(out, "      assert {field_expr} == 0");
            } else {
                let _ = writeln!(out, "      assert is_nil({field_expr}) or {trimmed_field_expr} == \"\"");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_elixir).collect();
                let list_str = items.join(", ");
                let _ = writeln!(
                    out,
                    "      assert Enum.any?([{list_str}], fn v -> String.contains?(to_string({field_expr}), v) end)"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} > {elixir_val}");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} < {elixir_val}");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} >= {elixir_val}");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {field_expr} <= {elixir_val}");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let _ = writeln!(out, "      assert String.starts_with?({field_expr}, {elixir_val})");
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let _ = writeln!(out, "      assert String.ends_with?({field_expr}, {elixir_val})");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "      assert (is_binary({field_expr}) && byte_size({field_expr}) >= {n}) || (is_list({field_expr}) && length({field_expr}) >= {n}) || (is_binary({field_expr}) == false && is_list({field_expr}) == false && String.length({field_expr}) >= {n})"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "      assert (is_binary({field_expr}) && byte_size({field_expr}) <= {n}) || (is_list({field_expr}) && length({field_expr}) <= {n}) || (is_binary({field_expr}) == false && is_list({field_expr}) == false && String.length({field_expr}) <= {n})"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "      assert length({field_expr}) >= {n}");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "      assert length({field_expr}) == {n}");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "      assert {field_expr} == true");
        }
        "is_false" => {
            let _ = writeln!(out, "      assert {field_expr} == false");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_elixir_method_call(result_var, method_name, assertion.args.as_ref(), module_path);
                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let elixir_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert {call_expr} == {elixir_val}");
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "      assert {call_expr} == true");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      assert {call_expr} == false");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "      assert {call_expr} >= {n}");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "      assert length({call_expr}) >= {n}");
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let elixir_val = json_to_elixir(val);
                            let _ = writeln!(out, "      assert String.contains?({call_expr}, {elixir_val})");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "      assert_raise RuntimeError, fn -> {call_expr} end");
                    }
                    other_check => {
                        panic!("Elixir e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Elixir e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let elixir_val = json_to_elixir(expected);
                let _ = writeln!(out, "      assert Regex.match?(~r/{elixir_val}/, {field_expr})");
            }
        }
        "not_error" => {}
        "error" => {}
        other => {
            panic!("Elixir e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build an Elixir call expression for a `method_result` assertion on a sample_language result.
/// Maps method names to the appropriate `module_path` function calls.
pub(super) fn build_elixir_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    module_path: &str,
) -> String {
    match method_name {
        "root_child_count" => format!("{module_path}.root_child_count({result_var})"),
        "has_error_nodes" => format!("{module_path}.tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{module_path}.tree_error_count({result_var})"),
        "tree_to_sexp" => format!("{module_path}.tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module_path}.tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module_path}.find_nodes_by_type({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{module_path}.run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => format!("{module_path}.{method_name}({result_var})"),
    }
}
