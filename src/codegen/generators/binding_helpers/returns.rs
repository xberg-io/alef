use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

/// Return the numeric cast suffix (e.g. `" as f64"`) needed when a method's primitive return type
/// is represented by a different binding-level type.
///
/// Backends that remap numeric types (extendr maps `u8/u16/u32/i8/i16 → i32` and
/// `u64/i64/usize/isize/f32 → f64`) must cast the core return value to the binding type. Returns an
/// empty string when no remap applies (the common case for backends that preserve numeric widths).
pub fn primitive_return_cast_suffix(
    return_type: &TypeRef,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
) -> &'static str {
    use crate::codegen::conversions::helpers::{needs_f64_cast, needs_i32_cast};
    if let TypeRef::Primitive(p) = return_type {
        if cast_uints_to_i32 && needs_i32_cast(p) {
            return " as i32";
        }
        if cast_large_ints_to_f64 && needs_f64_cast(p) {
            return " as f64";
        }
    }
    ""
}

/// Helper: wrap an opaque inner value in the correct smart pointer expression.
///
/// - Plain opaque types use `Arc::new(val)`.
/// - Mutex-wrapped opaque types use `Arc::new(std::sync::Mutex::new(val))`.
fn arc_wrap(val: &str, name: &str, mutex_types: &AHashSet<String>) -> String {
    let needs_mutex = mutex_types.contains(name);
    crate::codegen::template_env::render(
        "binding_helpers/arc_wrap.jinja",
        minijinja::context! {
            val => val,
            needs_mutex => needs_mutex,
        },
    )
    .trim_end_matches('\n')
    .to_string()
}

/// Detect whether the core-call expression already evaluates to `Arc<T>` (or the
/// equivalent mutex-wrapped variant) for the binding's `inner` field.
///
/// When the method body forwards `self.inner` (e.g. `Node::clone` whose core impl
/// calls `Arc::clone(&self.tree)` internally), the expression is already an
/// `Arc<Node>` and must not be wrapped in another `Arc::new(...)`.
fn expr_is_already_arc(expr: &str) -> bool {
    let trimmed = expr.trim();
    trimmed == "self.inner" || trimmed == "self.inner.clone()" || trimmed == "self.inner.as_ref().clone()"
}

