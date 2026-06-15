//! Emits function shims that call through to the underlying Rust crate.
//!
//! Each public `FunctionDef` gets a `pub fn` wrapper that:
//!   - accepts bridge types (String for JSON-bridged params, newtypes for Named)
//!   - converts parameters to native Rust types
//!   - calls the source function
//!   - converts the return value back to a bridge type
//!   - for async fns, blocks on a current-thread Tokio runtime

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_type_enum_aware_ref, bridge_type_with_handles, needs_json_bridge, needs_json_bridge_with_handles,
    swift_bridge_rust_type,
};
use crate::backends::swift::naming::swift_rust_shim_ident as swift_ident;
use crate::core::ir::{FunctionDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Returns true when a function can be fully bridged.
///
/// A function is unbridgeable when any parameter is an enum bridge wrapper (no reverse From),
/// any tuple-vec parameter has an unbridgeable inner type (e.g. `Vec<u8>,`), when the
/// return type requires JSON bridging but the inner Named type lacks serde, or when any
/// parameter is a Result type (Result types cannot be represented across the C FFI).
pub(crate) fn is_bridgeable_fn(
    f: &FunctionDef,
    unit_enum_names: &std::collections::HashSet<&str>,
    type_paths: &HashMap<String, String>,
    no_serde_names: &std::collections::HashSet<&str>,
    no_serde_enum_names: &std::collections::HashSet<&str>,
    handle_returned_types: &HashSet<String>,
) -> bool {
    for p in &f.params {
        // Skip functions with Result parameters — Results cannot be represented in C FFI.
        if matches!(&p.ty, TypeRef::Named(n) if n.starts_with("Result") || n == "Result") {
            return false;
        }
        match &p.ty {
            TypeRef::Named(n) if unit_enum_names.contains(n.as_str()) => {
                // Skip only if it's a ref (can't serialize refs) OR has no serde (can't serialize at all).
                // A serde enum by-value IS bridgeable.
                if p.is_ref || no_serde_enum_names.contains(n.as_str()) {
                    return false;
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if unit_enum_names.contains(n.as_str()) && (p.is_ref || no_serde_enum_names.contains(n.as_str())) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    // Tuple-vec with Vec<u8> inner: the IR-flattened bridge shape cannot be
    // losslessly converted back into `Vec<(Vec<u8>, T)>`.
    for p in &f.params {
        let Some(original) = p.original_type.as_deref() else {
            continue;
        };
        let stripped = original
            .trim()
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .replace(' ', "");
        if stripped.starts_with("Vec(Named(\"(Vec<u8>,") {
            return false;
        }
    }
    // Unbridgeable JSON return: inner Named type excluded or lacks serde.
    fn inner_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    if needs_json_bridge_with_handles(&f.return_type, handle_returned_types) {
        if let Some(inner_name) = inner_named(&f.return_type) {
            if !type_paths.contains_key(inner_name) || no_serde_names.contains(inner_name) {
                return false;
            }
        }
    }
    true
}

/// Build the call-site expression for a function parameter.
///
/// Handles JSON-bridged types, Path conversion, primitive casts, and reference borrows
/// based on `is_ref`/`optional`. Named types are wrapped as `pub struct T(pub SourceT)`,
/// so accessing the inner source type requires `.0` indirection.
pub(crate) fn swift_call_arg(
    p: &crate::core::ir::ParamDef,
    unit_enum_names: &HashSet<&str>,
    tagged_enum_names: &HashSet<&str>,
    type_paths: &HashMap<String, String>,
) -> String {
    let name = p.name.to_snake_case();
    let original = p.original_type.as_deref().unwrap_or("");
    let stripped_orig = original
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim();

    // Tuple parameters: same shape as the Dart codegen — IR flattens
    // `Vec<(A, B, ...)>` to `Vec<String>` and stores the original signature in
    // `original_type`. Reconstruct sensible defaults for known tuple shapes.
    if !stripped_orig.is_empty() && stripped_orig.starts_with("Vec(") && stripped_orig.contains("Named(\"(") {
        let tuple_inner = stripped_orig
            .find("Named(\"(")
            .and_then(|start| {
                let rest = &stripped_orig[start + 8..];
                rest.find(")\")")
                    .map(|end| rest[..end].trim_end_matches(')').to_string())
            })
            .unwrap_or_default();
        if tuple_inner.starts_with("PathBuf,") || tuple_inner.starts_with("PathBuf ,") {
            return format!("{name}.into_iter().map(|p| (std::path::PathBuf::from(p), None)).collect::<Vec<_>>()");
        }
        if tuple_inner.starts_with("Vec<u8>,") || tuple_inner.starts_with("Vec<u8> ,") {
            return format!(
                "{{ let _ = {name}; compile_error!(\"alef cannot bridge Vec<(Vec<u8>, ...)> through Swift; configure swift.exclude_functions for this item\") }}"
            );
        }
    }

    let source_type = |type_name: &str| {
        type_paths
            .get(type_name)
            .cloned()
            .unwrap_or_else(|| type_name.to_string())
            .replace('-', "_")
    };

    if let TypeRef::Named(n) = &p.ty {
        if unit_enum_names.contains(n.as_str()) {
            let native_ty = source_type(n);
            // Swift bridges unit enum values as plain wire strings (e.g. "person"), not as
            // JSON-encoded strings (e.g. "\"person\"").  Using serde_json::from_str on
            // an unquoted string fails.  Use `From<String>` instead, which alef unit enums
            // implement. Tagged enums (with variants containing fields) must use JSON deserialization.
            let from_expr = format!("<{native_ty} as ::std::convert::From<String>>::from({name})");
            if p.optional {
                return format!("{name}.map(|s| <{native_ty} as ::std::convert::From<String>>::from(s))");
            }
            return from_expr;
        }
    }

    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Named(n) = inner.as_ref() {
            if unit_enum_names.contains(n.as_str()) {
                let native_ty = source_type(n);
                // Same rationale: Swift delivers plain wire strings for unit enums; convert each via
                // `From<String>`.  The resulting `Vec<EnumType>` is then passed as a
                // slice reference (`&converted`) when `is_ref` is true.
                let map_expr = format!(
                    "values.into_iter().map(|s| <{native_ty} as ::std::convert::From<String>>::from(s)).collect::<Vec<_>>()"
                );
                let converted = if p.optional {
                    format!("{name}.map(|values| {map_expr})")
                } else {
                    format!("{{ let values = {name}; {map_expr} }}")
                };
                if p.is_ref && !p.optional {
                    // Core expects `&[EnumType]`; coerce `&Vec<T>` to `&[T]`. The temporary
                    // Vec lives for the enclosing call statement.
                    return format!("&{{ let values = {name}; {map_expr} }}");
                }
                return converted;
            }
            // Tagged enums (variants with fields) are JSON-serialized by Swift and must be
            // deserialized here. Swift bridges Vec<TaggedEnum> as Vec<String> (JSON-encoded).
            if tagged_enum_names.contains(n.as_str()) {
                let native_ty = source_type(n);
                let map_expr = format!(
                    "values.into_iter().map(|s| ::serde_json::from_str::<{native_ty}>(&s).expect(\"valid JSON for {name} element\")).collect::<Vec<_>>()"
                );
                let converted = if p.optional {
                    format!("{name}.map(|values| {map_expr})")
                } else {
                    format!("{{ let values = {name}; {map_expr} }}")
                };
                if p.is_ref && !p.optional {
                    // Core expects `&[EnumType]`; the temporary Vec lives for the enclosing call.
                    return format!("&{{ let values = {name}; {map_expr} }}");
                }
                return converted;
            }
        }
    }

    // AHashMap<Cow<'static, str>, Value> params: swift-bridge receives these as
    // HashMap<String, String> (user-friendly types). We need a two-step conversion:
    // (1) bind an owned AHashMap to a named `let` before the call so we can borrow it,
    // (2) pass the reference in the call arg. This requires pre-call binding emission
    // in emit_function_shim; here we return a reference to the pre-bound variable.
    if let TypeRef::Map(_, _) = &p.ty {
        if p.map_is_ahash && p.map_key_is_cow {
            let bound_name = format!("__{}_ahash", p.name);
            return if p.optional && p.is_ref {
                format!("{bound_name}.as_ref()")
            } else if p.is_ref {
                format!("{bound_name}.as_ref().unwrap()")
            } else {
                bound_name
            };
        }
    }

    // JSON-bridged: deserialize from the bridged String.
    if needs_json_bridge(&p.ty) {
        let native_ty = swift_bridge_rust_type(&p.ty);
        let deser = format!("::serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
        if p.is_ref {
            // When the source function expects a reference, borrow the deserialized value.
            return format!("&{deser}");
        }
        return deser;
    }

    // Path: bridged as String; convert to PathBuf or &Path.
    if matches!(p.ty, TypeRef::Path) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(std::path::Path::new)");
            }
            return format!("{name}.map(std::path::PathBuf::from)");
        }
        if p.is_ref {
            return format!("std::path::Path::new(&{name})");
        }
        return format!("std::path::PathBuf::from({name})");
    }

    // Named types: unwrap the swift-bridge wrapper newtype with `.0`.
    // EXCEPTION: Enum wrappers do not have a `.0` field — they are plain enums.
    // For enums, the parameter is already the correct type (the bridge deserialized it
    // via the JSON-bridge path above at lines 142-152). Just use it directly.
    if let TypeRef::Named(type_name) = &p.ty {
        if unit_enum_names.contains(type_name.as_str()) {
            return name;
        }
        // Struct wrappers have .0; access it with appropriate reference/mutability.
        if p.optional {
            if p.is_ref {
                if p.is_mut {
                    return format!("{name}.as_ref().map(|w| &mut w.0)");
                }
                return format!("{name}.as_ref().map(|w| &w.0)");
            }
            return format!("{name}.map(|w| w.0)");
        }
        if p.is_ref {
            if p.is_mut {
                return format!("&mut {name}.0");
            }
            return format!("&{name}.0");
        }
        return format!("{name}.0");
    }

    // Vec<Named>: elements are bridge wrapper newtypes; unwrap each with `.0`.
    // Vec<Named> is NOT json-bridged (Named is a bridge leaf), so it falls here.
    // Note: enums in Vec are guarded by is_bridgeable_fn, so only struct wrappers
    // should reach here.
    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Named(_) = inner.as_ref() {
            if p.optional {
                if p.is_ref {
                    // The source function expects Option<&[T]>. We collect to a temporary Vec
                    // and call as_deref(). The temporary lives for the enclosing
                    // statement (function call) so the reference is valid.
                    return format!(
                        "{name}.as_ref().map(|v| v.iter().map(|w| w.0.clone()).collect::<Vec<_>>()).as_deref()"
                    );
                }
                return format!("{name}.map(|v| v.into_iter().map(|w| w.0).collect::<Vec<_>>())");
            }
            if p.is_ref {
                // The source function expects &[T]. The temporary Vec lives for the enclosing call.
                return format!("&{name}.iter().map(|w| w.0.clone()).collect::<Vec<_>>()");
            }
            return format!("{name}.into_iter().map(|w| w.0).collect::<Vec<_>>()");
        }
    }

    // Vec<String> with &[&str]: convert Vec<String> to &[&str].
    // Core takes `&[&str]`; swift-bridge delivers `Vec<String>`.
    // Borrow the temporary Vec<&str> into &[&str].
    if p.is_ref
        && p.vec_inner_is_ref
        && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    {
        return format!("&{name}.iter().map(|s| s.as_str()).collect::<Vec<_>>()");
    }

    // Primitive: swift-bridge maps integer widths well, but reference params
    // still need `&`.
    if let TypeRef::Primitive(_) = &p.ty {
        if p.is_ref {
            return format!("&{name}");
        }
        return name;
    }

    if !p.is_ref {
        return name;
    }
    match (&p.ty, p.optional) {
        (TypeRef::Bytes, false) => format!("&{name}"),
        // Char: bridge type is String; convert back to char at the shim boundary.
        // Owned non-optional: extract the first char from the String (default '\0' for empty).
        // Optional ref: map Some(String) → Some(char).
        (TypeRef::Char, false) => format!("{name}.chars().next().unwrap_or('\\0')"),
        (TypeRef::Char, true) => format!("{name}.as_ref().and_then(|s| s.chars().next())"),
        (TypeRef::String, false) => format!("&{name}"),
        (TypeRef::String, true) => format!("{name}.as_deref()"),
        (TypeRef::Vec(_), true) => format!("{name}.as_deref()"),
        _ => format!("&{name}"),
    }
}

pub(crate) fn emit_function_shim(
    f: &FunctionDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    unit_enum_names: &HashSet<&str>,
    tagged_enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    handle_returned_types: &HashSet<String>,
) -> String {
    // Match the extern block's escaping so the wrapper fn matches the extern decl.
    let fn_name = swift_ident(&f.name.to_snake_case());

    // Combine unit and tagged enum names for parameter type bridging.
    // Both are serialized as String/Vec<String> at the Swift boundary.
    let all_enum_names: HashSet<&str> = unit_enum_names
        .iter()
        .chain(tagged_enum_names.iter())
        .copied()
        .collect();

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let bridge_ty = bridge_type_enum_aware_ref(&p.ty, &all_enum_names);
            let bridge_ty = if p.optional {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            // When the core function takes `&mut T` (is_ref=true, is_mut=true) the shim
            // receives the value by move (bridge types are always owned) and then borrows
            // it mutably as `&mut {name}.0`.  The borrow requires the local binding to be
            // declared `mut`, so emit `mut {name}` for by-move Named struct params that
            // will be mutably borrowed in the call.
            let needs_mut = p.is_ref
                && p.is_mut
                && !p.optional
                && matches!(&p.ty, TypeRef::Named(n) if !unit_enum_names.contains(n.as_str()));
            if needs_mut {
                format!("mut {name}: {bridge_ty}")
            } else {
                format!("{name}: {bridge_ty}")
            }
        })
        .collect();
    let params_str = params.join(", ");

    // Mirror the extern-block return mapper so the shim signature matches.
    let return_ty = if f.error_type.is_some() {
        let ok_ty = bridge_type_with_handles(&f.return_type, handle_returned_types);
        if matches!(f.return_type, TypeRef::Unit) {
            "Result<(), String>".to_string()
        } else {
            format!("Result<{ok_ty}, String>")
        }
    } else {
        bridge_type_with_handles(&f.return_type, handle_returned_types)
    };

    // Collect pre-call `let` bindings for params that require a two-step conversion
    // (e.g. AHashMap<Cow, Value> params received as HashMap<String, String>).
    // These must be bound before the call so the reference in the call arg can borrow
    // the owned value rather than a temporary that would drop immediately.
    let mut pre_call_bindings: Vec<String> = Vec::new();

    // Build call args, deserializing JSON-bridged params before passing to the real fn
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            // AHashMap<Cow<'static, str>, Value> params: swift-bridge receives these as
            // HashMap<String, String> (user-friendly types). We need a two-step conversion:
            // (1) bind an owned AHashMap to a named `let` before the call so we can borrow it,
            // (2) pass the reference in the call arg.
            if let TypeRef::Map(_, _) = &p.ty {
                if p.map_is_ahash && p.map_key_is_cow {
                    let bound_name = format!("__{}_ahash", p.name);
                    let name = p.name.to_snake_case();
                    pre_call_bindings.push(format!(
                        "    let {bound_name} = {name}.map(|json_str| {{ let hm = ::serde_json::from_str::<std::collections::HashMap<String, String>>(&json_str).expect(\"valid JSON for {name}\"); hm.into_iter().map(|(k, v)| (std::borrow::Cow::Owned(k), serde_json::Value::String(v))).collect::<ahash::AHashMap<std::borrow::Cow<'static, str>, serde_json::Value>>() }});"
                    ));
                }
            }
            swift_call_arg(p, unit_enum_names, tagged_enum_names, type_paths)
        })
        .collect();
    let call_args_str = call_args.join(", ");

    // Resolve the source call via the IR's full rust_path so module-prefixed
    // functions (e.g. `my_lib::utils::helper`) generate correct shim calls.
    // Falls back to the bare fn name if rust_path is empty.
    let resolved_path = if f.rust_path.is_empty() {
        format!("{source_crate}::{fn_name}")
    } else {
        f.rust_path.replace('-', "_")
    };
    let source_call = format!("{resolved_path}({call_args_str})");

    // If the return type is a JSON-bridged Optional/Vec/Named whose inner Named
    // type is not in the visible type table or does not implement serde, we
    // cannot generate a working serde-based bridge. This should already be filtered
    // by `is_bridgeable_fn`; keep a compile-time diagnostic here for any caller that
    // bypasses that filter.
    fn inner_named_type(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named_type(inner),
            _ => None,
        }
    }
    if needs_json_bridge_with_handles(&f.return_type, handle_returned_types) {
        if let Some(inner_name) = inner_named_type(&f.return_type) {
            if !type_paths.contains_key(inner_name) || no_serde_names.contains(inner_name) {
                // The inner Named type is not in the visible type table or lacks serde.
                // We cannot serde-bridge it.
                let fn_name_snake = swift_ident(&f.name.to_snake_case());
                return format!(
                    "// alef: skipped — return type `{inner_name}` is excluded from codegen (no serde derive)\n\
                     pub fn {fn_name_snake}({params_str}) -> {return_ty} {{\n    \
                     compile_error!(\"alef cannot bridge Swift return type {inner_name}; configure swift.exclude_functions for {fn_name_snake} or expose serde for the type\")\n\
                     }}\n"
                );
            }
        }
    }

    // Wrap return value with JSON serialization when the return type is not natively
    // supported by swift-bridge 0.1.59 (nested generics, HashMap). Named return
    // types must be wrapped in their swift-bridge newtype (`pub struct T(pub SourceT)`).
    // Async fns must `.await` before mapping; sync fns can chain directly.
    //
    // Note on async + Vec<Named>: opaque swift-bridge classes are not Sendable, so
    // crossing `Task.detached.value` with `[OpaqueDTO]` would fail Swift type-check.
    // The forwarder side handles this by emitting `@unchecked Sendable` extensions
    // for opaque DTO classes (see emit_opaque_dto_sendable_extensions). JSON-encoding
    // here only fires when the bridge surface itself cannot represent the type
    // (nested Vec / HashMap), which is what `needs_json_bridge_with_handles` checks.
    let json_wrap_ok = needs_json_bridge_with_handles(&f.return_type, handle_returned_types);

    // Build a wrapper expression for a Named type `t`.
    // Enum wrappers implement From<SourceT> — use T::from(val) or val.map(T::from).
    // Struct newtypes use T(val) constructor directly.
    let wrap_named = |t: &str| -> String {
        if unit_enum_names.contains(t) {
            format!("{t}::from")
        } else {
            t.to_string()
        }
    };
    let wrap_named_direct = |t: &str, source: &str| -> String {
        if unit_enum_names.contains(t) {
            format!("{t}::from({source})")
        } else {
            format!("{t}({source})")
        }
    };

    // Determine the wrapper to apply for Named return types — the swift-bridge
    // wrappers are nominal newtypes, so we need to construct them from the source
    // value. Three cases:
    //   - Named(T) → wrap with `T(value)` or `T::from(value)`
    //   - Optional(Named(T)) → `.map(T)` or `.map(T::from)`
    //   - Vec(Named(T)) → `.into_iter().map(T).collect::<Vec<_>>()`
    enum WrapShape {
        Direct(String), // T(value) or T::from(value)
        OptMap(String), // value.map(T) or value.map(T::from)
        VecMap(String), // value.into_iter().map(T).collect()
    }
    let wrap_shape = match &f.return_type {
        TypeRef::Named(n) => Some(WrapShape::Direct(n.clone())),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(WrapShape::OptMap(n.clone())),
            _ => None,
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(WrapShape::VecMap(n.clone())),
            _ => None,
        },
        _ => None,
    };
    let value_map_string: String = if json_wrap_ok {
        ".map(|v| serde_json::to_string(&v).expect(\"serializable return\"))".to_string()
    } else {
        match &wrap_shape {
            Some(WrapShape::Direct(t)) => format!(".map({})", wrap_named(t)),
            Some(WrapShape::OptMap(t)) => format!(".map(|v| v.map({}))", wrap_named(t)),
            Some(WrapShape::VecMap(t)) => {
                if f.returns_ref {
                    // v is &[T]; iter() yields &T — must clone before wrapping.
                    format!(".map(|v| v.iter().map(|x| {}(x.clone())).collect::<Vec<_>>())", t)
                } else {
                    format!(".map(|v| v.into_iter().map({}).collect::<Vec<_>>())", wrap_named(t))
                }
            }
            None => {
                // Apply coercions for &str → String and Vec<&str> → Vec<String>
                // in Result-returning functions, same as for direct returns. When
                // `f.returns_ref` is true the Ok arm is a slice reference, so emit
                // `.iter()` instead of `.into_iter()` to keep clippy 1.95's
                // `into_iter_on_ref` lint quiet.
                let iter_method = if f.returns_ref { "iter" } else { "into_iter" };
                match &f.return_type {
                    TypeRef::String => ".map(|s| s.to_string())".to_string(),
                    TypeRef::Path => ".map(|s| s.display().to_string())".to_string(),
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                        format!(".map(|v| v.{iter_method}().map(|s| s.to_string()).collect::<Vec<_>>())")
                    }
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
                        format!(".map(|v| v.{iter_method}().map(|s| s.display().to_string()).collect::<Vec<_>>())")
                    }
                    _ => String::new(),
                }
            }
        }
    };
    let value_map = value_map_string.as_str();
    // Direct (non-Result) wrap for the value.
    let direct_wrap = |source: String| -> String {
        if json_wrap_ok {
            return format!("serde_json::to_string(&({source})).expect(\"serializable return\")");
        }
        match &wrap_shape {
            Some(WrapShape::Direct(t)) => wrap_named_direct(t, &source),
            Some(WrapShape::OptMap(t)) => format!("({source}).map({})", wrap_named(t)),
            Some(WrapShape::VecMap(t)) => {
                if f.returns_ref {
                    // source is &[T]; iter() yields &T — must clone before wrapping.
                    format!("({source}).iter().map(|x| {t}(x.clone())).collect::<Vec<_>>()")
                } else {
                    format!("({source}).into_iter().map({}).collect::<Vec<_>>()", wrap_named(t))
                }
            }
            None => {
                // No wrapping needed, but the source function might return `&str`
                // when the IR says `String`, or `Vec<&str>` / `&[&str]` when the IR
                // says `Vec<String>`. Apply coercions to match the declared return
                // type. When `f.returns_ref` is true (the core fn returns
                // `&[String]` / `&'static [&'static str]`), emit `.iter()` rather
                // than `.into_iter()` — clippy 1.95's `into_iter_on_ref` rejects
                // `.into_iter()` on slice references even though it compiles.
                let iter_method = if f.returns_ref { "iter" } else { "into_iter" };
                match &f.return_type {
                    TypeRef::String => format!("{source}.to_string()"),
                    TypeRef::Path => format!("{source}.display().to_string()"),
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                        format!("{source}.{iter_method}().map(|s| s.to_string()).collect::<Vec<_>>()")
                    }
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
                        format!("{source}.{iter_method}().map(|s| s.display().to_string()).collect::<Vec<_>>()")
                    }
                    _ => source,
                }
            }
        }
    };
    let body = if f.is_async {
        let mut chain = format!("{source_call}.await");
        if f.error_type.is_some() {
            chain = format!("{chain}.map_err(|e| e.to_string()){value_map}");
        } else {
            chain = direct_wrap(chain);
        }
        chain
    } else if f.error_type.is_some() {
        format!("{source_call}.map_err(|e| e.to_string()){value_map}")
    } else {
        direct_wrap(source_call)
    };

    // The wrapper is always sync — swift-bridge 0.1.59 doesn't support async
    // functions in extern blocks, so async source calls are blocked-on inside
    // a tokio runtime.
    //
    // The runtime is a process-wide static (initialized lazily on first call)
    // rather than a per-call `Builder::new_current_thread().build()`. The
    // per-call pattern works for self-contained async work but breaks reqwest:
    // `reqwest::Client::builder().build()` lazily attaches its connection pool
    // to the FIRST tokio runtime it sees, and that pool dies when its host
    // runtime is dropped. Subsequent calls then fail with
    // `error sending request for url (...)` because the orphaned pool can't
    // service them. Sharing one persistent runtime keeps the pool alive.
    //
    // Uses `Builder::new_multi_thread()` (not `new_current_thread`) so the
    // runtime can be re-entered from any host thread (Swift, JVM, Python's
    // GIL release, etc.) without blocking on its own worker.

    // Emit pre-call bindings (if any) before the body expression.
    let bindings_str = if !pre_call_bindings.is_empty() {
        pre_call_bindings.join("\n") + "\n    "
    } else {
        String::new()
    };

    let cfg_prefix = f.cfg.as_deref().map(|c| format!("#[cfg({c})]\n")).unwrap_or_default();

    if f.is_async {
        format!(
            "{cfg_prefix}pub fn {fn_name}({params_str}) -> {return_ty} {{\n    \
            {bindings_str}{ALEF_TOKIO_RUNTIME_ACCESSOR}.block_on(async {{ {body} }})\n}}\n"
        )
    } else {
        format!("{cfg_prefix}pub fn {fn_name}({params_str}) -> {return_ty} {{\n    {bindings_str}{body}\n}}\n")
    }
}

