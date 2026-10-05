//! Error introspection methods that survive the C ABI as `{prefix}_last_error_<method>` getters.
//!
//! The FFI backend captures these values from the live error while it still has the typed value
//! in hand, and consumers that read the thread-local last error (Go today) read them back. Both
//! sides derive the set from this module so a getter the FFI layer exports and the one Go calls
//! cannot disagree on name, type or which methods exist.

use crate::core::ir::{ErrorDef, MethodDef, PrimitiveType, TypeRef};

/// How a captured method value crosses the C ABI.
#[derive(Debug, Clone, PartialEq)]
pub enum LastErrorFieldKind {
    /// A `bool` or fixed-width numeric returned by value.
    Scalar(PrimitiveType),
    /// A string returned as a borrowed, NUL-terminated pointer owned by the thread-local slot.
    Text,
    /// A `Duration` (or `Option<Duration>`) returned as whole milliseconds, `-1` when absent.
    DurationMillis,
}

/// One method exposed through a `{prefix}_last_error_<method>` getter.
#[derive(Debug, Clone)]
pub struct LastErrorField {
    pub name: String,
    pub kind: LastErrorFieldKind,
}

/// Names that already belong to the fixed `{prefix}_last_error_*` getters.
const RESERVED_NAMES: &[&str] = &["code", "context", "variant"];

/// Classify an error method, or `None` when its signature cannot be carried through the ABI.
///
/// Only zero-argument, synchronous instance methods qualify. `usize`/`isize` are excluded because
/// the host-language struct generators have no fixed-width mapping for them.
pub fn last_error_field_kind(method: &MethodDef) -> Option<LastErrorFieldKind> {
    if method.is_static
        || method.is_async
        || !method.params.is_empty()
        || RESERVED_NAMES.contains(&method.name.as_str())
    {
        return None;
    }
    match &method.return_type {
        TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::Isize) => None,
        TypeRef::Primitive(primitive) => Some(LastErrorFieldKind::Scalar(primitive.clone())),
        TypeRef::String => Some(LastErrorFieldKind::Text),
        TypeRef::Duration => Some(LastErrorFieldKind::DurationMillis),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Duration) => {
            Some(LastErrorFieldKind::DurationMillis)
        }
        _ => None,
    }
}

/// The union of capturable methods across `errors`, in first-seen order.
///
/// A method name that two error types declare with different kinds keeps the first declaration;
/// the later error simply does not populate that getter.
pub fn last_error_fields(errors: &[ErrorDef]) -> Vec<LastErrorField> {
    let mut fields: Vec<LastErrorField> = Vec::new();
    for method in errors.iter().flat_map(|error| &error.methods) {
        let Some(kind) = last_error_field_kind(method) else {
            continue;
        };
        if !fields.iter().any(|field| field.name == method.name) {
            fields.push(LastErrorField {
                name: method.name.clone(),
                kind,
            });
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(name: &str, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            return_type,
            ..MethodDef::default()
        }
    }

    fn error(name: &str, methods: Vec<MethodDef>) -> ErrorDef {
        ErrorDef {
            name: name.to_string(),
            rust_path: format!("sample::{name}"),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn unions_capturable_methods_and_skips_the_rest() {
        let mut static_method = method("with_arg", TypeRef::String);
        static_method.is_static = true;
        let mut async_method = method("later", TypeRef::String);
        async_method.is_async = true;
        let sample = error(
            "SampleError",
            vec![
                method("status_code", TypeRef::Primitive(PrimitiveType::U16)),
                method("is_transient", TypeRef::Primitive(PrimitiveType::Bool)),
                method("error_type", TypeRef::String),
                method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
                method("payload_len", TypeRef::Primitive(PrimitiveType::Usize)),
                method("code", TypeRef::Primitive(PrimitiveType::U32)),
                method("details", TypeRef::Vec(Box::new(TypeRef::String))),
                static_method,
                async_method,
            ],
        );

        let names: Vec<String> = last_error_fields(&[sample])
            .into_iter()
            .map(|field| field.name)
            .collect();
        assert_eq!(names, ["status_code", "is_transient", "error_type", "retry_after"]);
    }

    #[test]
    fn duration_methods_travel_as_milliseconds() {
        let optional = method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration)));
        let bare = method("timeout", TypeRef::Duration);
        assert_eq!(
            last_error_field_kind(&optional),
            Some(LastErrorFieldKind::DurationMillis)
        );
        assert_eq!(last_error_field_kind(&bare), Some(LastErrorFieldKind::DurationMillis));
    }

    #[test]
    fn first_declaration_wins_when_two_errors_disagree_on_a_kind() {
        let first = error(
            "FirstError",
            vec![method("status_code", TypeRef::Primitive(PrimitiveType::U16))],
        );
        let second = error("SecondError", vec![method("status_code", TypeRef::String)]);

        let fields = last_error_fields(&[first, second]);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].kind, LastErrorFieldKind::Scalar(PrimitiveType::U16));
    }
}
