use crate::codegen::naming::csharp_type_name;
use crate::core::config::HostCapsuleTypeConfig;
use crate::core::ir::{ApiSurface, PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

/// True when `ty` crosses the C ABI as the scalar `AlefHandle` (`uint64_t`) rather than a
/// real pointer. Mirrors `FfiParamMapper`/`FfiReturnMapper::named` in
/// `backends::ffi::type_map`, which map *every* `TypeRef::Named` — opaque handle or
/// JSON-backed data struct alike — to `AlefHandle`, including through `Optional`.
///
/// This is the *return*-position and default answer only. In parameter position the FFI backend
/// narrows a `Named` whose type is `Copy` to `i32`, so `[DllImport]` parameters must go through
/// [`pinvoke_param_type_with_scalars`] rather than reading this predicate directly. ~keep
pub(super) fn is_handle_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(_)),
        _ => false,
    }
}

/// C# names (already run through [`csharp_type_name`]) of every enum that needs the
/// `{Type}ToJson`/`{Type}Free` round trip on return.
///
/// Every enum in `api.enums` is boxed exactly like a plain data struct: the FFI crate's
/// `gen_owned_value_to_c` (`backends::ffi::gen_bindings::helpers`) has no enum-ness branch at
/// all for owned return conversion, so *every* `TypeRef::Named` return — enum or struct,
/// fieldless or data-carrying — crosses via `insert_handle` and is declared
/// [`HANDLE_PINVOKE_TYPE`]. A prior revision of this function excluded fieldless-only enums,
/// reasoning by analogy with the FFI crate's `LibSetupContext::ffi_param_enums`
/// (`backends::ffi::gen_bindings::lib_setup`) — but that set governs *parameter*-position
/// scalar-discriminant passing only, not return-position boxing, and the two never agreed on
/// return values. A consumer's `RefreshOutcome` (a fieldless-only enum: `Disabled`,
/// `FromCache`, `Fetched`) is a real instance: its FFI header exports
/// `<prefix>_refresh_outcome_to_json`/`<prefix>_refresh_outcome_free` unconditionally on
/// return-position boxing, but the excluded-fieldless filter here routed its C# wrapper straight
/// to `Marshal.PtrToStringUTF8(nativeResult)` on the raw `ulong` handle — the CS1503
/// `ulong`-to-`nint` defect that broke that consumer's v1.17.3 C# NuGet build. ~keep
pub(super) fn enum_names_with_data_variants(api: &ApiSurface) -> HashSet<String> {
    api.enums
        .iter()
        .map(|enum_def| csharp_type_name(&enum_def.name))
        .collect()
}

/// The P/Invoke return type of a host-native capsule function.
///
/// A capsule return is *not* an `AlefHandle`: the FFI crate exports it as a raw
/// `*const T` owned by the host runtime — see
/// `backends::ffi::gen_bindings::capsule::capsule_c_return_type` — so it crosses as a real
/// pointer even though the IR spells it `TypeRef::Named`. Every other C-ABI backend agrees
/// (Dart declares `Pointer<Void>`, Zig a `*const` extern). ~keep
pub(super) const CAPSULE_PINVOKE_RETURN_TYPE: &str = "IntPtr";

/// The P/Invoke type of an alef-owned `AlefHandle` — the handle-registry key the FFI crate
/// emits as `type AlefHandle = u64` / `typedef uint64_t {PREFIX}AlefHandle`. Every declaration
/// in this backend that carries a handle across the boundary reads this constant, including the
/// hand-rolled streaming `extern`s in `functions::gen_native_methods`, so the width is stated
/// once instead of being spelled `"ulong"` at each emitter.
///
/// It does **not** unify how the two `SafeHandle` templates *store* that value, and those two
/// conventions are still live and deliberately left alone:
/// `templates/opaque_handle_header.jinja:41` packs the `ulong` bit-pattern into `SafeHandle`'s
/// single `IntPtr` slot (`Pack`/`Unpack`), while `templates/service_class_header.jinja:15-19`
/// keeps a private `ulong _nativeHandle` and stores `new IntPtr(1)` in the slot as a
/// liveness sentinel. They differ in what `IsInvalid` and `ReleaseHandle` read, so collapsing
/// them changes finalization behaviour, not just spelling — it needs its own change with its
/// own generated-output verification. ~keep
pub(super) const HANDLE_PINVOKE_TYPE: &str = "ulong";

