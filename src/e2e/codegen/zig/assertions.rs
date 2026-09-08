use super::*;
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::field_access::is_format_metadata_variant_segment;

mod chunks_synthetic;

/// How an absent JSON key is unwrapped in a generated accessor chain.
///
/// `Panic` emits `.?`, which aborts the whole zig test when the key is missing.
/// That is fine for a path the fixture asserts must exist, but wrong inside a
/// wildcard element check, where "this element has no such key" must mean
/// "this element does not match" and let the loop try the next one. ~keep
#[derive(Clone, Copy)]
enum Unwrap {
    Panic,
    Error,
}

/// Error returned by a wildcard element check when the element lacks the field. ~keep
const WILDCARD_MISSING_FIELD_ERROR: &str = "error.WildcardElementFieldMissing";

fn json_path_expr(result_var: &str, field_path: &str, field_resolver: &FieldResolver) -> String {
    json_path_expr_with(result_var, field_path, Unwrap::Panic, field_resolver)
}

fn json_path_expr_with(result_var: &str, field_path: &str, unwrap: Unwrap, field_resolver: &FieldResolver) -> String {
    let segments: Vec<&str> = field_path.split('.').collect();
    let mut expr = result_var.to_string();
    let mut prev_seg: Option<&str> = None;
    for seg in &segments {
        // Skip variant-name accessor segments that follow a `format` key.
        // FormatMetadata is an internally-tagged enum (`#[serde(tag = "format_type")]`),
        // so variant fields are flattened directly into the format object — there is no
        // intermediate JSON key for the variant name.
        if is_format_metadata_variant_segment(prev_seg, seg) {
            prev_seg = Some(seg);
            continue;
        }
        // Handle array accessor notation:
        //   "links[]"     → bare trailing wildcard with no element field; falls back to
        //                   element 0. A wildcard with an element field ("links[].url")
        //                   never reaches here — the caller lowers it to a loop instead.
        //   "results[0]"  → access the array, then specific index N. ~keep
        if let Some(key) = seg.strip_suffix("[]") {
            expr = format!("{}.array.items[0]", json_get(&expr, key, unwrap, field_resolver));
        } else if let Some(bracket_pos) = seg.find('[') {
            if let Some(end_pos) = seg.find(']')
                && end_pos > bracket_pos + 1
                && end_pos == seg.len() - 1
            {
                let key = &seg[..bracket_pos];
                let idx = &seg[bracket_pos + 1..end_pos];
                if idx.chars().all(|c| c.is_ascii_digit()) {
                    expr = format!("{}.array.items[{idx}]", json_get(&expr, key, unwrap, field_resolver));
                    prev_seg = Some(seg);
                    continue;
                }
                // Non-numeric bracket: HashMap<String, _> key access. FRB / serde
                // serialize maps as JSON objects, so `field[key]` resolves to
                // `.object.get("field").?.object.get("key").?`. Used by nested fixture objects.
                // `metadata.document.open_graph[title]` alias pattern where
                // `open_graph` is a `HashMap<String, String>`.
                expr = json_get(
                    &json_get(&expr, key, unwrap, field_resolver),
                    idx,
                    unwrap,
                    field_resolver,
                );
                prev_seg = Some(seg);
                continue;
            }
            expr = json_get(&expr, seg, unwrap, field_resolver);
        } else {
            expr = json_get(&expr, seg, unwrap, field_resolver);
        }
        prev_seg = Some(seg);
    }
    expr
}

/// One `.object.get("key")` step, unwrapped according to `unwrap`.
///
/// `key` may be a JSON key serde omits entirely for some values even though the
/// underlying Rust field is always present (`#[serde(skip_serializing_if = "...")]` on a
/// required `Vec<T>`/`HashMap<K, V>`, or on an `Option<T>` alongside `Option::is_none`) —
/// see `FieldDef::serde_skip_serializing_if` and `FieldResolver::is_wire_optional_key`.
/// `.?`/`orelse return err` both assume the key is present, which panics/fails the whole
/// test on exactly the values that legitimately triggered the skip. Substituting a null value
/// for a missing wire-optional key is safe regardless of the caller's `unwrap` mode: it
/// matches how this same template already treats a *present* `null` value (the
/// `is_empty`/`not_empty`/`is_true`/`is_false` branches all special-case `.null`), so a
/// wire-optional field renders the same assertion outcome whether serde wrote `null` or
/// omitted the key outright. The fallback must be `std.json.Value{ .null = {} }`, not the bare
/// `.null` enum literal: chained straight into further `.object.get(...)` with no declaration
/// in between, a bare literal has no result type for Zig's peer resolution — an uncaught
/// non-compiling regression in 0.62.7. ~keep
fn json_get(expr: &str, key: &str, unwrap: Unwrap, field_resolver: &FieldResolver) -> String {
    if field_resolver.is_wire_optional_key(key) {
        return format!("({expr}.object.get(\"{key}\") orelse std.json.Value{{ .null = {{}} }})");
    }
    match unwrap {
        Unwrap::Panic => format!("{expr}.object.get(\"{key}\").?"),
        Unwrap::Error => format!("({expr}.object.get(\"{key}\") orelse return {WILDCARD_MISSING_FIELD_ERROR})"),
    }
}

