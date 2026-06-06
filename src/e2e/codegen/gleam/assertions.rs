use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToPascalCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;

use super::values::{default_gleam_value_for_optional, json_to_gleam};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_tagged_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    prefix: &str,
    variant: &str,
    suffix: &str,
    field_resolver: &FieldResolver,
    pkg_module: &str,
) {
    let prefix_expr = if prefix.is_empty() {
        result_var.to_string()
    } else {
        format!("{result_var}.{prefix}")
    };

    let constructor = variant.to_pascal_case();
    let module_qualifier = pkg_module;
    let inner_var = "fmt_inner__";

    let full_suffix_path = if prefix.is_empty() {
        format!("{variant}.{suffix}")
    } else {
        format!("{prefix}.{variant}.{suffix}")
    };
    let suffix_is_optional = field_resolver.is_optional(&full_suffix_path);
    let suffix_is_array = field_resolver.is_array(&full_suffix_path);

    let _ = writeln!(out, "  case {prefix_expr} {{");
    let _ = writeln!(
        out,
        "    option.Some({module_qualifier}.{constructor}({inner_var})) -> {{"
    );

    let inner_field_expr = if suffix.is_empty() {
        inner_var.to_string()
    } else {
        format!("{inner_var}.{suffix}")
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                if suffix_is_optional {
                    let default = default_gleam_value_for_optional(&gleam_val);
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> option.unwrap({default}) |> should.equal({gleam_val})"
                    );
                } else {
                    let _ = writeln!(out, "      {inner_field_expr} |> should.equal({gleam_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                if suffix_is_array {
                    let _ = writeln!(out, "      let items__ = {inner_field_expr} |> option.unwrap([])");
                    let _ = writeln!(
                        out,
                        "      items__ |> list.any(fn(item__) {{ string.contains(item__, {gleam_val}) }}) |> should.equal(True)"
                    );
                } else if suffix_is_optional {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> option.unwrap(\"\") |> string.contains({gleam_val}) |> should.equal(True)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                if suffix_is_array {
                    let _ = writeln!(out, "      let items__ = {inner_field_expr} |> option.unwrap([])");
                    for val in values {
                        let gleam_val = json_to_gleam(val);
                        let _ = writeln!(
                            out,
                            "      items__ |> list.any(fn(item__) {{ string.contains(item__, {gleam_val}) }}) |> should.equal(True)"
                        );
                    }
                } else if suffix_is_optional {
                    for val in values {
                        let gleam_val = json_to_gleam(val);
                        let _ = writeln!(
                            out,
                            "      {inner_field_expr} |> option.unwrap(\"\") |> string.contains({gleam_val}) |> should.equal(True)"
                        );
                    }
                } else {
                    for val in values {
                        let gleam_val = json_to_gleam(val);
                        let _ = writeln!(
                            out,
                            "      {inner_field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                        );
                    }
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                if suffix_is_optional {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> option.unwrap(0) |> fn(n__) {{ n__ >= {gleam_val} }} |> should.equal(True)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> fn(n__) {{ n__ >= {gleam_val} }} |> should.equal(True)"
                    );
                }
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                if suffix_is_optional {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> option.unwrap(0) |> fn(n__) {{ n__ > {gleam_val} }} |> should.equal(True)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> fn(n__) {{ n__ > {gleam_val} }} |> should.equal(True)"
                    );
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                if suffix_is_optional {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> option.unwrap(0) |> fn(n__) {{ n__ < {gleam_val} }} |> should.equal(True)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> fn(n__) {{ n__ < {gleam_val} }} |> should.equal(True)"
                    );
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                if suffix_is_optional {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> option.unwrap(0) |> fn(n__) {{ n__ <= {gleam_val} }} |> should.equal(True)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "      {inner_field_expr} |> fn(n__) {{ n__ <= {gleam_val} }} |> should.equal(True)"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if suffix_is_optional {
                        let _ = writeln!(
                            out,
                            "      {inner_field_expr} |> option.unwrap([]) |> list.length |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "      {inner_field_expr} |> list.length |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)"
                        );
                    }
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if suffix_is_optional {
                        let _ = writeln!(
                            out,
                            "      {inner_field_expr} |> option.unwrap([]) |> list.length |> should.equal({n})"
                        );
                    } else {
                        let _ = writeln!(out, "      {inner_field_expr} |> list.length |> should.equal({n})");
                    }
                }
            }
        }
        "not_empty" => {
            if suffix_is_optional {
                let _ = writeln!(
                    out,
                    "      {inner_field_expr} |> option.unwrap([]) |> list.is_empty |> should.equal(False)"
                );
            } else if suffix_is_array {
                let _ = writeln!(out, "      {inner_field_expr} |> list.is_empty |> should.equal(False)");
            } else {
                let _ = writeln!(
                    out,
                    "      {inner_field_expr} |> string.is_empty |> should.equal(False)"
                );
            }
        }
        "is_empty" => {
            if suffix_is_optional {
                let _ = writeln!(
                    out,
                    "      {inner_field_expr} |> option.unwrap([]) |> list.is_empty |> should.equal(True)"
                );
            } else if suffix_is_array {
                let _ = writeln!(out, "      {inner_field_expr} |> list.is_empty |> should.equal(True)");
            } else {
                let _ = writeln!(out, "      {inner_field_expr} |> string.is_empty |> should.equal(True)");
            }
        }
        "is_true" => {
            let _ = writeln!(out, "      {inner_field_expr} |> should.equal(True)");
        }
        "is_false" => {
            let _ = writeln!(out, "      {inner_field_expr} |> should.equal(False)");
        }
        other => {
            let _ = writeln!(
                out,
                "      // tagged-union assertion '{other}' not yet implemented for Gleam"
            );
        }
    }

    let _ = writeln!(out, "    }}");
    let _ = writeln!(
        out,
        "    _ -> panic as \"expected {module_qualifier}.{constructor} format metadata\""
    );
    let _ = writeln!(out, "  }}");
}

pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
    result_is_array: bool,
    pkg_module: &str,
) {
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "  // skipped: field '{f}' not available on result type");
            return;
        }
    }

    if let Some(f) = &assertion.field {
        let has_index = f.contains("[].") || {
            let mut chars = f.chars().peekable();
            let mut found = false;
            while let Some(c) = chars.next() {
                if c == '[' {
                    let mut has_digits = false;
                    while chars.peek().map(|d| d.is_ascii_digit()).unwrap_or(false) {
                        chars.next();
                        has_digits = true;
                    }
                    if has_digits && chars.next() == Some(']') && chars.peek() == Some(&'.') {
                        found = true;
                        break;
                    }
                }
            }
            found
        };
        if has_index {
            let _ = writeln!(
                out,
                "  // skipped: array element field '{f}' not yet supported in Gleam e2e"
            );
            return;
        }
    }

    if let Some(f) = &assertion.field {
        if !f.is_empty() {
            if let Some((prefix, variant, suffix)) = field_resolver.tagged_union_split(f) {
                render_tagged_union_assertion(
                    out,
                    assertion,
                    result_var,
                    &prefix,
                    &variant,
                    &suffix,
                    field_resolver,
                    pkg_module,
                );
                return;
            }
        }
    }

    if let Some(f) = &assertion.field {
        if !f.is_empty() {
            let parts: Vec<&str> = f.split('.').collect();
            let mut opt_prefix: Option<(String, usize)> = None;
            for i in 1..parts.len() {
                let prefix_path = parts[..i].join(".");
                if field_resolver.is_optional(&prefix_path) {
                    opt_prefix = Some((prefix_path, i));
                    break;
                }
            }
            if let Some((optional_prefix, suffix_start)) = opt_prefix {
                let prefix_expr = format!("{result_var}.{optional_prefix}");
                let suffix_parts = &parts[suffix_start..];
                let suffix_str = suffix_parts.join(".");
                let inner_var = "opt_inner__";
                let inner_expr = if suffix_str.is_empty() {
                    inner_var.to_string()
                } else {
                    format!("{inner_var}.{suffix_str}")
                };
                let _ = writeln!(out, "  case {prefix_expr} {{");
                let _ = writeln!(out, "    option.Some({inner_var}) -> {{");
                match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let _ = writeln!(
                                    out,
                                    "      {inner_expr} |> list.length |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)"
                                );
                            }
                        }
                    }
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let _ = writeln!(out, "      {inner_expr} |> list.length |> should.equal({n})");
                            }
                        }
                    }
                    "not_empty" => {
                        let is_arr = field_resolver.is_array(f) || field_resolver.is_array(field_resolver.resolve(f));
                        if is_arr {
                            let _ = writeln!(out, "      {inner_expr} |> list.is_empty |> should.equal(False)");
                        } else {
                            let _ = writeln!(out, "      {inner_expr} |> string.is_empty |> should.equal(False)");
                        }
                    }
                    "min_length" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let _ = writeln!(
                                    out,
                                    "      {inner_expr} |> string.length |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)"
                                );
                            }
                        }
                    }
                    other => {
                        let _ = writeln!(
                            out,
                            "      // optional-prefix assertion '{other}' not yet implemented for Gleam"
                        );
                    }
                }
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    option.None -> should.fail()");
                let _ = writeln!(out, "  }}");
                return;
            }
        }
    }

    let field_is_optional = assertion
        .field
        .as_deref()
        .is_some_and(|f| !f.is_empty() && field_resolver.is_optional(field_resolver.resolve(f)));

    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));
    if field_is_enum && assertion.assertion_type == "equals" {
        let f = assertion.field.as_deref().unwrap_or("");
        let _ = writeln!(
            out,
            "  // skipped: enum field '{f}' comparison not yet supported in Gleam e2e"
        );
        return;
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "gleam", result_var),
        _ => result_var.to_string(),
    };

    let field_is_array = {
        let f = assertion.field.as_deref().unwrap_or("");
        let is_root = f.is_empty();
        (is_root && result_is_array) || field_resolver.is_array(f) || field_resolver.is_array(field_resolver.resolve(f))
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                if field_is_optional {
                    let _ = writeln!(out, "  {field_expr} |> should.equal(option.Some({gleam_val}))");
                } else {
                    let _ = writeln!(out, "  {field_expr} |> should.equal({gleam_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                if field_is_array {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> list.any(fn(item__) {{ string.contains(item__, {gleam_val}) }}) |> should.equal(True)"
                    );
                } else if field_is_optional {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> option.unwrap(\"\") |> string.contains({gleam_val}) |> should.equal(True)"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let gleam_val = json_to_gleam(val);
                    if field_is_optional {
                        let _ = writeln!(
                            out,
                            "  {field_expr} |> option.unwrap(\"\") |> string.contains({gleam_val}) |> should.equal(True)"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "  {field_expr} |> string.contains({gleam_val}) |> should.equal(True)"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.contains({gleam_val}) |> should.equal(False)"
                );
            }
        }
        "not_empty" => {
            if field_is_optional {
                let _ = writeln!(out, "  {field_expr} |> option.is_some |> should.equal(True)");
            } else if field_is_array {
                let _ = writeln!(out, "  {field_expr} |> list.is_empty |> should.equal(False)");
            } else {
                let _ = writeln!(out, "  {field_expr} |> string.is_empty |> should.equal(False)");
            }
        }
        "is_empty" => {
            if field_is_optional {
                let _ = writeln!(out, "  {field_expr} |> option.is_none |> should.equal(True)");
            } else if field_is_array {
                let _ = writeln!(out, "  {field_expr} |> list.is_empty |> should.equal(True)");
            } else {
                let _ = writeln!(out, "  {field_expr} |> string.is_empty |> should.equal(True)");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.starts_with({gleam_val}) |> should.equal(True)"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let gleam_val = json_to_gleam(expected);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> string.ends_with({gleam_val}) |> should.equal(True)"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.length |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> string.length |> fn(n__) {{ n__ <= {n} }} |> should.equal(True)"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "  {field_expr} |> list.length |> fn(n__) {{ n__ >= {n} }} |> should.equal(True)"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "  {field_expr} |> list.length |> should.equal({n})");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "  {field_expr} |> should.equal(True)");
        }
        "is_false" => {
            let _ = writeln!(out, "  {field_expr} |> should.equal(False)");
        }
        "not_error" => {}
        "error" => {}
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> fn(n__) {{ n__ > {gleam_val} }} |> should.equal(True)"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> fn(n__) {{ n__ < {gleam_val} }} |> should.equal(True)"
                );
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> fn(n__) {{ n__ >= {gleam_val} }} |> should.equal(True)"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let gleam_val = json_to_gleam(val);
                let _ = writeln!(
                    out,
                    "  {field_expr} |> fn(n__) {{ n__ <= {gleam_val} }} |> should.equal(True)"
                );
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let vals_list = values.iter().map(json_to_gleam).collect::<Vec<_>>().join(", ");
                let _ = writeln!(
                    out,
                    "  [{vals_list}] |> list.any(fn(v__) {{ string.contains({field_expr}, v__) }}) |> should.equal(True)"
                );
            }
        }
        "matches_regex" => {
            let _ = writeln!(out, "  // regex match not yet implemented for Gleam");
        }
        "method_result" => {
            let _ = writeln!(out, "  // method_result assertions not yet implemented for Gleam");
        }
        other => {
            panic!("Gleam e2e generator: unsupported assertion type: {other}");
        }
    }
}
