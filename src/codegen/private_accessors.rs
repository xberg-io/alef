//! Carry the value of a public accessor over a private core field through mirroring bindings.
//!
//! A mirroring backend (pyo3, napi, PHP, ...) copies a core struct's public fields into its own
//! binding struct and runs a method by converting that struct back to the core type. A
//! `&self` accessor that reads a private field (html-to-markdown's `NodeContext::attributes()`)
//! then reads whatever the binding-to-core conversion put there, which is an empty default:
//! the binding never held the value (xberg-io/alef#865).
//!
//! [`promote_private_accessors`] makes such an accessor's value a binding field, read through
//! the accessor in the core-to-binding conversion and passed to the public constructor in the
//! binding-to-core conversion. The accessor method stays, and now returns the carried value.
//! When no public constructor accepts the value, the binding cannot carry it back: the method
//! keeps its current behavior and the accessor is reported.

use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

/// True when `field` of `typ` holds a private core field's value read through its accessor.
#[must_use]
pub fn is_private_accessor_field(typ: &TypeDef, field: &FieldDef) -> bool {
    typ.private_fields.iter().any(|private| private.name == field.name)
}

fn accessor<'a>(typ: &'a TypeDef, field_name: &str) -> Option<&'a MethodDef> {
    typ.methods
        .iter()
        .find(|method| method.name == field_name && method.receiver.is_some())
}

/// The suffix that turns the accessor's return value into an owned value of the field's type,
/// chosen by what the accessor returns (`&str`, `&[T]`, `&T`, `Option<&T>`, a `Cow` or a value).
fn owning_suffix(method: &MethodDef) -> &'static str {
    let owns_through_to_owned =
        |ty: &TypeRef| matches!(ty, TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Bytes);
    if method.returns_cow {
        return if matches!(method.return_type, TypeRef::Optional(_)) {
            ".map(std::borrow::Cow::into_owned)"
        } else {
            ".into_owned()"
        };
    }
    if !method.returns_ref {
        return "";
    }
    match &method.return_type {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Primitive(_)) => ".copied()",
        TypeRef::Optional(inner) if owns_through_to_owned(inner) => ".map(ToOwned::to_owned)",
        TypeRef::Optional(_) => ".cloned()",
        ty if owns_through_to_owned(ty) => ".to_owned()",
        _ => ".clone()",
    }
}

/// The accessor call that owns the field's value, and whether it is a dereferenced `Copy` value
/// (which needs parentheses before a method call).
fn owned_call(typ: &TypeDef, receiver: &str, field_name: &str) -> (String, bool) {
    let call = format!("{receiver}.{field_name}()");
    match accessor(typ, field_name) {
        Some(method) if method.returns_ref && matches!(method.return_type, TypeRef::Primitive(_)) => {
            (format!("*{call}"), true)
        }
        Some(method) => (format!("{call}{}", owning_suffix(method)), false),
        None => (format!("{call}.clone()"), false),
    }
}

fn place_pattern(receiver: &str, field_name: &str) -> regex::Regex {
    regex::Regex::new(&format!(
        r"\b{}\.{}\b(?:\(\))?",
        regex::escape(receiver),
        regex::escape(field_name)
    ))
    .expect("identifiers are valid regex literals once escaped")
}

/// Rewrite every `<receiver>.<field>` read in a core-to-binding conversion expression into a
/// call of the field's public accessor that owns its value as the field read would. A trailing
/// `.clone()` is folded into that call.
#[must_use]
pub fn read_through_accessor(conversion: &str, receiver: &str, typ: &TypeDef, field_name: &str) -> String {
    let (owned, derefs_copy) = owned_call(typ, receiver, field_name);
    let mut out = String::with_capacity(conversion.len());
    let mut last = 0;
    for found in place_pattern(receiver, field_name).find_iter(conversion) {
        out.push_str(&conversion[last..found.start()]);
        let rest = &conversion[found.end()..];
        let rest_start = found.end()
            + if rest.starts_with(".clone()") {
                ".clone()".len()
            } else {
                0
            };
        if derefs_copy && conversion[rest_start..].starts_with('.') {
            out.push_str(&format!("({owned})"));
        } else {
            out.push_str(&owned);
        }
        last = rest_start;
    }
    out.push_str(&conversion[last..]);
    out
}

