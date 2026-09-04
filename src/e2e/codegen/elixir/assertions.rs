use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
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

/// Build a call to the generated enum module's `wire_value/1` function.
///
/// Enum submodules (`AppModule.EnumName`) live under the app's top-level Elixir module,
/// regardless of which (possibly nested) module a given NIF call is dispatched through --
/// see `enum_module_header.jinja` in the rustler backend, which always nests enum modules one
/// level under `app_module`. `module_path` here is the *call's* module (`E2eConfig`'s
/// `[crates.e2e.call] module`, e.g. `MyLib` or a nested `MyLib.Service`), so only its first
/// dot-segment is the app root the enum module actually lives under. ~keep
fn elixir_enum_wire_value_expr(module_path: &str, enum_type_name: &str, field_expr: &str) -> String {
    let app_root = module_path.split('.').next().unwrap_or(module_path);
    format!("{app_root}.{enum_type_name}.wire_value({field_expr})")
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
    returns_void: bool,
    returns_result: bool,
    not_error_may_assert_presence: bool,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct field accesses on the result.
    if let Some(f) = &assertion.field {
        if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
            let _ = writeln!(out, "      # skipped: {reason}");
            return;
        }

        match f.as_str() {
            "chunks_have_content" => {
                let result_var = &chunks_result_var(field_resolver, "elixir", result_var);
                let pred =
                    format!("Enum.all?({result_var}.chunks || [], fn c -> c.content != nil and c.content != \"\" end)");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "      assert {pred}");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "      refute {pred}");
                    }
                    other => {
                        panic!("Elixir e2e generator: unsupported assertion type '{other}' on synthetic field '{f}'");
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let result_var = &chunks_result_var(field_resolver, "elixir", result_var);
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
                    other => {
                        panic!("Elixir e2e generator: unsupported assertion type '{other}' on synthetic field '{f}'");
                    }
                }
                return;
            }
            "chunks_have_heading_context" => {
                let result_var = &chunks_result_var(field_resolver, "elixir", result_var);
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
                    other => {
                        panic!("Elixir e2e generator: unsupported assertion type '{other}' on synthetic field '{f}'");
                    }
                }
                return;
            }
            "first_chunk_starts_with_heading" => {
                // Same real field as `chunks_have_heading_context` above (`c.metadata.heading_context`),
                // restricted to the first chunk -- not a `content`-prefix proxy. A chunk whose
                // content happens to start with "#" for an unrelated reason (a literal markdown
                // heading in the source, not `prepend_heading_context`) would pass the old proxy
                // and hide a genuine `heading_context` regression. ~keep
                let result_var = &chunks_result_var(field_resolver, "elixir", result_var);
                let expr = format!(
                    "case List.first({result_var}.chunks || []) do
        c when is_map(c) -> c.metadata != nil and c.metadata.heading_context != nil
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
                    other => {
                        panic!("Elixir e2e generator: unsupported assertion type '{other}' on synthetic field '{f}'");
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
                    other => {
                        panic!(
                            "Elixir e2e generator: unsupported assertion type '{other}' on synthetic field 'embeddings'"
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
                    other => {
                        panic!(
                            "Elixir e2e generator: unsupported assertion type '{other}' on synthetic field 'embedding_dimensions'"
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
                    other => {
                        panic!("Elixir e2e generator: unsupported assertion type '{other}' on synthetic field '{f}'");
                    }
                }
                return;
            }
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "      # skipped: {}",
                    FieldSkip::NotAvailableOnElixirResultType.message(f)
                );
                return;
            }
            _ => {}
        }
    }

    if is_streaming
        && let Some(f) = &assertion.field
        && !f.is_empty()
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
        if let Some(expr) =
            crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "elixir", result_var)
        {
            // ~keep Every value-narrowing arm below used to fall through to nothing when the
            // fixture's value did not survive `as_u64()` / the string pattern, so the assertion
            // disappeared with no line for any funnel to count.
            let value_skip = || streaming_assertion_value_skip_line("      ", "#", f, &assertion.assertion_type);
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "      assert length({expr}) >= {n}");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "      assert length({expr}) == {n}");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = escape_elixir(s);
                        let _ = writeln!(out, "      assert {expr} == \"{escaped}\"");
                    } else if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "      assert {expr} == {n}");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "not_empty" => {
                    let _ = writeln!(out, "      assert {expr} not in [nil, \"\", [], %{{}}]");
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
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "      assert {expr} >= {n}");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = escape_elixir(s);
                        let _ = writeln!(out, "      assert String.contains?({expr}, \"{escaped}\")");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "{}",
                        streaming_assertion_type_skip_line("      ", "#", f, &assertion.assertion_type)
                    );
                }
            }
        } else {
            // ~keep The accessor returns `None` for reachable inputs (a `stream.has_*_event`
            // predicate never resolves through `accessor`, which supplies no item type), and this
            // branch used to be absent: the assertion vanished with no line for
            // `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the gap,
            // so it is counted, never fatal.
            let _ = writeln!(
                out,
                "      # skipped: {}",
                FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            );
        }
        return;
    }

    if !result_is_simple
        && let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "      # skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return;
    }

    // A `foo[].bar` fixture path means "some element of foo satisfies this", but
    // `FieldResolver::accessor` lowers `[]` to index 0 (`Enum.at(result.foo, 0).bar`),
    // which silently checks only the first element and reads as coverage. Quantify over
    // every element instead. Explicit numeric indices (`foo[2].bar`) are a separate,
    // correct feature and keep their `Enum.at/2` lowering. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a
        // second wildcard in `elem_part` that the element accessor below lowers to index 0. ~keep
        if let Some(line) = nested_wildcard_skip_line("      ", "#", f, &elem_part) {
            let _ = writeln!(out, "{line}");
            return;
        }
        let array_accessor = if array_part.is_empty() {
            result_var.to_string()
        } else {
            field_resolver.accessor(&array_part, "elixir", result_var)
        };
        let elem_accessor = if elem_part.is_empty() {
            "e".to_string()
        } else {
            field_resolver.element_accessor(&elem_part, "elixir", "e")
        };
        render_wildcard_assertion(out, assertion, &array_accessor, &elem_accessor, f);
        return;
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
    // `fields_enum`/`per_call_enum_fields` carry the hand-maintained config. When neither
    // names the field, fall back to the IR-derived classification (`with_ir_enum_map`,
    // anchored at the call's declared Rust return type via `resolve_declared_result_type`) so
    // a consumer that never configured either still gets a correct classification instead of
    // the dynamically-typed default `to_string`-less comparison, which asserts the NIF's atom
    // (`:key_value`) against the fixture's wire string (`"key_value"`) and silently evaluates
    // to `false` rather than failing to compile. This is purely additive. ~keep
    let field_is_enum = assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        fields_enum.contains(f)
            || fields_enum.contains(resolved)
            || per_call_enum_fields.contains_key(f)
            || per_call_enum_fields.contains_key(resolved)
            || field_resolver.is_enum(f)
    });
    let field_is_format_metadata = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| f == "metadata.format" || f.ends_with(".metadata.format"));

    // Check if this field is configured as display_as_text (e.g., AssistantContent struct
    // with a .text field). When true, access the .text property and nil-guard to empty string.
    let field_is_display_as_text = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_display_as_text(f));

    let coerced_field_expr = if field_is_format_metadata {
        format!("alef_e2e_format_to_string({field_expr})")
    } else if field_is_display_as_text {
        // For display_as_text fields (e.g., content: AssistantContent), access .text
        // and provide empty string as fallback when nil
        format!("(({field_expr} && {field_expr}.text) || \"\")")
    } else if field_is_enum {
        // The binding exposes the exact serde wire value via `<Enum>.wire_value/1` (see
        // `gen_elixir_enum_module_with_known_types` in the rustler backend) rather than
        // `to_string/1`: `to_string(:key_value)` returns the atom's own Elixir spelling
        // ("key_value"), not the wire value ("KeyValue") the fixture literal carries, and a
        // data-carrying enum's flat-struct/tuple runtime shape has no `String.Chars` impl at
        // all. Compare the fixture literal verbatim -- no lowering on our side -- against
        // whatever `wire_value/1` returns. `ir_enum_type_name` only resolves when the IR
        // positively confirms the field's enum type (see `field_is_enum` above); a field
        // classified as enum only through the hand-maintained `fields_enum`/`enum_fields`
        // config (enum type name unknown) keeps the previous `to_string/1` behavior rather
        // than guessing a module path. ~keep
        match assertion
            .field
            .as_deref()
            .filter(|f| !f.is_empty())
            .and_then(|f| field_resolver.ir_enum_type_name(f))
        {
            Some(enum_type_name) => elixir_enum_wire_value_expr(module_path, &enum_type_name, &field_expr),
            None => format!("to_string({field_expr})"),
        }
    } else {
        field_expr.clone()
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
                    let _ = writeln!(out, "      assert {coerced_field_expr} == {elixir_val}");
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
            for expected in assertion.expected_values() {
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
            let _ = writeln!(out, "      assert {field_expr} not in [nil, \"\", [], %{{}}]");
        }
        "is_empty" => {
            if is_numeric {
                let _ = writeln!(out, "      assert {field_expr} == 0");
            } else {
                let _ = writeln!(out, "      assert {coerced_field_expr} in [nil, \"\", [], %{{}}]");
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
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(
                    out,
                    "      assert (is_binary({field_expr}) && byte_size({field_expr}) >= {n}) || (is_list({field_expr}) && length({field_expr}) >= {n})"
                );
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(
                    out,
                    "      assert (is_binary({field_expr}) && byte_size({field_expr}) <= {n}) || (is_list({field_expr}) && length({field_expr}) <= {n})"
                );
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "      assert length({field_expr}) >= {n}");
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "      assert length({field_expr}) == {n}");
            }
        }
        "is_true" => {
            let field_is_optional = assertion
                .field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && field_resolver.is_optional(f));
            if field_is_optional {
                // Optional field: "is_true" means "present" -- `== true` never matches a
                // present non-boolean value (e.g. a map), so it always fails even when the
                // field is present. `refute is_nil(...)` is the interpretation that holds
                // for any value, matching the Rust `.is_some()` convention. ~keep
                let _ = writeln!(out, "      refute is_nil({field_expr})");
            } else {
                let _ = writeln!(out, "      assert {field_expr} == true");
            }
        }
        "is_false" => {
            let field_is_optional = assertion
                .field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && field_resolver.is_optional(f));
            if field_is_optional {
                let _ = writeln!(out, "      assert is_nil({field_expr})");
            } else {
                let _ = writeln!(out, "      assert {field_expr} == false");
            }
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
        "not_error" => {
            // `test_case.rs` binds the call result via `{:ok, result} = call(...)` only when
            // `returns_result` is true — for that shape the match IS a real, meaningful check:
            // an `{:error, _}` return fails the match with a `MatchError`, failing the test.
            // A `returns_void` call taking that shape is the one exception within it: rustler
            // encodes a Rust `()` success payload as the Elixir atom `nil`
            // (`TypeRef::Unit => "nil"` in `backends/rustler/gen_bindings`), so `result` is
            // `nil` on every SUCCESSFUL void call, not just a failed one — `refute
            // is_nil(result)` there would fail every passing call, so this arm renders nothing
            // and relies on the match above, the same way Rust's `.expect()` and Gleam's `let
            // assert Ok(...)` already do for their own void calls.
            //
            // When `returns_result` is false there is no tuple and no match to raise on
            // failure — the NIF's wire value IS the whole story. Two such shapes exist:
            // a `returns_void` call whose fallible NIF returns a bare atom directly (rustler
            // convention: `Ok(_) => atom("ok")`, `Err(_) => atom("error")`, with no `Result`
            // wrapper for Rustler to auto-tuple), and a non-void call whose bare success value
            // could just as easily have arrived as that same `:error` sentinel. Neither has a
            // match to lean on, so both need a real assertion of their own: `assert result ==
            // :ok` for the void case, and `refute result in [nil, :error]` — strictly stronger
            // than a plain nil check — for the non-void case. ~keep
            //
            // WHETHER the non-void check may render at all is decided once, centrally, by
            // `not_error_presence::may_assert_presence` — a sibling assertion or an
            // `Option<T>` result both make it unsafe (e.g. a bare `Option<T>`-returning call
            // whose success path legitimately returns `None` -> Elixir `nil` would make this
            // arm directly contradict a sibling `is_empty`'s `assert is_nil(...)` on the same
            // variable). The caller (`test_case.rs`) already underscore-prefixes
            // `actual_result_var` when this renders nothing, so the unused-variable warning
            // this arm dodges by asserting is dodged there instead. This arm only decides how.
            // ~keep
            if returns_void {
                if !returns_result {
                    let _ = writeln!(out, "      assert {result_var} == :ok");
                }
            } else if not_error_may_assert_presence {
                if returns_result {
                    let _ = writeln!(out, "      refute is_nil({result_var})");
                } else {
                    let _ = writeln!(out, "      refute {result_var} in [nil, :error]");
                }
            }
        }
        // ~keep Unreachable by construction: `expects_error` in test_case.rs is true
        // whenever any assertion is type "error", and every such fixture returns early
        // (validation_creation_failure or the plain expects_error branch) before the
        // assertions loop that calls render_assertion is ever reached — so `result_var`
        // here is always the already-unwrapped Ok value, never the error tuple this
        // assertion type names. The declared error value is preserved at the two call
        // sites in test_case.rs (`emit_error_assertion`), not here.
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