/// Snippet that resolves the process-wide tokio runtime. Emitted alongside the
/// shim functions so async wrappers can `.block_on(...)` without rebuilding
/// the runtime per call.
pub(crate) const ALEF_TOKIO_RUNTIME_ACCESSOR: &str = "crate::__alef_tokio_runtime()";

/// Top-of-crate snippet that defines `__alef_tokio_runtime()`, a lazily-
/// initialized process-wide multi-thread runtime. Embedded once per crate.
pub(crate) const ALEF_TOKIO_RUNTIME_DEFINITION: &str = r#"
/// Process-wide tokio runtime shared across every swift-bridge async wrapper.
///
/// alef-emitted; see shims.rs for the rationale (orphaned reqwest connection
/// pools when each call creates and drops its own current-thread runtime).
fn __alef_tokio_runtime() -> &'static ::tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<::tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        ::tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build process-wide alef tokio runtime")
    })
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ParamDef, TypeRef};

    fn param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    fn function(params: Vec<ParamDef>) -> FunctionDef {
        FunctionDef {
            name: "interact".to_string(),
            rust_path: "sample_crawler::interact".to_string(),
            original_rust_path: String::new(),
            params,
            return_type: TypeRef::Named("InteractionResult".to_string()),
            is_async: true,
            error_type: Some("CrawlError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn vec_enum_params_bridge_as_json_strings() {
        let f = function(vec![param(
            "actions",
            TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
        )]);
        let enum_names = HashSet::from(["PageAction"]);
        let type_paths = HashMap::from([("PageAction".to_string(), "sample_crawler::PageAction".to_string())]);
        let no_serde_names = HashSet::new();
        let handle_returned_types = HashSet::new();

        assert!(is_bridgeable_fn(
            &f,
            &enum_names,
            &type_paths,
            &no_serde_names,
            &HashSet::new(),
            &handle_returned_types
        ));

        let shim = emit_function_shim(
            &f,
            "sample_crawler",
            &type_paths,
            &enum_names,
            &no_serde_names,
            &HashSet::new(),
            &handle_returned_types,
        );
        assert!(shim.contains("actions: Vec<String>"));
        // Enum params are converted via From<String>, not JSON deserialization.
        // Swift delivers plain wire strings (e.g. "click"), not JSON-encoded strings
        // (e.g. "\"click\""), so serde_json::from_str would fail on unquoted input.
        assert!(shim.contains("From<String>"));
        assert!(shim.contains("sample_crawler::PageAction"));
        assert!(!shim.contains(".0"));
    }

    #[test]
    fn direct_enum_params_bridge_as_from_string() {
        let f = function(vec![param("action", TypeRef::Named("PageAction".to_string()))]);
        let enum_names = HashSet::from(["PageAction"]);
        let type_paths = HashMap::from([("PageAction".to_string(), "sample_crawler::PageAction".to_string())]);
        let no_serde_names = HashSet::new();
        let handle_returned_types = HashSet::new();

        let shim = emit_function_shim(
            &f,
            "sample_crawler",
            &type_paths,
            &enum_names,
            &no_serde_names,
            &HashSet::new(),
            &handle_returned_types,
        );
        assert!(shim.contains("action: String"));
        // Enum params must use From<String>, not JSON deserialization.
        assert!(shim.contains("From<String>"));
        assert!(shim.contains("sample_crawler::PageAction"));
        assert!(!shim.contains("unimplemented!"));
    }

    #[test]
    fn vec_string_with_ref_inner_converts_to_slice_of_strs() {
        // Core function signature: download(names: &[&str])
        // Swift binding receives: Vec<String>
        // Shim must convert: &names.iter().map(|s| s.as_str()).collect::<Vec<_>>()
        let mut param = param("names", TypeRef::Vec(Box::new(TypeRef::String)));
        param.is_ref = true;
        param.vec_inner_is_ref = true;

        let f = function(vec![param]);
        let type_paths = HashMap::new();
        let unit_enum_names = HashSet::new();
        let tagged_enum_names = HashSet::new();
        let no_serde_names = HashSet::new();
        let handle_returned_types = HashSet::new();

        assert!(is_bridgeable_fn(
            &f,
            &unit_enum_names,
            &type_paths,
            &no_serde_names,
            &tagged_enum_names,
            &handle_returned_types
        ));

        let shim = emit_function_shim(
            &f,
            "sample_crawler",
            &type_paths,
            &unit_enum_names,
            &no_serde_names,
            &tagged_enum_names,
            &handle_returned_types,
        );
        // The shim should take Vec<String> from Swift.
        assert!(shim.contains("names: Vec<String>"));
        // The shim must convert to &[&str] via iter().map(|s| s.as_str()).
        assert!(shim.contains("&names.iter().map(|s| s.as_str()).collect::<Vec<_>>()"));
        // The function call must use the converted slice.
        assert!(shim.contains("sample_crawler::interact"));
    }
}
