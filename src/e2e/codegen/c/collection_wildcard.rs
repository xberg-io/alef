//! `field[].key` wildcard-leaf handling for the C e2e assertion generator.
//!
//! ~keep New module rather than growing `assertions.rs` (already well over the repo's
//! 1,000-line cap; see `file-modularization` in CLAUDE.md) or `call_patterns.rs` /
//! `test_function.rs`. A `field[].key` fixture path means "some element of the array
//! satisfies this", but `emit_nested_accessor`'s json-extraction leaf used to collapse it to
//! a scalar `alef_json_get_string(array_json, "key")` lookup against the ARRAY's own JSON
//! text — a lookup that can never find a `"key"` property on a JSON array, so every
//! "contains"-shaped assertion built from it asserted an unsatisfiable condition (see
//! `structure[].kind` in a consumer's generated `test_process.c`, alef task
//! #59). This module gives the leaf a real per-element quantifier instead, matching the
//! `.iter().any(..)` / `Enum.any?` / `any(...)` shape every other e2e backend already uses for
//! this fixture path shape (`rust/assertions.rs`, `python/assertions.rs`,
//! `elixir/assertions.rs`, ...).
//!
//! It also folds the primitive/opaque local-variable classification that used to be
//! copy-pasted inline at each of the three call sites (`call_patterns.rs`,
//! `test_function.rs` x2) into one function: the third repetition of identical dispatch logic
//! is what licenses extracting it (`avoid-duplication`).

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use crate::e2e::fixture::Assertion;

use super::{is_primitive_c_type, json_to_c};

/// What a leaf segment of a dotted field-access path resolved to, handed back by
/// `emit_nested_accessor` so its three call sites can register the local consistently.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NestedLeafOutcome {
    /// A primitive C scalar type name, an opaque struct's snake_case type name, or the
    /// `"__skip__"` sentinel. [`classify_nested_leaf`] tells these apart with
    /// `is_primitive_c_type`.
    Typed(String),
    /// A `field[].key` wildcard leaf. No scalar local was declared for it — `array_var` names
    /// the already-emitted JSON array variable and `key_snake` the key to extract from each of
    /// its elements once an assertion is rendered against this field.
    Wildcard { array_var: String, key_snake: String },
}

/// Classify an `emit_nested_accessor` leaf outcome into the caller's local-variable buckets.
///
/// Before this existed, `test_function.rs`'s first call site filed EVERY nested leaf under
/// `primitive_locals` regardless of what it actually was — including opaque handle types,
/// which the other two call sites correctly routed to `opaque_handle_locals` via
/// `is_primitive_c_type`. Unifying the three call sites onto this one function fixes that
/// divergence as a side effect of giving the wildcard case somewhere to go. ~keep
pub(super) fn classify_nested_leaf(
    outcome: NestedLeafOutcome,
    local_var: &str,
    primitive_locals: &mut HashMap<String, String>,
    opaque_handle_locals: &mut HashMap<String, String>,
    wildcard_locals: &mut HashMap<String, (String, String)>,
) {
    match outcome {
        NestedLeafOutcome::Typed(type_name) => {
            if type_name == "__skip__" || is_primitive_c_type(&type_name) {
                primitive_locals.insert(local_var.to_string(), type_name);
            } else {
                opaque_handle_locals.insert(local_var.to_string(), type_name);
            }
        }
        NestedLeafOutcome::Wildcard { array_var, key_snake } => {
            wildcard_locals.insert(local_var.to_string(), (array_var, key_snake));
        }
    }
}

/// Render a quantifier assertion for a `field[].key` wildcard: some (or, for `not_contains`,
/// no) element of the JSON array named by `array_var` has a `key_snake` value satisfying the
/// assertion's predicate.
///
/// `contains`, `contains_all`, `contains_any`, `not_contains`, `equals` and `not_empty` are
/// implemented — the assertion type tslp's own fixtures use against wildcard leaves
/// (`structure[].kind`, `imports[].source`) plus its natural string-comparison siblings.
/// `not_empty` matches the sibling wildcard renderers in `python`/`dart`/`elixir`'s
/// `assertions.rs`, which all read a wildcard `not_empty` the same way: some element's key
/// holds a non-empty value. Anything else renders a skip comment rather than a silently-wrong
/// quantifier.
pub(super) fn render_wildcard_assertion(out: &mut String, assertion: &Assertion, array_var: &str, key_snake: &str) {
    match assertion.assertion_type.as_str() {
        "contains" => {
            if let Some(expected) = &assertion.value {
                render_quantifier(
                    out,
                    array_var,
                    key_snake,
                    &[json_to_c(expected)],
                    false,
                    "contains",
                    "expected some element to contain a substring",
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                // Each expected value must show up in SOME element, not necessarily the same
                // one — one quantifier block per value, matching the scalar `contains_all`
                // arm's one-`assert`-per-value shape a few lines up in `assertions.rs`.
                for val in values {
                    render_quantifier(
                        out,
                        array_var,
                        key_snake,
                        &[json_to_c(val)],
                        false,
                        "contains",
                        "expected some element to contain a substring",
                    );
                }
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let needles: Vec<String> = values.iter().map(json_to_c).collect();
                render_quantifier(
                    out,
                    array_var,
                    key_snake,
                    &needles,
                    false,
                    "contains",
                    "expected some element to contain at least one of the specified values",
                );
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                render_quantifier(
                    out,
                    array_var,
                    key_snake,
                    &[json_to_c(expected)],
                    true,
                    "contains",
                    "expected no element to contain a substring",
                );
            }
        }
        "equals" => {
            if let Some(expected) = &assertion.value {
                render_quantifier(
                    out,
                    array_var,
                    key_snake,
                    &[json_to_c(expected)],
                    false,
                    "equals",
                    "expected some element to equal the expected value",
                );
            }
        }
        "not_empty" => {
            // No needle to compare against — every other sibling backend's wildcard `not_empty`
            // (`python/dart/elixir/assertions.rs`) asks the same question: does SOME element's
            // key hold a non-empty value. `compare_mode: "not_empty"` routes the jinja template
            // past the needle loop entirely rather than iterating zero needles.
            render_quantifier(
                out,
                array_var,
                key_snake,
                &[],
                false,
                "not_empty",
                "expected some element to have a non-empty value",
            );
        }
        other => {
            let field = assertion.field.as_deref().unwrap_or("");
            let _ = writeln!(
                out,
                "    // skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

fn render_quantifier(
    out: &mut String,
    array_var: &str,
    key_snake: &str,
    needles: &[String],
    negate: bool,
    compare_mode: &str,
    message: &str,
) {
    out.push_str(&crate::e2e::template_env::render(
        "c/wildcard_collection_assertion.jinja",
        crate::alef_context! {
            array_var => array_var,
            key_snake => key_snake,
            needles => needles,
            negate => negate,
            compare_mode => compare_mode,
            message => message,
        },
    ));
}