/// Which FFI emitter produced the C symbol a `[DllImport]` binds to.
///
/// The two FFI emitters do **not** agree about capsule returns, and the disagreement is
/// deliberate rather than a bug to paper over — so the C# declaration has to mirror whichever C
/// signature actually exists. This enum is the one place that shape rule is written down; both
/// P/Invoke emitters derive their return type through [`pinvoke_return_type_with_capsules`]
/// rather than each deciding for itself. ~keep
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FfiEmitter {
    /// `backends::ffi::gen_bindings::functions::orchestration::gen_free_function` is
    /// capsule-aware: at `orchestration.rs:723` a configured capsule return is emitted as
    /// `capsule::capsule_c_return_type` — `*const {into_raw_type}`, owned by the host runtime —
    /// so the value crosses as a real pointer and is declared [`CAPSULE_PINVOKE_RETURN_TYPE`].
    FreeFunction,
    /// `...::orchestration::gen_method_wrapper` (`orchestration.rs:69`) takes no capsule config
    /// at all and falls through to `c_return_type_with_paths`, which maps every `Named` to
    /// `AlefHandle` (`backends/ffi/type_map.rs:279`). Commit `6855c6d57` made that intentional: a
    /// capsule type reachable as a method return keeps its opaque `_new`/`_free` exports (the
    /// `capsule_used_as_opaque` sets at `ffi/gen_bindings/lib_rs.rs:176` and
    /// `ffi/gen_bindings/helpers.rs:396`) and is boxed by alef, so the method's C signature really
    /// is `uint64_t`. Declaring [`CAPSULE_PINVOKE_RETURN_TYPE`] on this path would be a live ABI
    /// mismatch, not a fix.
    Method,
}

/// True when `ty` is a host-native capsule return whose value crosses as a raw pointer.
///
/// Deliberately matches only a bare `Named`, exactly like the capsule-wrapper router in
/// `methods::class`, which calls this: if the router and the P/Invoke declaration disagreed
/// about which functions are capsules, the wrapper's zero check and the `extern` signature
/// would be derived from different facts — the CS0034 `ulong`/`nint` ambiguity this
/// predicate exists to prevent. ~keep
pub(super) fn is_capsule_return(ty: &TypeRef, capsule_types: &HashMap<String, HostCapsuleTypeConfig>) -> bool {
    matches!(ty, TypeRef::Named(name) if capsule_types.contains_key(name))
}

/// The C# zero-sentinel literal that pairs with an already-emitted P/Invoke return type.
///
/// This is the single source of truth for null checks: callers must pass the *same* string
/// the `[DllImport]` declaration was emitted from, so a check can never be derived from a
/// different fact than the signature it guards. ~keep
pub(super) fn zero_sentinel_for_pinvoke_type(pinvoke_ty: &str) -> &'static str {
    if pinvoke_ty == CAPSULE_PINVOKE_RETURN_TYPE {
        "IntPtr.Zero"
    } else {
        "0"
    }
}

/// The C# zero-sentinel literal for a P/Invoke return value: the scalar `0` for a value
/// carried as `AlefHandle`, or `IntPtr.Zero` for a value carried as a real native pointer.
pub(super) fn zero_sentinel(ty: &TypeRef) -> &'static str {
    zero_sentinel_for_pinvoke_type(pinvoke_return_type(ty))
}

/// Returns the C# type to use in a `[DllImport]` declaration for the given return type.
///
/// Key differences from the high-level `csharp_type`:
/// - Bool is marshalled as `int` (C FFI convention) — the wrapper compares != 0.
/// - Named types (and `Optional<Named>`) come back as `ulong`, matching the `AlefHandle`
///   (`uint64_t`) scalar the FFI crate registers them behind — see `is_handle_type`.
/// - `Optional<Primitive>` / `Optional<Duration>` come back as the bare primitive C# type — see
///   [`optional_scalar_pinvoke_return_type`].
/// - String / Vec / Map / Path / Json / Bytes all come back as `IntPtr` (real pointers).
/// - Numeric primitives use their natural C# types (`nuint`, `int`, etc.).
pub(super) fn pinvoke_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "void",
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::Isize) => "long",
        TypeRef::Duration => "ulong",
        _ if is_handle_type(ty) => HANDLE_PINVOKE_TYPE,
        TypeRef::Optional(inner) => optional_scalar_pinvoke_return_type(inner).unwrap_or("IntPtr"),
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Named(_)
        | TypeRef::Path
        | TypeRef::Json => "IntPtr",
    }
}

