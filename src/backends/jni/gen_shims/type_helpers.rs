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
        TypeRef::String | TypeRef::Named(_) | TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => "jstring",
        _ => "jlong",
    }
}

/// The JNI wire representation of a primitive: the `jni`-crate sys type name used in shim
/// signatures, paired with the actual Rust type that sys type is a type alias for (e.g.
/// `jni::sys::jbyte` is `i8`, `jlong` is `i64`). `jni_primitive_type` and `primitive_cast`
/// both read this single table, so the declared signature type and the "does this value need
/// an `as` cast" decision can never independently disagree about what crosses the boundary. A
/// second, separately-maintained cast table previously assumed every primitive needs a cast to
/// its own JNI wire type, which is false whenever the wire type already IS that Rust type (e.g.
/// `F64` against `jni::sys::jdouble` = `f64`), producing `clippy::unnecessary_cast`. ~keep
fn jni_wire_repr(p: &PrimitiveType) -> (&'static str, &'static str) {
    match p {
        PrimitiveType::Bool => ("jboolean", "bool"),
        PrimitiveType::I8 | PrimitiveType::U8 => ("jni::sys::jbyte", "i8"),
        PrimitiveType::I16 | PrimitiveType::U16 => ("jni::sys::jshort", "i16"),
        PrimitiveType::I32 | PrimitiveType::U32 => ("jni::sys::jint", "i32"),
        PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => ("jlong", "i64"),
        PrimitiveType::F32 => ("jni::sys::jfloat", "f32"),
        PrimitiveType::F64 => ("jni::sys::jdouble", "f64"),
    }
}

fn jni_primitive_type(p: &PrimitiveType) -> &'static str {
    jni_wire_repr(p).0
}

/// True when the primitive's JNI wire type is not itself the primitive's own Rust type, so a
/// value crossing the JNI boundary in either direction needs an explicit `as` cast. Used by
/// `primitive_cast` (JNI wire -> core, in function-parameter unmarshalling) and by the
/// core -> JNI wire return-value cast in `emit_return_marshal_with_indent`, so both directions
/// consult the same wire/core comparison instead of guessing independently.
fn jni_primitive_needs_cast(p: &PrimitiveType) -> bool {
    jni_wire_repr(p).1 != primitive_rust_type(p)
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
/// Consults [`jni_primitive_needs_cast`] rather than a hand-picked list of "types that need
/// casting" — e.g. `Bool` and `I32` need none because `jboolean`/`jint` already alias `bool`/
/// `i32`, and the same reasoning now covers every other primitive whose wire type happens to
/// equal its own Rust type.
fn primitive_cast(p: &PrimitiveType) -> &'static str {
    if jni_primitive_needs_cast(p) {
        primitive_rust_type(p)
    } else {
        ""
    }
}

/// Map a TypeRef to a Rust type path for serde deserialization.
fn type_ref_to_core_path(ty: &TypeRef, core_prefix: &str) -> String {
    type_ref_to_core_path_with_btree(ty, core_prefix, false)
}

fn jni_param_newtype_to_core(expr: &str, param: &ParamDef) -> Option<String> {
    let wrapper = param.newtype_wrapper.as_deref()?;
    let wrapper = jni_newtype_wrapper(wrapper);
    if let Some(constructor) = root_optional_transparent_constructor(&wrapper) {
        return Some(format!("({expr}).map({constructor})"));
    }
    let mut jni_param = param.clone();
    jni_param.newtype_wrapper = Some(wrapper);
    crate::codegen::conversions::helpers::apply_param_newtype_to_core(expr, &jni_param)
}

fn root_optional_transparent_constructor(wrapper: &str) -> Option<String> {
    let decoded = crate::core::ir::NewtypeWrapper::decode(wrapper).ok()?;
    let [metadata] = decoded.explicit_paths() else {
        return None;
    };
    if metadata.containers.as_slice() != [crate::core::ir::NewtypeContainer::Optional] {
        return None;
    }
    let crate::core::ir::NewtypeConversion::TransparentString { from, .. } = &metadata.conversion else {
        return None;
    };
    Some(format!("{}::{from}", metadata.rust_path))
}

