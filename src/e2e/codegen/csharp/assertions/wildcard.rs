//! Wildcard traversal assertions for C# generated tests.

use crate::e2e::codegen::field_skip::nested_wildcard_skip_line;
use crate::e2e::escape::escape_csharp;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};

/// Lambda-parameter suffix keyed to the assertion.
///
/// Generated C# test methods bind locals named after fixture fields, and two wildcard
/// assertions in one method must not reuse a parameter name that shadows one. Hashing the
/// assertion's discriminating fields keeps it unique and stable across regenerations. ~keep
fn wildcard_lambda_param(assertion: &Assertion) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    assertion.assertion_type.hash(&mut hasher);
    assertion.field.hash(&mut hasher);
    assertion
        .value
        .as_ref()
        .map(std::string::ToString::to_string)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("e{:x}", hasher.finish() & 0xffff_ffff)
}

/// Emit `Assert.True(<array>?.Any(e => …) == true)` for a bracket-wildcard path.
///
/// The null-conditional `?.` plus `== true` covers both nullable and non-nullable list
/// getters without needing the element type name to build an empty-list fallback. ~keep
pub(super) fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    field: &str,
    array_part: &str,
    elem_part: &str,
) {
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("        ", "//", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "csharp", result_var)
    };
    let param = wildcard_lambda_param(assertion);
    // `element_accessor`, not `accessor`: the path is already element-relative, so the
    // result-anchoring `accessor` applies would re-prefix it with the container. ~keep
    let elem_accessor = field_resolver.element_accessor(elem_part, "csharp", &param);

    let enum_leaf = field_resolver.is_enum(field);
    let text_accessor = if enum_leaf {
        let options = super::super::UNESCAPED_JSON_SERIALIZER_OPTIONS;
        format!("System.Text.Json.JsonSerializer.Serialize({elem_accessor}, {options}).Trim('\"')")
    } else {
        format!("Convert.ToString({elem_accessor})!")
    };
    let null_guard = if enum_leaf {
        format!("{elem_accessor} is {{ }} && ")
    } else {
        String::new()
    };

    let any_expr = |value: &serde_json::Value| -> Option<(String, String)> {
        let serde_json::Value::String(s) = value else {
            return None;
        };
        let escaped = escape_csharp(s);
        Some((
            format!("{array_accessor}?.Any({param} => {null_guard}{text_accessor}.Contains(\"{escaped}\")) == true"),
            escaped,
        ))
    };

    match assertion.assertion_type.as_str() {
        "contains" | "not_contains" if assertion.value.is_some() => {
            let value = assertion.value.as_ref().expect("guarded by the match arm");
            let Some((expr, escaped)) = any_expr(value) else {
                let _ = writeln!(
                    out,
                    "        // skipped: non-string value for '{field}' traversal assertion"
                );
                return;
            };
            let verb = if assertion.assertion_type == "contains" {
                "True"
            } else {
                "False"
            };
            let _ = writeln!(
                out,
                "        Assert.{verb}({expr}, \"element of '{field}': {escaped}\");"
            );
        }
        "contains" | "contains_all" | "not_contains" => {
            let Some(values) = &assertion.values else {
                let _ = writeln!(out, "        // skipped: '{field}' traversal assertion has no values");
                return;
            };
            let verb = if assertion.assertion_type == "not_contains" {
                "False"
            } else {
                "True"
            };
            for value in values {
                let Some((expr, escaped)) = any_expr(value) else {
                    continue;
                };
                let _ = writeln!(
                    out,
                    "        Assert.{verb}({expr}, \"element of '{field}': {escaped}\");"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "        Assert.True({array_accessor}?.Any({param} => \
                 !string.IsNullOrEmpty(Convert.ToString({elem_accessor}))) == true, \
                 \"expected some element of '{field}' to be non-empty\");"
            );
        }
        other => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}