/// Wrap a core-call result for opaque delegation methods.
///
/// - `TypeRef::Named(n)` where `n == type_name` → re-wrap in `Self { inner: Arc::new(...) }`
/// - `TypeRef::Named(n)` where `n` is another opaque type → wrap in `{n} { inner: Arc::new(...) }`
/// - `TypeRef::Named(n)` where `n` is a non-opaque type → `.into()` conversion
/// - Everything else (primitives, String, Vec, etc.) → pass through unchanged
/// - `TypeRef::Unit` → pass through unchanged
///
/// When `returns_cow` is true the core method returns `Cow<'_, T>`. `.into_owned()` is emitted
/// before any further type conversion to obtain an owned `T`.
///
/// `mutex_types` identifies opaque types that use `Arc<Mutex<T>>` instead of `Arc<T>`, so
/// constructor expressions use `Arc::new(Mutex::new(...))` where needed.
#[allow(clippy::too_many_arguments)]
pub fn wrap_return_with_mutex(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
) -> String {
    wrap_return_with_mutex_mapped(
        expr,
        return_type,
        type_name,
        opaque_types,
        mutex_types,
        self_is_opaque,
        returns_ref,
        returns_cow,
        &crate::codegen::type_mapper::IdentityMapper,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn wrap_return_with_mutex_mapped(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    mapper: &dyn TypeMapper,
) -> String {
    let self_arc = arc_wrap("", type_name, mutex_types); // used for pattern matching only
    let _ = self_arc; // just to reference mutex_types in context
    match return_type {
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            // If the expression already evaluates to `Arc<T>` (e.g. `self.inner.clone()`
            // where `inner: Arc<T>`), don't wrap in another Arc — pass through.
            if expr_is_already_arc(expr) {
                return format!("Self {{ inner: {expr} }}");
            }
            let inner = if returns_cow {
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.clone()")
            } else {
                expr.to_string()
            };
            format!("Self {{ inner: {} }}", arc_wrap(&inner, type_name, mutex_types))
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            let mapped_n = mapper.named(n);
            // Same already-Arc guard as the Self branch above.
            if expr_is_already_arc(expr) {
                return format!("{mapped_n} {{ inner: {expr} }}");
            }
            let inner = if returns_cow {
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.clone()")
            } else {
                expr.to_string()
            };
            format!("{mapped_n} {{ inner: {} }}", arc_wrap(&inner, n, mutex_types))
        }
        TypeRef::Named(_) => {
            // Non-opaque Named return type — use .into() for core→binding From conversion.
            // When the core returns a Cow, call .into_owned() first to get an owned T.
            // When the core returns a reference, clone first since From<&T> typically doesn't exist.
            // NOTE: If this type was sanitized to String in the binding, From won't exist.
            // The calling backend should check method.sanitized before delegating.
            // This code assumes non-sanitized Named types have From impls.
            if returns_cow {
                format!("{expr}.into_owned().into()")
            } else if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        // String: only convert when the core returns a reference (&str→String).
        TypeRef::String => {
            if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        // Bytes: always use .to_vec() which works for both &Bytes and owned Bytes.
        // &Bytes does not implement From<&Bytes> for Vec<u8>, so .into() fails.
        TypeRef::Bytes => format!("{expr}.to_vec()"),
        // Path: PathBuf→String needs to_string_lossy, &Path→String too
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        // Duration: core returns std::time::Duration, binding uses u64 (millis)
        TypeRef::Duration => format!("{expr}.as_millis() as u64"),
        // Json: serde_json::Value needs serialization to string
        TypeRef::Json => format!("{expr}.to_string()"),
        // Optional: wrap inner conversion in .map(...)
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let mapped_n = mapper.named(n);
                let wrap = arc_wrap("v", n, mutex_types);
                if returns_ref {
                    format!(
                        "{expr}.map(|v| {mapped_n} {{ inner: {} }})",
                        arc_wrap("v.clone()", n, mutex_types)
                    )
                } else {
                    format!("{expr}.map(|v| {mapped_n} {{ inner: {wrap} }})")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            TypeRef::Duration => format!("{expr}.map(|d| d.as_millis() as u64)"),
            TypeRef::Json => format!("{expr}.map(ToString::to_string)"),
            // Optional<Vec<Named>>: convert each element in the inner Vec
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    let mapped_n = mapper.named(n);
                    if returns_ref {
                        let wrap = arc_wrap("x.clone()", n, mutex_types);
                        format!("{expr}.map(|v| v.into_iter().map(|x| {mapped_n} {{ inner: {wrap} }}).collect())")
                    } else {
                        let wrap = arc_wrap("x", n, mutex_types);
                        format!("{expr}.map(|v| v.into_iter().map(|x| {mapped_n} {{ inner: {wrap} }}).collect())")
                    }
                }
                TypeRef::Named(_) => {
                    if returns_ref {
                        format!("{expr}.map(|v| v.into_iter().map(|x| x.clone().into()).collect())")
                    } else {
                        format!("{expr}.map(|v| v.into_iter().map(Into::into).collect())")
                    }
                }
                _ => expr.to_string(),
            },
            _ => expr.to_string(),
        },
        // Vec: map each element through the appropriate conversion
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let mapped_n = mapper.named(n);
                if returns_ref {
                    let wrap = arc_wrap("v.clone()", n, mutex_types);
                    format!("{expr}.into_iter().map(|v| {mapped_n} {{ inner: {wrap} }}).collect()")
                } else {
                    let wrap = arc_wrap("v", n, mutex_types);
                    format!("{expr}.into_iter().map(|v| {mapped_n} {{ inner: {wrap} }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    // `&[T]` → `Vec<U>`: use `.iter()` not `.into_iter()` to
                    // avoid clippy::into_iter_on_ref under -D warnings.
                    format!("{expr}.iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String => {
                if returns_ref {
                    // Core returns `&[&str]` / `&[String]`; `.iter()` yields `&&str` / `&String`.
                    // `Into::into` on those fails (`From<&&str>` is not implemented). Force
                    // an explicit ToString hop so the binding always sees owned `String`s.
                    format!("{expr}.iter().map(|s| s.to_string()).collect()")
                } else {
                    expr.to_string()
                }
            }
            TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.iter().map(|b| b.to_vec()).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Wrap a core-call result for opaque delegation methods.
///
/// This is the backward-compatible wrapper that passes an empty `mutex_types` set.
/// Use `wrap_return_with_mutex` when the type set contains mutex-wrapped opaque types.
pub fn wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
) -> String {
    wrap_return_with_mutex(
        expr,
        return_type,
        type_name,
        opaque_types,
        &AHashSet::new(),
        self_is_opaque,
        returns_ref,
        returns_cow,
    )
}

/// Unwrap a newtype return value when `return_newtype_wrapper` is set.
///
/// Core function returns a newtype (e.g. `NodeIndex(u32)`), but the binding return type
/// is the inner type (e.g. `u32`). Access `.0` to unwrap the newtype.
pub fn apply_return_newtype_unwrap(expr: &str, return_newtype_wrapper: &Option<String>) -> String {
    match return_newtype_wrapper {
        Some(_) => crate::codegen::template_env::render(
            "binding_helpers/return_newtype_unwrap.jinja",
            minijinja::context! {
                expr => expr,
            },
        )
        .trim_end_matches('\n')
        .to_string(),
        None => expr.to_string(),
    }
}
