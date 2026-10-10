//! Kotlin `render_assertion` scalar-pipeline dispatch: the per-assertion-type match that runs
//! once every field-shape gate has declined and `assertion_scalar_context` has computed the
//! accessor expressions and shape flags.
//!
//! Split out of `assertions.rs` at the concept boundary -- `render_scalar_assertion` mirrors the
//! original single `match` exactly; every arm's body is a verbatim extraction, grouped into
//! shared helpers only where several match patterns already funnelled into the same behavior
//! (the four ordering comparisons, `starts_with`/`ends_with`, `min_length`/`max_length`,
//! `count_min`/`count_equals`, `is_true`/`is_false`) — no behavior change. ~keep

use std::fmt::Write as FmtWrite;

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Non-(out, assertion) inputs to [`render_scalar_pipeline`], grouped to keep the function under
/// poly's `too-many-parameters` limit. Field order matches the parameter order it replaces -- no
/// behavior change.
pub(super) struct ScalarPipelineContext<'a> {
    pub(super) field_resolver: &'a FieldResolver,
    pub(super) result_var: &'a str,
    pub(super) result_is_simple: bool,
    pub(super) result_is_option: bool,
    pub(super) enum_fields: &'a std::collections::HashSet<String>,
    pub(super) json_scalar_fields: &'a std::collections::HashSet<String>,
    pub(super) fields_c_types: &'a std::collections::HashMap<String, String>,
    pub(super) kotlin_android_style: bool,
    pub(super) is_streaming: bool,
    pub(super) not_error_may_assert_presence: bool,
}

/// Compute the scalar-pipeline context (`assertion_scalar_context::compute_scalar_context`) and
/// immediately dispatch it (`render_scalar_assertion`) -- the single call `render_assertion`
/// makes once every field-shape gate has declined, so it stays under the file's
/// function-length cap. Verbatim orchestration, no behavior change. ~keep
pub(super) fn render_scalar_pipeline(out: &mut String, assertion: &Assertion, context: ScalarPipelineContext) {
    let ScalarPipelineContext {
        field_resolver,
        result_var,
        result_is_simple,
        result_is_option,
        enum_fields,
        json_scalar_fields,
        fields_c_types,
        kotlin_android_style,
        is_streaming,
        not_error_may_assert_presence,
    } = context;
    let (
        field_expr,
        string_field_expr,
        nonnull_field_expr,
        string_expr,
        field_is_optional,
        field_is_collection,
        field_is_long,
    ) = super::assertion_scalar_context::compute_scalar_context(
        &super::assertion_scalar_context::ScalarContextInputs {
            assertion,
            field_resolver,
            result_var,
            result_is_simple,
            result_is_option,
            enum_fields,
            json_scalar_fields,
            fields_c_types,
            kotlin_android_style,
        },
    );

    render_scalar_assertion(
        out,
        assertion,
        ScalarAssertionContext {
            field_expr: &field_expr,
            string_field_expr: &string_field_expr,
            nonnull_field_expr: &nonnull_field_expr,
            string_expr: &string_expr,
            field_is_optional,
            field_is_collection,
            field_is_long,
            result_var,
            result_is_simple,
            result_is_option,
            kotlin_android_style,
            is_streaming,
            not_error_may_assert_presence,
        },
    );
}

/// The assertion types `render_value_arm` handles -- every arm whose output depends only on the
/// already-computed field-expression variants, never on `result_is_option`/`is_streaming`/
/// presence-template concerns. Named once so the top dispatcher and `render_value_arm` can never
/// disagree about which arms belong to which half. ~keep
const VALUE_ASSERTION_TYPES: &[&str] = &[
    "equals",
    "contains",
    "contains_all",
    "not_contains",
    "contains_any",
    "greater_than",
    "less_than",
    "greater_than_or_equal",
    "less_than_or_equal",
    "starts_with",
    "ends_with",
    "min_length",
    "max_length",
    "count_min",
    "count_equals",
    "is_true",
    "is_false",
    "matches_regex",
];