/// The P/Invoke return type for `Optional<inner>` when the FFI crate exports it as a raw scalar
/// rather than a pointer.
///
/// The DECISION of which shapes cross as a scalar is not made here — it is asked of
/// [`crate::backends::ffi::type_map::optional_return_crosses_as_scalar`], the FFI layer that
/// actually emits the symbol. This function only maps the resolved leaf to its C# spelling.
/// Keeping a second copy of the branching is what produced the defect this fixes: `Option<u64>`
/// was declared `IntPtr` here while the FFI crate exported a raw scalar, so the wrapper read the
/// integer bit pattern as a pointer and passed it to `FreeString` — an arbitrary-address free.
/// A private mirror had already drifted from the authority on `Option<Option<Duration>>` before
/// this call replaced it. `None` means the FFI crate emits a real pointer and the caller should
/// fall back to `IntPtr`. ~keep
fn optional_scalar_pinvoke_return_type(inner: &TypeRef) -> Option<&'static str> {
    if !crate::backends::ffi::type_map::optional_return_crosses_as_scalar(inner) {
        return None;
    }
    Some(match inner {
        TypeRef::Optional(nested) => pinvoke_return_type(nested),
        _ => pinvoke_return_type(inner),
    })
}

/// Returns the C# `[DllImport]` return type, accounting for host-native capsule returns.
///
/// This is the one function every `extern` return type is emitted from — free functions and
/// methods alike — so the two P/Invoke emitters can no longer disagree by simply not knowing
/// capsules exist. `emitter` selects which FFI-side C signature is being mirrored; see
/// [`FfiEmitter`] for why the two answers legitimately differ. Pair the result with
/// [`zero_sentinel_for_pinvoke_type`] to derive the matching null check. ~keep
pub(super) fn pinvoke_return_type_with_capsules(
    ty: &TypeRef,
    capsule_types: &HashMap<String, HostCapsuleTypeConfig>,
    emitter: FfiEmitter,
) -> &'static str {
    if emitter == FfiEmitter::FreeFunction && is_capsule_return(ty, capsule_types) {
        CAPSULE_PINVOKE_RETURN_TYPE
    } else {
        pinvoke_return_type(ty)
    }
}

/// Returns the C# type to use for a parameter in a `[DllImport]` declaration.
///
/// Managed reference types (Vec, Map, Bytes) and strings/paths/json cannot be directly
/// marshalled by P/Invoke and cross as `IntPtr`/`string` (real pointers). Named types (and
/// `Optional<Named>`) cross as `ulong` — see `is_handle_type`. Primitive types use their
/// natural C# numeric types.
pub(super) fn pinvoke_param_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "string",
        _ if is_handle_type(ty) => HANDLE_PINVOKE_TYPE,
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes | TypeRef::Optional(_) => "IntPtr",
        TypeRef::Unit => "void",
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "ulong",
        TypeRef::Primitive(PrimitiveType::Isize) => "long",
        TypeRef::Duration => "ulong",
    }
}

/// The P/Invoke type of a `Named` parameter the C FFI crate declares as
/// `type_map::SCALAR_NAMED_C_PARAM_TYPE` (`i32`, rendered `int32_t` by cbindgen).
///
/// Declared next to [`HANDLE_PINVOKE_TYPE`] so the two widths a `Named` parameter can take are
/// stated in one place. ~keep
pub(super) const SCALAR_NAMED_PINVOKE_TYPE: &str = "int";

