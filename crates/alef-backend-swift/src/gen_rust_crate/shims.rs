//! Emits function shims that call through to the underlying Rust crate.
//!
//! Each public `FunctionDef` gets a `pub fn` wrapper that:
//!   - accepts bridge types (String for JSON-bridged params, newtypes for Named)
//!   - converts parameters to native Rust types
//!   - calls the source function
//!   - converts the return value back to a bridge type
//!   - for async fns, blocks on a current-thread Tokio runtime

use crate::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge, swift_bridge_rust_type};
use alef_core::ir::{FunctionDef, TypeRef};
use alef_core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Build the call-site expression for a function parameter.
///
/// Handles JSON-bridged types, Path conversion, primitive casts, and reference borrows
/// based on `is_ref`/`optional`. Named types are wrapped as `pub struct T(pub kreuzberg::T)`,
/// so accessing the inner kreuzberg type requires `.0` indirection.
pub(crate) fn swift_call_arg(p: &alef_core::ir::ParamDef) -> String {
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
                rest.find(")\")").map(|end| rest[..end].trim_end_matches(')').to_string())
            })
            .unwrap_or_default();
        if tuple_inner.starts_with("PathBuf,") || tuple_inner.starts_with("PathBuf ,") {
            return format!(
                "{name}.into_iter().map(|p| (std::path::PathBuf::from(p), None)).collect::<Vec<_>>()"
            );
        }
        if tuple_inner.starts_with("Vec<u8>,") || tuple_inner.starts_with("Vec<u8> ,") {
            return format!("{{ let _ = {name}; ::std::unimplemented!(\"batch_extract_bytes from Swift not yet bridged\") }}");
        }
    }

    // JSON-bridged: deserialize from the bridged String.
    if needs_json_bridge(&p.ty) {
        let native_ty = swift_bridge_rust_type(&p.ty);
        let deser = format!("serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
        if p.is_ref {
            // When the kreuzberg function expects a reference (e.g. &[Vec<String>]),
            // we must borrow the deserialized value.
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
    // Enum wrappers do not have a `.0` field — they are plain enums. When a
    // function parameter is a Named enum wrapper, we cannot reverse-convert
    // it to the underlying kreuzberg enum (no From<BridgeEnum> for kreuzberg::Enum).
    // The whole shim will be guarded at the emit_function_shim level; here we
    // still emit a `.0` access so the function body is structurally valid.
    if matches!(p.ty, TypeRef::Named(_)) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(|w| &w.0)");
            }
            return format!("{name}.map(|w| w.0)");
        }
        if p.is_ref {
            return format!("&{name}.0");
        }
        return format!("{name}.0");
    }

    // Vec<Named>: elements are bridge wrapper newtypes; unwrap each with `.0`.
    // Vec<Named> is NOT json-bridged (Named is a bridge leaf), so it falls here.
    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Named(_) = inner.as_ref() {
            if p.optional {
                if p.is_ref {
                    // kreuzberg expects Option<&[T]>. We collect to a temporary Vec
                    // and call as_deref(). The temporary lives for the enclosing
                    // statement (function call) so the reference is valid.
                    return format!(
                        "{name}.as_ref().map(|v| v.iter().map(|w| w.0.clone()).collect::<Vec<_>>()).as_deref()"
                    );
                }
                return format!(
                    "{name}.map(|v| v.into_iter().map(|w| w.0).collect::<Vec<_>>())"
                );
            }
            if p.is_ref {
                // kreuzberg expects &[T]. Collect to a temporary Vec and slice it.
                return format!(
                    "{{ let __tmp = {name}.iter().map(|w| w.0.clone()).collect::<Vec<_>>(); __tmp.as_slice() }}"
                );
            }
            return format!("{name}.into_iter().map(|w| w.0).collect::<Vec<_>>()");
        }
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
        (TypeRef::String | TypeRef::Char, false) => format!("&{name}"),
        (TypeRef::String | TypeRef::Char, true) => format!("{name}.as_deref()"),
        (TypeRef::Vec(_), true) => format!("{name}.as_deref()"),
        _ => format!("&{name}"),
    }
}

