//! Type mapping helpers for the swift-bridge Rust side.
//!
//! Provides `bridge_type`, `swift_bridge_rust_type`, `needs_json_bridge`,
//! `is_bridge_leaf`, and `rust_primitive_str` — pure functions with no
//! side effects used across all other submodules.

use alef_core::ir::{ApiSurface, PrimitiveType, TypeRef};
use std::collections::HashSet;

/// Compute the set of api type names that are directly returned as opaque handles.
///
/// Mirrors `compute_handle_returned_types` in the C# backend.  Any api type that
/// appears as a `Named(T)` return on a public function/method — possibly wrapped
/// in `Optional` or `Vec` — is exposed as `*mut T` in the FFI, so the Swift side
/// must route it through the opaque-class wrapper instead of a JSON String.
///
/// Without this set, `Option<Named(T)>` returns hit `needs_json_bridge(Optional) ==
/// true` and get collapsed to `String` (RustString in Swift), losing the handle.
pub(crate) fn compute_handle_returned_types(api: &ApiSurface) -> HashSet<String> {
    fn inner_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }

    let mut handle_types = HashSet::new();
    for func in &api.functions {
        if let Some(name) = inner_named(&func.return_type) {
            handle_types.insert(name.to_string());
        }
    }
    for typ in &api.types {
        for method in &typ.methods {
            if let Some(name) = inner_named(&method.return_type) {
                handle_types.insert(name.to_string());
            }
        }
    }
    handle_types
}

/// Like `needs_json_bridge` but lets `Optional<Named(T)>` / `Vec<Named(T)>` through
/// when T is a known opaque-handle-returned type.  swift-bridge supports these forms
/// natively when T is declared as `type T;` in the extern block.
pub(crate) fn needs_json_bridge_with_handles(ty: &TypeRef, handle_types: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if handle_types.contains(n) => false,
            _ => needs_json_bridge(ty),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if handle_types.contains(n) => false,
            _ => needs_json_bridge(ty),
        },
        _ => needs_json_bridge(ty),
    }
}

/// `bridge_type` variant that respects the handle-returned type set.
pub(crate) fn bridge_type_with_handles(ty: &TypeRef, handle_types: &HashSet<String>) -> String {
    if needs_json_bridge_with_handles(ty, handle_types) {
        return "String".to_string();
    }
    match ty {
        TypeRef::Optional(inner) => format!("Option<{}>", bridge_type_with_handles(inner, handle_types)),
        TypeRef::Vec(inner) => format!("Vec<{}>", bridge_type_with_handles(inner, handle_types)),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Returns true for types that swift-bridge 0.1.59 cannot handle inside `extern "Rust"` blocks.
///
/// Two distinct bugs in swift-bridge-ir-0.1.59 are covered:
///
/// 1. **Parser bug** (`bridged_type.rs:826`): `BridgedType::new_with_str` uses
///    `trim_end_matches(" >")` which strips ALL trailing ` >` occurrences. This corrupts
///    `Vec < Option < String > >` into `Vec < Option < String` causing `syn::parse2` to
///    fail with `Error("expected ','")`.
///
/// 2. **Codegen todo** (`bridged_type.rs:1986`): `BuiltInResult::is_custom_result_type()`
///    returns `true` when the ok type is a `StdLib` non-Vec type (e.g. `Option<T>`,
///    primitives). When true, the codegen calls `to_alpha_numeric_underscore_name` on the
///    ok type, but `StdLib::Option` and `StdLib::Vec` hit `_ => todo!()` there.
///
/// HashMap is completely unsupported by swift-bridge.
///
/// All affected types are serialized to JSON (`String`) at the bridge boundary.
pub(crate) fn needs_json_bridge(ty: &TypeRef) -> bool {
    match ty {
        // HashMap is unsupported by swift-bridge regardless of nesting.
        TypeRef::Map(_, _) => true,
        // Vec<T> is only safe when T is a "leaf" type (no angle brackets in its token form).
        // Primitives, String, char, Named opaques, and Vec<u8> (Bytes) are all safe.
        // Anything else (Vec<Option<..>>, Vec<Vec<..>>, Vec<Map<..>>) triggers the parser bug.
        TypeRef::Vec(inner) => !is_bridge_leaf(inner),
        // Option<T> as a Result ok-type causes is_custom_result_type()=true + todo!() in
        // to_alpha_numeric_underscore_name. JSON-bridge all Optional types to avoid this.
        TypeRef::Optional(_) => true,
        _ => false,
    }
}

/// Returns true when `ty` produces a token-stream representation with no angle brackets,
/// i.e. it is safe as the inner type of a `Vec<T>` in a swift-bridge extern block.
pub(crate) fn is_bridge_leaf(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Unit
        | TypeRef::Duration => true,
        // Bytes = Vec<u8> — swift-bridge supports this directly.
        TypeRef::Bytes => true,
        // Named opaque types (wrapper newtypes) have no angle brackets.
        TypeRef::Named(_) => true,
        // Everything else (Vec, Optional, Map) produces angle brackets in the token stream.
        _ => false,
    }
}

/// Maps an IR `TypeRef` to the Rust type used in swift-bridge `extern "Rust"` block declarations.
///
/// Types that swift-bridge 0.1.59 cannot parse (nested generics, HashMap) are collapsed to
/// `String` (JSON-serialized at the shim boundary). All other types use their native Rust form.
pub(crate) fn bridge_type(ty: &TypeRef) -> String {
    if needs_json_bridge(ty) {
        return "String".to_string();
    }
    match ty {
        // swift-bridge 0.1.59 has no built-in `char` primitive; map to String at the
        // bridge boundary and convert at the shim layer (.chars().next() / .to_string()).
        TypeRef::Char => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", bridge_type(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", bridge_type(inner)),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Like `bridge_type` but treats `Named(enum_name)` types as `String`.
///
/// swift-bridge 0.1.59 generates `Vectorizable` conformance (with `__swift_bridge__$Vec_T$*`
/// C-ABI symbols) for every opaque `extern "Rust" { type T; }` declaration, including enums.
/// The generated Swift references these symbols even when no `Vec<T>` field exists, and the
/// Rust macro does NOT generate the corresponding C symbols for enums.  To avoid the linker
/// error, enum-typed fields are serialized to `String` (via the wrapper's `to_string()`) at
/// the bridge boundary instead of being returned as opaque handles.
///
/// Accepts both `HashSet<&str>` (from within the generator) and `HashSet<String>` (via the
/// owned variant below) depending on call site.
pub(crate) fn bridge_type_enum_aware(ty: &TypeRef, enum_names: &HashSet<String>) -> String {
    match ty {
        TypeRef::Named(n) if enum_names.contains(n) => "String".to_string(),
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                if enum_names.contains(n) {
                    return "Vec<String>".to_string();
                }
            }
            bridge_type(ty)
        }
        _ => bridge_type(ty),
    }
}

