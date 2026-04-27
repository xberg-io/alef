//! Type mapping helpers for the swift-bridge Rust side.
//!
//! Provides `bridge_type`, `swift_bridge_rust_type`, `needs_json_bridge`,
//! `is_bridge_leaf`, and `rust_primitive_str` — pure functions with no
//! side effects used across all other submodules.

use alef_core::ir::{PrimitiveType, TypeRef};

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
        TypeRef::Optional(inner) => format!("Option<{}>", bridge_type(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", bridge_type(inner)),
        _ => swift_bridge_rust_type(ty),
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