/// Non-(out, assertion) inputs to [`render_scalar_assertion`], grouped to keep the function under
/// poly's `too-many-parameters` limit. Field order matches the parameter order it replaces -- no
/// behavior change.
pub(super) struct ScalarAssertionContext<'a> {
    pub(super) field_expr: &'a str,
    pub(super) string_field_expr: &'a str,
    pub(super) nonnull_field_expr: &'a str,
    pub(super) string_expr: &'a str,
    pub(super) field_is_optional: bool,
    pub(super) field_is_collection: bool,
    pub(super) field_is_long: bool,
    pub(super) result_var: &'a str,
    pub(super) result_is_simple: bool,
    pub(super) result_is_option: bool,
    pub(super) kotlin_android_style: bool,
    pub(super) is_streaming: bool,
    pub(super) not_error_may_assert_presence: bool,
}

/// Dispatch on assertion type once every field-shape gate has declined and the scalar-pipeline
/// context (`field_expr` and its derived string/non-null/enum-aware variants, plus the
/// `field_is_*` shape flags) has been computed. Split into `render_value_arm` and
/// `render_presence_arm` to keep this dispatcher itself under the file's function-length cap --
/// the split point is exactly where `render_assertion`'s original single `match` already read as
/// two families of arms; no behavior change.
pub(super) fn render_scalar_assertion(out: &mut String, assertion: &Assertion, context: ScalarAssertionContext) {
    let ScalarAssertionContext {
        field_expr,
        string_field_expr,
        nonnull_field_expr,
        string_expr,
        field_is_optional,
        field_is_collection,
        field_is_long,
        result_var,
        result_is_simple,
        result_is_option,
        kotlin_android_style,
        is_streaming,
        not_error_may_assert_presence,
    } = context;
    if VALUE_ASSERTION_TYPES.contains(&assertion.assertion_type.as_str()) {
        render_value_arm(
            out,
            assertion,
            ValueArmContext {
                field_expr,
                string_field_expr,
                nonnull_field_expr,
                string_expr,
                field_is_optional,
                field_is_collection,
                field_is_long,
                result_var,
                result_is_simple,
            },
        );
    } else {
        render_presence_arm(
            out,
            assertion,
            PresenceArmContext {
                field_expr,
                string_field_expr,
                field_is_collection,
                field_is_optional,
                result_var,
                result_is_option,
                kotlin_android_style,
                is_streaming,
                not_error_may_assert_presence,
            },
        );
    }
}

/// Non-(out, assertion) inputs to [`render_value_arm`], grouped to keep the function under poly's
/// `too-many-parameters` limit. Field order matches the parameter order it replaces -- no
/// behavior change.
pub(super) struct ValueArmContext<'a> {
    pub(super) field_expr: &'a str,
    pub(super) string_field_expr: &'a str,
    pub(super) nonnull_field_expr: &'a str,
    pub(super) string_expr: &'a str,
    pub(super) field_is_optional: bool,
    pub(super) field_is_collection: bool,
    pub(super) field_is_long: bool,
    pub(super) result_var: &'a str,
    pub(super) result_is_simple: bool,
}

