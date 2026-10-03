use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, TypeRef};

pub(crate) fn apply_explicit_field_newtype_to_core(expr: &str, field: &FieldDef) -> Option<String> {
    field
        .newtype_wrapper
        .as_ref()
        .filter(|wrapper| matches!(wrapper.conversion(), NewtypeConversion::TransparentString { .. }))
        .map(|wrapper| apply_field_newtype_to_core(expr, &field.ty, field.optional, wrapper))
}

pub(crate) fn apply_explicit_field_newtype_from_core(expr: &str, field: &FieldDef) -> Option<String> {
    field
        .newtype_wrapper
        .as_ref()
        .filter(|wrapper| matches!(wrapper.conversion(), NewtypeConversion::TransparentString { .. }))
        .map(|wrapper| apply_field_newtype_from_core(expr, &field.ty, field.optional, wrapper))
}

pub(crate) fn apply_field_newtype_to_core(
    expr: &str,
    ty: &TypeRef,
    optional: bool,
    wrapper: &NewtypeWrapper,
) -> String {
    if matches!(wrapper.conversion(), NewtypeConversion::TransparentString { .. }) {
        return apply_newtype_to_core(expr, wrapper);
    }
    match ty {
        TypeRef::Optional(_) => format!("({expr}).map({wrapper})"),
        TypeRef::Vec(_) => format!("({expr}).into_iter().map({wrapper}).collect()"),
        _ if optional => format!("({expr}).map({wrapper})"),
        _ => apply_newtype_to_core(expr, wrapper),
    }
}

pub(crate) fn apply_field_newtype_from_core(
    expr: &str,
    ty: &TypeRef,
    optional: bool,
    wrapper: &NewtypeWrapper,
) -> String {
    if matches!(wrapper.conversion(), NewtypeConversion::TransparentString { .. }) {
        return apply_newtype_from_core(expr, wrapper);
    }
    match ty {
        TypeRef::Optional(_) => format!("({expr}).map(|value| value.0)"),
        TypeRef::Vec(_) => format!("({expr}).into_iter().map(|value| value.0).collect()"),
        _ if optional => format!("({expr}).map(|value| value.0)"),
        _ => apply_newtype_from_core(expr, wrapper),
    }
}

pub(crate) fn apply_newtype_to_core(expr: &str, wrapper: &NewtypeWrapper) -> String {
    match wrapper.conversion() {
        NewtypeConversion::TupleField => format!("{}({expr})", wrapper.rust_path()),
        NewtypeConversion::TransparentString { from, .. } => {
            apply_at_container_path(expr, wrapper.containers(), &|value| {
                format!("{}::{from}({value})", wrapper.rust_path())
            })
        }
    }
}

pub(crate) fn apply_newtype_from_core(expr: &str, wrapper: &NewtypeWrapper) -> String {
    match wrapper.conversion() {
        NewtypeConversion::TupleField => format!("({expr}).0"),
        NewtypeConversion::TransparentString { into, .. } => {
            apply_at_container_path(expr, wrapper.containers(), &|value| format!("({value}).{into}()"))
        }
    }
}

fn apply_at_container_path(expr: &str, containers: &[NewtypeContainer], leaf: &impl Fn(&str) -> String) -> String {
    let Some((container, rest)) = containers.split_first() else {
        return leaf(expr);
    };
    match container {
        NewtypeContainer::Optional => {
            let converted = apply_at_container_path("value", rest, leaf);
            format!("({expr}).map(|value| {converted})")
        }
        NewtypeContainer::Vec => {
            let converted = apply_at_container_path("value", rest, leaf);
            format!("({expr}).into_iter().map(|value| {converted}).collect()")
        }
        NewtypeContainer::MapKey => {
            let converted = apply_at_container_path("key", rest, leaf);
            format!("({expr}).into_iter().map(|(key, value)| ({converted}, value)).collect()")
        }
        NewtypeContainer::MapValue => {
            let converted = apply_at_container_path("value", rest, leaf);
            format!("({expr}).into_iter().map(|(key, value)| (key, {converted})).collect()")
        }
    }
}
