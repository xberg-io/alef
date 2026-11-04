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
//! When no public constructor accepts the value, the binding cannot carry it back, so the method
//! is dropped and reported instead of returning empty data.

use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

/// True when `field` of `typ` holds a private core field's value read through its accessor.
#[must_use]
pub fn is_private_accessor_field(typ: &TypeDef, field: &FieldDef) -> bool {
    typ.private_fields.contains(&field.name)
}

/// Rewrite every `<receiver>.<field>` read in a core-to-binding conversion expression into a
/// call of the field's public accessor, cloned so the expression owns the value as a field read
/// would.
#[must_use]
pub fn read_through_accessor(conversion: &str, receiver: &str, field_name: &str, returns_ref: bool) -> String {
    let pattern = regex::Regex::new(&format!(
        r"\b{}\.{}\b(?:\(\))?",
        regex::escape(receiver),
        regex::escape(field_name)
    ))
    .expect("identifiers are valid regex literals once escaped");
    let replacement = if returns_ref {
        format!("{receiver}.{field_name}().clone()")
    } else {
        format!("{receiver}.{field_name}()")
    };
    pattern.replace_all(conversion, replacement.as_str()).into_owned()
}

/// Rewrite every `<receiver>.<field>` place expression into the field's accessor call, so
/// borrowing reads keep their meaning. With a reference-returning accessor, a method chain
/// (`ctx.field.iter()`) and a plain borrow (`&ctx.field`) take the accessor's reference as is,
/// and any other read dereferences it back to the field's place (`(*ctx.field()) as f64`).
#[must_use]
pub fn borrow_through_accessor(code: &str, receiver: &str, field_name: &str, returns_ref: bool) -> String {
    let pattern = regex::Regex::new(&format!(
        r"\b{}\.{}\b(?:\(\))?",
        regex::escape(receiver),
        regex::escape(field_name)
    ))
    .expect("identifiers are valid regex literals once escaped");
    let accessor = format!("{receiver}.{field_name}()");
    let mut out = String::with_capacity(code.len());
    let mut last = 0;
    for found in pattern.find_iter(code) {
        let mut prefix = &code[last..found.start()];
        let chained = code[found.end()..].trim_start().starts_with('.');
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
        last = found.end();
    }
    out.push_str(&code[last..]);
    out
}

/// Whether the accessor of `field_name` on `typ` returns a reference (and so needs a clone).
#[must_use]
pub fn accessor_returns_ref(typ: &TypeDef, field_name: &str) -> bool {
    typ.methods
        .iter()
        .find(|method| method.name == field_name && method.receiver.is_some())
        .is_none_or(|method| method.returns_ref)
}

fn is_private_field_accessor(typ: &TypeDef, method: &MethodDef) -> bool {
    method.receiver == Some(ReceiverKind::Ref)
        && !method.is_static
        && !method.is_async
        && method.params.is_empty()
        && method.error_type.is_none()
        && !method.sanitized
        && method.return_type != TypeRef::Unit
        && typ.private_fields.contains(&method.name)
        && !typ.fields.iter().any(|field| field.name == method.name)
}

/// A static constructor taking every binding field plus `extra` exists, so the
/// binding-to-core conversion (which calls such a constructor for a lifetime-bearing type) can
/// pass the carried value back.
fn constructor_accepts(typ: &TypeDef, extra: &str) -> bool {
    typ.has_lifetime_params
        && typ.methods.iter().any(|method| {
            method.receiver.is_none()
                && method.params.iter().any(|param| param.name == extra)
                && typ
                    .fields
                    .iter()
                    .filter(|field| !field.binding_excluded)
                    .all(|field| method.params.iter().any(|param| param.name == field.name))
        })
}

/// Promote every carriable private-field accessor of `typ` to a binding field; drop and report
/// the accessors whose value the binding could not carry back to the core type.
pub fn promote_private_accessors(typ: &mut TypeDef) {
    if typ.is_opaque || typ.private_fields.is_empty() {
        return;
    }
    let accessors: Vec<MethodDef> = typ
        .methods
        .iter()
        .filter(|method| is_private_field_accessor(typ, method))
        .cloned()
        .collect();
    for accessor in accessors {
        if constructor_accepts(typ, &accessor.name) {
            typ.fields.push(FieldDef {
                name: accessor.name.clone(),
                ty: accessor.return_type.clone(),
                doc: accessor.doc.clone(),
                cfg: accessor.cfg.clone(),
                // The core fills the value itself, so a host-built value may start empty.
                typed_default: Some(crate::core::ir::DefaultValue::Empty),
                ..FieldDef::default()
            });
        } else {
            tracing::warn!(
                "{}.{}() reads a private field and no public constructor accepts its value, so a \
                 binding cannot carry it; the method is not generated",
                typ.name,
                accessor.name
            );
            typ.methods
                .retain(|method| method.name != accessor.name || method.receiver.is_none());
        }
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
/// the carried value behind the accessor method: dropped accessors are gone, carried fields are
/// not listed.
#[must_use]
pub fn with_private_accessor_methods(api: &ApiSurface) -> ApiSurface {
    let mut promoted = with_private_accessor_fields(api);
    for typ in &mut promoted.types {
        let private_fields = std::mem::take(&mut typ.private_fields);
        typ.fields.retain(|field| !private_fields.contains(&field.name));
        typ.private_fields = private_fields;
    }
    promoted
}

#[cfg(test)]
#[path = "private_accessors_tests.rs"]
mod tests;
