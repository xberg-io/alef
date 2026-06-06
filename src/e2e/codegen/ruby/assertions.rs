//! Ruby assertion helpers.

use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

use super::values::json_to_ruby;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
    fields_enum: &HashSet<String>,
    per_call_enum_fields: &HashMap<String, String>,
) {
    // For simple-result methods (e.g. `speech` returning bytes), every field-based
    // assertion targets the result itself — there's no struct to access. Drop
    // length-only assertions onto the result directly and skip anything else.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            if !f.is_empty() {
                match assertion.assertion_type.as_str() {
                    "not_empty" => {
                        out.push_str(&format!("    expect({result_var}.to_s).not_to be_empty\n"));
                        return;
                    }
                    "is_empty" => {
                        out.push_str(&format!("    expect({result_var}.to_s).to be_empty\n"));
                        return;
                    }
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to eq({rb_val})\n"));
                        }
                        return;
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to be >= {rb_val}\n"));
                        }
                        return;
                    }
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}).to eq({rb_val})\n"));
                        }
                        return;
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = crate::e2e::escape::ruby_string_literal(s);
                            out.push_str(&format!("    expect({result_var}).to include({escaped})\n"));
                        }
                        return;
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: field '{f}' not applicable for simple result type\n"
                        ));
                        return;
                    }
                }
            }
        }
    }
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct attribute accesses on the result.
    if let Some(f) = &assertion.field {
        // Skip enum variant accessors (metadata.format.excel etc.) — Magnus serializes
        // FormatMetadata to JSON, so variants are unavailable in Ruby
        if f.contains("metadata.format.") && f.contains(".") {
            out.push_str(&format!(
                "    # skipped: enum variant accessor '{f}' not available on Ruby (serialized to Hash)\n"
            ));
            return;
        }

        // For metadata.format (enum, serialized to Hash), skip since the serialization
        // format differs between languages and doesn't preserve Display formatting
        if f == "metadata.format" {
            out.push_str("    # skipped: metadata.format enum field serialization differs in Ruby\n");
            return;
        }

        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!("({result_var}.chunks || []).all? {{ |c| c.content && !c.content.empty? }}");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        out.push_str(&format!("    expect({pred}).to be(true)\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({pred}).to be(false)\n"));
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                    }
                }
                return;
            }
            "chunks_have_heading_context" | "first_chunk_starts_with_heading" => {
                out.push_str(&format!(
                    "    # skipped: synthetic field '{f}' not available on Ruby Chunk binding\n"
                ));
                return;
            }
            "chunks_have_embeddings" => {
                let pred =
                    format!("({result_var}.chunks || []).all? {{ |c| !c.embedding.nil? && !c.embedding.empty? }}");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        out.push_str(&format!("    expect({pred}).to be(true)\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({pred}).to be(false)\n"));
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns Array<Array<Float>> in Ruby — no wrapper struct.
            // result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to eq({rb_val})\n"));
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({result_var}.length).to be >= {rb_val}\n"));
                        }
                    }
                    "not_empty" => {
                        out.push_str(&format!("    expect({result_var}).not_to be_empty\n"));
                    }
                    "is_empty" => {
                        out.push_str(&format!("    expect({result_var}).to be_empty\n"));
                    }
                    _ => {
                        out.push_str("    # skipped: unsupported assertion type on synthetic field 'embeddings'\n");
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("({result_var}.empty? ? 0 : {result_var}[0].length)");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({expr}).to eq({rb_val})\n"));
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let rb_val = json_to_ruby(val);
                            out.push_str(&format!("    expect({expr}).to be > {rb_val}\n"));
                        }
                    }
                    _ => {
                        out.push_str(
                            "    # skipped: unsupported assertion type on synthetic field 'embedding_dimensions'\n",
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{result_var}.all? {{ |e| !e.empty? }}")
                    }
                    "embeddings_finite" => {
                        format!("{result_var}.all? {{ |e| e.all? {{ |v| v.finite? }} }}")
                    }
                    "embeddings_non_zero" => {
                        format!("{result_var}.all? {{ |e| e.any? {{ |v| v != 0.0 }} }}")
                    }
                    "embeddings_normalized" => {
                        format!("{result_var}.all? {{ |e| n = e.sum {{ |v| v * v }}; (n - 1.0).abs < 1e-3 }}")
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        out.push_str(&format!("    expect({pred}).to be(true)\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({pred}).to be(false)\n"));
                    }
                    _ => {
                        out.push_str(&format!(
                            "    # skipped: unsupported assertion type on synthetic field '{f}'\n"
                        ));
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // Ruby ProcessingResult does not expose result_keywords; skip.
            "keywords" | "keywords_count" => {
                out.push_str(&format!(
                    "    # skipped: field '{f}' not available on Ruby ProcessingResult\n"
                ));
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            out.push_str(&format!("    # skipped: field '{f}' not available on result type\n"));
            return;
        }
    }

    // When result_is_simple, skip assertions that reference non-content fields.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure"))
            {
                return;
            }
        }
    }

    // result_is_simple: treat the result itself as the content string, but only
    // when there is no explicit field (or the field is "content"). Count/length
    // assertions on named fields (e.g. "warnings") must still walk the field path.
    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() && (!result_is_simple || !f.eq_ignore_ascii_case("content")) => {
            field_resolver.accessor(f, "ruby", result_var)
        }
        _ => result_var.to_string(),
    };

    // For string equality, strip trailing whitespace to handle trailing newlines
    // from the converter. Ruby enum fields (Magnus binds Rust enums as Symbols),
    // are coerced to String via .to_s so `eq("stop")` matches `:stop`. Look up the
    // field in both the global `[crates.e2e] fields_enum` set AND the per-call
    // override `[crates.e2e.calls.<x>.overrides.<lang>] enum_fields = { ... }` —
    // project config that already labels e.g. `status = "BatchStatus"` for the
    // Java/C#/Python sides should apply here too without a Ruby-only duplicate.
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f)
            || fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
    });
    // For string equality on simple-result calls we want `.to_s.strip` to absorb
    // trailing whitespace, but for numeric/bool simple results that coercion turns
    // `0` into `"0"` and the `eq(0)` Integer comparison fails. Only fold `.to_s.strip`
    // into the simple-result path when the expected value is a string; otherwise
    // keep the raw expression so numeric/bool comparisons stay typed.
    let expected_is_string = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let stripped_field_expr = if result_is_simple && expected_is_string {
        format!("{field_expr}.to_s.strip")
    } else if field_is_enum {
        format!("{field_expr}.to_s")
    } else {
        field_expr.clone()
    };

    // Detect whether the assertion field resolves to an array type so that
    // contains assertions can iterate items instead of calling .to_s on the array.
    let field_is_array = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let is_boolean_val = expected.as_bool().is_some();
                let bool_val = expected
                    .as_bool()
                    .map(|b| if b { "true" } else { "false" })
                    .unwrap_or("");
                let rb_val = json_to_ruby(expected);
                // Mirror Python's `expr.strip() == expected.strip()` pattern when comparing
                // string values: converters commonly emit a trailing newline that fixture
                // authors don't write into the expected string.
                let cmp_expr = if expected.is_string() && !field_is_enum {
                    format!("{stripped_field_expr}.to_s.strip")
                } else {
                    stripped_field_expr.clone()
                };
                let cmp_expected = if expected.is_string() && !field_is_enum {
                    format!("{rb_val}.strip")
                } else {
                    rb_val
                };

                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "equals",
                        stripped_field_expr => cmp_expr,
                        is_boolean_val => is_boolean_val,
                        bool_val => bool_val,
                        expected_val => cmp_expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains",
                        field_expr => field_expr.clone(),
                        field_is_array => field_is_array && expected.is_string(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let values_list: Vec<String> = values.iter().map(json_to_ruby).collect();
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_all",
                        field_expr => field_expr.clone(),
                        field_is_array => field_is_array,
                        values_list => values_list,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "not_contains",
                        field_expr => field_expr.clone(),
                        field_is_array => field_is_array && expected.is_string(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_empty",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_ruby).collect();
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_any",
                        field_expr => field_expr.clone(),
                        values_list => items,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "greater_than",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "less_than",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "greater_than_or_equal",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let rb_val = json_to_ruby(val);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "less_than_or_equal",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "starts_with",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "ends_with",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "min_length" | "max_length" | "count_min" | "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let rendered = crate::e2e::template_env::render(
                        "ruby/assertion.jinja",
                        minijinja::context! {
                            assertion_type => assertion.assertion_type.as_str(),
                            field_expr => field_expr.clone(),
                            check_n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
        }
        "is_true" => {
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_true",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_false",
                    field_expr => field_expr.clone(),
                },
            );
            out.push_str(&rendered);
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                // Derive call_receiver for module-level helper calls.
                let lang = "ruby";
                let call = &e2e_config.call;
                let overrides = call.overrides.get(lang);
                let module_path = overrides
                    .and_then(|o| o.module.as_ref())
                    .cloned()
                    .unwrap_or_else(|| call.module.clone());
                let call_receiver = super::values::ruby_module_name(&module_path);

                let call_expr =
                    build_ruby_method_call(&call_receiver, result_var, method_name, assertion.args.as_ref());
                let check = assertion.check.as_deref().unwrap_or("is_true");

                let (check_val_str, is_boolean_check, bool_check_val, check_n_val) = match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let is_bool = val.as_bool().is_some();
                            let bool_str = val.as_bool().map(|b| if b { "true" } else { "false" }).unwrap_or("");
                            let rb_val = json_to_ruby(val);
                            (rb_val, is_bool, bool_str.to_string(), 0)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            (json_to_ruby(val), false, String::new(), 0)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            (String::new(), false, String::new(), n)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            (json_to_ruby(val), false, String::new(), 0)
                        } else {
                            (String::new(), false, String::new(), 0)
                        }
                    }
                    _ => (String::new(), false, String::new(), 0),
                };

                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        call_expr => call_expr,
                        check => check,
                        check_val => check_val_str,
                        is_boolean_check => is_boolean_check,
                        bool_check_val => bool_check_val,
                        check_n => check_n_val,
                    },
                );
                out.push_str(&rendered);
            } else {
                panic!("Ruby e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let rb_val = json_to_ruby(expected);
                let rendered = crate::e2e::template_env::render(
                    "ruby/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "matches_regex",
                        field_expr => field_expr.clone(),
                        expected_val => rb_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the example level.
        }
        other => {
            panic!("Ruby e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build a Ruby call expression for a `method_result` assertion on a sample_language Tree.
/// Maps method names to the appropriate Ruby method or module-function calls.
pub(super) fn build_ruby_method_call(
    call_receiver: &str,
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.root_node.child_count"),
        "root_node_type" => format!("{result_var}.root_node.type"),
        "named_children_count" => format!("{result_var}.named_child_count"),
        "has_error_nodes" => format!("{call_receiver}.tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("{call_receiver}.tree_error_count({result_var})"),
        "tree_to_sexp" => format!("{call_receiver}.tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{call_receiver}.find_nodes_by_type({result_var}, \"{node_type}\")")
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
            format!("{call_receiver}.run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => format!("{result_var}.{method_name}"),
    }
}
