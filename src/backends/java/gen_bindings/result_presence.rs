//! Java's consumer side of the FFI presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`, so `None` and a legitimate
//! `Some(0)` arrive at the Panama downcall as the same bits. Java then wrapped that value with an
//! unconditional `Optional.of(...)` / `OptionalLong.of(...)`, which is *never* empty — so before
//! this gate every `None` reached the caller as `Optional[0]`, and the facade's `.orElse(null)`
//! turned it into a `0L` rather than a `null`. The FFI backend exports an additive
//! `{fn}_has_result` companion that answers presence out of band; this module supplies both the
//! `MethodHandle` that binds it and the conditional return expression that consults it.
//!
//! Whether the companion exists at all is asked of
//! [`crate::backends::ffi::type_map::result_presence_companion_exists`] — the same predicate the
//! FFI backend uses to decide whether to export the symbol. Re-deriving "is this an ambiguous
//! `Option` leaf" here would let the two sides drift, and a `MethodHandle` for a symbol the FFI
//! crate never exported fails at class-initialization time with
//! `ExceptionInInitializerError: Native symbol not found` — the whole binding refuses to load,
//! not just the one call. See `two-generators-disagree` in the repo's skill set. ~keep

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::codegen::c_consumer::result_presence_symbol;
use crate::core::ir::{ReceiverKind, TypeRef};

/// The companion's "result is present" return value. `0` means absent and `-1` means the
/// companion itself failed (bad handle, param conversion, caught panic), so the emitted test is
/// `== 1` rather than `!= 0` — treating a failed companion as "present" would hand the caller the
/// primary downcall's sentinel as if it were real data. ~keep
const PRESENT: &str = "1";

/// The Java local the presence downcall is captured into. Distinct from `result` and
/// `primitiveResult`, the two names the primary invocations already bind in the same scope.
const PRESENCE_LOCAL: &str = "presenceResult";

/// The companion always returns `i32` regardless of the primary's return shape, so its descriptor
/// is a fixed layout rather than anything derived from the return type. ~keep
const PRESENCE_RETURN_LAYOUT: &str = "ValueLayout.JAVA_INT";

/// Suffix distinguishing a companion's `MethodHandle` constant from the primary's.
///
/// One definition, shared by the declaration in `native_lib` and by the call sites in the two
/// result emitters, so the handle can never be declared under one name and invoked under another.
/// ~keep
const HANDLE_SUFFIX: &str = "_HAS_RESULT";

/// The `MethodHandle` constant name for a primary handle's presence companion.
pub(super) fn presence_handle_name(primary_handle: &str) -> String {
    format!("{primary_handle}{HANDLE_SUFFIX}")
}

/// The `_HAS_RESULT` downcall-handle declaration for one export, or `None` when the FFI crate
/// exports no companion for this return type and receiver.
///
/// `primary_handle_name` is the primary export's `MethodHandle` constant (`{PREFIX}_{FUNC}`),
/// `primary_ffi_name` its C symbol; the companion's spellings are derived from those through
/// [`presence_handle_name`] and [`result_presence_symbol`] so each has exactly one definition.
/// `param_layouts` must be the primary's own layout vector — the companion's C signature *is* the
/// primary's parameter list, so reusing the caller's already-computed vector keeps the two
/// descriptors from disagreeing about arity.
///
/// `receiver` is passed straight through to the eligibility authority: the companion re-invokes
/// the underlying method to observe presence, which an owned receiver cannot survive because its
/// first call already removed the handle from the registry.
pub(super) fn presence_handle_declaration(
    return_type: &TypeRef,
    receiver: Option<&ReceiverKind>,
    primary_handle_name: &str,
    primary_ffi_name: &str,
    param_layouts: &[String],
) -> Option<String> {
    if !result_presence_companion_exists(return_type, receiver) {
        return None;
    }
    Some(crate::backends::java::template_env::render(
        "method_handle_presence.jinja",
        crate::alef_context! {
            handle_name => presence_handle_name(primary_handle_name),
            ffi_name => result_presence_symbol(primary_ffi_name),
            layout => super::marshal::gen_function_descriptor(PRESENCE_RETURN_LAYOUT, param_layouts),
        },
    ))
}

/// The statement capturing the companion's answer, or `None` when no companion exists.
///
/// Emitted *before* the primary invocation, never after: the companion clears the FFI crate's
/// last-error slot on entry, so running it second would wipe an error the primary had just
/// recorded and `checkLastError()` would see a clean slate. ~keep
///
/// `primary_handle` is the fully qualified primary handle (`NativeLib.{PREFIX}_{FUNC}`) and
/// `call_args` the same argument text the primary invocation passes.
pub(super) fn presence_capture(
    return_type: &TypeRef,
    receiver: Option<&ReceiverKind>,
    primary_handle: &str,
    call_args: &str,
) -> Option<String> {
    if !result_presence_companion_exists(return_type, receiver) {
        return None;
    }
    Some(presence_capture_line(&presence_handle_name(primary_handle), call_args))
}

/// The same capture statement for a caller that already holds the resolved companion handle.
///
/// The opaque-method path resolves eligibility once when it builds its symbol table and keeps the
/// companion handle rather than the `MethodDef`, so it cannot go back through
/// [`presence_capture`]'s predicate — it would have to rebuild the return type and receiver the
/// predicate judges. Both entry points render the one template. ~keep
pub(super) fn presence_capture_line(companion_handle: &str, call_args: &str) -> String {
    crate::backends::java::template_env::render(
        "presence_capture.jinja",
        crate::alef_context! {
            local => PRESENCE_LOCAL,
            handle => companion_handle,
            call_args => call_args,
        },
    )
}

/// Wrap an already-built `Optional`/`OptionalLong` construction in the presence test.
///
/// `present_expr` is what the caller would have returned unconditionally before the presence
/// channel existed; `empty_expr` is the matching empty constructor for the same Optional flavour.
/// Callers pass both so this function never has to know which flavour is in play.
pub(super) fn presence_conditional(present_expr: &str, empty_expr: &str) -> String {
    format!("{PRESENCE_LOCAL} == {PRESENT} ? {present_expr} : {empty_expr}")
}