/// Split a resolved field path on its first bracket-wildcard segment.
///
/// `"links[].url"` → `Some(("links", "url"))`. Returns `None` for paths with no
/// wildcard, and for a bare trailing `"links[]"` (which names the array itself,
/// not a per-element field).
fn split_wildcard(field_path: &str) -> Option<(&str, &str)> {
    let pos = field_path.find("[].")?;
    let array_root = &field_path[..pos];
    let element_sub_path = &field_path[pos + 3..];
    if array_root.is_empty() || element_sub_path.is_empty() {
        return None;
    }
    Some((array_root, element_sub_path))
}

/// Emit an "at least one element satisfies the assertion" check over a JSON array.
///
/// The per-element assertion is the ordinary rendered template with its accessor
/// rooted at the loop element, hoisted into a nested function so a failing
/// element is an error that `catch continue` turns into "try the next element"
/// rather than a failure of the whole test. Rendering the same template keeps
/// wildcard and non-wildcard assertions from drifting apart. ~keep
fn render_wildcard_json_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    array_root: &str,
    element_sub_path: &str,
    is_length_access: bool,
    field_resolver: &FieldResolver,
) {
    let field_name = assertion.field.as_deref().unwrap_or_default();

    // A second wildcard inside the element sub-path would need a nested loop.
    // Emitting `items[0]` for it would reintroduce exactly the false green this
    // function exists to remove, so leave a visible gap instead. ~keep
    if let Some(line) = nested_wildcard_skip_line("    ", "//", field_name, element_sub_path) {
        let _ = writeln!(out, "{line}");
        return;
    }

    let element_expr = json_path_expr_with("_wce", element_sub_path, Unwrap::Error, field_resolver);
    // Wildcard loop elements are `Vec<T>` items, not the `Option<T>` field itself, so the
    // per-element assertion has no leaf-optionality of its own to consult here.
    let body = render_json_assertion_template(assertion, &element_expr, is_length_access, false);
    // An assertion type the template has no branch for renders to nothing. The
    // nested function would then never use `_wce`, which Zig rejects as an unused
    // parameter, so this must stay a skip rather than an empty loop. ~keep
    if body.trim().is_empty() {
        let atype = &assertion.assertion_type;
        let _ = writeln!(
            out,
            "    // skipped: assertion '{atype}' on array-wildcard field '{field_name}' not supported in zig"
        );
        return;
    }

    let array_expr = json_path_expr(result_var, array_root, field_resolver);
    let _ = writeln!(out, "    {{");
    let _ = writeln!(out, "        const _WildcardCheck = struct {{");
    let _ = writeln!(out, "            fn check(_wce: std.json.Value) anyerror!void {{");
    for line in body.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "            {line}");
        }
    }
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "        }};");
    let _ = writeln!(out, "        var _wc_found = false;");
    let _ = writeln!(out, "        for ({array_expr}.array.items) |_wc_item| {{");
    let _ = writeln!(out, "            _WildcardCheck.check(_wc_item) catch continue;");
    let _ = writeln!(out, "            _wc_found = true;");
    let _ = writeln!(out, "            break;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(
        out,
        "        try testing.expect(_wc_found); // no element of '{array_root}' matched"
    );
    let _ = writeln!(out, "    }}");
}

