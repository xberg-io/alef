use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{
    FieldDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, ParamDef, TypeRef,
};

pub(crate) fn is_explicit_newtype(wrapper: &str) -> bool {
    !NewtypeWrapper::decode(wrapper)
        .expect("newtype metadata must be validated during extraction")
        .explicit_paths()
        .is_empty()
}

pub(crate) fn explicit_newtype_uses_wasm_jsvalue(wrapper: &str) -> bool {
    NewtypeWrapper::decode(wrapper).is_ok_and(|decoded| {
        decoded.explicit_paths().iter().any(|metadata| {
            metadata.containers.len() > 1
                || metadata
                    .containers
                    .iter()
                    .any(|container| matches!(container, NewtypeContainer::MapKey | NewtypeContainer::MapValue))
        })
    })
}

pub(crate) fn core_container_type(ty: &TypeRef, type_paths: Option<&ahash::AHashMap<String, String>>) -> String {
    match ty {
        TypeRef::Primitive(value) => value.rust_source_display().to_string(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", core_container_type(inner, type_paths)),
        TypeRef::Vec(inner) => format!("Vec<{}>", core_container_type(inner, type_paths)),
        TypeRef::Map(key, value) => format!(
            "std::collections::HashMap<{}, {}>",
            core_container_type(key, type_paths),
            core_container_type(value, type_paths)
        ),
        TypeRef::Named(name) => type_paths
            .and_then(|paths| paths.get(name))
            .cloned()
            .unwrap_or_else(|| name.clone()),
        TypeRef::Path => "String".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "u64".to_string(),
    }
}

pub(crate) fn explicit_newtype_covers_non_identity_leaves(ty: &TypeRef, optional: bool, wrapper: &str) -> bool {
    let decoded = NewtypeWrapper::decode(wrapper).expect("newtype metadata must be validated before codegen");
    let mut path = Vec::new();
    if optional {
        path.push(NewtypeContainer::Optional);
    }
    non_identity_leaves_are_wrapped(ty, &path, decoded.explicit_paths())
}

pub(crate) fn explicit_newtype_replaces_base_conversion(
    ty: &TypeRef,
    optional: bool,
    wrapper: &str,
    config: &ConversionConfig<'_>,
) -> bool {
    explicit_newtype_covers_non_identity_leaves(ty, optional, wrapper)
        && !(contains_map(ty) && (config.map_uses_jsvalue || config.map_flatten_to_string || config.map_as_string))
}

fn contains_map(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_map(inner),
        TypeRef::Map(_, _) => true,
        _ => false,
    }
}

fn non_identity_leaves_are_wrapped(
    ty: &TypeRef,
    path: &[NewtypeContainer],
    metadata: &[NewtypeWrapperMetadata],
) -> bool {
    if metadata.iter().any(|item| item.containers == path) {
        return true;
    }
    match ty {
        TypeRef::String | TypeRef::Unit => true,
        TypeRef::Optional(inner) => {
            let mut inner_path = path.to_vec();
            inner_path.push(NewtypeContainer::Optional);
            non_identity_leaves_are_wrapped(inner, &inner_path, metadata)
        }
        TypeRef::Vec(inner) => {
            let mut inner_path = path.to_vec();
            inner_path.push(NewtypeContainer::Vec);
            non_identity_leaves_are_wrapped(inner, &inner_path, metadata)
        }
        TypeRef::Map(key, value) => {
            let mut key_path = path.to_vec();
            key_path.push(NewtypeContainer::MapKey);
            let mut value_path = path.to_vec();
            value_path.push(NewtypeContainer::MapValue);
            non_identity_leaves_are_wrapped(key, &key_path, metadata)
                && non_identity_leaves_are_wrapped(value, &value_path, metadata)
        }
        _ => false,
    }
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
    apply_field_newtype_to_core_with_map_collection(expr, ty, optional, wrapper, None)
}

pub(crate) fn apply_param_newtype_to_core(expr: &str, param: &ParamDef) -> Option<String> {
    let wrapper = param.newtype_wrapper.as_deref()?;
    let map_collection = param.map_is_btree.then_some("std::collections::BTreeMap<_, _>");
    Some(apply_field_newtype_to_core_with_map_collection(
        expr,
        &param.ty,
        param.optional,
        wrapper,
        map_collection,
    ))
}