/// Rewrite every `<receiver>.<field>` place expression into the field's accessor call, so
/// borrowing reads keep their meaning. With a reference-returning accessor, a method chain
/// (`ctx.field.iter()`) and a plain borrow (`&ctx.field`) take the accessor's reference as is,
/// a `.clone()` becomes the owning call, and any other read dereferences the reference back to
/// the field's place (`(*ctx.field()) as f64`).
#[must_use]
pub fn borrow_through_accessor(code: &str, receiver: &str, typ: &TypeDef, field_name: &str) -> String {
    let returns_ref = accessor_returns_ref(typ, field_name);
    let accessor = format!("{receiver}.{field_name}()");
    let mut out = String::with_capacity(code.len());
    let mut last = 0;
    for found in place_pattern(receiver, field_name).find_iter(code) {
        let mut prefix = &code[last..found.start()];
        let rest = &code[found.end()..];
        last = found.end();
        if rest.starts_with(".clone()") {
            out.push_str(prefix);
            let (owned, derefs_copy) = owned_call(typ, receiver, field_name);
            last += ".clone()".len();
            if derefs_copy && code[last..].starts_with('.') {
                out.push_str(&format!("({owned})"));
            } else {
                out.push_str(&owned);
            }
            continue;
        }
        let chained = rest.trim_start().starts_with('.');
        let borrowed = returns_ref && prefix.ends_with('&') && !chained;
        if borrowed {
            prefix = &prefix[..prefix.len() - 1];
        }
        out.push_str(prefix);
        if !returns_ref || chained || borrowed {
            out.push_str(&accessor);
        } else {
            out.push_str(&format!("(*{accessor})"));
        }
    }
    out.push_str(&code[last..]);
    out
}

/// Whether the accessor of `field_name` on `typ` returns a reference.
#[must_use]
pub fn accessor_returns_ref(typ: &TypeDef, field_name: &str) -> bool {
    accessor(typ, field_name).is_none_or(|method| method.returns_ref)
}

/// The field's type as a whole, with `Option` folded back in.
fn full_type(ty: &TypeRef, optional: bool) -> TypeRef {
    if optional && !matches!(ty, TypeRef::Optional(_)) {
        TypeRef::Optional(Box::new(ty.clone()))
    } else {
        ty.clone()
    }
}

/// The private field a plain `&self` getter of the same name reads. The field's storage type is
/// not compared: an accessor may materialize it (html-to-markdown stores a lazy attribute source
/// and returns the map), and what has to agree is the accessor's type with the constructor
/// parameter that takes the value back ([`constructor_accepts`]). A field serde skips is left
/// alone, since a JSON round trip could not carry it.
fn returned_private_field<'a>(typ: &'a TypeDef, method: &MethodDef) -> Option<&'a FieldDef> {
    let simple_getter = method.receiver == Some(ReceiverKind::Ref)
        && !method.is_static
        && !method.is_async
        && method.params.is_empty()
        && method.error_type.is_none()
        && !method.sanitized
        && method.return_type != TypeRef::Unit;
    if !simple_getter || typ.fields.iter().any(|field| field.name == method.name) {
        return None;
    }
    typ.private_fields
        .iter()
        .find(|private| private.name == method.name && !private.serde_skip)
}

