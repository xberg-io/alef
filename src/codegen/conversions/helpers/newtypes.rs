use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, TypeRef};

pub(crate) fn is_explicit_newtype(wrapper: &str) -> bool {
    !NewtypeWrapper::decode(wrapper).explicit_paths().is_empty()
}

pub(crate) fn apply_explicit_field_newtype_to_core(expr: &str, field: &FieldDef) -> Option<String> {
    field
        .newtype_wrapper
        .as_deref()
        .filter(|wrapper| is_explicit_newtype(wrapper))
        .map(|wrapper| apply_field_newtype_to_core(expr, &field.ty, field.optional, wrapper))
}

pub(crate) fn apply_explicit_field_newtype_from_core(expr: &str, field: &FieldDef) -> Option<String> {
    field
        .newtype_wrapper
        .as_deref()
        .filter(|wrapper| is_explicit_newtype(wrapper))
        .map(|wrapper| apply_field_newtype_from_core(expr, &field.ty, field.optional, wrapper))
}

pub(crate) fn apply_field_newtype_to_core(expr: &str, ty: &TypeRef, optional: bool, wrapper: &str) -> String {
    let decoded = NewtypeWrapper::decode(wrapper);
    if !decoded.explicit_paths().is_empty() {
        return apply_explicit_paths(expr, decoded.explicit_paths(), Direction::ToCore);
    }
    let path = decoded.tuple_path().expect("legacy wrapper must have a tuple path");
    match ty {
        TypeRef::Optional(_) => format!("({expr}).map({path})"),
        TypeRef::Vec(_) => format!("({expr}).into_iter().map({path}).collect()"),
        _ if optional => format!("({expr}).map({path})"),
        _ => format!("{path}({expr})"),
    }
}

pub(crate) fn apply_field_newtype_from_core(expr: &str, ty: &TypeRef, optional: bool, wrapper: &str) -> String {
    let decoded = NewtypeWrapper::decode(wrapper);
    if !decoded.explicit_paths().is_empty() {
        return apply_explicit_paths(expr, decoded.explicit_paths(), Direction::FromCore);
    }
    match ty {
        TypeRef::Optional(_) => format!("({expr}).map(|value| value.0)"),
        TypeRef::Vec(_) => format!("({expr}).into_iter().map(|value| value.0).collect()"),
        _ if optional => format!("({expr}).map(|value| value.0)"),
        _ => apply_newtype_from_core(expr, wrapper),
    }
}

pub(crate) fn apply_newtype_to_core(expr: &str, wrapper: &str) -> String {
    apply_field_newtype_to_core(expr, &TypeRef::String, false, wrapper)
}

pub(crate) fn apply_newtype_from_core(expr: &str, wrapper: &str) -> String {
    apply_field_newtype_from_core(expr, &TypeRef::String, false, wrapper)
}

pub(crate) fn apply_newtype_from_core_after_optionals(expr: &str, wrapper: &str, count: usize) -> String {
    let mut paths = NewtypeWrapper::decode(wrapper).explicit_paths().to_vec();
    for metadata in &mut paths {
        for _ in 0..count {
            if metadata.containers.first() == Some(&NewtypeContainer::Optional) {
                metadata.containers.remove(0);
            }
        }
    }
    apply_explicit_paths(expr, &paths, Direction::FromCore)
}

#[derive(Clone, Copy)]
enum Direction {
    ToCore,
    FromCore,
}

fn apply_explicit_paths(expr: &str, paths: &[NewtypeWrapperMetadata], direction: Direction) -> String {
    paths
        .iter()
        .enumerate()
        .fold(expr.to_string(), |current, (index, metadata)| {
            let concrete_intermediate = index + 1 < paths.len();
            apply_at_container_path(
                &current,
                &metadata.containers,
                direction,
                concrete_intermediate,
                &|value| match (&metadata.conversion, direction) {
                    (NewtypeConversion::TransparentString { from, .. }, Direction::ToCore) => {
                        format!("{}::{from}({value})", metadata.rust_path)
                    }
                    (NewtypeConversion::TransparentString { into, .. }, Direction::FromCore) => {
                        format!("({value}).{into}()")
                    }
                    (NewtypeConversion::TupleField, Direction::ToCore) => {
                        format!("{}({value})", metadata.rust_path)
                    }
                    (NewtypeConversion::TupleField, Direction::FromCore) => format!("({value}).0"),
                },
            )
        })
}

fn apply_at_container_path(
    expr: &str,
    containers: &[NewtypeContainer],
    direction: Direction,
    concrete_intermediate: bool,
    leaf: &impl Fn(&str) -> String,
) -> String {
    let Some((container, rest)) = containers.split_first() else {
        return leaf(expr);
    };
    match container {
        NewtypeContainer::Optional => {
            let converted = apply_at_container_path("value", rest, direction, concrete_intermediate, leaf);
            format!("({expr}).map(|value| {converted})")
        }
        NewtypeContainer::Vec => {
            let converted = apply_at_container_path("value", rest, direction, concrete_intermediate, leaf);
            let collect = collection_suffix(direction, concrete_intermediate, "Vec<_>");
            format!("({expr}).into_iter().map(|value| {converted}).collect{collect}")
        }
        NewtypeContainer::MapKey => {
            let converted = apply_at_container_path("key", rest, direction, concrete_intermediate, leaf);
            let collect = collection_suffix(direction, concrete_intermediate, "std::collections::HashMap<_, _>");
            format!("({expr}).into_iter().map(|(key, value)| ({converted}, value)).collect{collect}")
        }
        NewtypeContainer::MapValue => {
            let converted = apply_at_container_path("value", rest, direction, concrete_intermediate, leaf);
            let collect = collection_suffix(direction, concrete_intermediate, "std::collections::HashMap<_, _>");
            format!("({expr}).into_iter().map(|(key, value)| (key, {converted})).collect{collect}")
        }
    }
}

fn collection_suffix(direction: Direction, concrete_intermediate: bool, collection_type: &str) -> String {
    match (direction, concrete_intermediate) {
        (Direction::ToCore, false) => "()".to_string(),
        _ => format!("::<{collection_type}>()"),
    }
}