/// Returns the C# `[DllImport]` parameter type, resolving a `Named` position against the same
/// scalar/handle split the C FFI crate emits.
///
/// `scalar_named_types` must come from `backends::ffi::type_map::scalar_c_abi_named_types` — the
/// one construction of that set — so the C# declaration and the C header are two renderings of a
/// single fact instead of two independent assertions. Declaring `ulong` where the header says
/// `int32_t` is not merely a C# compile error against the wrapper's `(int)` cast: it is a live
/// ABI violation, and the absent-sentinel `-1` that `named_param_enum_optional.jinja` emits is
/// not even representable in `ulong`. ~keep
pub(super) fn pinvoke_param_type_with_scalars(
    ty: &TypeRef,
    scalar_named_types: &ahash::AHashSet<String>,
) -> &'static str {
    if crate::backends::ffi::type_map::crosses_c_abi_as_scalar(ty, scalar_named_types) {
        SCALAR_NAMED_PINVOKE_TYPE
    } else {
        pinvoke_param_type(ty)
    }
}

/// Returns true if a parameter should be hidden from the public API because it is a
/// trait-bridge param (e.g. the FFI visitor handle).
pub(super) fn is_bridge_param(
    param: &crate::core::ir::ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    bridge_param_names.contains(&param.name)
        || matches!(&param.ty, crate::core::ir::TypeRef::Named(n) if bridge_type_aliases.contains(n))
}

/// Does the return type need IntPtr→string marshalling in the wrapper?
pub(super) fn returns_string(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json)
}

/// Does the return type come back as a C int that should be converted to bool?
///
/// Includes `Optional<bool>`: `c_return_optional` maps it to a bare `i32` exactly like a
/// non-optional bool (see [`optional_scalar_pinvoke_return_type`]), so the wrapper must convert
/// via `!= 0` rather than fall through to a raw scalar assignment — `int` does not implicitly
/// convert to `bool?`.
pub(super) fn returns_bool_via_int(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Primitive(PrimitiveType::Bool))
        || matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::Bool)))
}

/// Does the return type need JSON deserialization from an IntPtr string?
///
/// `Optional<Primitive>` / `Optional<Duration>` are deliberately excluded: those cross the C ABI
/// as a raw scalar (see [`optional_scalar_pinvoke_return_type`]), not a pointer, so treating them
/// as a JSON object would read the scalar's bit pattern as an address — the exact memory-unsafe
/// defect this predicate used to cause for `Option<u64>` returns.
pub(super) fn returns_json_object(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) | TypeRef::Bytes
    ) || matches!(ty, TypeRef::Optional(inner) if optional_scalar_pinvoke_return_type(inner).is_none())
}

/// Returns true if the FFI return value carries its own absent/failure sentinel — a real null
/// pointer, or the reserved `AlefHandle` zero — as opposed to a bare numeric value.
///
/// The `Optional` arm asks [`crate::backends::ffi::type_map::optional_return_crosses_as_scalar`],
/// the FFI layer that actually emits the symbol, rather than matching `Optional(_)`
/// unconditionally. That unconditional match is the defect this replaces: `Option<i64>` crosses
/// as a bare `i64`, so the wrapper emitted `if (nativeResult == 0) { return null; }` and reported
/// a legitimate `Some(0)` — and `Some(false)`, and a zero-valued `Duration` — as absent. It also
/// shadowed the wrapper's `else if error_type.is_some()` arm, so a genuine FFI failure on an
/// optional-returning call surfaced as `null` rather than an exception. Absence for those shapes
/// is recovered from the presence companion instead; see `gen_bindings::result_presence`. ~keep
pub(super) fn returns_ptr(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Named(_)
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Bytes => true,
        TypeRef::Optional(inner) => !crate::backends::ffi::type_map::optional_return_crosses_as_scalar(inner),
        _ => false,
    }
}

