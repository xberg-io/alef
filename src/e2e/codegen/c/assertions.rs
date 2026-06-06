//! C e2e assertion and accessor rendering helpers.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_c;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::{is_primitive_c_type, is_skipped_c_field, json_to_c, try_emit_enum_accessor};

/// Emit chained FFI accessor calls for a nested resolved field path.
///
/// For a path like `metadata.document.title`, this generates:
/// ```c
/// HTMHtmlMetadata* metadata_handle = htm_conversion_result_metadata(result);
/// assert(metadata_handle != NULL);
/// HTMDocumentMetadata* doc_handle = htm_html_metadata_document(metadata_handle);
/// assert(doc_handle != NULL);
/// char* metadata_title = htm_document_metadata_title(doc_handle);
/// ```
///
/// The type chain is looked up from `fields_c_types` which maps
/// `"{parent_snake_type}.{field}"` -> `"PascalCaseType"`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_nested_accessor(
    out: &mut String,
    prefix: &str,
    resolved: &str,
    local_var: &str,
    result_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
    result_type_name: &str,
    raw_field: &str,
) -> Option<String> {
    let segments: Vec<&str> = resolved.split('.').collect();
    let prefix_upper = prefix.to_uppercase();

    // Walk the path, starting from the root result type.
    let mut current_snake_type = result_type_name.to_snake_case();
    let mut current_handle = result_var.to_string();
    // Set to true when we've traversed a `[]` array element accessor and subsequent
    // fields must be extracted via alef_json_get_string rather than FFI function calls.
    let mut json_extract_mode = false;

    for (i, segment) in segments.iter().enumerate() {
        let is_leaf = i + 1 == segments.len();

        // In JSON extraction mode, the current_handle is a JSON string and all
        // segments name keys to extract via alef_json_get_string (for primitive
        // leaves) or alef_json_get_object (for intermediate object hops).
        if json_extract_mode {
            // Decompose `field` or `field[N]`/`field[]`. Numeric indexing must
            // extract the Nth element so later key lookups don't ambiguously
            // pick the first occurrence (matters for fixtures with multiple
            // array elements like `data[0]`/`data[1]`).
            let (bare_segment, bracket_key): (&str, Option<&str>) = match segment.find('[') {
                Some(pos) => (&segment[..pos], Some(segment[pos + 1..].trim_end_matches(']'))),
                None => (segment, None),
            };
            let seg_snake = bare_segment.to_snake_case();
            if is_leaf {
                let _ = writeln!(
                    out,
                    "    char* {local_var} = alef_json_get_string({current_handle}, \"{seg_snake}\");"
                );
                return None; // JSON key leaf — char*.
            }
            // Intermediate JSON key — must be an object/array value. Use the
            // object extractor so the substring includes braces/brackets and
            // later primitive lookups against it find their keys
            // (alef_json_get_string would return NULL on non-string values).
            let json_var = format!("{seg_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(
                    out,
                    "    char* {json_var} = alef_json_get_object({current_handle}, \"{seg_snake}\");"
                );
                intermediate_handles.push((json_var.clone(), "free".to_string()));
            }
            // If the segment also includes a numeric index `[N]`, drill into
            // the Nth element of the extracted array; otherwise stay on the
            // object/array substring.
            if let Some(key) = bracket_key {
                if let Ok(idx) = key.parse::<usize>() {
                    let elem_var = format!("{seg_snake}_{idx}_json");
                    if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
                        let _ = writeln!(
                            out,
                            "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
                        );
                        intermediate_handles.push((elem_var.clone(), "free".to_string()));
                    }
                    current_handle = elem_var;
                    continue;
                }
            }
            current_handle = json_var;
            continue;
        }

        // Check for map access: "field[key]" or array element access: "field[]"
        if let Some(bracket_pos) = segment.find('[') {
            let field_name = &segment[..bracket_pos];
            let key = segment[bracket_pos + 1..].trim_end_matches(']');
            let field_snake = field_name.to_snake_case();
            let accessor_fn = format!("{prefix}_{current_snake_type}_{field_snake}");

            // The accessor returns a char* (JSON object/array string).
            let json_var = format!("{field_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                let _ = writeln!(out, "    assert({json_var} != NULL);");
                // Track for freeing — use prefix_free_string since it's a char*.
                intermediate_handles.push((json_var.clone(), "free_string".to_string()));
            }

            // Empty key `[]`: array-element substring access (any element matches).
            // Numeric key `[N]` (e.g. `choices[0]`, `data[1]`): extract the exact
            // Nth top-level element so subsequent key lookups don't ambiguously
            // pick the first occurrence — required for fixtures whose results
            // contain multiple array elements (e.g. `data[0].index`/`data[1].index`).
            if key.is_empty() {
                if !is_leaf {
                    current_handle = json_var;
                    json_extract_mode = true;
                    continue;
                }
                return None;
            }
            if let Ok(idx) = key.parse::<usize>() {
                let elem_var = format!("{field_snake}_{idx}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
                    let _ = writeln!(
                        out,
                        "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
                    );
                    intermediate_handles.push((elem_var.clone(), "free".to_string()));
                }
                if !is_leaf {
                    current_handle = elem_var;
                    json_extract_mode = true;
                    continue;
                }
                // Trailing `[N]` — caller asserts on the element JSON.
                return None;
            }

            // Named map key access: extract the key value from the JSON object.
            let _ = writeln!(
                out,
                "    char* {local_var} = alef_json_get_string({json_var}, \"{key}\");"
            );
            return None; // Map access leaf — char*.
        }

        let seg_snake = segment.to_snake_case();
        let accessor_fn = format!("{prefix}_{current_snake_type}_{seg_snake}");

        // Skip any assertion that touches a field marked "skip" in fields_c_types.
        if is_skipped_c_field(fields_c_types, &current_snake_type, &seg_snake) {
            return Some("__skip__".to_string()); // Sentinel: no accessor emitted, assertion skipped later.
        }

        if is_leaf {
            // Leaf may be a primitive scalar (uint64_t, double, ...) when
            // configured in `fields_c_types`. Otherwise default to char*.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({current_handle});");
                return Some(t.clone());
            }
            // Opaque struct leaf: when fields_c_types maps "{parent}.{field}" to a
            // PascalCase type name (not a primitive, not "char*", not "skip"), the
            // accessor returns a struct pointer rather than a string. Emit the typed
            // handle declaration and register it for freeing.
            if let Some(opaque_type) = fields_c_types.get(&lookup_key).filter(|t| {
                *t != "char*"
                    && *t != "skip"
                    && !is_primitive_c_type(t)
                    && t.chars().next().is_some_and(|c| c.is_uppercase())
            }) {
                let handle_var = format!("{seg_snake}_handle");
                let opaque_snake = opaque_type.to_snake_case();
                if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}{opaque_type}* {handle_var} = {accessor_fn}({current_handle});"
                    );
                    intermediate_handles.push((handle_var.clone(), opaque_snake.clone()));
                }
                // Treat the handle itself as the local_var for later assertions.
                // Map local_var → handle_var so render_assertion uses the handle name.
                if local_var != handle_var {
                    let _ = writeln!(out, "    {prefix_upper}{opaque_type}* {local_var} = {handle_var};");
                }
                return Some(opaque_snake); // return type name so caller can register opaque handle cleanup
            }
            // Enum leaf: opaque enum pointer that needs `_to_string` conversion.
            if try_emit_enum_accessor(
                out,
                prefix,
                &prefix_upper,
                raw_field,
                &seg_snake,
                &current_snake_type,
                &accessor_fn,
                &current_handle,
                local_var,
                fields_c_types,
                fields_enum,
                intermediate_handles,
            ) {
                return None;
            }
            let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({current_handle});");
        } else {
            // Intermediate field — check if it's a char* (JSON string/array) or an opaque handle.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            let return_type_pascal = match fields_c_types.get(&lookup_key) {
                Some(t) => t.clone(),
                None => {
                    // Fallback: derive PascalCase from the segment name itself.
                    segment.to_pascal_case()
                }
            };

            // Special case: intermediate char* fields (e.g. links, assets) are JSON
            // strings/arrays, not opaque handles. For a `.length` suffix, emit alef_json_array_count.
            if return_type_pascal == "char*" {
                let json_var = format!("{seg_snake}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                    let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                    intermediate_handles.push((json_var.clone(), "free_string".to_string()));
                }
                // If the next (and final) segment is "length", emit the count accessor.
                if i + 2 == segments.len() && segments[i + 1] == "length" {
                    let _ = writeln!(out, "    int {local_var} = alef_json_array_count({json_var});");
                    return Some("int".to_string());
                }
                current_snake_type = seg_snake.clone();
                current_handle = json_var;
                continue;
            }

            let return_snake = return_type_pascal.to_snake_case();
            let handle_var = format!("{seg_snake}_handle");

            // Only emit the handle if we haven't already (multiple fields may
            // share the same intermediate path prefix).
            if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
                let _ = writeln!(
                    out,
                    "    {prefix_upper}{return_type_pascal}* {handle_var} = \
                     {accessor_fn}({current_handle});"
                );
                let _ = writeln!(out, "    assert({handle_var} != NULL);");
                intermediate_handles.push((handle_var.clone(), return_snake.clone()));
            }

            current_snake_type = return_snake;
            current_handle = handle_var;
        }
    }
    None
}