fn jni_return_newtype_from_core(expr: &str, wrapper: &str) -> String {
    let wrapper = jni_newtype_wrapper(wrapper);
    crate::codegen::conversions::helpers::apply_newtype_from_core(expr, &wrapper)
}

fn jni_newtype_wrapper(wrapper: &str) -> String {
    let decoded = crate::core::ir::NewtypeWrapper::decode(wrapper)
        .expect("newtype metadata must be validated before JNI code generation");
    let crate::core::ir::NewtypeWrapper::Explicit(mut paths) = decoded else {
        return wrapper.to_string();
    };
    for path in &mut paths {
        let type_path = path
            .rust_path
            .split_once("::")
            .map_or(path.rust_path.as_str(), |(_, suffix)| suffix)
            .to_string();
        path.rust_path = format!("core_crate::{type_path}");
    }
    crate::core::ir::NewtypeWrapper::encode_explicit(&paths)
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
        crate::alef_context! {
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
/// Delegates to [`crate::core::jni::jni_package`] — the canonical resolver both this backend and
/// `alef-backend-kotlin`'s JNI-mode emitter must share, so this backend's own `service.rs` symbols
/// (which call that function directly) can never fall back to a different package than the
/// `lib.rs` symbols this function's callers emit. This used to be a second, hand-copied precedence
/// chain here; see that function's doc comment for the drift that duplication caused. ~keep
fn jni_kotlin_package(config: &ResolvedCrateConfig) -> String {
    crate::core::jni::jni_package(config)
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

#[cfg(test)]
mod type_helpers_tests {
    use super::*;

    /// Regression coverage for the `clippy::unnecessary_cast` bug: `primitive_cast` (used to
    /// build the JNI-wire -> core Rust cast at a call site, e.g. `record_cost_usd(...,
    /// cost_usd as f64)` where `cost_usd` is already `f64`) must return `""` for every
    /// primitive whose JNI wire type is already that primitive's own Rust type, and a real
    /// cast target for every primitive whose wire type differs. This test was red before the
    /// fix -- `primitive_cast(&PrimitiveType::F64)` returned `"f64"`, not `""`.
    #[test]
    fn primitive_cast_omits_cast_when_wire_type_already_matches() {
        for p in [
            PrimitiveType::Bool,
            PrimitiveType::I32,
            PrimitiveType::I8,
            PrimitiveType::I16,
            PrimitiveType::I64,
            PrimitiveType::F32,
            PrimitiveType::F64,
        ] {
            assert_eq!(
                primitive_cast(&p),
                "",
                "{p:?} wire type already matches its own Rust type"
            );
        }
    }

    #[test]
    fn primitive_cast_still_emits_a_genuinely_needed_cast() {
        assert_eq!(primitive_cast(&PrimitiveType::U8), "u8");
        assert_eq!(primitive_cast(&PrimitiveType::U16), "u16");
        assert_eq!(primitive_cast(&PrimitiveType::U32), "u32");
        assert_eq!(primitive_cast(&PrimitiveType::U64), "u64");
        assert_eq!(primitive_cast(&PrimitiveType::Usize), "usize");
        assert_eq!(primitive_cast(&PrimitiveType::Isize), "isize");
    }

    /// Same defect, return direction: `emit_return_marshal_with_indent` builds `v as {jni_ty}`
    /// from `jni_primitive_needs_cast`. F64's wire type (`jni::sys::jdouble`) is `f64` itself,
    /// so a `f64`-returning method must not cast its return value.
    #[test]
    fn jni_primitive_needs_cast_agrees_with_primitive_cast_directionally() {
        for p in [
            PrimitiveType::Bool,
            PrimitiveType::I8,
            PrimitiveType::I16,
            PrimitiveType::I32,
            PrimitiveType::I64,
            PrimitiveType::F32,
            PrimitiveType::F64,
        ] {
            assert!(!jni_primitive_needs_cast(&p), "{p:?} needs no cast in either direction");
        }
        for p in [
            PrimitiveType::U8,
            PrimitiveType::U16,
            PrimitiveType::U32,
            PrimitiveType::U64,
            PrimitiveType::Usize,
            PrimitiveType::Isize,
        ] {
            assert!(
                jni_primitive_needs_cast(&p),
                "{p:?} genuinely needs a cast in either direction"
            );
        }
    }
}