/// Returns the argument expression to pass to the native method for a given parameter.
///
/// For truly opaque types (is_opaque = true), the C# class wraps an IntPtr; pass `.Handle`.
/// For data-struct `Named` types this is the handle variable (e.g. `optionsHandle`).
/// For everything else it is the parameter name (with `!` for optional).
pub(super) fn native_call_arg(
    ty: &TypeRef,
    param_name: &str,
    optional: bool,
    true_opaque_types: &HashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(type_name) if true_opaque_types.contains(type_name) => {
            let bang = if optional { "!" } else { "" };
            format!("{param_name}{bang}.Handle")
        }
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!("{param_name}Handle")
        }
        TypeRef::Bytes => {
            format!("{param_name}Handle.AddrOfPinnedObject()")
        }
        TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
            if optional {
                format!("(({param_name} ?? false) ? 1 : 0)")
            } else {
                format!("({param_name} ? 1 : 0)")
            }
        }
        ty => {
            if optional {
                if let TypeRef::Primitive(prim) = ty {
                    use crate::core::ir::PrimitiveType;
                    let sentinel = match prim {
                        PrimitiveType::U8 => "byte.MaxValue",
                        PrimitiveType::U16 => "ushort.MaxValue",
                        PrimitiveType::U32 => "uint.MaxValue",
                        PrimitiveType::U64 | PrimitiveType::Usize => "ulong.MaxValue",
                        PrimitiveType::I8 => "sbyte.MaxValue",
                        PrimitiveType::I16 => "short.MaxValue",
                        PrimitiveType::I32 => "int.MaxValue",
                        PrimitiveType::I64 | PrimitiveType::Isize => "long.MaxValue",
                        PrimitiveType::F32 => "float.NaN",
                        PrimitiveType::F64 => "double.NaN",
                        PrimitiveType::Bool => unreachable!("handled above"),
                    };
                    format!("{param_name} ?? {sentinel}")
                } else if matches!(ty, TypeRef::Duration) {
                    format!("{param_name}.GetValueOrDefault()")
                } else {
                    format!("{param_name}!")
                }
            } else {
                param_name.to_string()
            }
        }
    }
}

/// Build the byte-slice length argument passed to a native call.
///
/// `cast` is the C# cast prefix the P/Invoke length parameter expects (e.g. `"(UIntPtr)"`
/// or `"(nuint)"`). For optional `byte[]?` parameters the array may be null — pinning a
/// null array yields `IntPtr.Zero` and a zero length, so we null-coalesce the length to
/// `0` rather than dereferencing `.Length` (which would trip CS8602 under
/// `<TreatWarningsAsErrors>` / nullable-reference analysis).
pub(super) fn bytes_len_arg(cast: &str, param_name: &str, optional: bool) -> String {
    if optional {
        format!("{cast}({param_name}?.Length ?? 0)")
    } else {
        format!("{cast}{param_name}.Length")
    }
}

/// The positional argument expressions a wrapper passes to its P/Invoke stub, in order.
///
/// Built once per wrapper and consumed twice: by the primary call, and by the presence
/// companion's gate (`gen_bindings::result_presence`). The companion's C signature *is* the
/// primary's parameter list, so constructing the list a second time for the gate is exactly how
/// the two calls drift into disagreeing about arity. `receiver` is the receiver argument for an
/// instance method and `None` for a free function or a static method — the same distinction the
/// FFI backend makes when it decides whether the companion takes a leading handle. ~keep
pub(super) fn native_call_args(
    receiver: Option<&str>,
    params: &[crate::core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
    bytes_len_cast: &str,
) -> Vec<String> {
    let mut args: Vec<String> = receiver.into_iter().map(str::to_string).collect();
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        args.push(native_call_arg(
            &param.ty,
            &param_name,
            param.optional,
            true_opaque_types,
        ));
        if matches!(param.ty, TypeRef::Bytes) {
            args.push(bytes_len_arg(bytes_len_cast, &param_name, param.optional));
        }
    }
    args
}

