//! Zig's consumer side of the FFI presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`, so `None` and a legitimate
//! `Some(0)` arrive at the Zig wrapper as the same bits. `unwrap_return_expr` passes that raw
//! value straight through and lets Zig coerce `i64` into the declared `?i64` — a coercion that is
//! *never* null, so before this gate every `None` reached the caller as `0`.
//!
//! Zig needs no declaration for the companion: the backend pulls the entire C surface in through
//! one build-system-translated C header import (see `c_import.jinja`), so
//! `c.{sym}_has_result` is already in scope wherever the primary is. What it does need is to call
//! the companion only when the FFI
//! crate actually exported one, which is asked of
//! [`crate::backends::ffi::type_map::result_presence_companion_exists`] — the same predicate
//! `gen_method_result_presence_wrapper` uses to decide whether to emit the symbol. Re-deriving
//! "is this an ambiguous `Option` leaf" here would let the two sides drift, and C translation
//! resolves externs at comptime: a name the header never declared is a build error in the
//! consumer's package, not a wrong value. See `two-generators-disagree` in the repo's skill set.
//! ~keep

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::backends::zig::template_env::render;
use crate::codegen::c_consumer::result_presence_symbol;
use crate::core::ir::{ReceiverKind, TypeRef};

/// The companion's "result is present" return value. `0` means absent and `-1` means the
/// companion itself failed (bad handle, param conversion, caught panic), so the gate tests for
/// `!= 1` rather than `== 0` — treating a failed companion as "present" would hand the caller the
/// primary getter's sentinel as if it were real data. ~keep
const PRESENT: &str = "1";

/// The indent a free-function wrapper body sits at.
const FUNCTION_INDENT: &str = "    ";

/// The indent an opaque-handle method body sits at — one level deeper, inside the struct.
pub(super) const METHOD_INDENT: &str = "        ";

/// Re-indent a fragment that was emitted at the wrapper's own indent so it can sit one block
/// deeper, inside the gate.
///
/// The parameter-teardown emitters write at a fixed indent because every other caller emits them
/// at statement level; the gate is the one place they land inside a block. ~keep
fn indent_fragment(fragment: &str, extra: &str) -> String {
    fragment
        .lines()
        .map(|line| format!("{extra}{line}\n"))
        .collect::<String>()
}

/// The presence guard for a free-function wrapper, or `None` when the FFI crate exports no
/// companion for this return type.
///
/// Emitted before the primary call, never after: every FFI wrapper clears the crate's last-error
/// slot on entry, so a companion invoked second would erase an error the primary had just
/// recorded. Because the gate returns early it has to run the parameter teardown the wrapper
/// would otherwise run after the call, so that teardown is rebuilt here at the gate's indent. ~keep
pub(super) fn free_function_presence_gate(
    func: &crate::core::ir::FunctionDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    primary_c_call: &str,
    error_type: Option<&str>,
) -> Option<String> {
    if !result_presence_companion_exists(&func.return_type, None) {
        return None;
    }
    let mut teardown = String::new();
    for param in &func.params {
        super::functions::emit_param_free(param, prefix, struct_names, opaque_creator_map, &mut teardown);
    }
    result_presence_gate(
        PresenceTarget {
            return_type: &func.return_type,
            receiver: None,
        },
        primary_c_call,
        prefix,
        FUNCTION_INDENT,
        &indent_fragment(&teardown, FUNCTION_INDENT),
        error_type,
    )
}

/// The return shape and (optional) receiver identifying which companion presence check applies.
/// Kept together because both feed [`result_presence_companion_exists`] as a single eligibility
/// query, and grouping them keeps `result_presence_gate` at 6 parameters. ~keep
pub(super) struct PresenceTarget<'a> {
    pub(super) return_type: &'a TypeRef,
    pub(super) receiver: Option<&'a ReceiverKind>,
}

/// Render the presence guard a Zig wrapper emits immediately before its primary C call, or `None`
/// when the FFI crate exports no companion for this return type and receiver.
///
/// `primary_c_call` is the wrapper's own already-built call expression
/// (`c.{sym}({args})`); the companion's call is derived from it rather than rebuilt, so the two
/// cannot disagree about arity, and the `_has_result` spelling still comes from
/// [`result_presence_symbol`] so the suffix has exactly one definition.
///
/// `target.receiver` is passed straight through to the eligibility authority: the companion
/// re-invokes the underlying method to observe presence, which an owned receiver cannot survive
/// because its first call already removed the handle from the registry.
///
/// `cleanup` is the parameter teardown the wrapper would otherwise run after the call — the gate
/// returns early, so it has to run it too. `error_type` is the wrapper's resolved Zig error set
/// when it is fallible; the gate then reports the companion's own `-1` failure as that error
/// instead of silently claiming absence. Emitting it here is safe for
/// `assert_error_set_covers_body` because the surrounding body already returns the same error.
pub(super) fn result_presence_gate(
    target: PresenceTarget<'_>,
    primary_c_call: &str,
    prefix: &str,
    indent: &str,
    cleanup: &str,
    error_type: Option<&str>,
) -> Option<String> {
    if !result_presence_companion_exists(target.return_type, target.receiver) {
        return None;
    }

    let (callee, rest) = primary_c_call
        .split_once('(')
        .unwrap_or_else(|| panic!("a Zig C call expression must have an argument list: {primary_c_call}"));
    let symbol = callee
        .strip_prefix("c.")
        .unwrap_or_else(|| panic!("a Zig C call expression must be translated-C-module-qualified: {primary_c_call}"));
    let args = rest
        .strip_suffix(')')
        .unwrap_or_else(|| panic!("a Zig C call expression must close its argument list: {primary_c_call}"));

    let failure_block = error_type.map_or_else(String::new, |error_type| {
        render(
            "result_presence_error_check.jinja",
            minijinja::context! {
                indent => format!("{indent}    "),
                prefix,
                error_type,
            },
        )
    });

    Some(render(
        "result_presence_gate.jinja",
        minijinja::context! {
            symbol => result_presence_symbol(symbol),
            args,
            present => PRESENT,
            indent,
            cleanup,
            failure_block,
        },
    ))
}

#[cfg(test)]
mod tests;