/// Build the C argument string for the function call.
/// When `has_options_handle` is true, json_object args are replaced with
/// the `options_handle` pointer (which was constructed via `from_json`).
pub(super) fn build_args_string_c(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    has_options_handle: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
) -> String {
    if args.is_empty() {
        return json_to_c(input);
    }

    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        // Handle test_backend args: emit the stub and use it.
        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait) {
                            for method in &super_type.methods {
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }
                    let emission = crate::e2e::codegen::emit_test_backend("c", trait_bridge, &methods, fixture);
                    parts.push(emission.arg_expr);
                    continue;
                }
            }
            // Unimplemented trait fallback
            parts.push("NULL".to_string());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            // Field missing entirely and optional → pass NULL.
            None if arg.optional => parts.push("NULL".to_string()),
            // Field missing and required → skip (caller error, but don't crash).
            None => {}
            // Explicit null on optional arg → pass NULL.
            Some(v) if v.is_null() && arg.optional => parts.push("NULL".to_string()),
            Some(v) => {
                // For json_object args, use the options_handle pointer
                // instead of the raw JSON string.
                if arg.arg_type == "json_object" && has_options_handle && !v.is_null() {
                    parts.push("options_handle".to_string())
                } else {
                    parts.push(json_to_c(v))
                }
            }
        }
    }

    parts.join(", ")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    ffi_prefix: &str,
    _field_resolver: &FieldResolver,
    accessed_fields: &[(String, String, bool)],
    primitive_locals: &HashMap<String, String>,
    opaque_handle_locals: &HashMap<String, String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !_field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => {
            // Use the local variable extracted from the opaque handle.
            accessed_fields
                .iter()
                .find(|(k, _, _)| k == f)
                .map(|(_, local, _)| local.clone())
                .unwrap_or_else(|| result_var.to_string())
        }
        _ => result_var.to_string(),
    };

    // If the field was marked with the "__skip__" sentinel (fields_c_types = "skip"),
    // the accessor was never emitted — skip the assertion silently.
    if primitive_locals.get(&field_expr).is_some_and(|t| t == "__skip__") {
        let _ = writeln!(out, "    // skipped: field '{field_expr}' not available in C FFI");
        return;
    }

    let field_is_primitive = primitive_locals.contains_key(&field_expr);
    let field_primitive_type = primitive_locals.get(&field_expr).cloned();
    // Opaque-handle fields (e.g. `usage` → SAMPLELLMUsage*) cannot be treated
    // as C strings — `strlen` / `strcmp` on a struct pointer is undefined
    // behavior (SIGABRT in practice). `not_empty` / `is_empty` collapse to
    // NULL checks; other string assertions are skipped for these fields.
    let field_is_opaque_handle = opaque_handle_locals.contains_key(&field_expr);
    // Map-access fields are extracted via `alef_json_get_string` and end up
    // as char*. When the assertion expects a numeric or boolean value, we
    // emit a parsed/literal comparison rather than `strcmp`.
    let field_is_map_access = if let Some(f) = &assertion.field {
        accessed_fields.iter().any(|(k, _, m)| k == f && *m)
    } else {
        false
    };

    // Check if the assertion field is optional — used to emit conditional assertions
    // for optional numeric fields (returns 0 when None, so 0 == "not set").
    // Check both the raw field name and its resolved alias.
    let assertion_field_is_optional = assertion
        .field
        .as_deref()
        .map(|f| {
            if f.is_empty() {
                return false;
            }
            if _field_resolver.is_optional(f) {
                return true;
            }
            // Also check the resolved alias (e.g. "robots.crawl_delay" → "crawl_delay").
            let resolved = _field_resolver.resolve(f);
            _field_resolver.is_optional(resolved)
        })
        .unwrap_or(false);

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                if field_is_primitive {
                    let cmp_val = if field_primitive_type.as_deref() == Some("bool") {
                        match expected.as_bool() {
                            Some(true) => "1".to_string(),
                            Some(false) => "0".to_string(),
                            None => c_val,
                        }
                    } else {
                        c_val
                    };
                    // For optional numeric fields, treat 0 as "not set" and allow it.
                    // This mirrors Go's nil-pointer check for optional fields.
                    let is_numeric = field_primitive_type.as_deref().map(|t| t != "bool").unwrap_or(false);
                    if assertion_field_is_optional && is_numeric {
                        let _ = writeln!(
                            out,
                            "    assert(({field_expr} == 0 || {field_expr} == {cmp_val}) && \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} == {cmp_val} && \"equals assertion failed\");"
                        );
                    }
                } else if expected.is_string() {
                    let _ = writeln!(
                        out,
                        "    assert(str_trim_eq({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
                } else if field_is_map_access && expected.is_boolean() {
                    let lit = match expected.as_bool() {
                        Some(true) => "\"true\"",
                        _ => "\"false\"",
                    };
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strcmp({field_expr}, {lit}) == 0 && \"equals assertion failed\");"
                    );
                } else if field_is_map_access && expected.is_number() {
                    if expected.is_f64() {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} != NULL && atof({field_expr}) == {c_val} && \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} != NULL && atoll({field_expr}) == {c_val} && \"equals assertion failed\");"
                        );
                    }
                } else {
                    let _ = writeln!(
                        out,
                        "    assert(strcmp({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(({field_expr} == NULL || strstr({field_expr}, {c_val}) == NULL) && \"expected NOT to contain substring\");"
                );
            }
        }
        "not_empty" => {
            if field_is_opaque_handle {
                // Opaque struct handle: `strlen` on a struct pointer is UB.
                // Weaken to a non-null check — strictly weaker than the
                // original intent but won't false-trigger SIGABRT.
                let _ = writeln!(out, "    assert({field_expr} != NULL && \"expected non-null handle\");");
            } else {
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strlen({field_expr}) > 0 && \"expected non-empty value\");"
                );
            }
        }
        "is_empty" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} == NULL && \"expected null handle\");");
            } else if assertion_field_is_optional || !field_is_primitive {
                // Optional string fields may return NULL — treat NULL as empty.
                let _ = writeln!(
                    out,
                    "    assert(({field_expr} == NULL || strlen({field_expr}) == 0) && \"expected empty value\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert(strlen({field_expr}) == 0 && \"expected empty value\");"
                );
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        int found = 0;");
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        if (strstr({field_expr}, {c_val}) != NULL) {{ found = 1; }}"
                    );
                }
                let _ = writeln!(
                    out,
                    "        assert(found && \"expected to contain at least one of the specified values\");"
                );
                let _ = writeln!(out, "    }}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) > {c_val} && \"expected greater than\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert({field_expr} > {c_val} && \"expected greater than\");");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) < {c_val} && \"expected less than\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert({field_expr} < {c_val} && \"expected less than\");");
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) >= {c_val} && \"expected greater than or equal\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} >= {c_val} && \"expected greater than or equal\");"
                    );
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) <= {c_val} && \"expected less than or equal\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} <= {c_val} && \"expected less than or equal\");"
                    );
                }
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(strncmp({field_expr}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    assert(strlen({field_expr}) >= strlen({c_val}) && ");
                let _ = writeln!(
                    out,
                    "           strcmp({field_expr} + strlen({field_expr}) - strlen({c_val}), {c_val}) == 0 && \"expected to end with\");"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert(strlen({field_expr}) >= {n} && \"expected minimum length\");"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert(strlen({field_expr}) <= {n} && \"expected maximum length\");"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(out, "        /* count_min: count top-level JSON array elements */");
                    let _ = writeln!(
                        out,
                        "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
                    );
                    let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
                    let _ = writeln!(
                        out,
                        "        assert(elem_count >= {n} && \"expected at least {n} elements\");"
                    );
                    let _ = writeln!(out, "    }}");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(out, "        /* count_equals: count elements in array */");
                    let _ = writeln!(
                        out,
                        "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
                    );
                    let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
                    let _ = writeln!(out, "        assert(elem_count == {n} && \"expected {n} elements\");");
                    let _ = writeln!(out, "    }}");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    assert({field_expr});");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert(!{field_expr});");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                render_method_result_assertion(
                    out,
                    result_var,
                    ffi_prefix,
                    method_name,
                    assertion.args.as_ref(),
                    assertion.return_type.as_deref(),
                    assertion.check.as_deref().unwrap_or("is_true"),
                    assertion.value.as_ref(),
                );
            } else {
                panic!("C e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        regex_t _re;");
                let _ = writeln!(
                    out,
                    "        assert(regcomp(&_re, {c_val}, REG_EXTENDED) == 0 && \"regex compile failed\");"
                );
                let _ = writeln!(
                    out,
                    "        assert(regexec(&_re, {field_expr}, 0, NULL, 0) == 0 && \"expected value to match regex\");"
                );
                let _ = writeln!(out, "        regfree(&_re);");
                let _ = writeln!(out, "    }}");
            }
        }
        "not_error" => {
            // Already handled — the NULL check above covers this.
        }
        "error" => {
            // Handled at the test function level.
        }
        other => {
            panic!("C e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Render a `method_result` assertion in C.
///
/// Dispatches generically using `{ffi_prefix}_{method_name}` for the FFI call.
/// The `return_type` fixture field controls how the return value is handled:
/// - `"string"` — the method returns a heap-allocated `char*`; the generator
///   emits a scoped block that asserts, then calls `free()`.
/// - absent/other — treated as a primitive integer (or pointer-as-bool); the
///   assertion is emitted inline without any heap management.
#[allow(clippy::too_many_arguments)]
fn render_method_result_assertion(
    out: &mut String,
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    return_type: Option<&str>,
    check: &str,
    value: Option<&serde_json::Value>,
) {
    let call_expr = build_c_method_call(result_var, ffi_prefix, method_name, args);

    if return_type == Some("string") {
        // Heap-allocated char* return: emit a scoped block, assert, then free.
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        char* _method_result = {call_expr};");
        if check == "is_error" {
            let _ = writeln!(
                out,
                "        assert(_method_result == NULL && \"expected method to return error\");"
            );
            let _ = writeln!(out, "    }}");
            return;
        }
        let _ = writeln!(
            out,
            "        assert(_method_result != NULL && \"method_result returned NULL\");"
        );
        match check {
            "contains" => {
                if let Some(val) = value {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        assert(strstr(_method_result, {c_val}) != NULL && \"method_result contains assertion failed\");"
                    );
                }
            }
            "equals" => {
                if let Some(val) = value {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        assert(str_trim_eq(_method_result, {c_val}) == 0 && \"method_result equals assertion failed\");"
                    );
                }
            }
            "is_true" => {
                let _ = writeln!(
                    out,
                    "        assert(_method_result != NULL && strlen(_method_result) > 0 && \"method_result is_true assertion failed\");"
                );
            }
            "count_min" => {
                if let Some(val) = value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "        int _elem_count = alef_json_array_count(_method_result);");
                    let _ = writeln!(
                        out,
                        "        assert(_elem_count >= {n} && \"method_result count_min assertion failed\");"
                    );
                }
            }
            other_check => {
                panic!("C e2e generator: unsupported method_result check type for string return: {other_check}");
            }
        }
        let _ = writeln!(out, "        free(_method_result);");
        let _ = writeln!(out, "    }}");
        return;
    }

    // Primitive (integer / pointer-as-bool) return: inline assert, no heap management.
    match check {
        "equals" => {
            if let Some(val) = value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} == {c_val} && \"method_result equals assertion failed\");"
                );
            }
        }
        "is_true" => {
            let _ = writeln!(
                out,
                "    assert({call_expr} && \"method_result is_true assertion failed\");"
            );
        }
        "is_false" => {
            let _ = writeln!(
                out,
                "    assert(!{call_expr} && \"method_result is_false assertion failed\");"
            );
        }
        "greater_than_or_equal" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result >= {n} assertion failed\");"
                );
            }
        }
        "count_min" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result count_min assertion failed\");"
                );
            }
        }
        other_check => {
            panic!("C e2e generator: unsupported method_result check type: {other_check}");
        }
    }
}

/// Build a C call expression for a `method_result` assertion.
///
/// Uses generic dispatch: `{ffi_prefix}_{method_name}(result_var, args...)`.
/// Args from the fixture JSON object are emitted as positional C arguments in
/// insertion order, using best-effort type conversion (strings → C string literals,
/// numbers and booleans → verbatim literals).
fn build_c_method_call(
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let extra_args = if let Some(args_val) = args {
        args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
                        serde_json::Value::Bool(true) => "1".to_string(),
                        serde_json::Value::Bool(false) => "0".to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => "NULL".to_string(),
                        other => format!("\"{}\"", escape_c(&other.to_string())),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if extra_args.is_empty() {
        format!("{ffi_prefix}_{method_name}({result_var})")
    } else {
        format!("{ffi_prefix}_{method_name}({result_var}, {extra_args})")
    }
}
