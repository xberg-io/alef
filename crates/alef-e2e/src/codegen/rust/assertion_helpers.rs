//! Sub-helper functions for rendering individual assertion types in Rust e2e tests.

use std::fmt::Write as FmtWrite;

use crate::field_access::FieldResolver;
use crate::fixture::Assertion;

use super::args::json_to_rust_literal;
use super::assertion_synthetic::{numeric_literal, value_to_rust_string};

pub(super) fn render_equals_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(val) = &assertion.value {
        let expected = value_to_rust_string(val);
        // For string equality, trim trailing whitespace to handle trailing newlines
        // from the converter.
        if val.is_string() {
            // When the field is Optional<String> and was NOT pre-unwrapped to a local
            // var (e.g. inside a result_is_vec iteration where the call-site unwrap
            // pass is skipped), emit `.as_deref().unwrap_or("").trim()` so the
            // expression is `&str` rather than `Option<String>`.
            let is_opt_str_not_unwrapped = assertion.field.as_ref().is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                let is_opt = field_resolver.is_optional(resolved);
                let is_arr = field_resolver.is_array(resolved);
                is_opt && !is_arr && !is_unwrapped
            });
            let field_expr = if is_opt_str_not_unwrapped {
                format!("{field_access}.as_deref().unwrap_or(\"\").trim()")
            } else {
                format!("{field_access}.trim()")
            };
            let _ = writeln!(
                out,
                "    assert_eq!({field_expr}, {expected}, \"equals assertion failed\");"
            );
        } else if val.is_boolean() {
            // Use assert!/assert!(!...) for booleans — clippy prefers this over assert_eq!(_, true/false).
            if val.as_bool() == Some(true) {
                let _ = writeln!(out, "    assert!({field_access}, \"equals assertion failed\");");
            } else {
                let _ = writeln!(out, "    assert!(!{field_access}, \"equals assertion failed\");");
            }
        } else {
            // Wrap expected value in Some() for optional fields.
            let is_opt = assertion.field.as_ref().is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                field_resolver.is_optional(resolved)
            });
            if is_opt && !is_unwrapped && assertion.field.as_ref().is_some_and(|_| true) {
                let _ = writeln!(
                    out,
                    "    assert_eq!({field_access}, Some({expected}), \"equals assertion failed\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert_eq!({field_access}, {expected}, \"equals assertion failed\");"
                );
            }
        }
    }
}

pub(super) fn render_not_empty_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    result_var: &str,
    result_is_option: bool,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(f) = &assertion.field {
        let resolved = field_resolver.resolve(f);
        let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
        let is_arr = field_resolver.is_array(resolved);
        if is_opt && is_arr {
            // Option<Vec<T>>: must be Some AND inner non-empty.
            let accessor = field_resolver.accessor(f, "rust", result_var);
            let _ = writeln!(
                out,
                "    assert!({accessor}.as_ref().is_some_and(|v| !v.is_empty()), \"expected {f} to be present and non-empty\");"
            );
        } else if is_opt {
            // Non-collection optional field (e.g., Option<Struct>): use is_some().
            let accessor = field_resolver.accessor(f, "rust", result_var);
            let _ = writeln!(
                out,
                "    assert!({accessor}.is_some(), \"expected {f} to be present\");"
            );
        } else {
            let _ = writeln!(
                out,
                "    assert!(!{field_access}.is_empty(), \"expected non-empty value\");"
            );
        }
    } else if result_is_option {
        // Bare result is Option<T>: not_empty == is_some().
        let _ = writeln!(
            out,
            "    assert!({field_access}.is_some(), \"expected non-empty value\");"
        );
    } else {
        // Bare result is a struct/string/collection — non-empty via is_empty().
        let _ = writeln!(
            out,
            "    assert!(!{field_access}.is_empty(), \"expected non-empty value\");"
        );
    }
}