/// The field-expression-only half of the original `match`: every arm whose rendering depends
/// only on the already-computed `field_expr` variants, not on `result_is_option`/`is_streaming`/
/// presence-template concerns. ~keep
fn render_value_arm(out: &mut String, assertion: &Assertion, context: ValueArmContext) {
    let ValueArmContext {
        field_expr,
        string_field_expr,
        nonnull_field_expr,
        string_expr,
        field_is_optional,
        field_is_collection,
        field_is_long,
        result_var,
        result_is_simple,
    } = context;
    match assertion.assertion_type.as_str() {
        "equals" => render_equals_arm(out, assertion, field_is_long, string_expr, nonnull_field_expr),
        "contains" => render_contains_arm(out, assertion, field_is_collection, string_expr),
        "contains_all" => render_contains_all_arm(out, assertion, field_is_collection, string_expr),
        "not_contains" => render_not_contains_arm(out, assertion, field_is_collection, string_expr),
        "contains_any" => render_contains_any_arm(out, assertion, string_expr),
        "greater_than" => render_greater_than_arm(out, assertion, nonnull_field_expr),
        "less_than" => render_less_than_arm(out, assertion, nonnull_field_expr),
        "greater_than_or_equal" => render_greater_than_or_equal_arm(out, assertion, nonnull_field_expr),
        "less_than_or_equal" => render_less_than_or_equal_arm(out, assertion, nonnull_field_expr),
        "starts_with" => render_starts_with_arm(out, assertion, string_expr),
        "ends_with" => render_ends_with_arm(out, assertion, string_expr),
        "min_length" => {
            render_min_length_arm(
                out,
                assertion,
                result_is_simple,
                field_expr,
                result_var,
                string_field_expr,
            );
        }
        "max_length" => {
            render_max_length_arm(
                out,
                assertion,
                result_is_simple,
                field_expr,
                result_var,
                string_field_expr,
            );
        }
        "count_min" => render_count_min_arm(out, assertion, nonnull_field_expr),
        "count_equals" => render_count_equals_arm(out, assertion, nonnull_field_expr),
        "is_true" => render_is_true_arm(out, field_is_optional, field_expr),
        "is_false" => render_is_false_arm(out, field_is_optional, field_expr),
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let kotlin_val = super::values::json_to_kotlin(expected);
                let _ = writeln!(
                    out,
                    "        assertTrue(Regex({kotlin_val}).containsMatchIn({string_expr}), \"expected value to match regex: \" + {kotlin_val})"
                );
            }
        }
        other => unreachable!("render_value_arm called for non-value assertion type {other}"),
    }
}

/// Non-(out, assertion) inputs to [`render_presence_arm`], grouped to keep the function under
/// poly's `too-many-parameters` limit. Field order matches the parameter order it replaces -- no
/// behavior change.
pub(super) struct PresenceArmContext<'a> {
    pub(super) field_expr: &'a str,
    pub(super) string_field_expr: &'a str,
    pub(super) field_is_collection: bool,
    pub(super) field_is_optional: bool,
    pub(super) result_var: &'a str,
    pub(super) result_is_option: bool,
    pub(super) kotlin_android_style: bool,
    pub(super) is_streaming: bool,
    pub(super) not_error_may_assert_presence: bool,
}

/// The presence/misc half of the original `match`: `not_empty`/`is_empty` (which need
/// `result_is_option`/`kotlin_android_style` for their Optional-vs-nullable presence templates),
/// `not_error`/`error`/`method_result`, and the final catch-all panic. ~keep
fn render_presence_arm(out: &mut String, assertion: &Assertion, context: PresenceArmContext) {
    let PresenceArmContext {
        field_expr,
        string_field_expr,
        field_is_collection,
        field_is_optional,
        result_var,
        result_is_option,
        kotlin_android_style,
        is_streaming,
        not_error_may_assert_presence,
    } = context;
    match assertion.assertion_type.as_str() {
        "not_empty" | "is_empty" => render_empty_presence_arm(
            out,
            assertion,
            EmptyPresenceContext {
                result_is_option,
                kotlin_android_style,
                field_is_collection,
                field_is_optional,
                field_expr,
                string_field_expr,
            },
        ),
        // See `not_error::render_not_error` for why this is not a no-op. WHETHER it may assert
        // presence at all is decided once, centrally, by `not_error_presence::may_assert_presence`
        // -- passed in as `not_error_may_assert_presence` -- not re-derived here.
        "not_error" => {
            super::not_error::render_not_error(out, result_var, not_error_may_assert_presence, is_streaming);
        }
        "error" => {
            // Handled at the test method level.
        }
        "method_result" => {
            // Placeholder: Kotlin support for method_result would need sample_language integration.
            let _ = writeln!(
                out,
                "        // method_result assertions not yet implemented for Kotlin"
            );
        }
        other => {
            panic!("Kotlin e2e generator: unsupported assertion type: {other}");
        }
    }
}