/// The binding-to-core conversion goes through a static constructor here (a lifetime-bearing
/// type's, or a type's beside its own static `new`), and the constructor it selects once the
/// value is a binding field takes it back: a by-value parameter named after the accessor with
/// the accessor's own type. A by-reference parameter is refused, since the conversion passes an
/// owned value.
fn constructor_accepts(typ: &TypeDef, accessor: &MethodDef, carried: &FieldDef) -> bool {
    let has_static_new = typ
        .methods
        .iter()
        .any(|method| method.is_static && method.name == "new");
    if !typ.has_lifetime_params && !has_static_new {
        return false;
    }
    let mut promoted = typ.clone();
    promoted.fields.push(carried.clone());
    crate::codegen::conversions::select_constructor(&promoted).is_some_and(|constructor| {
        constructor.params.iter().any(|param| {
            param.name == accessor.name && !param.is_ref && full_type(&param.ty, param.optional) == accessor.return_type
        })
    })
}

/// Whether the type's own zero (`DefaultValue::Empty`) is a value every binding can render for
/// `ty`, so a host-built value may start empty.
fn has_renderable_zero(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Map(_, _)
            | TypeRef::Vec(_)
            | TypeRef::String
            | TypeRef::Bytes
            | TypeRef::Primitive(_)
            | TypeRef::Optional(_)
    )
}

fn warn_once(typ: &TypeDef, accessor: &str) {
    static WARNED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    let key = format!("{}.{accessor}", typ.rust_path);
    let first = WARNED
        .get_or_init(Default::default)
        .lock()
        .map(|mut warned| warned.insert(key))
        .unwrap_or(true);
    if first {
        tracing::warn!(
            "{}.{}() reads a private field and no public constructor accepts its value, so a \
             mirroring binding cannot carry it and the method returns the value of a rebuilt core",
            typ.name,
            accessor
        );
    }
}

/// Promote every carriable private-field accessor of `typ` to a binding field. An accessor the
/// binding cannot carry back to the core type keeps today's method and is reported.
pub fn promote_private_accessors(typ: &mut TypeDef) {
    if typ.is_opaque || typ.private_fields.is_empty() {
        return;
    }
    let candidates: Vec<(MethodDef, FieldDef)> = typ
        .methods
        .iter()
        .filter_map(|method| returned_private_field(typ, method).map(|private| (method.clone(), private.clone())))
        .collect();
    for (accessor, private) in candidates {
        let (ty, optional) = match &accessor.return_type {
            TypeRef::Optional(inner) => (inner.as_ref().clone(), true),
            other => (other.clone(), false),
        };
        let carried = FieldDef {
            name: private.name.clone(),
            // The core fills the value itself, so a host-built value may start empty.
            typed_default: has_renderable_zero(&ty).then_some(DefaultValue::Empty),
            ty,
            optional,
            doc: accessor.doc.clone(),
            cfg: accessor.cfg.clone(),
            serde_rename: private.serde_rename.clone(),
            ..FieldDef::default()
        };
        if !constructor_accepts(typ, &accessor, &carried) {
            warn_once(typ, &accessor.name);
            continue;
        }
        typ.fields.push(carried);
    }
}

/// Apply [`promote_private_accessors`] to every type of `api`.
#[must_use]
pub fn with_private_accessor_fields(api: &ApiSurface) -> ApiSurface {
    let mut promoted = api.clone();
    for typ in &mut promoted.types {
        promote_private_accessors(typ);
    }
    promoted
}

/// [`with_private_accessor_fields`] for a public surface (type stubs, docs) whose backend keeps
/// the carried value behind the accessor method: carried fields are not listed.
#[must_use]
pub fn with_private_accessor_methods(api: &ApiSurface) -> ApiSurface {
    let mut promoted = with_private_accessor_fields(api);
    for typ in &mut promoted.types {
        let private_fields = std::mem::take(&mut typ.private_fields);
        typ.fields
            .retain(|field| !private_fields.iter().any(|private| private.name == field.name));
        typ.private_fields = private_fields;
    }
    promoted
}

#[cfg(test)]
#[path = "private_accessors_tests.rs"]
mod tests;
