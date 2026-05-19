//! Emits function shims that call through to the underlying Rust crate.
//!
//! Each public `FunctionDef` gets a `pub fn` wrapper that:
//!   - accepts bridge types (String for JSON-bridged params, newtypes for Named)
//!   - converts parameters to native Rust types
//!   - calls the source function
//!   - converts the return value back to a bridge type
//!   - for async fns, blocks on a current-thread Tokio runtime

use crate::gen_rust_crate::type_bridge::{
    bridge_type_enum_aware_ref, bridge_type_with_handles, needs_json_bridge, needs_json_bridge_with_handles,
    swift_bridge_rust_type,
};
use alef_core::ir::{FunctionDef, TypeRef};
use alef_core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

/// Returns true when a function can be fully bridged without emitting `unimplemented!()`.
///
/// A function is unbridgeable when any parameter is an enum bridge wrapper (no reverse From),
/// any tuple-vec parameter has an unbridgeable inner type (e.g. `Vec<u8>,`), or when the
/// return type requires JSON bridging but the inner Named type lacks serde.
pub(crate) fn is_bridgeable_fn(
    f: &FunctionDef,
    enum_names: &std::collections::HashSet<&str>,
    type_paths: &HashMap<String, String>,
    no_serde_names: &std::collections::HashSet<&str>,
    no_serde_enum_names: &std::collections::HashSet<&str>,
    handle_returned_types: &HashSet<String>,
) -> bool {
    for p in &f.params {
        match &p.ty {
            TypeRef::Named(n)
                if enum_names.contains(n.as_str()) && (p.is_ref || no_serde_enum_names.contains(n.as_str())) =>
            {
                return false;
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if enum_names.contains(n.as_str()) && (p.is_ref || no_serde_enum_names.contains(n.as_str())) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    // Tuple-vec with Vec<u8> inner: batch_extract_bytes pattern.
    for p in &f.params {
        let original = p.original_type.as_deref().unwrap_or("");
        let stripped = original
            .trim()
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .trim();
        if !stripped.is_empty() && stripped.starts_with("Vec(") && stripped.contains("Named(\"(") {
            let tuple_inner = stripped
                .find("Named(\"(")
                .and_then(|start| {
                    let rest = &stripped[start + 8..];
                    rest.find(")\")")
                        .map(|end| rest[..end].trim_end_matches(')').to_string())
                })
                .unwrap_or_default();
            if tuple_inner.starts_with("Vec<u8>,") || tuple_inner.starts_with("Vec<u8> ,") {
                return false;
            }
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
    p: &alef_core::ir::ParamDef,
    enum_names: &HashSet<&str>,
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
                "{{ let _ = {name}; ::std::unimplemented!(\"batch_extract_bytes from Swift not yet bridged\") }}"
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
        if enum_names.contains(n.as_str()) {
            let native_ty = source_type(n);
            let deser = format!("::serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
            if p.optional {
                return format!(
                    "{name}.map(|json| ::serde_json::from_str::<{native_ty}>(&json).expect(\"valid JSON for {name}\"))"
                );
            }
            return deser;
        }
    }

    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Named(n) = inner.as_ref() {
            if enum_names.contains(n.as_str()) {
                let native_ty = source_type(n);
                let map_expr = format!(
                    "values.into_iter().map(|json| ::serde_json::from_str::<{native_ty}>(&json).expect(\"valid JSON for {name}\")).collect::<Vec<_>>()"
                );
                if p.optional {
                    return format!("{name}.map(|values| {map_expr})");
                }
                return format!("{{ let values = {name}; {map_expr} }}");
            }
        }
    }

    // JSON-bridged: deserialize from the bridged String.
    if needs_json_bridge(&p.ty) {
        let native_ty = swift_bridge_rust_type(&p.ty);
        let deser = format!("::serde_json::from_str::<{native_ty}>(&{name}).expect(\"valid JSON for {name}\")");
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
    // it to the underlying source enum (no From<BridgeEnum> for SourceEnum).
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
                return format!("{name}.map(|v| v.into_iter().map(|w| w.0).collect::<Vec<_>>())");
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
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    handle_returned_types: &HashSet<String>,
) -> String {
    // Match the extern block's escaping so the wrapper fn matches the extern decl.
    let fn_name = swift_ident(&f.name.to_snake_case());
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let bridge_ty = bridge_type_enum_aware_ref(&p.ty, enum_names);
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

    // Build call args, deserializing JSON-bridged params before passing to the real fn
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| swift_call_arg(p, enum_names, type_paths))
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
    if needs_json_bridge_with_handles(&f.return_type, handle_returned_types) {
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
    // types must be wrapped in their swift-bridge newtype (`pub struct T(pub SourceT)`).
    // Async fns must `.await` before mapping; sync fns can chain directly.
    let json_wrap_ok = needs_json_bridge_with_handles(&f.return_type, handle_returned_types);

    // Build a wrapper expression for a Named type `t`.
    // Enum wrappers implement From<SourceT> — use T::from(val) or val.map(T::from).
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
    if f.is_async {
        format!(
            "pub fn {fn_name}({params_str}) -> {return_ty} {{\n    \
            {ALEF_TOKIO_RUNTIME_ACCESSOR}.block_on(async {{ {body} }})\n}}\n"
        )
    } else {
        format!("pub fn {fn_name}({params_str}) -> {return_ty} {{\n    {body}\n}}\n")
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
    use alef_core::ir::{ParamDef, TypeRef};

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
        }
    }

    fn function(params: Vec<ParamDef>) -> FunctionDef {
        FunctionDef {
            name: "interact".to_string(),
            rust_path: "kreuzcrawl::interact".to_string(),
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
        }
    }

    #[test]
    fn vec_enum_params_bridge_as_json_strings() {
        let f = function(vec![param(
            "actions",
            TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
        )]);
        let enum_names = HashSet::from(["PageAction"]);
        let type_paths = HashMap::from([("PageAction".to_string(), "kreuzcrawl::PageAction".to_string())]);
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
            "kreuzcrawl",
            &type_paths,
            &enum_names,
            &no_serde_names,
            &handle_returned_types,
        );
        assert!(shim.contains("actions: Vec<String>"));
        assert!(shim.contains("::serde_json::from_str::<kreuzcrawl::PageAction>"));
        assert!(!shim.contains(".0"));
    }

    #[test]
    fn direct_enum_params_bridge_as_json_strings() {
        let f = function(vec![param("action", TypeRef::Named("PageAction".to_string()))]);
        let enum_names = HashSet::from(["PageAction"]);
        let type_paths = HashMap::from([("PageAction".to_string(), "kreuzcrawl::PageAction".to_string())]);
        let no_serde_names = HashSet::new();
        let handle_returned_types = HashSet::new();

        let shim = emit_function_shim(
            &f,
            "kreuzcrawl",
            &type_paths,
            &enum_names,
            &no_serde_names,
            &handle_returned_types,
        );
        assert!(shim.contains("action: String"));
        assert!(shim.contains("::serde_json::from_str::<kreuzcrawl::PageAction>"));
        assert!(!shim.contains("unimplemented!"));
    }
}
