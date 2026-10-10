//! C#'s consumer side of the FFI presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`, so `None` and a legitimate
//! `Some(0)` arrive at the P/Invoke stub as the same bits. C#'s defect here was worse than a
//! missing channel: `returns_ptr` classified *every* `Optional` as pointer-shaped, so a scalar
//! optional got `if (nativeResult == 0) { return null; }` — a legitimate `Some(0)` (and
//! `Some(false)`, and a zero `Duration`) was reported to the caller as absent. The same branch
//! also shadowed the wrapper's `else if error_type.is_some()` arm, so a genuine FFI failure on an
//! optional-returning call surfaced as `null` instead of an exception.
//!
//! The FFI backend exports an additive `{fn}_has_result` companion that answers presence out of
//! band. This module supplies both the `[DllImport]` that binds it and the guard that consults
//! it. Whether the companion exists at all is asked of
//! [`crate::backends::ffi::type_map::result_presence_companion_exists`] — the same predicate the
//! FFI backend uses to decide whether to export the symbol. Re-deriving "is this an ambiguous
//! `Option` leaf" here would let the two sides drift, and a `[DllImport]` for a symbol the FFI
//! crate never exported fails at first call with `EntryPointNotFoundException`. See
//! `two-generators-disagree` in the repo's skill set. ~keep

use crate::backends::csharp::template_env::render;
use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::codegen::c_consumer::result_presence_symbol;
use crate::core::ir::{ReceiverKind, TypeRef};

/// The companion's "result is present" return value. `0` means absent and `-1` means the
/// companion itself failed (bad handle, param conversion, caught panic), so the emitted test is
/// `!= 1` rather than `== 0` — treating a failed companion as "present" would hand the caller the
/// primary stub's sentinel as if it were real data. ~keep
const PRESENT: &str = "1";

/// The companion always returns `i32` regardless of the primary's return shape, so its P/Invoke
/// return type is fixed rather than derived from the return type. ~keep
const PRESENCE_PINVOKE_RETURN_TYPE: &str = "int";

/// Suffix distinguishing a companion's P/Invoke stub from the primary's.
///
/// One definition, shared by the declaration in [`presence_declaration`] and by the call site in
/// [`presence_gate`], so the stub can never be declared under one name and invoked under
/// another. ~keep
const HAS_RESULT_SUFFIX: &str = "HasResult";

/// The `NativeMethods` stub name for a primary stub's presence companion.
pub(super) fn presence_cs_name(primary_cs_name: &str) -> String {
    format!("{primary_cs_name}{HAS_RESULT_SUFFIX}")
}

/// The companion's `[DllImport]` declaration for one export, or `None` when the FFI crate exports
/// no companion for this return type and receiver.
///
/// `primary_c_name` is the primary export's C symbol and `primary_cs_name` its `NativeMethods`
/// stub name; the companion's spellings are derived from those through [`result_presence_symbol`]
/// and [`presence_cs_name`] so each has exactly one definition. `params` must be the primary's
/// own already-rendered parameter block — the companion's C signature *is* the primary's
/// parameter list, so reusing the caller's text keeps the two declarations from disagreeing about
/// arity or width.
///
/// `receiver` is passed straight through to the eligibility authority: the companion re-invokes
/// the underlying method to observe presence, which an owned receiver cannot survive because its
/// first call already removed the handle from the registry.
pub(super) fn presence_declaration(
    return_type: &TypeRef,
    receiver: Option<&ReceiverKind>,
    primary_c_name: &str,
    primary_cs_name: &str,
    params: &str,
) -> Option<String> {
    if !result_presence_companion_exists(return_type, receiver) {
        return None;
    }
    let mut out = render(
        "dll_import_attr.jinja",
        crate::alef_context! { entry_point => result_presence_symbol(primary_c_name) },
    );
    out.push_str(&render(
        "pinvoke_declaration.jinja",
        crate::alef_context! {
            return_type => PRESENCE_PINVOKE_RETURN_TYPE,
            cs_name => presence_cs_name(primary_cs_name),
            params,
        },
    ));
    Some(out)
}

/// The guard a C# wrapper emits immediately before its primary P/Invoke call, or `None` when the
/// FFI crate exports no companion for this return type and receiver.
///
/// Emitted *before* the primary call, never after: every FFI wrapper clears the crate's
/// last-error slot on entry, so running the companion second would wipe an error the primary had
/// just recorded. Emitting it first also means `failure_block` still sees the companion's own
/// `-1` failure, which would otherwise be indistinguishable from a clean absence. ~keep
///
/// `args` must be the same comma-separated argument text the primary call passes.
/// `failure_block` is the caller's own already-indented last-error throw — the two wrapper
/// families use different exception idioms, so the block is supplied rather than chosen here.
/// Pass an empty string for an infallible wrapper, which reports absence and nothing else.
pub(super) fn presence_gate(
    return_type: &TypeRef,
    receiver: Option<&ReceiverKind>,
    primary_cs_name: &str,
    args: &str,
    indent: &str,
    failure_block: &str,
) -> Option<String> {
    if !result_presence_companion_exists(return_type, receiver) {
        return None;
    }
    Some(render(
        "result_presence_gate.jinja",
        crate::alef_context! {
            cs_name => presence_cs_name(primary_cs_name),
            args,
            present => PRESENT,
            indent,
            failure_block,
            absent => absent_expr(return_type),
        },
    ))
}

/// The wrapper's absent value, spelled as an explicit cast rather than a bare `null`.
///
/// A bare `null` is fine in a plain method body but not inside the `Task.Run(() => { ... })`
/// lambda the async wrappers emit: the lambda's return type is inferred from every `return` in
/// it, and `null` alongside the marshalling block's `return returnValue;` (a non-nullable `long`
/// for a scalar optional) has no best common type — CS0173. Casting pins the candidate set to
/// `{long?, long}`, whose best common type is `long?`. ~keep
fn absent_expr(return_type: &TypeRef) -> String {
    format!("({})null", crate::backends::csharp::type_map::csharp_type(return_type))
}
