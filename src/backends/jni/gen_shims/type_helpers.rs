fn internal_class_component(name: &str) -> String {
    to_class_name(name)
}

/// Return the ` -> <JniReturnType>` suffix for a method shim signature.
fn method_return_type_decl(return_type: &TypeRef) -> String {
    match return_type {
        TypeRef::Unit => String::new(),
        TypeRef::Primitive(PrimitiveType::Bool) => " -> jboolean".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            " -> jbyteArray".to_string()
        }
        TypeRef::Bytes => " -> jbyteArray".to_string(),
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Bytes)
                || matches!(inner.as_ref(), TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8))) =>
        {
            " -> jbyteArray".to_string()
        }
        TypeRef::Primitive(_) => {
            let jni_ty = jni_return_type(return_type);
            format!(" -> {jni_ty}")
        }
        _ => " -> jstring".to_string(),
    }
}

/// Return the "null" / zero value for a method return type (used in error paths).
fn method_return_null(return_type: &TypeRef) -> &'static str {
    match return_type {
        TypeRef::Unit => "()",
        // jni 0.22 + jni-sys 0.4 changed `jboolean` from `u8` to `bool`; the
        // sentinel value for an error-path return therefore needs to be `false`,
        // not the legacy `0u8`.
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::Primitive(PrimitiveType::F32) => "0.0f32",
        TypeRef::Primitive(PrimitiveType::F64) => "0.0f64",
        TypeRef::Primitive(_) => "0",
        TypeRef::Bytes => "std::ptr::null_mut()",
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            "std::ptr::null_mut()"
        }
        _ => "std::ptr::null_mut()",
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a TypeRef to a JNI return type string.
fn jni_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "()",
        TypeRef::Primitive(p) => jni_primitive_type(p),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => "jbyteArray",
        TypeRef::Bytes => "jbyteArray",
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Bytes)
                || matches!(inner.as_ref(), TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8))) =>
        {
            "jbyteArray"
        }
        // String and complex types cross the boundary as Java objects.
        TypeRef::String | TypeRef::Named(_) | TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => "jstring",
        // Opaque handles → Long.
        _ => "jlong",
    }
}

fn jni_primitive_type(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "jboolean",
        PrimitiveType::I8 | PrimitiveType::U8 => "jni::sys::jbyte",
        PrimitiveType::I16 | PrimitiveType::U16 => "jni::sys::jshort",
        PrimitiveType::I32 | PrimitiveType::U32 => "jni::sys::jint",
        PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "jlong",
        PrimitiveType::F32 => "jni::sys::jfloat",
        PrimitiveType::F64 => "jni::sys::jdouble",
    }
}

/// Return the Rust zero-literal for a JNI primitive, used as the null-sentinel
/// for optional primitive parameters.  Returns None for `Bool`, which has no
/// meaningful "absent" sentinel (false is a real value); optional bools cannot
/// be marshalled through plain JNI primitives.
fn primitive_zero_literal(p: &PrimitiveType) -> Option<&'static str> {
    match p {
        PrimitiveType::Bool => None,
        PrimitiveType::I8
        | PrimitiveType::U8
        | PrimitiveType::I16
        | PrimitiveType::U16
        | PrimitiveType::I32
        | PrimitiveType::U32
        | PrimitiveType::I64
        | PrimitiveType::U64
        | PrimitiveType::Usize
        | PrimitiveType::Isize => Some("0"),
        PrimitiveType::F32 | PrimitiveType::F64 => Some("0.0"),
    }
}

/// Return a Rust cast target for a JNI primitive → Rust type conversion, or "" if no cast needed.
/// jboolean is bool (jni 0.22+) and jint is i32, so those types need no cast.
fn primitive_cast(p: &PrimitiveType) -> &'static str {
    match p {
        // jboolean is now `bool` in jni 0.22+; no cast needed
        PrimitiveType::Bool => "",
        PrimitiveType::I8 => "i8",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U16 => "u16",
        // jint is i32; no cast needed
        PrimitiveType::I32 => "",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::U64 => "u64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    }
}

/// Map a TypeRef to a Rust type path for serde deserialization.
fn type_ref_to_core_path(ty: &TypeRef, core_prefix: &str) -> String {
    type_ref_to_core_path_with_btree(ty, core_prefix, false)
}