fn render_equals_arm(
    out: &mut String,
    assertion: &Assertion,
    field_is_long: bool,
    string_expr: &str,
    nonnull_field_expr: &str,
) {
    if let Some(expected) = &assertion.value {
        // Suffix integer literals with `L` when the target field is a Java `long`
        // (uint64_t / int64_t in C FFI terms). Without the suffix, Kotlin infers
        // the literal as `Int`, causing a type mismatch with `Long` at runtime.
        let kotlin_val = if field_is_long && expected.is_number() && !expected.is_f64() {
            format!("{}L", expected)
        } else {
            super::values::json_to_kotlin(expected)
        };
        if expected.is_string() {
            let _ = writeln!(out, "        assertEquals({kotlin_val}, {string_expr})");
        } else {
            let _ = writeln!(out, "        assertEquals({kotlin_val}, {nonnull_field_expr})");
        }
    }
}

fn render_contains_arm(out: &mut String, assertion: &Assertion, field_is_collection: bool, string_expr: &str) {
    if let Some(expected) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(expected);
        if field_is_collection {
            // `(list as List<String>)` is an unchecked erasure cast that
            // succeeds at runtime even for `List<StructureItem>` etc.
            // `.contains("Module")` then compares records against a
            // String and always fails. Stringifying the collection
            // mirrors the Java emitter (`toString().toLowerCase().contains(...)`)
            // and matches both `List<String>` and `List<ComplexType>`.
            let _ = writeln!(
                out,
                "        assertTrue({string_expr}.toString().lowercase().contains({kotlin_val}.toString().lowercase()), \"expected to contain: \" + {kotlin_val})"
            );
        } else {
            // String substring check. Use the field expression directly so
            // `String.contains(CharSequence)` resolves without a cast.
            let _ = writeln!(
                out,
                "        assertTrue({string_expr}.contains({kotlin_val}), \"expected to contain: \" + {kotlin_val})"
            );
        }
    }
}

fn render_contains_all_arm(out: &mut String, assertion: &Assertion, field_is_collection: bool, string_expr: &str) {
    if let Some(values) = &assertion.values {
        for val in values {
            let kotlin_val = super::values::json_to_kotlin(val);
            if field_is_collection {
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.toString().lowercase().contains({kotlin_val}.toString().lowercase()), \"expected to contain: \" + {kotlin_val})"
                );
            } else {
                let _ = writeln!(
                    out,
                    "        assertTrue({string_expr}.contains({kotlin_val}), \"expected to contain: \" + {kotlin_val})"
                );
            }
        }
    }
}

fn render_not_contains_arm(out: &mut String, assertion: &Assertion, field_is_collection: bool, string_expr: &str) {
    for expected in assertion.expected_values() {
        let kotlin_val = super::values::json_to_kotlin(expected);
        if field_is_collection {
            let _ = writeln!(
                out,
                "        assertFalse({string_expr}.toString().lowercase().contains({kotlin_val}.toString().lowercase()), \"expected NOT to contain: \" + {kotlin_val})"
            );
        } else {
            let _ = writeln!(
                out,
                "        assertFalse({string_expr}.contains({kotlin_val}), \"expected NOT to contain: \" + {kotlin_val})"
            );
        }
    }
}

/// For optional fields, the field type may be a non-String object (e.g. DocumentStructure) for
/// which `.orEmpty()` is undefined. A null-check is the safe primitive: it works for any
/// reference type and matches the Java codegen's `Optional.ofNullable(...).isEmpty()`. When the
/// bare result is `T?` (result_is_option) the same null-check applies, because `.isEmpty()` is
/// undefined on arbitrary nullable types. The JVM Kotlin e2e tests call the Java facade class
/// which returns `java.util.Optional<T>` for option results — use `.isPresent` rather than `!=
/// null` so the assertion semantics match the JVM return type. The kotlin-android wrapper
/// unwraps `Optional<T>` to Kotlin's `T?` at the boundary, so its bare-option result is a
/// nullable reference and must use `!= null` instead.
#[derive(Clone, Copy)]
struct EmptyPresenceContext<'a> {
    result_is_option: bool,
    kotlin_android_style: bool,
    field_is_collection: bool,
    field_is_optional: bool,
    field_expr: &'a str,
    string_field_expr: &'a str,
}