pub(crate) fn emit_function_shim(
    f: &FunctionDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
) -> String {
    // Match the extern block's escaping so the wrapper fn matches the extern decl.
    let fn_name = swift_ident(&f.name.to_snake_case());
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let bridge_ty = bridge_type(&p.ty);
            let bridge_ty = if p.optional {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            format!("{name}: {bridge_ty}")
        })
        .collect();
    let params_str = params.join(", ");

    let return_ty = if f.error_type.is_some() {
        let ok_ty = bridge_type(&f.return_type);
        if matches!(f.return_type, TypeRef::Unit) {
            "Result<(), String>".to_string()
        } else {
            format!("Result<{ok_ty}, String>")
        }
    } else {
        bridge_type(&f.return_type)
    };

    // Build call args, deserializing JSON-bridged params before passing to the real fn
    let call_args: Vec<String> = f.params.iter().map(|p| swift_call_arg(p)).collect();
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

    // If any parameter is a Named enum wrapper, we cannot pass it into kreuzberg
    // because the bridge enum only generates From<kreuzberg::T> for BridgeT (not the
    // reverse). Emit an unimplemented!() body — the function will panic at runtime.
    if f.params.iter().any(|p| matches!(&p.ty, TypeRef::Named(n) if enum_names.contains(n.as_str()))) {
        let problematic = f
            .params
            .iter()
            .filter(|p| matches!(&p.ty, TypeRef::Named(n) if enum_names.contains(n.as_str())))
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return format!(
            "// alef: skipped — parameter(s) `{problematic}` are enum bridge wrappers; reverse From not generated\n\
             pub fn {fn_name}({params_str}) -> {return_ty} {{\n    \
             ::std::unimplemented!(\"{fn_name}: enum parameter(s) [{problematic}] cannot be converted back to kreuzberg types\")\n\
             }}\n"
        );
    }

    // Bug B: If the return type is a JSON-bridged Optional/Vec/Named whose inner
    // Named type is NOT in the visible type table (i.e., excluded from codegen —
    // such as EmbeddingPreset or InternalDocument which don't impl serde), we
    // cannot generate a working serde-based bridge. Emit an unimplemented!() body
    // so the crate at least compiles rather than failing with "trait not satisfied".
    // The function will panic at runtime, but that is preferable to compile errors.
    fn inner_named_type(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named_type(inner),
            _ => None,
        }
    }
    if needs_json_bridge(&f.return_type) {
        if let Some(inner_name) = inner_named_type(&f.return_type) {
            if !type_paths.contains_key(inner_name) || no_serde_names.contains(inner_name) {
                // The inner Named type is not in the visible type table or lacks serde.
                // We cannot serde-bridge it — emit an unimplemented shim.
                let fn_name_snake = swift_ident(&f.name.to_snake_case());
                return format!(
                    "// alef: skipped — return type `{inner_name}` is excluded from codegen (no serde derive)\n\
                     pub fn {fn_name_snake}({params_str}) -> {return_ty} {{\n    \
                     ::std::unimplemented!(\"{fn_name_snake}: return type {inner_name} is not bridgeable\")\n\
                     }}\n"
                );
            }
        }
    }

    // Wrap return value with JSON serialization when the return type is not natively
    // supported by swift-bridge 0.1.59 (nested generics, HashMap). Named return
    // types must be wrapped in their swift-bridge newtype (`pub struct T(pub kreuzberg::T)`).
    // Async fns must `.await` before mapping; sync fns can chain directly.
    let json_wrap_ok = needs_json_bridge(&f.return_type);

    // Build a wrapper expression for a Named type `t`.
    // Enum wrappers implement From<kreuzberg::T> — use T::from(val) or val.map(T::from).
    // Struct newtypes use T(val) constructor directly.
    let wrap_named = |t: &str| -> String {
        if enum_names.contains(t) {
            format!("{t}::from")
        } else {
            t.to_string()
        }
    };
    let wrap_named_direct = |t: &str, source: &str| -> String {
        if enum_names.contains(t) {
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
        Direct(String),  // T(value) or T::from(value)
        OptMap(String),  // value.map(T) or value.map(T::from)
        VecMap(String),  // value.into_iter().map(T).collect()
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
            Some(WrapShape::VecMap(t)) => format!(".map(|v| v.into_iter().map({}).collect::<Vec<_>>())", wrap_named(t)),
            None => {
                // Apply coercions for &str → String and Vec<&str> → Vec<String>
                // in Result-returning functions, same as for direct returns.
                match &f.return_type {
                    TypeRef::String | TypeRef::Path => ".map(|s| s.to_string())".to_string(),
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path) => {
                        ".map(|v| v.into_iter().map(|s| s.to_string()).collect::<Vec<_>>())".to_string()
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
            Some(WrapShape::VecMap(t)) => format!("({source}).into_iter().map({}).collect::<Vec<_>>()", wrap_named(t)),
            None => {
                // No wrapping needed — but the kreuzberg function might return `&str`
                // when the IR says `String`, or `Vec<&str>` when the IR says `Vec<String>`.
                // Apply coercions to match the declared return type.
                match &f.return_type {
                    TypeRef::String | TypeRef::Path => format!("{source}.to_string()"),
                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path) => {
                        format!("{source}.into_iter().map(|s| s.to_string()).collect::<Vec<_>>()")
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
    // a tokio runtime. The runtime is created lazily on first call.
    if f.is_async {
        format!(
            "pub fn {fn_name}({params_str}) -> {return_ty} {{\n    \
            ::tokio::runtime::Builder::new_current_thread()\n        \
                .enable_all()\n        \
                .build()\n        \
                .expect(\"build tokio runtime\")\n        \
                .block_on(async {{ {body} }})\n}}\n"
        )
    } else {
        format!("pub fn {fn_name}({params_str}) -> {return_ty} {{\n    {body}\n}}\n")
    }
}