/// Like [`type_ref_to_core_path`] but honours the concrete map container declared
/// by the core function. When `map_is_btree` is true and `ty` is a `Map`, the
/// outermost (possibly `Option`/`Vec`-wrapped) map is emitted as
/// `std::collections::BTreeMap` so the deserialization target and the call-site
/// argument match the core signature (`&BTreeMap<K, V>`). Passing a `HashMap`
/// where the core expects a `BTreeMap` fails with E0308.
///
/// The `TypeRef` IR erases the distinction between `HashMap`/`BTreeMap`, so the
/// container choice is carried separately on `ParamDef::map_is_btree`.
fn type_ref_to_core_path_with_btree(ty: &TypeRef, core_prefix: &str, map_is_btree: bool) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Primitive(p) => primitive_rust_type(p).to_string(),
        TypeRef::Named(n) => format!("{core_prefix}::{n}"),
        TypeRef::Optional(inner) => {
            format!(
                "Option<{}>",
                type_ref_to_core_path_with_btree(inner, core_prefix, map_is_btree)
            )
        }
        TypeRef::Vec(inner) => {
            format!(
                "Vec<{}>",
                type_ref_to_core_path_with_btree(inner, core_prefix, map_is_btree)
            )
        }
        TypeRef::Map(k, v) => {
            let container = if map_is_btree {
                "std::collections::BTreeMap"
            } else {
                "std::collections::HashMap"
            };
            format!(
                "{container}<{}, {}>",
                type_ref_to_core_path(k, core_prefix),
                type_ref_to_core_path(v, core_prefix)
            )
        }
        _ => "serde_json::Value".to_string(),
    }
}

/// True when `ty` is the byte-slice base type: `bytes::Bytes` (`TypeRef::Bytes`)
/// or `Vec<u8>` (`TypeRef::Vec(U8)`). The IR has already unwrapped any outer
/// `Option`, so this checks the inner element type only.
fn is_byte_slice(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Bytes)
        || matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)))
}

/// Build the call-site argument for a byte-slice parameter.
///
/// The unmarshal step binds `Vec<u8>` (or `Option<Vec<u8>>` when optional). The
/// core function may want any of four shapes, distinguished by `optional`/`is_ref`:
/// - `Option<&[u8]>` (`optional && is_ref`): `name.as_deref()` — `Option<Vec<u8>>`
///   does not coerce to `Option<&[u8]>`, so the deref-conversion is required (E0308
///   otherwise).
/// - `Option<Vec<u8>>` (`optional && !is_ref`): pass the owned `Option` through.
/// - `&[u8]` (`!optional && is_ref`): `&name` — `&Vec<u8>` coerces to `&[u8]`.
/// - `Vec<u8>` (`!optional && !is_ref`): pass the owned `Vec` through.
fn bytes_call_arg(name: &str, optional: bool, is_ref: bool) -> String {
    match (optional, is_ref) {
        (true, true) => format!("{name}.as_deref()"),
        (false, true) => format!("&{name}"),
        (_, false) => name.to_string(),
    }
}

fn needs_vec_string_refs(param: &ParamDef, ty: &TypeRef) -> bool {
    param.is_ref
        && param.vec_inner_is_ref
        && matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
}

fn render_vec_string_refs_binding(name: &str) -> String {
    let refs_name = format!("{name}_refs");
    template_env::render(
        "vec_string_refs.rs.jinja",
        context! {
            refs_name => refs_name,
            source_name => name,
        },
    )
}

fn vec_string_refs_arg(name: &str) -> String {
    format!("&{name}_refs")
}

fn primitive_rust_type(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "bool",
        PrimitiveType::I8 => "i8",
        PrimitiveType::U8 => "u8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::U16 => "u16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::U64 => "u64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    }
}

/// Resolve the Kotlin package string used when constructing JNI symbols.
///
/// Prefers `[crates.kotlin_android] package`, then `[crates.kotlin] package`,
/// then falls back to `config.kotlin_package()`.
fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    config
        .kotlin_android
        .as_ref()
        .and_then(|a| a.package.clone())
        .or_else(|| config.kotlin.as_ref().and_then(|k| k.package.clone()))
        .unwrap_or_else(|| config.kotlin_package())
}

/// Resolve the fully-qualified error class name for `ERROR_CLASS`.
///
/// Uses `<package_slashed>/<BridgeName>Exception` as default.
fn resolve_error_class(config: &ResolvedCrateConfig, package: &str) -> String {
    let package_slashed = package.replace('.', "/");
    let bridge = bridge_class_name(&config.name);
    format!("{package_slashed}/{bridge}Exception")
}

/// Return the `use` path for the core crate from the JNI shim.
///
/// Uses the `name` field of the config (which is the crate name, e.g.
/// `sample-llm`), converting hyphens to underscores per Rust convention.
fn core_use_path(config: &ResolvedCrateConfig) -> String {
    config.name.replace('-', "_")
}
