//! Ruby assertion helpers.

use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

use super::enum_variant_access;
use super::values::json_to_ruby;

mod chunks_synthetic;

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
    if result_is_simple
        && let Some(f) = &assertion.field
        && !f.is_empty()
    {
        match assertion.assertion_type.as_str() {
            "not_empty" => {
                // `.to_s` stringifies before measuring, so an empty collection becomes "[]"
                // and the check can never fail. Ask the value itself whether it is empty.
                out.push_str(&format!(
                    "    expect({result_var}.respond_to?(:empty?) ? !{result_var}.empty? : !{result_var}.nil?).to be(true)\n"
                ));
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
                    "    # skipped: {}\n",
                    FieldSkip::NotApplicableForSimpleResultType.message(f)
                ));
                return;
            }
        }
    }
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct attribute accesses on the result.
    let mut enum_variant_field_expr = None;
    if let Some(f) = &assertion.field {
        enum_variant_field_expr = enum_variant_access::variant_field_accessor(field_resolver, f, result_var);
        // Magnus lowers any data-carrying enum to a plain Ruby Hash (see
        // `enum_variant_access`'s module doc). A single field under a proven single-payload
        // variant is reachable as a Symbol-keyed Hash entry; every richer traversal keeps the
        // explicit gap marker. Both decisions come from the crate's IR, never a field-name
        // special case. ~keep
        match enum_variant_access::classify(field_resolver, f) {
            enum_variant_access::RubyEnumAccess::VariantAccessorUnavailable if enum_variant_field_expr.is_none() => {
                out.push_str(&format!(
                    "    # skipped: {}\n",
                    FieldSkip::EnumVariantAccessorNotAvailableInRuby.message(f)
                ));
                return;
            }
            enum_variant_access::RubyEnumAccess::SerializedAsHash => {
                out.push_str(&format!(
                    "    # skipped: {}\n",
                    FieldSkip::EnumSerializationDiffersInRuby.message(f)
                ));
                return;
            }
            enum_variant_access::RubyEnumAccess::VariantAccessorUnavailable
            | enum_variant_access::RubyEnumAccess::Available => {}
        }

        match f.as_str() {
            _ if chunks_synthetic::try_render(out, assertion, result_var, f, field_resolver) => {
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
                    "    # skipped: {}\n",
                    FieldSkip::NotAvailableOnRubyProcessingResult.message(f)
                ));
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && enum_variant_field_expr.is_none()
        && !field_resolver.is_valid_for_result(f)
    {
        out.push_str(&format!(
            "    # skipped: {}\n",
            FieldSkip::NotAvailableOnResultType.message(f)
        ));
        return;
    }

    // When result_is_simple, skip assertions that reference non-content fields.
    if result_is_simple && let Some(f) = &assertion.field {
        let f_lower = f.to_lowercase();
        if !f.is_empty()
            && f_lower != "content"
            && (f_lower.starts_with("metadata") || f_lower.starts_with("document") || f_lower.starts_with("structure"))
        {
            return;
        }
    }

    // Bracket-wildcard traversal (`links[].link_type`) means "any element", so it must
    // render an `any?` quantifier. Falling through to `accessor` would lower the wildcard
    // to index 0 and silently assert against only the first element. Keyed off the fixture
    // path alone, never off the `[]`-spelled config sets. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref().filter(|f| !f.is_empty())
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a
        // second wildcard in `elem_part` that the element accessor below lowers to index 0. ~keep
        if let Some(line) = nested_wildcard_skip_line("    ", "#", f, &elem_part) {
            out.push_str(&line);
            out.push('\n');
            return;
        }
        let raw_array_accessor = if array_part.is_empty() {
            result_var.to_string()
        } else {
            field_resolver.accessor(&array_part, "ruby", result_var)
        };
        // A nil array raises NoMethodError on `.any?`; `|| []` makes the quantifier
        // simply false instead. ~keep
        let array_accessor = if !array_part.is_empty() && field_resolver.is_optional(&array_part) {
            format!("({raw_array_accessor} || [])")
        } else {
            raw_array_accessor
        };
        // Passing the block parameter as the result var is what lets a nested element
        // sub-path resolve against the block variable instead of the result. ~keep
        let elem_accessor = field_resolver.element_accessor(&elem_part, "ruby", "e");
        match assertion.assertion_type.as_str() {
            "contains" | "contains_all" | "not_contains" => {
                let expected_bool = if assertion.assertion_type == "not_contains" {
                    "false"
                } else {
                    "true"
                };
                for expected in assertion.expected_values() {
                    let rb_val = json_to_ruby(expected);
                    out.push_str(&format!(
                        "    expect({array_accessor}.any? {{ |e| {elem_accessor}.to_s.include?({rb_val}) }}).to be {expected_bool}\n"
                    ));
                }
            }
            "not_empty" => {
                // ~keep Assert on element *content*, not on the stringified array: `[].to_s`
                // is the non-empty string "[]", so a `.to_s.empty?` check over the array
                // would pass vacuously on an empty array. `any?` is false when empty.
                out.push_str(&format!(
                    "    expect({array_accessor}.any? {{ |e| !{elem_accessor}.to_s.empty? }}).to be true\n"
                ));
            }
            other => {
                out.push_str(&format!(
                    "    # skipped: unsupported traversal assertion '{other}' on '{f}'\n"
                ));
            }
        }
        return;
    }

    // result_is_simple: treat the result itself as the content string, but only
    // when there is no explicit field (or the field is "content"). Count/length
    // assertions on named fields (e.g. "warnings") must still walk the field path.
    let field_expr = match (&assertion.field, enum_variant_field_expr) {
        (_, Some(expr)) => expr,
        (Some(f), None) if !f.is_empty() && (!result_is_simple || !f.eq_ignore_ascii_case("content")) => {
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
    // `fields_enum`/`per_call_enum_fields` carry the hand-maintained config. When neither
    // names the field, fall back to the IR-derived classification (`with_ir_enum_map`,
    // anchored at the call's declared Rust return type via `resolve_declared_result_type`) so
    // a consumer that never configured either still gets a correct classification instead of
    // the dynamically-typed default of comparing the raw Magnus `Symbol` against the fixture's
    // wire `String` — `:key_value == "key_value"` silently evaluates to `false` rather than
    // failing to compile. This is purely additive. ~keep
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f)
            || fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
            || field_resolver.is_enum(f)
    });
    // ~keep String coercion only, never whitespace normalization: coercing a numeric or bool
    // simple result turns `0` into `"0"` and the `eq(0)` Integer comparison fails, so `.to_s`
    // is folded in only when the expected value is a string. The equals arm adds the `.to_s`
    // for that case, so the raw expression is kept here.
    //
    // `result_is_simple` must still defer to `field_is_enum`: a call whose entire return value
    // IS the enum (bare `Status`, asserted via `field: None`/`"content"`) needs the same `.to_s`
    // coercion a struct-nested enum field gets. Dropping `field_is_enum` from this branch made
    // a correctly IR-classified bare-enum result LOSE its `.to_s` (the "equals" arm's own
    // `!field_is_enum` guard then also declined to add one, since it now believed no coercion
    // was needed), silently comparing a Magnus `Symbol` against the fixture's wire `String`. ~keep
    let expected_is_string = assertion.value.as_ref().is_some_and(|v| v.is_string());
    let stripped_field_expr = if result_is_simple && expected_is_string && !field_is_enum {
        field_expr.clone()
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
                // ~keep Coerce to String for comparison but normalize neither side. Ruby used to
                // strip both, which is symmetric and so never produced an unsatisfiable
                // assertion — but it also made a real trailing-whitespace regression invisible
                // in Ruby alone, while every other backend now compares exactly.
                let cmp_expr = if expected.is_string() && !field_is_enum {
                    format!("{stripped_field_expr}.to_s")
                } else {
                    stripped_field_expr.clone()
                };
                let cmp_expected = rb_val;

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
            for expected in assertion.expected_values() {
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
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
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
        "is_true" => {
            let field_is_optional = assertion
                .field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && field_resolver.is_optional(f));
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_true",
                    field_expr => field_expr.clone(),
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let field_is_optional = assertion
                .field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && field_resolver.is_optional(f));
            let rendered = crate::e2e::template_env::render(
                "ruby/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_false",
                    field_expr => field_expr.clone(),
                    field_is_optional => field_is_optional,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::fixture::Assertion;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn render(expected: serde_json::Value, result_is_simple: bool) -> String {
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(expected),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &empty_resolver(),
            result_is_simple,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
        out
    }

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `ruby/spec_file.rs` now
    /// threads `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn ruby_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new(), HashSet::new());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("data".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
        assert!(!out.contains("skipped"), "got: {out}");
    }

    /// The negative-control half of the same regression: `internal_diagnostics`
    /// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
    /// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
    /// NOT `#[serde(skip)]`, which alone does not exclude a field from the
    /// binding surface. Even though it is listed in `result_fields` (a stale/
    /// wrong config entry), the IR must still win and reject it. ~keep
    #[test]
    fn ruby_ir_excluded_field_present_in_result_fields_is_still_skipped() {
        let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(HashSet::new(), excluded, HashSet::new());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("internal_diagnostics".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    /// ~keep Ruby normalized BOTH sides, which is symmetric and so never produced an
    /// unsatisfiable assertion — but it also made a genuine trailing-whitespace regression
    /// invisible in Ruby while every other backend compares exactly.
    #[test]
    fn equals_does_not_normalize_either_side() {
        for simple in [true, false] {
            let out = render(serde_json::Value::String("hello\n".into()), simple);
            assert!(!out.contains(".strip"), "equals must not strip either side; got: {out}");
        }
    }

    /// Control: an exact snapshot, not a `!contains` probe. A `!contains(".strip")` assertion
    /// alone would still pass if the expected literal silently lost its trailing newline, so
    /// pin the whole emitted line on both sides.
    #[test]
    fn equals_emits_the_newline_terminated_expected_verbatim() {
        assert_eq!(
            render(serde_json::Value::String("hello\n".into()), true),
            "    expect(result.to_s).to eq(\"hello\\n\")\n"
        );
    }

    /// Control: a numeric expected value must stay typed — no `.to_s` coercion — or an
    /// `eq(0)` Integer comparison would compare against the String `"0"` and fail.
    #[test]
    fn equals_keeps_numeric_comparisons_typed() {
        let out = render(serde_json::json!(0), true);
        assert!(!out.contains(".to_s"), "numeric equals must stay typed; got: {out}");
        assert!(out.contains("eq(0)"), "got: {out}");
    }

    fn wildcard_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::from(["links".to_string(), "pages".to_string()]),
            &HashSet::from(["links".to_string(), "pages".to_string()]),
            &HashSet::new(),
        )
    }

    fn render_wildcard(assertion_type: &str, field: &str) -> String {
        let assertion = Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: if assertion_type == "not_empty" {
                None
            } else {
                Some(serde_json::json!("internal"))
            },
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &wildcard_resolver(),
            false,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
        out
    }

    /// A bracket-wildcard fixture path means "every element", so the emitted Ruby
    /// must quantify with `any?` over the whole array.
    #[test]
    fn ruby_wildcard_contains_emits_any_over_all_elements() {
        let out = render_wildcard("contains", "links[].link_type");
        assert!(
            out.contains("result.links.any? { |e| e.link_type.to_s.include?("),
            "expected an any-element quantifier, got:\n{out}"
        );
        assert!(
            out.contains("internal"),
            "expected value in the emitted assertion, got:\n{out}"
        );
    }

    /// THE CANARY. A fixture whose match lives only in element 1 is satisfied by an
    /// any-element quantifier and missed by an index-0 lookup. This unit test observes
    /// the emitted source rather than executing it, so it pins the property that makes
    /// the runtime difference: the wildcard must NOT lower to a single-element access.
    /// Pre-fix the wildcard rendered `result.links[0].link_type`, which reads element 0
    /// only and reports a false green; this assertion is red then.
    #[test]
    fn ruby_wildcard_does_not_collapse_to_element_zero() {
        let out = render_wildcard("contains", "links[].link_type");
        assert!(
            !out.contains("[0]"),
            "wildcard must not lower to a single-element access, got:\n{out}"
        );
    }

    /// Regression lock: an explicit numeric index is not a wildcard and must keep
    /// resolving to that exact element.
    #[test]
    fn ruby_explicit_index_still_resolves_to_element_zero() {
        let out = render_wildcard("contains", "links[0].link_type");
        assert!(
            out.contains("result.links[0].link_type"),
            "explicit index 0 must keep its index-preserving accessor, got:\n{out}"
        );
        assert!(
            !out.contains(".any?"),
            "explicit index must not become a quantifier, got:\n{out}"
        );
    }

    /// `[].to_s` is `"[]"` — a non-empty String — so a not_empty traversal that
    /// measured the stringified array would pass vacuously on an empty array. The
    /// emitted check must look at element *content*, which `any?` makes false when
    /// the array is empty.
    #[test]
    fn ruby_wildcard_not_empty_asserts_element_content_not_stringified_array() {
        let out = render_wildcard("not_empty", "links[].link_type");
        assert!(
            out.contains("result.links.any? { |e| !e.link_type.to_s.empty? }"),
            "not_empty must quantify over element content, got:\n{out}"
        );
        assert!(
            !out.contains("result.links.to_s"),
            "must not measure the stringified array, which is never empty, got:\n{out}"
        );
    }

    /// `wildcard_split` consumes the first `[].` only, so before the guard the `any?` ranged
    /// over `pages` while its block read `e.links[0].url` — a whole-array claim that only ever
    /// inspected element zero of the inner array. Pre-guard this test fails: the skip line is
    /// absent and an `any?` over `[0]` is present. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_wildcard("contains", "pages[].links[].url");
        assert_eq!(
            out, "    # skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got:\n{out}"
        );
    }
}
