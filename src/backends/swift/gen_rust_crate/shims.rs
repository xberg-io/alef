//! Emits function shims that call through to the underlying Rust crate.
//!
//! Each public `FunctionDef` gets a `pub fn` wrapper that:
//!   - accepts bridge types (String for JSON-bridged params, newtypes for Named)
//!   - converts parameters to native Rust types
//!   - calls the source function
//!   - converts the return value back to a bridge type
//!   - for async fns, blocks on a current-thread Tokio runtime

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_result_ok_type_with_handles, bridge_type_enum_aware_ref, bridge_type_with_handles, enum_from_string_fn_name,
    forces_fallible_enum_bridge, needs_json_bridge, needs_json_bridge_with_handles, swift_bridge_rust_type,
};
use crate::backends::swift::naming::swift_rust_shim_ident as swift_ident;
use crate::core::ir::{FunctionDef, PrimitiveType, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

pub(crate) struct FunctionShimContext<'a> {
    pub(crate) source_crate: &'a str,
    pub(crate) type_paths: &'a HashMap<String, String>,
    pub(crate) unit_enum_names: &'a HashSet<&'a str>,
    pub(crate) tagged_enum_names: &'a HashSet<&'a str>,
    pub(crate) no_serde_names: &'a HashSet<&'a str>,
    pub(crate) handle_returned_types: &'a HashSet<String>,
    pub(crate) capsule_types: &'a HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    pub(crate) opaque_types: &'a ahash::AHashSet<String>,
}

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
        if matches!(&p.ty, TypeRef::Named(n) if n.starts_with("Result") || n == "Result") {
            return false;
        }
        match &p.ty {
            TypeRef::Named(n) if unit_enum_names.contains(n.as_str()) => {
                if p.is_ref || no_serde_enum_names.contains(n.as_str()) {
                    return false;
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(n) = inner.as_ref()
                    && unit_enum_names.contains(n.as_str())
                    && (p.is_ref || no_serde_enum_names.contains(n.as_str()))
                {
                    return false;
                }
            }
            _ => {}
        }
    }
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
    fn inner_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    if needs_json_bridge_with_handles(&f.return_type, handle_returned_types)
        && let Some(inner_name) = inner_named(&f.return_type)
        && (!type_paths.contains_key(inner_name) || no_serde_names.contains(inner_name))
    {
        return false;
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
    pre_call_bindings: &mut Vec<String>,
) -> String {
    let name = p.name.to_snake_case();
    let original = p.original_type.as_deref().unwrap_or("");
    let stripped_orig = original
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim();

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

    if let TypeRef::Named(n) = &p.ty
        && unit_enum_names.contains(n.as_str())
    {
        let fn_name = enum_from_string_fn_name(n);
        // An unrecognised wire string used to `panic!` inside `{fn_name}`, which unwinds across
        // the swift-bridge FFI boundary -- undefined behaviour at best. The generator now emits
        // `Result<_, String>` from `{fn_name}`; bind it here with `?` and force the enclosing
        // shim to return `Result` (see `forces_fallible_enum_bridge`) instead of letting the
        // panic reach the boundary. ~keep
        let bound = format!("__{name}_enum");
        if p.optional {
            pre_call_bindings.push(format!("    let {bound} = {name}.map(|s| {fn_name}(&s)).transpose()?;"));
        } else {
            pre_call_bindings.push(format!("    let {bound} = {fn_name}(&{name})?;"));
        }
        return bound;
    }

    if let TypeRef::Named(n) = &p.ty
        && tagged_enum_names.contains(n.as_str())
    {
        let native_ty = source_type(n);
        let deserialize =
            |value: &str| format!("::serde_json::from_str::<{native_ty}>({value}).expect(\"valid JSON for {name}\")");
        if p.optional {
            let converted = format!("{name}.as_ref().map(|value| {})", deserialize("value"));
            if p.is_ref {
                return if p.is_mut {
                    format!("{converted}.as_mut()")
                } else {
                    format!("{converted}.as_ref()")
                };
            }
            return converted;
        }
        let converted = deserialize(&format!("&{name}"));
        if p.is_ref {
            if p.is_mut {
                return format!("&mut {converted}");
            }
            return format!("&{converted}");
        }
        return converted;
    }

    if let TypeRef::Vec(inner) = &p.ty
        && let TypeRef::Named(n) = inner.as_ref()
    {
        if unit_enum_names.contains(n.as_str()) {
            let fn_name = enum_from_string_fn_name(n);
            let bound = format!("__{name}_vec_enum");
            // Same `?`-propagation rationale as the direct-enum-param branch above: a bad
            // element used to reach `{fn_name}`'s `panic!` from inside `.map(...)`. The
            // `collect::<Result<..>>()` here is deliberately left un-`?`'d in the optional arm --
            // `?` inside a closure needs an explicit `Result` return type to type-check, so the
            // closure instead hands `.transpose()` an `Option<Result<..>>` and the single `?`
            // after it does the unwrapping at this function's own scope. ~keep
            let collect_expr = format!("values.into_iter().map(|s| {fn_name}(&s)).collect::<Result<Vec<_>, String>>()");
            if p.optional {
                pre_call_bindings.push(format!(
                    "    let {bound} = {name}.map(|values| {collect_expr}).transpose()?;"
                ));
            } else {
                pre_call_bindings.push(format!("    let {bound} = {{ let values = {name}; {collect_expr}? }};"));
            }
            if p.is_ref && !p.optional {
                return format!("&{bound}");
            }
            return bound;
        }
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
                return format!("&{{ let values = {name}; {map_expr} }}");
            }
            return converted;
        }
    }

    if let TypeRef::Map(_, _) = &p.ty
        && p.map_is_ahash
        && p.map_key_is_cow
    {
        let bound_name = format!("__{}_ahash", p.name);
        return if p.optional && p.is_ref {
            format!("{bound_name}.as_ref()")
        } else if p.is_ref {
            format!("{bound_name}.as_ref().unwrap()")
        } else {
            bound_name
        };
    }

    if needs_json_bridge(&p.ty) {
        let native_ty = swift_bridge_rust_type(&p.ty);
        let deser = format!("::serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
        if p.is_ref {
            return format!("&{deser}");
        }
        return deser;
    }

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

    if let TypeRef::Named(type_name) = &p.ty {
        if unit_enum_names.contains(type_name.as_str()) {
            return name;
        }
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

    if let TypeRef::Vec(inner) = &p.ty
        && let TypeRef::Named(_) = inner.as_ref()
    {
        if p.optional {
            if p.is_ref {
                return format!(
                    "{name}.as_ref().map(|v| v.iter().map(|w| w.0.clone()).collect::<Vec<_>>()).as_deref()"
                );
            }
            return format!("{name}.map(|v| v.into_iter().map(|w| w.0).collect::<Vec<_>>())");
        }
        if p.is_ref {
            return format!("&{name}.iter().map(|w| w.0.clone()).collect::<Vec<_>>()");
        }
        return format!("{name}.into_iter().map(|w| w.0).collect::<Vec<_>>()");
    }

    if p.is_ref
        && p.vec_inner_is_ref
        && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    {
        return format!("&{name}.iter().map(|s| s.as_str()).collect::<Vec<_>>()");
    }

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
        (TypeRef::Char, false) => format!("{name}.chars().next().unwrap_or('\\0')"),
        (TypeRef::Char, true) => format!("{name}.as_ref().and_then(|s| s.chars().next())"),
        (TypeRef::String, false) => format!("&{name}"),
        (TypeRef::String, true) => format!("{name}.as_deref()"),
        (TypeRef::Vec(_), true) => format!("{name}.as_deref()"),
        _ => format!("&{name}"),
    }
}