/// Render a single assertion for a JSON-struct result (result_is_json_struct = true).
///
/// The `result_var` variable is `*std.json.Value` (pointer to the parsed root object).
/// Field paths are traversed via `.object.get("key").?` chains.
pub(super) fn render_json_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    uses_streaming: bool,
) {
    // Intercept streaming-virtual fields before the result-type validity check,
    // but ONLY when the test is actually using the streaming-virtual path.
    // When `uses_streaming = false` the `chunks` local is never declared, so
    // generating `chunks.items.len` would produce a compile error. Fields like
    // "chunks" that happen to share a streaming-virtual name are regular JSON
    // fields in non-streaming results and must fall through to the JSON path.
    if let Some(f) = &assertion.field
        && uses_streaming
        && !f.is_empty()
        && is_streaming_virtual_field(f)
    {
        if let Some(expr) = StreamingFieldResolver::accessor(f, "zig", "chunks") {
            // ~keep The value-narrowing arms below used to fall through to nothing when the
            // fixture's value did not survive `as_u64()`, so the assertion disappeared with no
            // line for any funnel to count.
            let value_skip = || streaming_assertion_value_skip_line("    ", "//", f, &assertion.assertion_type);
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "    try testing.expect({expr}.len >= {n});");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "    try testing.expectEqual(@as(usize, {n}), {expr}.len);");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = escape_zig(s);
                        let _ = writeln!(out, "    try testing.expectEqualStrings(\"{escaped}\", {expr});");
                    } else if let Some(v) = &assertion.value {
                        let zig_val = json_to_zig(v);
                        let _ = writeln!(out, "    try testing.expectEqual({zig_val}, {expr});");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "not_empty" => {
                    let _ = writeln!(out, "    try testing.expect({expr}.len > 0);");
                }
                "is_true" => {
                    let _ = writeln!(out, "    try testing.expect({expr});");
                }
                "is_false" => {
                    let _ = writeln!(out, "    try testing.expect(!{expr});");
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "{}",
                        streaming_assertion_type_skip_line("    ", "//", f, &assertion.assertion_type)
                    );
                }
            }
        } else {
            // ~keep The accessor returns `None` for reachable inputs — every `stream.has_*_event`
            // predicate does, since this call supplies no item type, and zig's deep
            // `tool_calls[N].…` paths return `None` by design (accessors.rs) — and this branch used
            // to be absent: the assertion vanished with no line for
            // `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the gap, so
            // it is counted, never fatal.
            let _ = writeln!(
                out,
                "    // skipped: {}",
                FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            );
        }
        return;
    }

    // Synthetic `embeddings` field on a JSON-array result (e.g. embed_texts
    // returns `Vec<Vec<f32>>` → JSON `[[...],[...]]`). The field name is a
    // convention from the fixture schema — the JSON value IS the embeddings
    // array. Apply the assertion against `result.array.items` directly. The
    // synthetic path is only used when no explicit result_fields configure
    // `embeddings` as a real struct field.
    if let Some(f) = &assertion.field
        && f == "embeddings"
        && !field_resolver.has_explicit_field("embeddings")
    {
        match assertion.assertion_type.as_str() {
            "count_min" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    let _ = writeln!(out, "    try testing.expect({result_var}.array.items.len >= {n});");
                }
                return;
            }
            "count_equals" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    let _ = writeln!(
                        out,
                        "    try testing.expectEqual(@as(usize, {n}), {result_var}.array.items.len);"
                    );
                }
                return;
            }
            "not_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var}.array.items.len > 0);");
                return;
            }
            "is_empty" => {
                let _ = writeln!(
                    out,
                    "    try testing.expectEqual(@as(usize, 0), {result_var}.array.items.len);"
                );
                return;
            }
            _ => {}
        }
    }

    // Synthesised chunk-inspection virtual fields. These are not real JSON
    // fields but are derived predicates over a result object's `chunks` array.
    // Other backends (python, ruby, java, etc.) compute
    // these inline; zig parses to `std.json.Value`, so we compute them
    // against `result.object.get("chunks").?.array`.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            _ if chunks_synthetic::try_render(out, assertion, result_var, f, field_resolver) => {
                return;
            }
            // `keywords` is a fixture alias that does not map cleanly onto the
            // serialized JSON result shape. Matching the Python codegen, skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "    // skipped: {}",
                    FieldSkip::NotAvailableOnJsonStructResult.message(f)
                );
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return;
    }
    // error/not_error are handled at the call level, not assertion level.
    if matches!(assertion.assertion_type.as_str(), "not_error" | "error") {
        return;
    }

    let raw_field_path = assertion.field.as_deref().unwrap_or("").trim();
    let field_path = if raw_field_path.is_empty() {
        raw_field_path.to_string()
    } else {
        field_resolver.result_relative_path(raw_field_path).to_string()
    };
    let field_path = field_path.trim();

    // "{array_field}.length" → strip suffix; use .array.items.len in the template.
    let (field_path_for_expr, is_length_access) = if let Some(parent) = field_path.strip_suffix(".length") {
        (parent, true)
    } else {
        (field_path, false)
    };

    // Bracket-wildcard path (`foo[].bar`): the fixture means "some element of foo
    // satisfies this". Lowering it to `foo.array.items[0]` checks exactly one
    // element and passes for the wrong reason, so it is handled as a loop instead
    // of as a plain accessor expression. Explicit numeric indices (`foo[0].bar`)
    // are a different, correct feature and do not come through here. ~keep
    if let Some((array_root, element_sub_path)) = split_wildcard(field_path_for_expr) {
        render_wildcard_json_assertion(
            out,
            assertion,
            result_var,
            array_root,
            element_sub_path,
            is_length_access,
            field_resolver,
        );
        return;
    }

    let field_expr = if field_path_for_expr.is_empty() {
        result_var.to_string()
    } else {
        json_path_expr(result_var, field_path_for_expr, field_resolver)
    };
    let field_is_optional = !field_path_for_expr.is_empty() && field_resolver.is_optional(field_path_for_expr);

    // Special-case `metadata.format` equals-string: `FormatMetadata` is an
    // internally-tagged enum serialized as a JSON object (`{"format_type": "image",
    // "format": "PNG", ...}`), so `metadata.format` resolves to a JSON object,
    // not a string. The fixture asserts the `Display` impl: for Image variant
    // emit the inner `format` field; otherwise emit the `format_type` discriminant.
    if field_path_for_expr == "metadata.format"
        && matches!(
            assertion.assertion_type.as_str(),
            "equals" | "contains" | "not_empty" | "is_empty" | "starts_with" | "ends_with"
        )
    {
        let base = json_path_expr(result_var, field_path_for_expr, field_resolver);
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        const _fmt_obj = {base}.object;");
        let _ = writeln!(out, "        const _fmt_type = _fmt_obj.get(\"format_type\").?.string;");
        let _ = writeln!(
            out,
            "        const _fmt_display: []const u8 = if (std.mem.eql(u8, _fmt_type, \"image\")) _fmt_obj.get(\"format\").?.string else _fmt_type;"
        );
        match assertion.assertion_type.as_str() {
            "equals" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expectEqualStrings(\"{escaped}\", _fmt_display);"
                    );
                }
            }
            "contains" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expect(std.mem.indexOf(u8, _fmt_display, \"{escaped}\") != null);"
                    );
                }
            }
            "starts_with" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expect(std.mem.startsWith(u8, _fmt_display, \"{escaped}\"));"
                    );
                }
            }
            "ends_with" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = escape_zig(s);
                    let _ = writeln!(
                        out,
                        "        try testing.expect(std.mem.endsWith(u8, _fmt_display, \"{escaped}\"));"
                    );
                }
            }
            "not_empty" => {
                let _ = writeln!(out, "        try testing.expect(_fmt_display.len > 0);");
            }
            "is_empty" => {
                let _ = writeln!(out, "        try testing.expectEqual(@as(usize, 0), _fmt_display.len);");
            }
            _ => {}
        }
        let _ = writeln!(out, "    }}");
        return;
    }

    out.push_str(&render_json_assertion_template(
        assertion,
        &field_expr,
        is_length_access,
        field_is_optional,
    ));
}

