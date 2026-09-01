//! Sub-helper functions for rendering individual assertion types in Rust e2e tests.

use std::fmt::Write as FmtWrite;

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::args::json_to_rust_literal;
use super::assertion_synthetic::{numeric_literal, value_to_rust_string};
use super::assertion_wire::renamed_variant_expected;

pub(super) fn render_equals_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_access: &str,
    is_unwrapped: bool,
    field_resolver: &FieldResolver,
) {
    if let Some(val) = &assertion.value {
        let expected = value_to_rust_string(val);
        if val.is_string() {
            // Enum-typed fields are not guaranteed to implement `Display` — only `Debug`
            // is a safe assumption (the containment predicates already rely on it). For a
            // unit variant, `{:?}` prints exactly the RUST identifier. That equals the
            // fixture's expected literal only while no serde rename is in effect; when one
            // is, the fixture carries the WIRE value and the two surfaces disagree, so
            // `renamed_variant_expected` translates the expectation back onto the surface
            // `{:?}` actually renders. This takes priority over the optional/display-as-text
            // branches below because those assume the inner type is string-like. ~keep
            let field_is_enum = assertion.field.as_ref().is_some_and(|f| field_resolver.is_enum(f));
            // A field whose declared type is `Vec<EnumType>` is BOTH enum- and array-typed
            // (`is_enum` walks through `Vec` the same way `named_type` does). When such a
            // field is ALSO `Option<Vec<EnumType>>` and the fixture (unusually) writes a
            // plain-string `equals` assertion straight against it,
            // `FieldResolver::rust_unwrap_binding` takes its `is_array` branch and produces a
            // `&[EnumType]` slice local, not a `String` -- the `is_unwrapped` branches below
            // assume a `String`/`Display` surface and must not apply to that shape. Checked
            // against the SAME predicate `rust_unwrap_binding` uses so the two never drift. ~keep
            let is_array_field = assertion
                .field
                .as_ref()
                .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));
            // ~keep When `is_unwrapped` is also true (and the field is not array-shaped), the
            // expression side below is NOT the `{:?}` Debug surface `renamed_variant_expected`
            // exists to reconcile — it is the pre-unwrapped `String` local
            // `FieldResolver::rust_unwrap_binding` built via `.as_ref().map(|v|
            // v.to_string()).unwrap_or_default()`, i.e. the enum's OWN `Display` rendering,
            // which for a serde-derived `Display` impl serialises through serde and therefore
            // already equals the fixture's WIRE spelling. Translating `expected` to the Rust
            // identifier here compares two different surfaces: for a renamed variant (e.g.
            // `FinishReason::ContentFilter`, wire `"content_filter"`) the pre-unwrapped local
            // holds `"content_filter"` while the translated `expected` is `ContentFilter` —
            // same defect class as the quoting bug, on the other operand.
            let expected = if field_is_enum && !(is_unwrapped && !is_array_field) {
                renamed_variant_expected(assertion.field.as_deref(), val, field_resolver).unwrap_or(expected)
            } else {
                expected
            };
            // When the field is Optional<String> and was NOT pre-unwrapped to a local
            // var (e.g. inside a result_is_vec iteration where the call-site unwrap
            // pass is skipped), emit `.as_deref().unwrap_or("")` so the expression is
            // `&str` rather than `Option<String>`.
            // ~keep: intentionally NOT trimming here — fixture `expected` values are
            // captured verbatim (including trailing newlines the converter legitimately
            // emits), so trimming only the actual side made those assertions
            // unsatisfiable. Neither side is trimmed; exact equality is the contract.
            let is_opt_str_not_unwrapped = assertion.field.as_ref().is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                let is_opt = field_resolver.is_optional(resolved);
                let is_arr = field_resolver.is_array(resolved);
                is_opt && !is_arr && !is_unwrapped
            });
            // For fields whose `Option<T>` inner type is a display/content union (not
            // plain `String`), `.as_deref()` does not compile because the inner type
            // does not implement `Deref<Target=str>`. Use `.as_ref().map(|v|
            // v.to_string()).unwrap_or_default()` instead, which works for any type
            // that implements `Display` (including `String` itself).
            let is_display_as_text = assertion
                .field
                .as_ref()
                .is_some_and(|f| field_resolver.is_display_as_text(f));
            let field_expr = if field_is_enum && is_opt_str_not_unwrapped {
                format!("{field_access}.as_ref().map(|v| format!(\"{{v:?}}\")).unwrap_or_default()")
            } else if field_is_enum && is_unwrapped && !is_array_field {
                // `is_unwrapped` means `field_access` already names a local the call-site
                // unwrap pass built via `FieldResolver::rust_unwrap_binding`'s optional-scalar
                // branch: `let <local> = <accessor>.as_ref().map(|v| v.to_string())
                // .unwrap_or_default();` — a `String` holding the enum's own Display
                // rendering, not the enum value itself. Debug-formatting that `String` AGAIN
                // wraps it in an extra pair of quotes (`format!("{:?}", "Stop".to_string())`
                // is `"\"Stop\""`), which never equals the fixture's unquoted wire literal.
                // The local is already the comparable string — use it as-is. ~keep
                field_access.to_string()
            } else if field_is_enum {
                format!("format!(\"{{:?}}\", {field_access})")
            } else if is_opt_str_not_unwrapped && is_display_as_text {
                // Optional non-String content field not yet pre-unwrapped: use Display via
                // `.as_ref().map(|v| v.to_string())` so the inner type need not impl
                // `Deref<Target=str>`.
                format!("{field_access}.as_ref().map(|v| v.to_string()).unwrap_or_default()")
            } else if is_opt_str_not_unwrapped {
                // Optional string-like field that wasn't pre-unwrapped: use `.as_deref()`
                // when the inner type is `String`; for inner types that impl Display we
                // can also do `.as_ref().map(ToString::to_string)`. Default to as_deref
                // which is the common String case — types without Display (rare) need
                // a separate fixture-level path resolution to land on a string child.
                format!("{field_access}.as_deref().unwrap_or(\"\")")
            } else {
                // Non-optional string-like field: rely on Display impl via `.to_string()`.
                // This is correct for `String`, `&str`, and `Cow<str>` — Debug would
                // wrap them in extra quotes and break literal comparison.
                format!("{field_access}.to_string()")
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

/// Whether a wildcard-traversed array element (`links[].link_type`) is enum-typed.
///
/// Checked against both spellings: `elem_part` (e.g. `"link_type"`) is what a hand-written
/// `fields_enum` config entry names, while `field` (e.g. `"links[].link_type"`) is what the
/// IR-derived check in `FieldResolver::is_enum` needs — its IR fallback walks the whole path
/// from the call's root type, and that walk only sees the array traversal segment when given
/// the un-split original path. ~keep
pub(super) fn wildcard_elem_is_enum(field_resolver: &FieldResolver, elem_part: &str, field: &str) -> bool {
    field_resolver.is_enum(elem_part) || field_resolver.is_enum(field)
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
        // ~keep `field_access` is the caller's decision about what this assertion targets. When it
        // is the bare result variable -- a `result_is_simple` call, or the `field == result_var`
        // sentinel -- the field names no member here, and the two branches below would discard
        // that decision by re-deriving `accessor(f, ..)`, emitting `result.<field>` against a
        // value that has no such field (`E0609` in the generated test). This is the only assertion
        // renderer that rebuilds its own accessor instead of using the one it was handed.
        let targets_whole_result = field_access == result_var;
        let is_opt = !is_unwrapped && !targets_whole_result && field_resolver.is_optional(resolved);
        let is_arr = field_resolver.is_array(resolved);
        if is_opt && is_arr {
            // Option<Vec<T>>: must be Some AND inner non-empty.
            let accessor = field_resolver.accessor(f, "rust", result_var);
            let _ = writeln!(
                out,
                "    assert!({accessor}.as_ref().is_some_and(|v| !v.is_empty()), \"expected {f} to be present and non-empty\");"
            );
        } else if is_opt {
            // `is_optional` registers ANY path that crosses an Option<...> on the
            // way down, even when the leaf itself is concrete. For e.g. summary.text
            // (`Option<Summary>`, leaf String), the accessor already auto-unwraps the
            // parent — `result.summary.as_ref().unwrap().text` — so the final
            // expression has type String. Emitting `.is_some()` against that is a
            // compile error. Detect "leaf is post-unwrap concrete" by checking that
            // the accessor contains `.as_ref().unwrap().` (the trailing dot is the
            // marker that more field access follows the unwrap) and fall through to
            // the is_empty() form. If the accessor ENDS with `.as_ref().unwrap()`
            // (i.e. the Option itself IS the leaf), keep the is_some() form.
            let accessor = field_resolver.accessor(f, "rust", result_var);
            let leaf_is_concrete = accessor.contains(".as_ref().unwrap().");
            if leaf_is_concrete {
                let _ = writeln!(
                    out,
                    "    assert!(!{accessor}.is_empty(), \"expected {f} to be non-empty\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert!({accessor}.is_some(), \"expected {f} to be present\");"
                );
            }
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
            // Option<usize> / Option<u64> / Option<f64>: unwrap with appropriate zero literal
            // depending on whether the comparison value is float or integer.
            // Check if the rendered literal contains _f64 or a decimal point (float type indicator).
            let default_literal = if lit.contains("_f64") || lit.contains('.') {
                "0.0"
            } else {
                "0"
            };
            let _ = writeln!(
                out,
                "    assert!({field_access}.unwrap_or({default_literal}) >= {lit}, \"expected >= {lit}\");"
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
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let opt_arr_field = assertion.field.as_ref().is_some_and(|f| {
            let resolved = field_resolver.resolve(f);
            let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
            let is_arr = field_resolver.is_array(resolved);
            is_opt && is_arr
        });
        let base = field_access.strip_suffix(".len()").unwrap_or(field_access);
        if opt_arr_field {
            // Option<Vec<T>>: must be Some AND inner len >= n.
            if n == 0 {
                // count_min: 0 is always true — no assertion needed
            } else if n == 1 {
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
        } else if n == 0 {
            // count_min: 0 is always true — no assertion needed
        } else if n == 1 {
            let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected >= {n}\");");
        } else {
            let _ = writeln!(
                out,
                "    assert!({field_access}.len() >= {n}, \"expected at least {n} elements, got {{}}\", {field_access}.len());"
            );
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
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
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
    use crate::e2e::field_access::FieldResolver;
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

    /// Resolver with `content` as optional and display_as_text.
    fn display_as_text_resolver() -> FieldResolver {
        let mut optional = HashSet::new();
        optional.insert("content".to_string());
        let mut dat_fields = HashSet::new();
        dat_fields.insert("content".to_string());
        FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_display_as_text_fields(dat_fields)
    }

    fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|s| s.to_string()),
            value,
            ..Default::default()
        }
    }

    #[test]
    fn render_equals_assertion_string_uses_to_string() {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello".into())));
        let mut out = String::new();
        render_equals_assertion(&mut out, &assertion, "result", false, &resolver);
        assert!(out.contains("result.to_string()"), "got: {out}");
    }

    /// Regression test for a one-sided-trim bug: `.trim()` was previously appended to the
    /// actual-value expression but fixture `expected` values are captured verbatim (they may
    /// legitimately end in `\n`), making such assertions impossible to satisfy. Equals
    /// assertions must compare the full, untrimmed value on both sides across every field
    /// shape the generator can emit (plain, optional-not-unwrapped, display-as-text).
    /// Control for the trim fix: the tightened contract must still DISCRIMINATE values that
    /// differ only in trailing whitespace. If either side were normalized, the emitted
    /// assertion for "hello\n" and for "hello" would be identical and a real trailing-newline
    /// regression would pass unnoticed.
    #[test]
    fn render_equals_assertion_still_discriminates_trailing_whitespace() {
        let render_for = |value: &str| {
            let resolver = empty_resolver();
            let assertion = make_assertion("equals", None, Some(serde_json::Value::String(value.into())));
            let mut out = String::new();
            render_equals_assertion(&mut out, &assertion, "result", false, &resolver);
            out
        };
        let emitted = render_for("hello\n");
        // The actual side must be the bare expression: any normalizing call (trim/strip/
        // case-folding) wrapped around it would silently accept a mismatched value.
        assert_eq!(
            emitted, "    assert_eq!(result.to_string(), r#\"hello\n\"#, \"equals assertion failed\");\n",
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

    #[test]
    fn render_equals_assertion_never_trims_either_side() {
        let plain_resolver = empty_resolver();
        let mut out = String::new();
        render_equals_assertion(
            &mut out,
            &make_assertion("equals", None, Some(serde_json::Value::String("hello\n".into()))),
            "result",
            false,
            &plain_resolver,
        );
        assert!(!out.contains(".trim()"), "plain field must not trim; got: {out}");

        let mut optional = HashSet::new();
        optional.insert("content".to_string());
        let optional_resolver = FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_equals_assertion(
            &mut out,
            &make_assertion(
                "equals",
                Some("content"),
                Some(serde_json::Value::String("hi\n".into())),
            ),
            "result.content",
            false,
            &optional_resolver,
        );
        assert!(
            !out.contains(".trim()"),
            "optional-not-unwrapped field must not trim; got: {out}"
        );

        let dat_resolver = display_as_text_resolver();
        let mut out = String::new();
        render_equals_assertion(
            &mut out,
            &make_assertion(
                "equals",
                Some("content"),
                Some(serde_json::Value::String("hello\n".into())),
            ),
            "result.content",
            false,
            &dat_resolver,
        );
        assert!(
            !out.contains(".trim()"),
            "display-as-text optional field must not trim; got: {out}"
        );

        let mut out = String::new();
        render_equals_assertion(
            &mut out,
            &make_assertion(
                "equals",
                Some("content"),
                Some(serde_json::Value::String("hello\n".into())),
            ),
            "_content",
            true,
            &dat_resolver,
        );
        assert!(
            !out.contains(".trim()"),
            "already-unwrapped field must not trim; got: {out}"
        );
    }

    /// When a field is `Option<String>` (NOT display_as_text) and not pre-unwrapped,
    /// the assertion must use `.as_deref().unwrap_or("")` — not `map(|v| v.to_string())`.
    /// This guards against regression where the DAT path is taken for plain strings.
    #[test]
    fn render_equals_assertion_plain_optional_string_uses_as_deref_not_to_string() {
        let mut optional = HashSet::new();
        optional.insert("content".to_string());
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = make_assertion("equals", Some("content"), Some(serde_json::Value::String("hi".into())));
        let mut out = String::new();
        // is_unwrapped=false simulates result_is_vec=true where the pre-unwrap pass is skipped.
        render_equals_assertion(&mut out, &assertion, "result.content", false, &resolver);
        assert!(out.contains(".as_deref().unwrap_or(\"\")"), "got: {out}");
        assert!(
            !out.contains("to_string"),
            "plain optional string should NOT use to_string(); got: {out}"
        );
    }

    /// When the field is `Option<AssistantContent>` (display_as_text) and not pre-unwrapped,
    /// the assertion must use `.as_ref().map(|v| v.to_string()).unwrap_or_default()`
    /// so that `AssistantContent` (which implements `Display` but NOT `Deref<Target=str>`)
    /// compiles correctly.
    #[test]
    fn render_equals_assertion_display_as_text_optional_uses_map_to_string_not_as_deref() {
        let resolver = display_as_text_resolver();
        let assertion = make_assertion(
            "equals",
            Some("content"),
            Some(serde_json::Value::String("hello".into())),
        );
        let mut out = String::new();
        // is_unwrapped=false — simulates the result_is_vec=true path where pre-unwrapping is skipped.
        render_equals_assertion(&mut out, &assertion, "result.content", false, &resolver);
        // Must use .to_string() path via Display, NOT .as_deref() which requires Deref<Target=str>.
        assert!(
            out.contains(".as_ref().map(|v| v.to_string()).unwrap_or_default()"),
            "display_as_text field must use map(|v| v.to_string()) path; got: {out}"
        );
        assert!(
            !out.contains("as_deref"),
            "display_as_text field must NOT emit as_deref(); got: {out}"
        );
    }

    /// When `is_unwrapped=true` (pre-unwrap pass already ran), display_as_text fields
    /// should fall through to the non-optional path, same as plain strings.
    #[test]
    fn render_equals_assertion_display_as_text_already_unwrapped_uses_to_string() {
        let resolver = display_as_text_resolver();
        let assertion = make_assertion(
            "equals",
            Some("content"),
            Some(serde_json::Value::String("hello".into())),
        );
        let mut out = String::new();
        // is_unwrapped=true — the pre-unwrap pass already produced a local `_content: String`.
        render_equals_assertion(&mut out, &assertion, "_content", true, &resolver);
        // Should use the regular to_string() path for an already-unwrapped value.
        assert!(out.contains("_content.to_string()"), "got: {out}");
        assert!(
            !out.contains("as_deref"),
            "unwrapped field must NOT emit as_deref(); got: {out}"
        );
        assert!(
            !out.contains("unwrap_or_default"),
            "unwrapped field must NOT emit unwrap_or_default(); got: {out}"
        );
    }

    fn enum_field_resolver(field: &str) -> FieldResolver {
        empty_resolver().with_enum_fields(HashSet::from([field.to_string()]))
    }

    /// Table-driven regression for the `.to_string()`-on-enum defect: an enum field emits a
    /// `format!("{:?}", ...)` expression (works with only `Debug`, the far more commonly
    /// derived trait), while non-enum string fields keep the pre-existing `Display`-based
    /// `.to_string()` path unchanged. The IR carries no signal for whether an enum ALSO
    /// implements `Display` (verified: no `has_display`/`strum`/derive(Display) detection
    /// anywhere in `src/extract/` or `src/core/ir`), so this is uniform for every enum field
    /// regardless of whether the real Rust type also derives/implements `Display` — for a
    /// unit variant, `{:?}` renders exactly the variant name, which is what `Display` also
    /// renders for the idiomatic (unrenamed) case, so behavior is preserved for enums that
    /// do implement Display too.
    ///
    /// That equivalence holds ONLY while the variant is unrenamed, which is why the cases
    /// below all use unrenamed variants. `{:?}` renders the Rust identifier, whereas a
    /// `#[serde(rename)]`/`#[serde(rename_all)]` variant's fixture value is the WIRE
    /// spelling; comparing those two directly checks the wrong surface. That case is
    /// reconciled on the expectation side by `renamed_variant_expected` and is covered by
    /// `render_equals_assertion_renamed_enum_variant_compares_wire_value`. ~keep
    #[test]
    fn render_equals_assertion_field_stringification_matches_field_kind() {
        struct Case {
            name: &'static str,
            field: &'static str,
            enum_field: bool,
            must_contain: &'static str,
            must_not_contain: &'static str,
        }
        let cases = [
            Case {
                name: "enum field without Display uses Debug, not to_string",
                field: "kind",
                enum_field: true,
                must_contain: "format!(\"{:?}\", result.kind)",
                must_not_contain: "result.kind.to_string()",
            },
            Case {
                name: "enum field that also implements Display still uses Debug (no IR signal to prefer Display)",
                field: "kind",
                enum_field: true,
                must_contain: "format!(\"{:?}\", result.kind)",
                must_not_contain: "result.kind.to_string()",
            },
            Case {
                name: "non-enum string field is unchanged (Display via to_string)",
                field: "name",
                enum_field: false,
                must_contain: "result.name.to_string()",
                must_not_contain: "format!(\"{:?}\", result.name)",
            },
        ];
        for case in cases {
            let resolver = if case.enum_field {
                enum_field_resolver(case.field)
            } else {
                empty_resolver()
            };
            let field_access = format!("result.{}", case.field);
            let assertion = make_assertion(
                "equals",
                Some(case.field),
                Some(serde_json::Value::String("KeyValue".into())),
            );
            let mut out = String::new();
            render_equals_assertion(&mut out, &assertion, &field_access, false, &resolver);
            assert!(
                out.contains(case.must_contain),
                "case '{}': expected output to contain '{}', got: {out}",
                case.name,
                case.must_contain
            );
            assert!(
                !out.contains(case.must_not_contain),
                "case '{}': expected output to NOT contain '{}', got: {out}",
                case.name,
                case.must_not_contain
            );
        }
    }

    /// The optional-not-unwrapped enum path must not fall through to `.as_deref()`
    /// (which requires `Deref<Target=str>` and does not compile for an enum type).
    #[test]
    fn render_equals_assertion_optional_enum_field_uses_debug_map_not_as_deref() {
        let mut optional = HashSet::new();
        optional.insert("kind".to_string());
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_enum_fields(HashSet::from(["kind".to_string()]));
        let assertion = make_assertion(
            "equals",
            Some("kind"),
            Some(serde_json::Value::String("KeyValue".into())),
        );
        let mut out = String::new();
        render_equals_assertion(&mut out, &assertion, "result.kind", false, &resolver);
        assert!(
            out.contains("result.kind.as_ref().map(|v| format!(\"{v:?}\")).unwrap_or_default()"),
            "got: {out}"
        );
        assert!(
            !out.contains("as_deref"),
            "must NOT emit as_deref() for an enum; got: {out}"
        );
    }

    /// THE REGRESSION for the CI-confirmed double-Debug defect: when the call-site unwrap
    /// pass (`FieldResolver::rust_unwrap_binding`) has already pre-unwrapped an optional
    /// enum field into a `String` local via `.as_ref().map(|v| v.to_string())
    /// .unwrap_or_default()`, `is_unwrapped=true` and `field_access` names that local
    /// directly (not an `Option<Enum>` needing `.as_ref()` first). The pre-fix code fell
    /// into the same branch as a REQUIRED (never-unwrapped) enum field access and
    /// Debug-formatted the local AGAIN — `format!("{:?}", "Stop".to_string())` renders
    /// `"\"Stop\""`, which can never equal the fixture's unquoted wire literal `"Stop"`.
    /// liter-llm's real `finish_reason`/`content` assertions failed on exactly this shape
    /// (12 of 14 chat tests, CI run 33482291337) while xberg's rust e2e stayed green,
    /// because xberg has no fixture combining enum-typed AND pre-unwrapped-to-local.
    ///
    /// This resolver only knows `finish_reason` is an enum via the hand-maintained
    /// `fields_enum` config (no `with_ir_enum_map`), so it can never resolve a variant rename
    /// and `expected` was already untranslated before this fix -- checking the FULL emitted
    /// line here proves only the quoting half of the defect. The renamed-variant case, where
    /// BOTH halves must move together, is covered by
    /// `render_equals_assertion_pre_unwrapped_renamed_enum_variant_compares_wire_value` in
    /// `renamed_enum_wire_assertion_tests.rs`. ~keep
    #[test]
    fn render_equals_assertion_pre_unwrapped_enum_local_is_not_debug_formatted_again() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_enum_fields(HashSet::from(["finish_reason".to_string()]));
        let assertion = make_assertion(
            "equals",
            Some("finish_reason"),
            Some(serde_json::Value::String("Stop".into())),
        );
        let mut out = String::new();
        // is_unwrapped=true: the pre-unwrap pass already produced a `String` local
        // `_choices_0_finish_reason` via `.as_ref().map(|v| v.to_string()).unwrap_or_default()`.
        render_equals_assertion(&mut out, &assertion, "_choices_0_finish_reason", true, &resolver);
        assert_eq!(
            out, "    assert_eq!(_choices_0_finish_reason, r#\"Stop\"#, \"equals assertion failed\");\n",
            "the pre-unwrapped String local must be compared directly, not re-wrapped in \
             format!(\"{{:?}}\", ..); got: {out}"
        );
    }

    /// EDGE CASE the `is_unwrapped` fix must NOT reach: a field whose type is
    /// `Option<Vec<EnumType>>` is classified as BOTH enum- and array-typed (`is_enum` walks
    /// through `Vec` the same way `named_type` does). `FieldResolver::rust_unwrap_binding`
    /// takes its `is_array` branch for such a field — `.as_deref().unwrap_or(&[])`, a
    /// `&[EnumType]` slice local, never a `String` — so `field_access.to_string()` would embed
    /// a slice identifier where `assert_eq!` expects a string operand
    /// (`E0308: expected &str, found &[EnumType]`). A plain-string `equals` assertion against
    /// a whole array field is not how any real fixture is authored, but the generator must not
    /// newly fail to COMPILE for it: `is_array_field` routes this case back through the
    /// pre-existing `format!("{:?}", ..)` branch, which Debug-formats a slice fine (even though
    /// the resulting assertion could never pass, exactly as before this fix).
    #[test]
    fn render_equals_assertion_pre_unwrapped_array_of_enum_field_keeps_the_debug_branch() {
        let mut array_fields = HashSet::new();
        array_fields.insert("reasons".to_string());
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &array_fields,
            &HashSet::new(),
        )
        .with_enum_fields(HashSet::from(["reasons".to_string()]));
        let assertion = make_assertion(
            "equals",
            Some("reasons"),
            Some(serde_json::Value::String("Stop".into())),
        );
        let mut out = String::new();
        render_equals_assertion(&mut out, &assertion, "_reasons", true, &resolver);
        assert_eq!(
            out, "    assert_eq!(format!(\"{:?}\", _reasons), r#\"Stop\"#, \"equals assertion failed\");\n",
            "an array-of-enum field must keep the Debug-on-slice branch (compiles, even if the \
             assertion could never pass), not the String-local branch; got: {out}"
        );
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