pub(crate) fn emit_function_shim(f: &FunctionDef, context: &FunctionShimContext<'_>) -> anyhow::Result<String> {
    let source_crate = context.source_crate;
    let type_paths = context.type_paths;
    let unit_enum_names = context.unit_enum_names;
    let tagged_enum_names = context.tagged_enum_names;
    let no_serde_names = context.no_serde_names;
    let handle_returned_types = context.handle_returned_types;
    let capsule_types = context.capsule_types;
    let opaque_types = context.opaque_types;

    crate::codegen::mut_writeback::reject_unsupported_writeback(&f.name, &f.params, &f.return_type, opaque_types)?;
    // Only a unit-returning function with exactly one `&mut` DTO param qualifies; the
    // rejection above already ruled out every other `&mut` DTO shape. The swift-bridge
    // wrapper newtype's Swift-side identifier is the same `swift_ident(snake_case(name))`
    // used to declare and reference the parameter everywhere else in this function. ~keep
    let writeback_return_expr: Option<String> =
        crate::codegen::mut_writeback::writeback_param(&f.params, &f.return_type, opaque_types)
            .map(|p| swift_ident(&p.name.to_snake_case()));
    let effective_return_type: TypeRef =
        crate::codegen::mut_writeback::effective_return_type(&f.params, &f.return_type, opaque_types)
            .unwrap_or_else(|| f.return_type.clone());

    let fn_name = swift_ident(&f.name.to_snake_case());

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

    let is_capsule_return =
        matches!(&effective_return_type, TypeRef::Named(n) if capsule_types.contains_key(n.as_str()));

    // A unit-enum parameter's binding->core conversion can fail on an unrecognised wire string
    // (see `swift_call_arg`'s `?`-based pre_call_bindings). When the underlying core call is
    // already fallible (`f.error_type.is_some()`) that failure rides the existing `Result`; when
    // it is not, the shim's own return type must become `Result<_, String>` purely to carry this
    // one failure mode, or the `?` in `pre_call_bindings` has nothing to propagate into.
    //
    // An async shim carries a second, independent failure mode: the work runs as a spawned
    // task (see the `f.is_async` branch below), and the task's `JoinHandle` can resolve to a
    // `JoinError` if the task panicked or was cancelled. Unwinding across the FFI boundary is
    // undefined behavior, so that `JoinError` must also become an `Err(String)`, not a `panic!`
    // -- which means every async shim needs a `Result`-shaped return type even when the wrapped
    // core call itself is infallible and has no enum param to force one. ~keep
    let forced_fallible = f.is_async || forces_fallible_enum_bridge(&f.params, f.error_type.as_ref(), unit_enum_names);

    let (return_ty, has_explicit_return) = if is_capsule_return {
        if forced_fallible {
            ("Result<usize, String>".to_string(), true)
        } else {
            ("usize".to_string(), true)
        }
    } else if f.error_type.is_some() {
        let ok_ty = bridge_result_ok_type_with_handles(&effective_return_type, handle_returned_types);
        if matches!(effective_return_type, TypeRef::Unit) {
            ("Result<(), String>".to_string(), true)
        } else {
            (format!("Result<{ok_ty}, String>"), true)
        }
    } else if forced_fallible {
        let ok_ty = bridge_result_ok_type_with_handles(&effective_return_type, handle_returned_types);
        if matches!(effective_return_type, TypeRef::Unit) {
            ("Result<(), String>".to_string(), true)
        } else {
            (format!("Result<{ok_ty}, String>"), true)
        }
    } else if matches!(effective_return_type, TypeRef::Unit) {
        (String::new(), false)
    } else {
        (
            bridge_type_with_handles(&effective_return_type, handle_returned_types),
            true,
        )
    };

    let mut pre_call_bindings: Vec<String> = Vec::new();

    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Map(_, _) = &p.ty
                && p.map_is_ahash && p.map_key_is_cow {
                    let bound_name = format!("__{}_ahash", p.name);
                    let name = p.name.to_snake_case();
                    pre_call_bindings.push(format!(
                        "    let {bound_name} = {name}.map(|json_str| {{ let hm = ::serde_json::from_str::<std::collections::HashMap<String, String>>(&json_str).expect(\"valid JSON for {name}\"); hm.into_iter().map(|(k, v)| (std::borrow::Cow::Owned(k), serde_json::Value::String(v))).collect::<ahash::AHashMap<std::borrow::Cow<'static, str>, serde_json::Value>>() }});"
                    ));
                }
            swift_call_arg(p, unit_enum_names, tagged_enum_names, type_paths, &mut pre_call_bindings)
        })
        .collect();
    let call_args_str = call_args.join(", ");

    let resolved_path = if f.rust_path.is_empty() {
        format!("{source_crate}::{fn_name}")
    } else {
        f.rust_path.replace('-', "_")
    };
    let source_call = format!("{resolved_path}({call_args_str})");

    fn inner_named_type(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named_type(inner),
            _ => None,
        }
    }
    if needs_json_bridge_with_handles(&f.return_type, handle_returned_types)
        && let Some(inner_name) = inner_named_type(&f.return_type)
        && (!type_paths.contains_key(inner_name) || no_serde_names.contains(inner_name))
    {
        let fn_name_snake = swift_ident(&f.name.to_snake_case());
        return Ok(format!(
            "// alef: skipped — return type `{inner_name}` is excluded from codegen (no serde derive)\n\
                     pub fn {fn_name_snake}({params_str}) -> {return_ty} {{\n    \
                     compile_error!(\"alef cannot bridge Swift return type {inner_name}; configure swift.exclude_functions for {fn_name_snake} or expose serde for the type\")\n\
                     }}\n"
        ));
    }

    // `result_ok_needs_json_bridge_with_handles` widens the plain check with the u64/i64 Result
    // gap (see its doc comment), but only when this shim's return really is a `Result<_, String>`
    // (`f.error_type.is_some()` or `forced_fallible` -- see the matching `ok_ty` computation
    // above): a bare, non-`Result` `u64`/`i64` return never reaches swift-bridge-ir's panicking
    // path and must keep its native type, not be forced through JSON. ~keep
    let is_result_return = f.error_type.is_some() || forced_fallible;
    let json_wrap_ok = needs_json_bridge_with_handles(&f.return_type, handle_returned_types)
        || (is_result_return
            && matches!(
                &effective_return_type,
                TypeRef::Primitive(PrimitiveType::U64) | TypeRef::Primitive(PrimitiveType::I64)
            ));

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

    enum WrapShape {
        Direct(String),
        OptMap(String),
        VecMap(String),
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
                    format!(".map(|v| v.iter().map(|x| {}(x.clone())).collect::<Vec<_>>())", t)
                } else {
                    format!(".map(|v| v.into_iter().map({}).collect::<Vec<_>>())", wrap_named(t))
                }
            }
            None => {
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
    let direct_wrap = |source: String| -> String {
        if json_wrap_ok {
            return format!("serde_json::to_string(&({source})).expect(\"serializable return\")");
        }
        match &wrap_shape {
            Some(WrapShape::Direct(t)) => wrap_named_direct(t, &source),
            Some(WrapShape::OptMap(t)) => format!("({source}).map({})", wrap_named(t)),
            Some(WrapShape::VecMap(t)) => {
                if f.returns_ref {
                    format!("({source}).iter().map(|x| {t}(x.clone())).collect::<Vec<_>>()")
                } else {
                    format!("({source}).into_iter().map({}).collect::<Vec<_>>()", wrap_named(t))
                }
            }
            None => {
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
    let body = if let Some(return_expr) = &writeback_return_expr {
        crate::backends::swift::template_env::render(
            "rust_writeback_body.rs.jinja",
            minijinja::context! {
                call => &source_call,
                is_async => f.is_async,
                has_error => f.error_type.is_some(),
                // The writeback path's own success value never needed `Ok(...)` before -- it is
                // needed only when something else forces the *shim's* return type to `Result`
                // while the core call itself stays infallible: a fallible enum param, or (see
                // `forced_fallible` above) every async shim now that a spawned task's `JoinError`
                // must surface as `Err(..)` rather than unwind across the FFI boundary. ~keep
                force_ok => forced_fallible,
                return_expr => return_expr,
            },
        )
        .trim_end()
        .to_string()
    } else if is_capsule_return {
        if f.is_async {
            let expr = format!("{source_call}.await.map(|__cap| __cap.into_raw() as usize).unwrap_or(0)");
            if forced_fallible { format!("Ok({expr})") } else { expr }
        } else if f.error_type.is_some() {
            format!("{source_call}.map(|__cap| __cap.into_raw() as usize).unwrap_or(0)")
        } else if forced_fallible {
            format!("Ok({source_call}.into_raw() as usize)")
        } else {
            format!("{source_call}.into_raw() as usize")
        }
    } else if f.is_async {
        let mut chain = format!("{source_call}.await");
        if f.error_type.is_some() {
            chain = format!("{chain}.map_err(|e| e.to_string()){value_map}");
        } else if forced_fallible {
            chain = format!("Ok({})", direct_wrap(chain));
        } else {
            chain = direct_wrap(chain);
        }
        chain
    } else if f.error_type.is_some() {
        format!("{source_call}.map_err(|e| e.to_string()){value_map}")
    } else if forced_fallible {
        if matches!(f.return_type, TypeRef::Unit) {
            format!("{source_call};\n    Ok(())")
        } else {
            format!("Ok({})", direct_wrap(source_call))
        }
    } else {
        if matches!(f.return_type, TypeRef::Unit) {
            format!("{source_call};")
        } else {
            direct_wrap(source_call)
        }
    };

    let bindings_str = if !pre_call_bindings.is_empty() {
        pre_call_bindings.join("\n") + "\n    "
    } else {
        String::new()
    };

    let cfg_prefix = f.cfg.as_deref().map(|c| format!("#[cfg({c})]\n")).unwrap_or_default();

    let return_annotation = if has_explicit_return {
        format!(" -> {return_ty}")
    } else {
        String::new()
    };

    if f.is_async {
        // `Runtime::block_on(future)` drives `future` on the CALLING thread -- only tasks
        // handed to `Runtime::spawn` run on one of the runtime's own worker threads (the ones
        // sized by `RUNTIME_STACK_SIZE_BYTES` below). The Swift-side caller reaches this
        // function from a `Task.detached` closure, i.e. a Swift concurrency cooperative-pool
        // thread whose stack we do not control and cannot resize. So the deep async work is
        // spawned onto a worker (large stack) and the calling thread only blocks on the
        // resulting `JoinHandle`, which is a cheap, shallow wait -- not the deep poll chain.
        //
        // A spawned task's `JoinHandle` resolves to `Err(JoinError)` if the task panicked or
        // was cancelled; unwinding that across the FFI boundary is undefined behavior, so it
        // is converted to an ordinary `Err(String)` instead of a `panic!`/`resume_unwind`. This
        // is why `forced_fallible` above is `true` for every async shim: the body this closure
        // wraps always evaluates to `Result<_, String>`, giving the join failure somewhere to
        // land. A task panicking does not poison the shared runtime or affect other in-flight
        // or future calls -- tokio's own task harness polls every spawned task inside
        // `catch_unwind` and reports the panic through that one task's `JoinHandle` only.
        Ok(format!(
            "{cfg_prefix}pub fn {fn_name}({params_str}){return_annotation} {{\n    \
            {bindings_str}let __alef_task = {ALEF_TOKIO_RUNTIME_ACCESSOR}.spawn(async move {{ {body} }});\n    \
            {ALEF_TOKIO_RUNTIME_ACCESSOR}.block_on(__alef_task).unwrap_or_else(|__alef_join_error| {{\n        \
            Err(format!(\"alef: spawned async task failed: {{__alef_join_error}}\"))\n    }})\n}}\n"
        ))
    } else {
        Ok(format!(
            "{cfg_prefix}pub fn {fn_name}({params_str}){return_annotation} {{\n    {bindings_str}{body}\n}}\n"
        ))
    }
}

/// Snippet that resolves the process-wide tokio runtime. Emitted alongside the shim
/// functions so async wrappers can `.spawn(...)` the real work onto a large-stack worker
/// thread and `.block_on(...)` only the resulting `JoinHandle`, without rebuilding the
/// runtime per call.
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
    // 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep extraction
    // future (a nested archive member, a multi-stage OCR pipeline), and a stack overflow
    // aborts the process with SIGBUS instead of raising a catchable panic.
    const RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
    static RT: OnceLock<::tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        ::tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(RUNTIME_STACK_SIZE_BYTES)
            .build()
            .expect("build process-wide alef tokio runtime")
    })
}
"#;

#[cfg(test)]
mod tests;
