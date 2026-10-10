//! Dart's consumer side of the FFI presence channel, for the opt-in `dart.style = "ffi"` mode.
//!
//! The default `frb` style is not a consumer of this channel at all: flutter_rust_bridge generates
//! Dart from alef's Rust facade, so an `Option<i64>` stays a Rust `Option` the whole way and
//! lowers to a real Dart `null`. Only the raw `dart:ffi` style crosses the C ABI, where `None` and
//! a legitimate `Some(0)` are the same bits.
//!
//! This layer sits *above* a separate fix: `native_return_type` used to fall through to
//! `Pointer<Void>` for every `Optional`, so the typedef disagreed with the `int64_t` the FFI crate
//! actually returns. A presence gate over a wrong-width call would have meant nothing, so
//! `type_map` now asks `c_return_type` for the shape before this gate guards it. ~keep
//!
//! Whether the companion exists at all is asked of
//! [`crate::backends::ffi::type_map::result_presence_companion_exists`] — the same predicate the
//! FFI backend uses to decide whether to export the symbol. Re-deriving "is this an ambiguous
//! `Option` leaf" here would let the two sides drift, and `lookupFunction` resolves eagerly at
//! top-level initialization: a symbol the crate never exported throws `ArgumentError` when the
//! library loads, taking the whole binding down rather than one call. See
//! `two-generators-disagree` in the repo's skill set. ~keep

use crate::backends::dart::template_env;
use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::codegen::c_consumer::result_presence_symbol;
use crate::core::ir::TypeRef;

/// The companion's "result is present" return value. `0` means absent and `-1` means the
/// companion itself failed (param conversion, caught panic), so the emitted test is `!= 1` rather
/// than `== 0` — treating a failed companion as "present" would hand the caller the primary
/// lookup's sentinel as if it were real data. ~keep
const PRESENT: &str = "1";

/// The companion always returns `i32` regardless of the primary's return shape.
const PRESENCE_NATIVE_RETURN: &str = "Int32";
const PRESENCE_DART_RETURN: &str = "int";

/// Suffix distinguishing the companion's generated Dart identifiers from the primary's. One
/// definition, shared by the lookup and by the call site, so the binding can never be declared
/// under one name and invoked under another. ~keep
const HAS_RESULT_SUFFIX: &str = "HasResult";

/// The primary's declared parameter lists, reused verbatim for the companion.
///
/// The companion's C signature *is* the primary export's parameter list, so passing the caller's
/// already-joined text keeps the two typedefs from disagreeing about arity or width. ~keep
pub(super) struct PrimarySignature<'a> {
    pub fn_name: &'a str,
    pub c_symbol: &'a str,
    pub native_params: &'a str,
    pub dart_params: &'a str,
}

/// Emit the companion's typedefs and `lookupFunction`, and return the Dart identifier stem the
/// gate calls it by — or `None` when the FFI crate exports no companion for this return type.
///
/// Free functions are the only `dart:ffi` emission surface (`gen_ffi::emit` walks `api.functions`
/// and nothing else), so the receiver passed to the eligibility authority is always `None`.
pub(super) fn emit_presence_lookup(
    return_type: &TypeRef,
    primary: &PrimarySignature<'_>,
    out: &mut String,
) -> Option<String> {
    if !result_presence_companion_exists(return_type, None) {
        return None;
    }

    let fn_name = format!("{}{HAS_RESULT_SUFFIX}", primary.fn_name);
    let typedef_native = format!("_{fn_name}Native");
    let typedef_dart = format!("_{fn_name}Dart");

    out.push_str(&template_env::render(
        "ffi_typedef_native_sig.jinja",
        crate::alef_context! {
            typedef_native => typedef_native.as_str(),
            native_return => PRESENCE_NATIVE_RETURN,
            native_params => primary.native_params,
        },
    ));
    out.push_str(&template_env::render(
        "ffi_typedef_dart_sig.jinja",
        crate::alef_context! {
            typedef_dart => typedef_dart.as_str(),
            dart_return => PRESENCE_DART_RETURN,
            dart_params => primary.dart_params,
        },
    ));
    out.push_str(&template_env::render(
        "ffi_function_lookup_sig.jinja",
        crate::alef_context! {
            dart_return => PRESENCE_DART_RETURN,
            dart_params => primary.dart_params,
            fn_name => fn_name.as_str(),
            typedef_native => typedef_native.as_str(),
            typedef_dart => typedef_dart.as_str(),
            c_symbol => result_presence_symbol(primary.c_symbol),
        },
    ));

    Some(fn_name)
}

/// The guard the wrapper emits immediately before its primary call.
///
/// Before, never after: every FFI wrapper clears the crate's last-error slot on entry, so a
/// companion invoked second would erase an error the primary had just recorded — and running it
/// first is also what lets `_checkError()` here see the companion's own `-1` failure instead of
/// reporting it as a clean absence. Because the gate returns early it also has to run the
/// parameter teardown the wrapper would otherwise run after the call. ~keep
pub(super) fn presence_gate(fn_name: &str, call_args: &str, cleanup: &str, checks_error: bool) -> String {
    template_env::render(
        "ffi_result_presence_gate.jinja",
        crate::alef_context! {
            fn_name,
            call_args,
            present => PRESENT,
            cleanup,
            checks_error,
        },
    )
}

/// Re-indent a fragment emitted at the wrapper's own statement indent so it can sit one block
/// deeper, inside the gate.
pub(super) fn indent_fragment(fragment: &str, extra: &str) -> String {
    fragment
        .lines()
        .map(|line| format!("{extra}{line}\n"))
        .collect::<String>()
}