fn render_empty_presence_arm(out: &mut String, assertion: &Assertion, context: EmptyPresenceContext<'_>) {
    if assertion.assertion_type == "not_empty" {
        render_not_empty_arm(out, assertion, context);
    } else {
        render_is_empty_arm(out, assertion, context);
    }
}

fn render_not_empty_arm(out: &mut String, assertion: &Assertion, context: EmptyPresenceContext<'_>) {
    let bare_result_is_option =
        context.result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    if bare_result_is_option && !context.kotlin_android_style {
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/not_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("{}.isPresent", context.field_expr) },
        ));
    } else if context.field_is_collection && (bare_result_is_option || context.field_is_optional) {
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/not_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("{}?.isNotEmpty() == true", context.field_expr) },
        ));
    } else if bare_result_is_option || context.field_is_optional {
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/not_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("{} != null", context.field_expr) },
        ));
    } else {
        let _ = writeln!(
            out,
            "        assertFalse({}.isEmpty(), \"expected non-empty value\")",
            context.string_field_expr
        );
    }
}

fn render_is_empty_arm(out: &mut String, assertion: &Assertion, context: EmptyPresenceContext<'_>) {
    let bare_result_is_option =
        context.result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    if bare_result_is_option && !context.kotlin_android_style {
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/is_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("{}.isEmpty", context.field_expr) },
        ));
    } else if context.field_is_collection && (bare_result_is_option || context.field_is_optional) {
        // Symmetric with `not_empty`'s `field_is_collection && (bare_result_is_option ||
        // field_is_optional)` branch above: an optional collection reached through
        // `field_is_optional` (e.g. `Option<Vec<T>>`) must null-check before calling
        // `.isEmpty()`, or a genuinely-empty-but-present collection throws instead of
        // asserting true. `?: true` treats a null (absent) collection as empty too,
        // matching every other backend's "null counts as empty" semantics for this
        // assertion type. ~keep
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/is_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("({}?.isEmpty() ?: true)", context.field_expr) },
        ));
    } else if bare_result_is_option || context.field_is_optional {
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/is_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("{} == null", context.field_expr) },
        ));
    } else {
        out.push_str(&crate::e2e::template_env::render(
            "kotlin/is_empty_assertion.kt.jinja",
            crate::alef_context! { predicate => format!("{}.isEmpty()", context.string_field_expr) },
        ));
    }
}

fn render_contains_any_arm(out: &mut String, assertion: &Assertion, string_expr: &str) {
    if let Some(values) = &assertion.values {
        let checks: Vec<String> = values
            .iter()
            .map(|v| {
                let kotlin_val = super::values::json_to_kotlin(v);
                format!("{string_expr}.contains({kotlin_val})")
            })
            .collect();
        let joined = checks.join(" || ");
        let _ = writeln!(
            out,
            "        assertTrue({joined}, \"expected to contain at least one of the specified values\")"
        );
    }
}

fn render_greater_than_arm(out: &mut String, assertion: &Assertion, nonnull_field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(val);
        let _ = writeln!(
            out,
            "        assertTrue({nonnull_field_expr} > {kotlin_val}, \"expected > {kotlin_val}\")"
        );
    }
}

fn render_less_than_arm(out: &mut String, assertion: &Assertion, nonnull_field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(val);
        let _ = writeln!(
            out,
            "        assertTrue({nonnull_field_expr} < {kotlin_val}, \"expected < {kotlin_val}\")"
        );
    }
}