pub(super) fn render_is_empty_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(f) = &assertion.field {
        let resolved = field_resolver.resolve(f);
        let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
        let is_arr = field_resolver.is_array(resolved);
        if is_opt && is_arr {
            // Option<Vec<T>>: empty means None or empty vec.
            let _ = writeln!(
                out,
                "    assert!({field_access}.as_ref().is_none_or(|v| v.is_empty()), \"expected {f} to be empty or absent\");"
            );
        } else if is_opt {
            let _ = writeln!(
                out,
                "    assert!({field_access}.is_none(), \"expected {f} to be absent\");"
            );
        } else {
            let _ = writeln!(out, "    assert!({field_access}.is_empty(), \"expected empty value\");");
        }
    } else {
        let _ = writeln!(out, "    assert!({field_access}.is_none(), \"expected empty value\");");
    }
}

pub(super) fn render_gte_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(val) = &assertion.value {
        let lit = numeric_literal(val);
        // Check whether this field is optional but not an array — e.g. Option<usize>.
        // Directly comparing Option<usize> >= N is a type error; wrap with unwrap_or(0).
        let is_opt_numeric = assertion.field.as_ref().is_some_and(|f| {
            let resolved = field_resolver.resolve(f);
            let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
            let is_arr = field_resolver.is_array(resolved);
            is_opt && !is_arr
        });
        if val.as_u64() == Some(1) && field_access.ends_with(".len()") {
            // Clippy prefers !is_empty() over len() >= 1 for collections.
            let base = field_access.strip_suffix(".len()").unwrap_or(field_access);
            let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected >= 1\");");
        } else if is_opt_numeric {
            // Option<usize> / Option<u64>: unwrap to 0 before comparing.
            let _ = writeln!(
                out,
                "    assert!({field_access}.unwrap_or(0) >= {lit}, \"expected >= {lit}\");"
            );
        } else {
            let _ = writeln!(out, "    assert!({field_access} >= {lit}, \"expected >= {lit}\");");
        }
    }
}

pub(super) fn render_count_min_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(val) = &assertion.value {
        if let Some(n) = val.as_u64() {
            let opt_arr_field = assertion.field.as_ref().is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
                let is_arr = field_resolver.is_array(resolved);
                is_opt && is_arr
            });
            let base = field_access.strip_suffix(".len()").unwrap_or(field_access);
            if opt_arr_field {
                // Option<Vec<T>>: must be Some AND inner len >= n.
                if n <= 1 {
                    let _ = writeln!(
                        out,
                        "    assert!({base}.as_ref().is_some_and(|v| !v.is_empty()), \"expected >= {n}\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert!({base}.as_ref().is_some_and(|v| v.len() >= {n}), \"expected at least {n} elements\");"
                    );
                }
            } else if n <= 1 {
                let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected >= {n}\");");
            } else {
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.len() >= {n}, \"expected at least {n} elements, got {{}}\", {field_access}.len());"
                );
            }
        }
    }
}

pub(super) fn render_count_equals_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(val) = &assertion.value {
        if let Some(n) = val.as_u64() {
            let opt_arr_field = assertion.field.as_ref().is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
                let is_arr = field_resolver.is_array(resolved);
                is_opt && is_arr
            });
            let base = field_access.strip_suffix(".len()").unwrap_or(field_access);
            if opt_arr_field {
                let _ = writeln!(
                    out,
                    "    assert!({base}.as_ref().is_some_and(|v| v.len() == {n}), \"expected exactly {n} elements\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert_eq!({field_access}.len(), {n}, \"expected exactly {n} elements, got {{}}\", {field_access}.len());"
                );
            }
        }
    }
}

