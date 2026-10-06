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

/// The bracket-wildcard path an assertion traverses, already split into its container and the
/// element-relative remainder.
pub(super) struct WildcardPath<'a> {
    pub(super) result_expr: &'a str,
    pub(super) field_resolver: &'a FieldResolver,
    pub(super) field: &'a str,
    pub(super) array_part: &'a str,
    pub(super) elem_part: &'a str,
}

impl<'a> WildcardPath<'a> {
    pub(super) fn new(
        result_expr: &'a str,
        field_resolver: &'a FieldResolver,
        field: &'a str,
        array_part: &'a str,
        elem_part: &'a str,
    ) -> Self {
        Self {
            result_expr,
            field_resolver,
            field,
            array_part,
            elem_part,
        }
    }
}

/// The C# expressions every wildcard assertion arm is built from.
struct WildcardExprs {
    array_accessor: String,
    param: String,
    elem_accessor: String,
    text_accessor: String,
    null_guard: String,
}

impl WildcardExprs {
    fn new(assertion: &Assertion, path: &WildcardPath<'_>) -> Self {
        let array_accessor = if path.array_part.is_empty() {
            path.result_expr.to_string()
        } else {
            path.field_resolver
                .accessor(path.array_part, "csharp", path.result_expr)
        };
        let param = wildcard_lambda_param(assertion);
        // `element_accessor`, not `accessor`: the path is already element-relative, so the
        // result-anchoring `accessor` applies would re-prefix it with the container. ~keep
        let elem_accessor = path.field_resolver.element_accessor(path.elem_part, "csharp", &param);

        let enum_leaf = path.field_resolver.is_enum(path.field);
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
        Self {
            array_accessor,
            param,
            elem_accessor,
            text_accessor,
            null_guard,
        }
    }

    /// `(any-element expression, escaped literal)` for a string value; `None` for any other JSON type.
    fn any_expr(&self, value: &serde_json::Value) -> Option<(String, String)> {
        let serde_json::Value::String(s) = value else {
            return None;
        };
        let escaped = escape_csharp(s);
        let Self {
            array_accessor,
            param,
            text_accessor,
            null_guard,
            ..
        } = self;
        Some((
            format!("{array_accessor}?.Any({param} => {null_guard}{text_accessor}.Contains(\"{escaped}\")) == true"),
            escaped,
        ))
    }
}

fn render_single_contains(out: &mut String, assertion: &Assertion, exprs: &WildcardExprs, field: &str) {
    let value = assertion.value.as_ref().expect("guarded by the match arm");
    let Some((expr, escaped)) = exprs.any_expr(value) else {
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

fn render_multi_contains(out: &mut String, assertion: &Assertion, exprs: &WildcardExprs, field: &str) {
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
        let Some((expr, escaped)) = exprs.any_expr(value) else {
            continue;
        };
        let _ = writeln!(
            out,
            "        Assert.{verb}({expr}, \"element of '{field}': {escaped}\");"
        );
    }
}

/// Emit `Assert.True(<array>?.Any(e => …) == true)` for a bracket-wildcard path.
///
/// The null-conditional `?.` plus `== true` covers both nullable and non-nullable list
/// getters without needing the element type name to build an empty-list fallback. ~keep
pub(super) fn render_wildcard_assertion(out: &mut String, assertion: &Assertion, path: &WildcardPath<'_>) {
    let field = path.field;
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("        ", "//", field, path.elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let exprs = WildcardExprs::new(assertion, path);

    match assertion.assertion_type.as_str() {
        "contains" | "not_contains" if assertion.value.is_some() => {
            render_single_contains(out, assertion, &exprs, field)
        }
        "contains" | "contains_all" | "not_contains" => render_multi_contains(out, assertion, &exprs, field),
        "not_empty" => {
            let WildcardExprs {
                array_accessor,
                param,
                elem_accessor,
                ..
            } = &exprs;
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