/// Render the JSON-struct assertion template for one already-built `field_expr`.
///
/// Split out of `render_json_assertion` so the wildcard path can render the very
/// same assertion against a loop element instead of against the result root. ~keep
fn render_json_assertion_template(
    assertion: &Assertion,
    field_expr: &str,
    is_length_access: bool,
    field_is_optional: bool,
) -> String {
    // Compute context variables for the template.
    let zig_val = match &assertion.value {
        Some(serde_json::Value::String(s)) => format!("\"{}\"", escape_zig(s)),
        _ => String::new(),
    };
    let is_string_val = matches!(&assertion.value, Some(serde_json::Value::String(_)));
    let is_bool_val = matches!(&assertion.value, Some(serde_json::Value::Bool(_)));
    let bool_val = match &assertion.value {
        Some(serde_json::Value::Bool(b)) if *b => "true",
        _ => "false",
    };
    let is_null_val = matches!(&assertion.value, Some(serde_json::Value::Null));
    let n = assertion.value.as_ref().map(json_to_zig).unwrap_or_default();
    let has_n = assertion.value.as_ref().is_some_and(|v| v.is_number() || v.is_u64());
    // Distinguish float vs integer JSON values: `std.json.Value` exposes
    // `.integer` (i64) and `.float` (f64) as separate variants. Comparing
    // `.integer` against a literal with a fractional part (e.g. `0.9`) is a
    // Zig compile error, so the template must select the right tag.
    let is_float_val = matches!(&assertion.value, Some(serde_json::Value::Number(n)) if !n.is_i64() && !n.is_u64());
    let n_as_i64 = if has_n {
        format!("@as(i64, {})", n)
    } else {
        String::new()
    };
    // For usize comparisons, use i64 if n is negative (can't cast -1 to usize directly).
    // Zig comparison operators handle i64 on both sides implicitly.
    let n_as_usize = if has_n {
        if n.starts_with('-') {
            format!("@as(i64, {})", n)
        } else {
            format!("@as(usize, {})", n)
        }
    } else {
        String::new()
    };
    let n_as_f64 = if is_float_val {
        format!("@as(f64, {})", n)
    } else {
        String::new()
    };
    let values_list: Vec<String> = assertion
        .values
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| {
            if let serde_json::Value::String(s) = v {
                Some(format!("\"{}\"", escape_zig(s)))
            } else {
                None
            }
        })
        .collect();

    crate::e2e::template_env::render(
        "zig/json_assertion.jinja",
        minijinja::context! {
            assertion_type => assertion.assertion_type.as_str(),
            field_expr => field_expr,
            field_is_optional => field_is_optional,
            is_length_access => is_length_access,
            zig_val => zig_val,
            is_string_val => is_string_val,
            is_bool_val => is_bool_val,
            bool_val => bool_val,
            is_null_val => is_null_val,
            n => n,
            n_as_i64 => n_as_i64,
            n_as_usize => n_as_usize,
            n_as_f64 => n_as_f64,
            has_n => has_n,
            is_float_val => is_float_val,
            values_list => values_list,
        },
    )
}