/// Like `bridge_type_enum_aware` but accepts `HashSet<&str>` (cheaper at call sites that
/// already hold the borrowed set).
pub(crate) fn bridge_type_enum_aware_ref(ty: &TypeRef, enum_names: &HashSet<&str>) -> String {
    match ty {
        TypeRef::Named(n) if enum_names.contains(n.as_str()) => "String".to_string(),
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                if enum_names.contains(n.as_str()) {
                    return "Vec<String>".to_string();
                }
            }
            bridge_type(ty)
        }
        _ => bridge_type(ty),
    }
}

/// Returns `true` when the field type is an enum-typed `Named` reference.
///
/// Used by getter emitters to decide whether to serialize via `to_string()` rather than
/// returning the opaque enum wrapper handle.
pub(crate) fn is_enum_named(ty: &TypeRef, enum_names: &HashSet<&str>) -> bool {
    match ty {
        TypeRef::Named(n) => enum_names.contains(n.as_str()),
        _ => false,
    }
}

/// Returns `true` when a `Vec<Named(T)>` field has T as an enum.
pub(crate) fn is_vec_of_enum(ty: &TypeRef, enum_names: &HashSet<&str>) -> bool {
    match ty {
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str()))
        }
        _ => false,
    }
}

/// Maps an IR `TypeRef` to the native Rust type string (used in wrapper impls and shim bodies).
///
/// This is the full native type including nested generics and HashMap — used on the Rust side
/// of the shim where swift-bridge is not involved.
pub(crate) fn swift_bridge_rust_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => rust_primitive_str(p).to_string(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", swift_bridge_rust_type(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", swift_bridge_rust_type(inner)),
        TypeRef::Map(k, v) => {
            format!(
                "std::collections::HashMap<{}, {}>",
                swift_bridge_rust_type(k),
                swift_bridge_rust_type(v)
            )
        }
        TypeRef::Named(name) => name.clone(),
    }
}

pub(crate) fn rust_primitive_str(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "bool",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I8 => "i8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U64 => "u64",
        PrimitiveType::I64 => "i64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
    }
}