pub(super) fn render_method_result_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    result_is_tree: bool,
    module: &str,
) {
    if let Some(method_name) = &assertion.method {
        // Build the call expression. When the result is a tree-sitter Tree (an opaque
        // type), methods like `root_child_count` do not exist on `Tree` directly —
        // they are free functions in the crate or are accessed via `root_node()`.
        let call_expr = if result_is_tree {
            super::assertion_synthetic::build_tree_call_expr(field_access, method_name, assertion.args.as_ref(), module)
        } else if let Some(args) = &assertion.args {
            let arg_lit = json_to_rust_literal(args, "");
            format!("{field_access}.{method_name}({arg_lit})")
        } else {
            format!("{field_access}.{method_name}()")
        };

        // Determine whether the call expression returns a numeric type so we can
        // choose the right comparison strategy for `greater_than_or_equal`.
        let returns_numeric = result_is_tree && super::assertion_synthetic::is_tree_numeric_method(method_name);

        let check = assertion.check.as_deref().unwrap_or("is_true");
        match check {
            "equals" => {
                if let Some(val) = &assertion.value {
                    if val.is_boolean() {
                        if val.as_bool() == Some(true) {
                            let _ = writeln!(
                                out,
                                "    assert!({call_expr}, \"method_result equals assertion failed\");"
                            );
                        } else {
                            let _ = writeln!(
                                out,
                                "    assert!(!{call_expr}, \"method_result equals assertion failed\");"
                            );
                        }
                    } else {
                        let expected = value_to_rust_string(val);
                        let _ = writeln!(
                            out,
                            "    assert_eq!({call_expr}, {expected}, \"method_result equals assertion failed\");"
                        );
                    }
                }
            }
            "is_true" => {
                let _ = writeln!(
                    out,
                    "    assert!({call_expr}, \"method_result is_true assertion failed\");"
                );
            }
            "is_false" => {
                let _ = writeln!(
                    out,
                    "    assert!(!{call_expr}, \"method_result is_false assertion failed\");"
                );
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let lit = numeric_literal(val);
                    if returns_numeric {
                        // Numeric return (e.g., child_count()) — always use >= comparison.
                        let _ = writeln!(out, "    assert!({call_expr} >= {lit}, \"expected >= {lit}\");");
                    } else if val.as_u64() == Some(1) {
                        // Clippy prefers !is_empty() over len() >= 1 for collections.
                        let _ = writeln!(out, "    assert!(!{call_expr}.is_empty(), \"expected >= 1\");");
                    } else {
                        let _ = writeln!(out, "    assert!({call_expr} >= {lit}, \"expected >= {lit}\");");
                    }
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    if n <= 1 {
                        let _ = writeln!(out, "    assert!(!{call_expr}.is_empty(), \"expected >= {n}\");");
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert!({call_expr}.len() >= {n}, \"expected at least {n} elements, got {{}}\", {call_expr}.len());"
                        );
                    }
                }
            }
            "is_error" => {
                // For is_error we need the raw Result without .unwrap().
                let raw_call = call_expr.strip_suffix(".unwrap()").unwrap_or(&call_expr);
                let _ = writeln!(
                    out,
                    "    assert!({raw_call}.is_err(), \"expected method to return error\");"
                );
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let expected = value_to_rust_string(val);
                    let _ = writeln!(
                        out,
                        "    assert!({call_expr}.contains({expected}), \"expected result to contain {{}}\", {expected});"
                    );
                }
            }
            "not_empty" => {
                let _ = writeln!(
                    out,
                    "    assert!(!{call_expr}.is_empty(), \"expected non-empty result\");"
                );
            }
            "is_empty" => {
                let _ = writeln!(out, "    assert!({call_expr}.is_empty(), \"expected empty result\");");
            }
            other_check => {
                panic!("Rust e2e generator: unsupported method_result check type: {other_check}");
            }
        }
    } else {
        panic!("Rust e2e generator: method_result assertion missing 'method' field");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::field_access::FieldResolver;
    use crate::fixture::Assertion;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &HashSet::new(), &HashSet::new())
    }

    fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|s| s.to_string()),
            value,
            values: None,
            method: None,
            args: None,
            check: None,
        }
    }

    #[test]
    fn render_equals_assertion_string_produces_trim_call() {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello".into())));
        let mut out = String::new();
        render_equals_assertion(&mut out, &assertion, "result", false, &resolver);
        assert!(out.contains(".trim()"), "got: {out}");
    }

    #[test]
    fn render_not_empty_assertion_bare_result_emits_is_empty_check() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_not_empty_assertion(&mut out, &assertion, "result", "result", false, false, &resolver);
        assert!(out.contains("is_empty()"), "got: {out}");
    }

    #[test]
    fn render_count_min_assertion_small_n_uses_is_empty() {
        let resolver = empty_resolver();
        let assertion = make_assertion("count_min", None, Some(serde_json::json!(1u64)));
        let mut out = String::new();
        render_count_min_assertion(&mut out, &assertion, "result", false, &resolver);
        assert!(out.contains("is_empty()"), "got: {out}");
    }
}