/// Predicate matching `render_assertion`: returns true when the assertion
/// would emit at least one statement that references the result variable.
pub(super) fn assertion_emits_code(assertion: &Assertion, field_resolver: &FieldResolver) -> bool {
    if let Some(f) = &assertion.field {
        if !f.is_empty() && is_streaming_virtual_field(f) {
            // Streaming virtual fields always emit code — they are handled in a
            // dedicated collect path, not skipped.
        } else if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            return false;
        }
    }
    matches!(
        assertion.assertion_type.as_str(),
        "equals"
            | "contains"
            | "contains_all"
            | "not_contains"
            | "not_empty"
            | "is_empty"
            | "starts_with"
            | "ends_with"
            | "min_length"
            | "max_length"
            | "count_min"
            | "count_equals"
            | "is_true"
            | "is_false"
            | "greater_than"
            | "less_than"
            | "greater_than_or_equal"
            | "less_than_or_equal"
            | "contains_any"
    )
}

/// Build setup lines and the argument list for the function call.
///
/// Returns `(setup_lines, args_str, setup_needs_gpa)` where `setup_needs_gpa`
/// is `true` when at least one setup line requires the GPA `allocator` binding.
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_option: bool,
    result_is_simple: bool,
) {
    // Bare-result assertions on `?T` (Optional) translate to null-checks instead
    // of `.len`. Mirrors the same behaviour in kotlin.rs (bare_result_is_option).
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    if bare_result_is_option {
        match assertion.assertion_type.as_str() {
            "is_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var} == null);");
                return;
            }
            "not_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var} != null);");
                return;
            }
            "not_error" => {
                // not_error is covered by `try` propagation — the call would have
                // returned early on error. Emit a comment-only line so the assertion
                // is visible but inert, avoiding contradictory checks when paired
                // with `is_empty` on an Optional result.
                let _ = writeln!(out, "    // not_error: covered by try propagation");
                return;
            }
            "equals" => {
                if let Some(expected) = &assertion.value {
                    let zig_val = json_to_zig(expected);
                    let _ = writeln!(out, "    try testing.expectEqualStrings({zig_val}, {result_var}.?);");
                    return;
                }
            }
            _ => {}
        }
    }
    // Synthetic-field 'embeddings' on a JSON-bytes result (e.g. embed_texts
    // returns `Vec<Vec<f32>>` serialised as JSON). Parse the JSON array and
    // apply count_min/count_equals/not_empty/is_empty against the element count.
    //
    // The Zig binding for `Vec<T>`/`result_is_array` returns `[]u8` (the JSON
    // payload), not a typed struct — so a fixture field named `embeddings` is
    // a convention for "the bare JSON array is the embeddings". Gate on
    // `has_explicit_field` rather than `is_valid_for_result`, because the
    // latter is permissive (returns true) when `result_fields` is empty —
    // which is the common case for these bare-JSON returns and would
    // wrongly route through `result.embeddings.len` direct field access on
    // a `[]u8` slice.
    if let Some(f) = &assertion.field
        && f == "embeddings"
        && !field_resolver.has_explicit_field(f)
    {
        match assertion.assertion_type.as_str() {
            "count_min" | "count_equals" | "not_empty" | "is_empty" => {
                let _ = writeln!(out, "    {{");
                let _ = writeln!(
                    out,
                    "        var _eparse = try std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {result_var}, .{{}});"
                );
                let _ = writeln!(out, "        defer _eparse.deinit();");
                let _ = writeln!(out, "        const _embeddings_len = _eparse.value.array.items.len;");
                match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            let _ = writeln!(out, "        try testing.expect(_embeddings_len >= {n});");
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            let _ = writeln!(
                                out,
                                "        try testing.expectEqual(@as(usize, {n}), _embeddings_len);"
                            );
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "        try testing.expect(_embeddings_len > 0);");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "        try testing.expectEqual(@as(usize, 0), _embeddings_len);");
                    }
                    _ => {}
                }
                let _ = writeln!(out, "    }}");
                return;
            }
            _ => {}
        }
    }

    // When result_is_simple, the Zig binding returns a scalar type like []u8 or ?T.
    // Skip assertions on fields that don't exist on the scalar (e.g., metadata,
    // document, structure fields).
    if result_is_simple && let Some(f) = &assertion.field {
        let f_lower = f.to_lowercase();
        if !f.is_empty()
            && f_lower != "content"
            && (f_lower.starts_with("metadata") || f_lower.starts_with("document") || f_lower.starts_with("structure"))
        {
            let _ = writeln!(
                out,
                "    // skipped: {}",
                FieldSkip::NotAvailableWhenResultIsSimple.message(f)
            );
            return;
        }
    }

    // Synthetic-field 'result' on a bare-string/JSON-bytes return (e.g.
    // `detect_mime_type_from_bytes` returns `String` → Zig `[]u8`). The
    // fixture convention is `field: "result", contains: "pdf"` meaning the
    // bare result itself contains the substring. The Zig binding returns
    // `[]u8`, so the substring check applies directly to `result_var`.
    if let Some(f) = &assertion.field
        && f == "result"
        && !field_resolver.has_explicit_field(f)
    {
        match assertion.assertion_type.as_str() {
            "contains" => {
                if let Some(expected) = &assertion.value {
                    let zig_val = json_to_zig(expected);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {result_var}, {zig_val}) != null);"
                    );
                    return;
                }
            }
            "not_contains" => {
                if let Some(expected) = &assertion.value {
                    let zig_val = json_to_zig(expected);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {result_var}, {zig_val}) == null);"
                    );
                    return;
                }
            }
            "equals" => {
                if let Some(expected) = &assertion.value {
                    let zig_val = json_to_zig(expected);
                    let _ = writeln!(out, "    try testing.expectEqualStrings({zig_val}, {result_var});");
                    return;
                }
            }
            "not_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var}.len > 0);");
                return;
            }
            "is_empty" => {
                let _ = writeln!(out, "    try testing.expectEqual(@as(usize, 0), {result_var}.len);");
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return;
    }

    // `field_resolver.is_enum` consults the hand-maintained `fields_enum`/`enum_fields` config
    // first and, when the config is silent, the IR-derived classification (`with_ir_enum_map`).
    // A Zig enum does not compare against a `[]const u8` literal via `testing.expectEqual` (a
    // type mismatch `zig build` rejects), so an `equals` assertion on an enum-typed field is
    // skipped rather than emitting code that cannot compile. The JSON-struct path needs no such
    // guard: there the field is a raw JSON string and `equals` already compares wire values.
    let field_is_enum = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_enum(f));
    if field_is_enum && assertion.assertion_type == "equals" {
        let f = assertion.field.as_deref().unwrap_or("");
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::EnumEqualsNotSupportedOnZigTypedResult.message(f)
        );
        return;
    }

    let field_expr = match &assertion.field {
        // When result_is_simple, the result is a scalar ([]u8 or ?T, etc.) — any
        // field access on it would fail. Treat all assertions as referring to the
        // result itself.
        _ if result_is_simple => result_var.to_string(),
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "zig", result_var),
        _ => result_var.to_string(),
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(out, "    try testing.expectEqual({zig_val}, {field_expr});");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) != null);"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) != null);"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) == null);"
                );
            } else if let Some(values) = &assertion.values {
                // not_contains with a plural `values` list: assert none of the entries
                // appear in the field. Emit one expect line per needle so failures
                // pinpoint the offending value.
                for val in values {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) == null);"
                    );
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    try testing.expect({field_expr}.len > 0);");
        }
        "is_empty" => {
            let _ = writeln!(out, "    try testing.expect({field_expr}.len == 0);");
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.startsWith(u8, {field_expr}, {zig_val}));"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.endsWith(u8, {field_expr}, {zig_val}));"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    try testing.expect({field_expr}.len >= {n});");
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    try testing.expect({field_expr}.len <= {n});");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    try testing.expect({field_expr}.len >= {n});");
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                // When there is no field (field_expr == result_var), the result
                // is `[]u8` JSON (e.g. batch functions). Parse the JSON array
                // and count its elements; `.len` would give byte count, not item count.
                let has_field = assertion.field.as_deref().is_some_and(|f| !f.is_empty());
                if has_field {
                    let _ = writeln!(out, "    try testing.expectEqual(@as(usize, {n}), {field_expr}.len);");
                } else {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(
                        out,
                        "        var _cparse = try std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {field_expr}, .{{}});"
                    );
                    let _ = writeln!(out, "        defer _cparse.deinit();");
                    let _ = writeln!(
                        out,
                        "        try testing.expectEqual(@as(usize, {n}), _cparse.value.array.items.len);"
                    );
                    let _ = writeln!(out, "    }}");
                }
            }
        }
        "is_true" => {
            if let Some(optional_expr) = field_expr.strip_suffix(".?") {
                // `?T`: "is_true" means "present" -- `field_expr` here already force-unwraps
                // with `.?` (a runtime panic on `null`, before the value is even
                // compared), and even past that, `testing.expect` requires a `bool` so a
                // struct T does not compile. `!= null` on the un-force-unwrapped optional
                // is the interpretation that holds for any T, matching the Rust `.is_some()`
                // convention for this assertion type. ~keep
                let _ = writeln!(out, "    try testing.expect({optional_expr} != null);");
            } else {
                let _ = writeln!(out, "    try testing.expect({field_expr});");
            }
        }
        "is_false" => {
            if let Some(optional_expr) = field_expr.strip_suffix(".?") {
                let _ = writeln!(out, "    try testing.expect({optional_expr} == null);");
            } else {
                let _ = writeln!(out, "    try testing.expect(!{field_expr});");
            }
        }
        "not_error" => {
            // Already handled by the call succeeding.
        }
        "error" => {
            // Handled at the test function level.
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons like `len > -1` when the value is negative: they are always-true
                // tautologies for unsigned types and create invalid Zig code (@as(usize, -1)).
                let is_negative = matches!(val, serde_json::Value::Number(n) if n.as_i64().is_some_and(|i| i < 0));
                if !is_negative {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(out, "    try testing.expect({field_expr} > {zig_val});");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} < {zig_val});");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons like `len >= -1` when the value is negative: they are always-true
                // tautologies for unsigned types and create invalid Zig code (@as(usize, -1)).
                let is_negative = matches!(val, serde_json::Value::Number(n) if n.as_i64().is_some_and(|i| i < 0));
                if !is_negative {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(out, "    try testing.expect({field_expr} >= {zig_val});");
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} <= {zig_val});");
            }
        }
        "contains_any" => {
            // At least ONE of the values must be found in the field (OR logic).
            if let Some(values) = &assertion.values {
                let string_values: Vec<String> = values
                    .iter()
                    .filter_map(|v| {
                        if let serde_json::Value::String(s) = v {
                            Some(format!(
                                "std.mem.indexOf(u8, {field_expr}, \"{}\") != null",
                                escape_zig(s)
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();
                if !string_values.is_empty() {
                    let condition = string_values.join(" or\n        ");
                    let _ = writeln!(out, "    try testing.expect(\n        {condition}\n    );");
                }
            }
        }
        "matches_regex" => {
            let _ = writeln!(out, "    // regex match not yet implemented for Zig");
        }
        "method_result" => {
            let _ = writeln!(out, "    // method_result assertions not yet implemented for Zig");
        }
        other => {
            panic!("Zig e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Zig literal string.
pub(super) fn json_to_zig(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_zig(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_zig).collect();
            format!("&.{{{}}}", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_zig(&json_str))
        }
    }
}

#[cfg(test)]
mod wildcard_tests {
    use super::*;
    use std::collections::HashMap;

    fn resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn assertion(assertion_type: &str, field: &str, value: serde_json::Value) -> Assertion {
        Assertion {
            assertion_type: assertion_type.into(),
            field: Some(field.into()),
            value: Some(value),
            ..Assertion::default()
        }
    }

    fn render(assertion: &Assertion) -> String {
        let mut out = String::new();
        render_json_assertion(&mut out, assertion, "result", &resolver(), false);
        out
    }

    #[test]
    fn wildcard_field_should_emit_a_loop_over_every_element() {
        let rendered = render(&assertion("contains", "links[].url", serde_json::json!("example.com")));
        assert!(
            rendered.contains("for (result.object.get(\"links\").?.array.items) |_wc_item|"),
            "expected a loop over every element, got:\n{rendered}"
        );
        assert!(
            rendered.contains("try testing.expect(_wc_found);"),
            "expected the any-element flag to be asserted, got:\n{rendered}"
        );
    }

    /// CANARY. A fixture whose match lives in element 1 passes only if the generated
    /// code visits element 1. The pre-fix generator emitted `items[0]`, which checks
    /// element 0 and nothing else — so this assertion fails against the old lowering.
    ///
    /// This is a shape canary, not a runtime one: this crate generates Zig but cannot
    /// compile or run it, so "element 1 matches, element 0 does not" cannot be
    /// executed here. What is verifiable is that the emitted code is not pinned to
    /// index 0 and that a non-matching element advances the loop instead of failing
    /// the test. ~keep
    #[test]
    fn wildcard_field_should_not_pin_the_assertion_to_element_zero() {
        let rendered = render(&assertion("contains", "links[].url", serde_json::json!("example.com")));
        assert!(
            !rendered.contains("items[0]"),
            "wildcard must not lower to element 0, got:\n{rendered}"
        );
        assert!(
            rendered.contains("catch continue;"),
            "a non-matching element must advance the loop, not fail the test, got:\n{rendered}"
        );
    }

    #[test]
    fn wildcard_element_accessor_should_error_instead_of_panicking_on_a_missing_key() {
        let rendered = render(&assertion("contains", "links[].url", serde_json::json!("example.com")));
        assert!(
            rendered.contains("orelse return error.WildcardElementFieldMissing"),
            "an element without the key must be skipped, not abort the test, got:\n{rendered}"
        );
    }

    #[test]
    fn explicit_numeric_index_should_still_lower_to_that_index() {
        let rendered = render(&assertion(
            "contains",
            "results[0].url",
            serde_json::json!("example.com"),
        ));
        assert!(
            rendered.contains("result.object.get(\"results\").?.array.items[0].object.get(\"url\").?"),
            "explicit index accessor changed, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("_wc_found"),
            "explicit index must not take the wildcard path, got:\n{rendered}"
        );
    }

    #[test]
    fn plain_field_path_should_be_unchanged() {
        let rendered = render(&assertion("equals", "metadata.title", serde_json::json!("Hello")));
        assert!(
            rendered.contains("result.object.get(\"metadata\").?.object.get(\"title\").?.string"),
            "plain accessor changed, got:\n{rendered}"
        );
        assert!(!rendered.contains("_wc_found"), "got:\n{rendered}");
    }

    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_a_wrong_check() {
        let rendered = render(&assertion(
            "contains",
            "pages[].links[].url",
            serde_json::json!("example.com"),
        ));
        assert!(
            rendered.contains("// skipped: nested array-wildcard field 'pages[].links[].url'"),
            "expected a visible skip, got:\n{rendered}"
        );
        assert!(!rendered.contains("items[0]"), "got:\n{rendered}");
        assert!(!rendered.contains("testing.expect"), "got:\n{rendered}");
    }

    #[test]
    fn wildcard_length_assertion_should_measure_the_element_array() {
        let rendered = render(&assertion("count_min", "links[].tags.length", serde_json::json!(2)));
        assert!(
            rendered.contains("_wce.object.get(\"tags\")") || rendered.contains("(_wce.object.get(\"tags\")"),
            "element accessor must be rooted at the loop element, got:\n{rendered}"
        );
        assert!(rendered.contains("_wc_found"), "got:\n{rendered}");
    }

    #[test]
    fn split_wildcard_should_ignore_a_trailing_bracket_with_no_element_field() {
        assert_eq!(split_wildcard("links[].url"), Some(("links", "url")));
        assert_eq!(split_wildcard("links[]"), None);
        assert_eq!(split_wildcard("results[0].url"), None);
        assert_eq!(split_wildcard("metadata.title"), None);
    }
}

#[cfg(test)]
mod chunks_heading_context_tests {
    use super::*;
    use std::collections::HashMap;

    fn resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn render(field: &str, assertion_type: &str) -> String {
        let assertion = Assertion {
            assertion_type: assertion_type.into(),
            field: Some(field.into()),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_json_assertion(&mut out, &assertion, "result", &resolver(), false);
        out
    }

    /// `heading_context` is reachable via the same `.object.get(...)` mechanism the codegen
    /// already uses for `content`/`embedding` -- it just sits one hop deeper, inside
    /// `chunk.metadata`. This is the positive half: the field must be asserted for real, not
    /// approximated via `content` shape and not left as a comment-only skip.
    #[test]
    fn chunks_have_heading_context_reads_the_real_field_not_a_content_proxy() {
        let rendered = render("chunks_have_heading_context", "is_true");

        assert!(
            !rendered.contains("skipped"),
            "must not skip a reachable field, got:\n{rendered}"
        );
        assert!(
            rendered.contains("\"metadata\"") && rendered.contains("\"heading_context\""),
            "must read the real metadata.heading_context field, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("\"content\""),
            "must not fall back to a content-shape proxy, got:\n{rendered}"
        );
        assert!(
            rendered.contains("try testing.expect(_all);"),
            "is_true must assert the aggregate flag, got:\n{rendered}"
        );
    }

    #[test]
    fn chunks_have_heading_context_is_false_negates_the_aggregate_flag() {
        let rendered = render("chunks_have_heading_context", "is_false");
        assert!(rendered.contains("try testing.expect(!_all);"), "got:\n{rendered}");
    }

    /// A chunk is only proven to carry heading context by an explicit, non-null
    /// `metadata.heading_context` key; every other case (missing `metadata`, `metadata` not an
    /// object, missing `heading_context`, or JSON `null`) must fall through to "no heading" by
    /// construction, not by an extra check that could itself be wrong. `_has_heading` starts
    /// `false` and only one line can flip it to `true`, so absence is the structural default.
    #[test]
    fn chunks_have_heading_context_defaults_to_false_absent_explicit_proof() {
        let rendered = render("chunks_have_heading_context", "is_true");
        assert_eq!(
            rendered.matches("_has_heading = true").count(),
            1,
            "exactly one line may prove heading context present, got:\n{rendered}"
        );
        assert!(
            rendered.contains("var _has_heading = false;"),
            "must default to false, got:\n{rendered}"
        );
    }

    /// `first_chunk_starts_with_heading` must inspect only element 0 -- not every chunk (that
    /// is what `chunks_have_heading_context` is for) and not a `content`-prefix proxy.
    #[test]
    fn first_chunk_starts_with_heading_only_inspects_the_first_chunk() {
        let rendered = render("first_chunk_starts_with_heading", "is_true");

        assert!(
            !rendered.contains("skipped"),
            "must not skip a reachable field, got:\n{rendered}"
        );
        assert!(
            rendered.contains("\"heading_context\""),
            "must read the real field, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("startsWith") && !rendered.contains("\"#\""),
            "must not fall back to a markdown-heading content-prefix proxy, got:\n{rendered}"
        );
        // `only_first` adds exactly one unconditional `break;` beyond the two the loop body
        // already has (the `c != .object` guard and the `!_has_heading` check) -- proving the
        // loop stops after element 0 instead of scanning (and asserting over) every chunk,
        // which is what `chunks_have_heading_context` is for.
        assert_eq!(
            rendered.matches("break;").count(),
            3,
            "expected exactly one extra unconditional break restricting the loop to element 0, got:\n{rendered}"
        );
    }

    /// Negative control for the test above: `chunks_have_heading_context` must NOT carry the
    /// element-0-only `break;` — it is required to check every chunk, not just the first.
    #[test]
    fn chunks_have_heading_context_inspects_every_chunk_not_only_the_first() {
        let rendered = render("chunks_have_heading_context", "is_true");
        assert_eq!(
            rendered.matches("break;").count(),
            2,
            "must not carry the element-0-only break, got:\n{rendered}"
        );
    }
}