/// Returns true when wrapper setup allocates a temporary handle that must be
/// released after the native call.
pub(super) fn needs_param_teardown(
    params: &[crate::core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> bool {
    params.iter().any(|param| match &param.ty {
        TypeRef::Named(type_name) => !true_opaque_types.contains(type_name) && !enum_names.contains(type_name),
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => true,
        _ => false,
    })
}

/// For each `Named` parameter, emit code to serialise it to JSON and obtain a native handle.
///
/// For truly opaque types (is_opaque = true), the C# class already wraps the native handle, so
/// we pass `param.Handle` directly without any JSON serialisation.
pub(super) fn emit_named_param_setup(
    out: &mut String,
    params: &[crate::core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
    exception_name: &str,
    _types: &[TypeDef],
    enum_names: &HashSet<String>,
) {
    use crate::backends::csharp::template_env::render;

    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let json_var = format!("{param_name}Json");
        let handle_var = format!("{param_name}Handle");

        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                if enum_names.contains(type_name) {
                    if param.optional {
                        out.push_str(&render(
                            "named_param_enum_optional.jinja",
                            crate::alef_context! { indent, handle_var, param_name },
                        ));
                    } else {
                        out.push_str(&render(
                            "named_param_enum_required.jinja",
                            crate::alef_context! { indent, handle_var, param_name },
                        ));
                    }
                    continue;
                }
                let from_json_method = format!("{}FromJson", csharp_type_name(type_name));

                if param.optional {
                    out.push_str(&crate::backends::csharp::template_env::render(
                        "named_param_handle_from_json_optional.jinja",
                        crate::alef_context! {
                            indent,
                            handle_var => &handle_var,
                            from_json_method => &from_json_method,
                            json_var => &json_var,
                            param_name => &param_name,
                            exception_name => exception_name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::csharp::template_env::render(
                        "named_param_json_serialize.jinja",
                        crate::alef_context! { indent, json_var => &json_var, param_name => &param_name },
                    ));
                    out.push_str(&crate::backends::csharp::template_env::render(
                        "named_param_handle_from_json.jinja",
                        crate::alef_context! {
                            indent,
                            handle_var => &handle_var,
                            from_json_method => &from_json_method,
                            json_var => &json_var,
                            exception_name => exception_name,
                        },
                    ));
                }
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_json_serialize.jinja",
                    crate::alef_context! { indent, json_var => &json_var, param_name => &param_name },
                ));
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_handle_string.jinja",
                    crate::alef_context! { indent, handle_var => &handle_var, json_var => &json_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_handle_pin.jinja",
                    crate::alef_context! { indent, handle_var => &handle_var, param_name => &param_name },
                ));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code to free native handles allocated for `Named` parameters.
///
/// Truly opaque handles (is_opaque = true) are NOT freed here — their lifetime is managed by
/// the C# wrapper class (IDisposable). Only data-struct handles (from_json-allocated) are freed.
/// Enums are not freed (they are stack values, not heap-allocated).
pub(super) fn emit_named_param_teardown(
    out: &mut String,
    params: &[crate::core::ir::ParamDef],
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let handle_var = format!("{param_name}Handle");
        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                if enum_names.contains(type_name) {
                    continue;
                }
                let free_method = format!("{}Free", csharp_type_name(type_name));
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_free.jinja",
                    crate::alef_context! { indent => "        ", free_method => &free_method, handle_var => &handle_var },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_hglobal.jinja",
                    crate::alef_context! { indent => "        ", handle_var => &handle_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_gchandle.jinja",
                    crate::alef_context! { indent => "        ", handle_var => &handle_var },
                ));
            }
            _ => {}
        }
    }
}

/// Emit cleanup code with configurable indentation (used inside `Task.Run` lambdas).
pub(super) fn emit_named_param_teardown_indented(
    out: &mut String,
    params: &[crate::core::ir::ParamDef],
    indent: &str,
    true_opaque_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) {
    for param in params {
        let param_name = param.name.to_lower_camel_case();
        let handle_var = format!("{param_name}Handle");
        match &param.ty {
            TypeRef::Named(type_name) => {
                if true_opaque_types.contains(type_name) {
                    continue;
                }
                if enum_names.contains(type_name) {
                    continue;
                }
                let free_method = format!("{}Free", csharp_type_name(type_name));
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_free.jinja",
                    crate::alef_context! { indent, free_method => &free_method, handle_var => &handle_var },
                ));
            }
            TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_hglobal.jinja",
                    crate::alef_context! { indent, handle_var => &handle_var },
                ));
            }
            TypeRef::Bytes => {
                out.push_str(&crate::backends::csharp::template_env::render(
                    "named_param_teardown_gchandle.jinja",
                    crate::alef_context! { indent, handle_var => &handle_var },
                ));
            }
            _ => {}
        }
    }
}