fn render_greater_than_or_equal_arm(out: &mut String, assertion: &Assertion, nonnull_field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(val);
        let _ = writeln!(
            out,
            "        assertTrue({nonnull_field_expr} >= {kotlin_val}, \"expected >= {kotlin_val}\")"
        );
    }
}

fn render_less_than_or_equal_arm(out: &mut String, assertion: &Assertion, nonnull_field_expr: &str) {
    if let Some(val) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(val);
        let _ = writeln!(
            out,
            "        assertTrue({nonnull_field_expr} <= {kotlin_val}, \"expected <= {kotlin_val}\")"
        );
    }
}

fn render_starts_with_arm(out: &mut String, assertion: &Assertion, string_expr: &str) {
    if let Some(expected) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(expected);
        let _ = writeln!(
            out,
            "        assertTrue({string_expr}.startsWith({kotlin_val}), \"expected to start with: \" + {kotlin_val})"
        );
    }
}

fn render_ends_with_arm(out: &mut String, assertion: &Assertion, string_expr: &str) {
    if let Some(expected) = &assertion.value {
        let kotlin_val = super::values::json_to_kotlin(expected);
        let _ = writeln!(
            out,
            "        assertTrue({string_expr}.endsWith({kotlin_val}), \"expected to end with: \" + {kotlin_val})"
        );
    }
}

/// For simple result types (ByteArray), use .size; for String use .length
fn render_min_length_arm(
    out: &mut String,
    assertion: &Assertion,
    result_is_simple: bool,
    field_expr: &str,
    result_var: &str,
    string_field_expr: &str,
) {
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let length_accessor = if result_is_simple && field_expr == result_var {
            "size"
        } else {
            "length"
        };
        let _ = writeln!(
            out,
            "        assertTrue({string_field_expr}.{length_accessor} >= {n}, \"expected {length_accessor} >= {n}\")"
        );
    }
}

/// For simple result types (ByteArray), use .size; for String use .length
fn render_max_length_arm(
    out: &mut String,
    assertion: &Assertion,
    result_is_simple: bool,
    field_expr: &str,
    result_var: &str,
    string_field_expr: &str,
) {
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let length_accessor = if result_is_simple && field_expr == result_var {
            "size"
        } else {
            "length"
        };
        let _ = writeln!(
            out,
            "        assertTrue({string_field_expr}.{length_accessor} <= {n}, \"expected {length_accessor} <= {n}\")"
        );
    }
}

fn render_count_min_arm(out: &mut String, assertion: &Assertion, nonnull_field_expr: &str) {
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let _ = writeln!(
            out,
            "        assertTrue({nonnull_field_expr}.size >= {n}, \"expected at least {n} elements\")"
        );
    }
}

fn render_count_equals_arm(out: &mut String, assertion: &Assertion, nonnull_field_expr: &str) {
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let _ = writeln!(
            out,
            "        assertEquals({n}, {nonnull_field_expr}.size, \"expected exactly {n} elements\")"
        );
    }
}

/// `T?`: "is_true" means "present" -- `field_expr == true` never type-errors in Kotlin (`==` is
/// Any?-to-Any? structural equality) but it also never matches a non-Boolean nullable (e.g.
/// `DataNode?`), so the assertion always fails at runtime even when the field is present. `!=
/// null` is the interpretation that holds for any T, matching the Rust `.is_some()` convention
/// for this assertion type. ~keep
fn render_is_true_arm(out: &mut String, field_is_optional: bool, field_expr: &str) {
    if field_is_optional {
        let _ = writeln!(
            out,
            "        assertTrue({field_expr} != null, \"expected true (non-null)\")"
        );
    } else {
        let _ = writeln!(out, "        assertTrue({field_expr} == true, \"expected true\")");
    }
}

fn render_is_false_arm(out: &mut String, field_is_optional: bool, field_expr: &str) {
    if field_is_optional {
        let _ = writeln!(
            out,
            "        assertTrue({field_expr} == null, \"expected false (null)\")"
        );
    } else {
        let _ = writeln!(out, "        assertTrue({field_expr} == false, \"expected false\")");
    }
}