fn apply_field_newtype_to_core_with_map_collection(
    expr: &str,
    ty: &TypeRef,
    optional: bool,
    wrapper: &str,
    map_collection: Option<&str>,
) -> String {
    let decoded = NewtypeWrapper::decode(wrapper).expect("newtype metadata must be validated during extraction");
    if !decoded.explicit_paths().is_empty() {
        return apply_explicit_paths(expr, decoded.explicit_paths(), Direction::ToCore { map_collection });
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
    let decoded = NewtypeWrapper::decode(wrapper).expect("newtype metadata must be validated during extraction");
    if !decoded.explicit_paths().is_empty() {
        return apply_explicit_paths(expr, decoded.explicit_paths(), Direction::FromCore);
    }
    match ty {
        TypeRef::Optional(_) => format!("({expr}).map(|value| value.0)"),
        TypeRef::Vec(_) => format!("({expr}).into_iter().map(|value| value.0).collect()"),
        _ if optional => format!("({expr}).map(|value| value.0)"),
        _ => format!("({expr}).0"),
    }
}

pub(crate) fn apply_newtype_from_core(expr: &str, wrapper: &str) -> String {
    apply_field_newtype_from_core(expr, &TypeRef::String, false, wrapper)
}

pub(crate) fn apply_newtype_from_core_after_optionals(expr: &str, wrapper: &str, count: usize) -> String {
    let mut paths = NewtypeWrapper::decode(wrapper)
        .expect("newtype metadata must be validated during extraction")
        .explicit_paths()
        .to_vec();
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
enum Direction<'a> {
    ToCore { map_collection: Option<&'a str> },
    FromCore,
}

fn apply_explicit_paths(expr: &str, paths: &[NewtypeWrapperMetadata], direction: Direction<'_>) -> String {
    let cursors: Vec<_> = paths
        .iter()
        .map(|metadata| ConversionPath {
            metadata,
            containers: &metadata.containers,
        })
        .collect();
    apply_at_container_paths(expr, &cursors, direction)
}

#[derive(Clone, Copy)]
struct ConversionPath<'a> {
    metadata: &'a NewtypeWrapperMetadata,
    containers: &'a [NewtypeContainer],
}

fn apply_at_container_paths(expr: &str, paths: &[ConversionPath<'_>], direction: Direction<'_>) -> String {
    if let [path] = paths
        && path.containers.is_empty()
    {
        return apply_leaf(expr, path.metadata, direction);
    }
    let first = paths
        .first()
        .and_then(|path| path.containers.first())
        .expect("validated conversion paths must not overlap a leaf");
    match first {
        NewtypeContainer::Optional => {
            let nested = advance_uniform_paths(paths, NewtypeContainer::Optional);
            let converted = apply_at_container_paths("value", &nested, direction);
            format!("({expr}).map(|value| {converted})")
        }
        NewtypeContainer::Vec => {
            let nested = advance_uniform_paths(paths, NewtypeContainer::Vec);
            let converted = apply_at_container_paths("value", &nested, direction);
            let collect = collection_suffix(direction, "Vec<_>");
            let typed_expr = if let Some(prefix) = expr.strip_suffix(".collect()") {
                format!("{prefix}.collect::<Vec<_>>()")
            } else {
                expr.to_string()
            };
            format!("({typed_expr}).into_iter().map(|value| {converted}).collect{collect}")
        }
        NewtypeContainer::MapKey | NewtypeContainer::MapValue => {
            let key_paths = advance_matching_paths(paths, NewtypeContainer::MapKey);
            let value_paths = advance_matching_paths(paths, NewtypeContainer::MapValue);
            let nested_direction = match direction {
                Direction::ToCore { .. } => Direction::ToCore { map_collection: None },
                Direction::FromCore => Direction::FromCore,
            };
            let key = if key_paths.is_empty() {
                "key".to_string()
            } else {
                apply_at_container_paths("key", &key_paths, nested_direction)
            };
            let value = if value_paths.is_empty() {
                "value".to_string()
            } else {
                apply_at_container_paths("value", &value_paths, nested_direction)
            };
            let collect = collection_suffix(direction, "std::collections::HashMap<_, _>");
            format!("({expr}).into_iter().map(|(key, value)| ({key}, {value})).collect{collect}")
        }
    }
}

fn advance_uniform_paths<'a>(paths: &[ConversionPath<'a>], expected: NewtypeContainer) -> Vec<ConversionPath<'a>> {
    paths
        .iter()
        .map(|path| {
            let (first, rest) = path.containers.split_first().expect("validated path has a container");
            assert_eq!(*first, expected, "validated conversion paths share a container shape");
            ConversionPath {
                metadata: path.metadata,
                containers: rest,
            }
        })
        .collect()
}

fn advance_matching_paths<'a>(paths: &[ConversionPath<'a>], expected: NewtypeContainer) -> Vec<ConversionPath<'a>> {
    paths
        .iter()
        .filter_map(|path| {
            let (first, rest) = path.containers.split_first()?;
            (*first == expected).then_some(ConversionPath {
                metadata: path.metadata,
                containers: rest,
            })
        })
        .collect()
}

fn apply_leaf(expr: &str, metadata: &NewtypeWrapperMetadata, direction: Direction<'_>) -> String {
    match (&metadata.conversion, direction) {
        (NewtypeConversion::TransparentString { from, .. }, Direction::ToCore { .. }) => {
            format!("{}::{from}({expr})", metadata.rust_path)
        }
        (NewtypeConversion::TransparentString { into, .. }, Direction::FromCore) => {
            format!("({expr}).{into}()")
        }
        (NewtypeConversion::TupleField, Direction::ToCore { .. }) => format!("{}({expr})", metadata.rust_path),
        (NewtypeConversion::TupleField, Direction::FromCore) => format!("({expr}).0"),
    }
}

fn collection_suffix(direction: Direction<'_>, collection_type: &str) -> String {
    match direction {
        Direction::ToCore { map_collection } if collection_type.starts_with("std::collections::HashMap") => {
            map_collection.map_or_else(|| "()".to_string(), |collection| format!("::<{collection}>()"))
        }
        Direction::ToCore { .. } => "()".to_string(),
        Direction::FromCore => format!("::<{collection_type}>()"),
    }
}