/// Render the `foo[].bar` wildcard forms as an `Enum.any?/2` quantifier over every
/// element of the array, rather than an index-0 lookup. The array expression is
/// nil-guarded with `|| []` because an absent optional list surfaces as `nil` and
/// `Enum.any?/2` would raise on it.
fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    array_accessor: &str,
    elem_accessor: &str,
    field: &str,
) {
    let guarded = format!("({array_accessor} || [])");
    let any_expr = |elixir_val: &str| {
        format!("Enum.any?({guarded}, fn e -> String.contains?(to_string({elem_accessor}), {elixir_val}) end)")
    };
    match assertion.assertion_type.as_str() {
        "contains" => {
            if let Some(val) = &assertion.value {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      assert {}", any_expr(&elixir_val));
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let elixir_val = json_to_elixir(val);
                    let _ = writeln!(out, "      assert {}", any_expr(&elixir_val));
                }
            }
        }
        "not_contains" => {
            for val in assertion.expected_values() {
                let elixir_val = json_to_elixir(val);
                let _ = writeln!(out, "      refute {}", any_expr(&elixir_val));
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "      assert Enum.any?({guarded}, fn e -> to_string({elem_accessor}) != \"\" end)"
            );
        }
        other => {
            let _ = writeln!(
                out,
                "      # skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::e2e::field_access::FieldResolver;
    use std::collections::{HashMap, HashSet};

    use super::render_assertion;
    use crate::e2e::codegen::assertion_type_skip::{AssertionTypeSkip, streaming_assertion_type_skip_line};
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

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `elixir/test_case.rs` now
    /// threads `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn elixir_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
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
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
        assert!(!out.contains("skipped"), "got: {out}");
    }

    /// `first_chunk_starts_with_heading` must assert the real `metadata.heading_context`
    /// field, matching `chunks_have_heading_context` a few lines above it in the generator —
    /// not a `content`-prefix proxy (`String.starts_with?(c.content, "#")`), which can pass on
    /// a chunk whose heading metadata was never attached (a literal "#" in unrelated source
    /// content) and would not catch a regression that dropped `heading_context` entirely.
    #[test]
    fn first_chunk_starts_with_heading_asserts_the_real_field_not_a_content_proxy() {
        let assertion = Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("first_chunk_starts_with_heading".to_string()),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &empty_resolver(),
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
        assert!(
            out.contains("c.metadata.heading_context != nil"),
            "must read the real field, got: {out}"
        );
        assert!(
            !out.contains("starts_with?") && !out.contains("\"#\""),
            "must not fall back to a content-prefix proxy, got: {out}"
        );
    }

    /// The negative-control half of the same regression: `internal_diagnostics`
    /// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
    /// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
    /// NOT `#[serde(skip)]`, which alone does not exclude a field from the
    /// binding surface. Even though it is listed in `result_fields` (a stale/
    /// wrong config entry), the IR must still win and reject it. ~keep
    #[test]
    fn elixir_ir_excluded_field_present_in_result_fields_is_still_skipped() {
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
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    /// Regression test for a one-sided-trim bug: `String.trim/1` wrapped the actual value
    /// while the fixture `expected` literal was emitted verbatim. Fixture expectations may
    /// legitimately end in `\n`, so trimming only one side made those assertions impossible
    /// to satisfy — and trimming both would silently mask real trailing-whitespace
    /// regressions. Equals is exact: neither side is normalized.
    #[test]
    fn render_assertion_equals_string_compares_exactly_without_trim() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(serde_json::Value::String("hello\n".into())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            true,
            false,
            false,
            false,
            true,
        );
        assert!(
            !out.contains("String.trim("),
            "equals must not trim either side; got: {out}"
        );
        assert!(out.contains("assert result =="), "got: {out}");
    }

    /// Control for the trim fix: the tightened contract must still DISCRIMINATE values that
    /// differ only in trailing whitespace. If either side were normalized, the emitted
    /// assertion for "hello\n" and for "hello" would be identical and a real trailing-newline
    /// regression would pass unnoticed.
    /// Control for the `is_empty` leniency: Elixir used to emit `String.trim(actual) == ""`,
    /// which accepts a whitespace-only value like "  \n". Every other backend rejects it
    /// (python emits a falsy check, typescript a `.length` check), so the same fixture
    /// passed in Elixir and failed elsewhere. The emitted check must compare the real value.
    #[test]
    fn render_assertion_is_empty_does_not_trim_actual_value() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "is_empty".to_string(),
            field: None,
            value: None,
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            true,
            false,
            false,
            false,
            true,
        );
        assert!(
            !out.contains("String.trim("),
            "is_empty must not trim the actual value; got: {out}"
        );
        assert_eq!(
            out, "      assert result in [nil, \"\", [], %{}]\n",
            "emitted is_empty check drifted: {out}"
        );
    }

    #[test]
    fn render_assertion_equals_still_discriminates_trailing_whitespace() {
        let render_for = |value: &str| {
            let resolver = empty_resolver();
            let assertion = Assertion {
                assertion_type: "equals".to_string(),
                field: None,
                value: Some(serde_json::Value::String(value.into())),
                ..Default::default()
            };
            let mut out = String::new();
            render_assertion(
                &mut out,
                &assertion,
                "result",
                &resolver,
                "Sample",
                &HashSet::new(),
                &HashMap::new(),
                true,
                false,
                false,
                false,
                true,
            );
            out
        };
        let emitted = render_for("hello\n");
        // The actual side must be the bare expression: any normalizing call (trim/strip/
        // case-folding) wrapped around it would silently accept a mismatched value.
        assert_eq!(
            emitted, "      assert result == \"hello\\n\"\n",
            "emitted assertion drifted: {emitted}"
        );
        // And a value differing only by the trailing newline must still produce a
        // different expectation, proving trailing whitespace is discriminated.
        assert_ne!(
            emitted,
            render_for("hello"),
            "trailing newline must still change the emitted assertion"
        );
    }

    fn render_not_empty(field: Option<&str>, is_streaming: bool) -> String {
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            field: field.map(str::to_string),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &empty_resolver(),
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            is_streaming,
            false,
            false,
            true,
        );
        out
    }

    #[test]
    fn not_empty_rejects_every_empty_shape_for_values_and_streams() {
        assert_eq!(
            render_not_empty(None, false).trim(),
            "assert result not in [nil, \"\", [], %{}]"
        );
        assert_eq!(
            render_not_empty(Some("chunks"), true).trim(),
            "assert result not in [nil, \"\", [], %{}]"
        );
    }

    #[test]
    fn display_as_text_field_accessor_emits_text_property_access() {
        // Build a field resolver with 'content' configured as display_as_text
        let mut display_fields = HashSet::new();
        display_fields.insert("content".to_string());

        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_display_as_text_fields(display_fields);

        // Test that is_display_as_text recognizes the field
        assert!(
            resolver.is_display_as_text("content"),
            "resolver should recognize 'content' as display_as_text"
        );

        // Build the coerced_field_expr as the assertions code does
        let field_expr = "result.content";
        let field_is_display_as_text = resolver.is_display_as_text("content");

        assert!(
            field_is_display_as_text,
            "field_is_display_as_text should be true for 'content'"
        );

        let coerced = if field_is_display_as_text {
            format!("(({field_expr} && {field_expr}.text) || \"\")")
        } else {
            field_expr.to_string()
        };

        // Verify the coerced expression accesses .text with nil-guard
        assert_eq!(
            coerced, "((result.content && result.content.text) || \"\")",
            "display_as_text field should emit nil-guarded .text accessor"
        );
    }

    #[test]
    fn non_display_as_text_field_accessor_uses_bare_expression() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_display_as_text_fields(HashSet::new());

        let field_expr = "result.content";
        let field_is_display_as_text = resolver.is_display_as_text("content");

        assert!(
            !field_is_display_as_text,
            "field_is_display_as_text should be false when not in display_as_text set"
        );

        let coerced = if field_is_display_as_text {
            format!("(({field_expr} && {field_expr}.text) || \"\")")
        } else {
            field_expr.to_string()
        };

        // Verify the coerced expression does NOT access .text
        assert_eq!(
            coerced, "result.content",
            "non-display_as_text field should use bare expression"
        );
    }

    /// Regression test for the not_error vacuous/unused-variable defect: before this
    /// fix, `not_error` rendered nothing, leaving `test_case.rs`'s `{:ok, result} =
    /// call(...)` binding referenced nowhere — an "unused variable" warning that
    /// `mix compile --warnings-as-errors` promotes to a build failure downstream.
    /// Must emit a real, variable-consuming assertion instead.
    #[test]
    fn not_error_emits_a_real_refute_is_nil_and_consumes_the_binding() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            true,
            true,
        );
        assert_eq!(out, "      refute is_nil(result)\n");
    }

    /// The caller (`test_case.rs`) already substitutes the collected `chunks`
    /// variable for `result_var` on streaming fixtures before calling
    /// `render_assertion` — confirm the not_error arm asserts on whatever
    /// variable it's given rather than hardcoding "result".
    #[test]
    fn not_error_asserts_on_whatever_variable_the_caller_passes() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "chunks",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            true,
            false,
            true,
            true,
        );
        assert_eq!(out, "      refute is_nil(chunks)\n");
    }

    /// Regression test for the void `not_error` defect: before this fix, a `returns_void`
    /// fixture whose only assertion was `not_error` still fell into the `refute is_nil(result)`
    /// branch above — but rustler encodes a Rust `()` success payload as the atom `nil`, so
    /// that assertion FAILED on every successful call, not just an unsuccessful one. Covers
    /// only the `returns_result: true` shape, where `test_case.rs` binds `{:ok, result} =
    /// call(...)`: an `{:error, _}` return raises `MatchError`, failing the test on its own, so
    /// this arm has nothing left to check. See `bare_atom_not_error_tests` for the
    /// `returns_result: false` sibling, where there is no such match to rely on.
    #[test]
    fn void_not_error_emits_nothing_relying_on_the_ok_match_above() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            true,
            true,
            false,
        );
        assert!(
            out.is_empty(),
            "a void call's result is always nil; asserting non-nil would fail every successful \
             call, got: {out}"
        );
    }

    #[test]
    #[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'chunks_have_content'")]
    fn elixir_synthetic_field_unsupported_type_fails_loudly() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "bogus_type".to_string(),
            field: Some("chunks_have_content".to_string()),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
    }

    #[test]
    fn elixir_synthetic_chunks_have_content_supported_type_renders_assertion() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("chunks_have_content".to_string()),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
        assert_eq!(
            out,
            "      assert Enum.all?(result.chunks || [], fn c -> c.content != nil and c.content != \"\" end)\n"
        );
    }

    #[test]
    #[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'embeddings'")]
    fn elixir_synthetic_embeddings_unsupported_type_fails_loudly() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "bogus_type".to_string(),
            field: Some("embeddings".to_string()),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
    }

    #[test]
    fn elixir_synthetic_embeddings_supported_type_renders_assertion() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("embeddings".to_string()),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            false,
            false,
            true,
        );
        assert_eq!(out, "      assert result != []\n");
    }

    /// This asserted a panic until the streaming type gap was routed through the skip funnel.
    /// A panic here aborts the whole consumer regen over one unrenderable assertion, which is a
    /// worse signal than a registered marker: the marker is counted by the census, is attributable
    /// to a named field and type, and the strict gate can still escalate it to a failure. Loud and
    /// fatal are not the same property, and only the first one was wanted. ~keep
    #[test]
    fn elixir_streaming_virtual_field_unsupported_type_is_counted_not_fatal() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "bogus_type".to_string(),
            field: Some("chunks".to_string()),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            true,
            false,
            false,
            true,
        );
        assert_eq!(
            out.trim_end(),
            streaming_assertion_type_skip_line("      ", "#", "chunks", "bogus_type"),
            "the gap must be recorded on the registered wording, not dropped: {out}"
        );
        assert_eq!(
            AssertionTypeSkip::extract_classified(out.trim_end()),
            Some(("bogus_type", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "an emitted marker the type funnel cannot recognise is uncounted, which is the \
             silent drop wearing a comment: {out}"
        );
    }

    #[test]
    fn elixir_streaming_virtual_field_supported_type_renders_assertion() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("chunks".to_string()),
            value: Some(serde_json::Value::from(1)),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            "Sample",
            &HashSet::new(),
            &HashMap::new(),
            false,
            true,
            false,
            false,
            true,
        );
        assert_eq!(out, "      assert length(result) >= 1\n");
    }
}

#[cfg(test)]
#[path = "assertions/skip_marker_tests.rs"]
mod skip_marker_tests;
#[cfg(test)]
#[path = "assertions/wildcard_tests.rs"]
mod wildcard_tests;
