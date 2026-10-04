use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, TypeRef};

pub(super) fn from_core(expr: &str, field: &FieldDef) -> Option<String> {
    convert_field(expr, field, Direction::FromCore)
}

pub(super) fn to_core(expr: &str, field: &FieldDef) -> Option<String> {
    convert_field(expr, field, Direction::ToCore)
}

fn convert_field(expr: &str, field: &FieldDef, direction: Direction) -> Option<String> {
    let wrapper = field.newtype_wrapper.as_deref()?;
    let decoded = NewtypeWrapper::decode(wrapper).expect("newtype metadata must be validated before codegen");
    let paths = decoded.explicit_paths();
    if paths.is_empty() {
        return None;
    }
    if crate::codegen::conversions::helpers::explicit_newtype_covers_non_identity_leaves(
        &field.ty,
        field.optional,
        wrapper,
    ) {
        return match direction {
            Direction::FromCore => {
                crate::codegen::conversions::helpers::apply_explicit_field_newtype_from_core(expr, field)
            }
            Direction::ToCore => {
                crate::codegen::conversions::helpers::apply_explicit_field_newtype_to_core(expr, field)
            }
        };
    }
    if field.optional {
        let nested = advance_matching(paths, NewtypeContainer::Optional);
        let converted = convert("value", &field.ty, &nested, direction);
        Some(format!("({expr}).map(|value| {converted})"))
    } else {
        Some(convert(expr, &field.ty, paths, direction))
    }
}

#[derive(Clone, Copy)]
enum Direction {
    FromCore,
    ToCore,
}

fn convert(expr: &str, ty: &TypeRef, paths: &[NewtypeWrapperMetadata], direction: Direction) -> String {
    if paths.is_empty() {
        return fallback(expr, ty, direction);
    }
    if let [path] = paths
        && path.containers.is_empty()
    {
        return convert_wrapper(expr, path, direction);
    }
    match ty {
        TypeRef::Optional(inner) => {
            let nested = advance_matching(paths, NewtypeContainer::Optional);
            let converted = convert("value", inner, &nested, direction);
            format!("({expr}).map(|value| {converted})")
        }
        TypeRef::Vec(inner) => {
            let nested = advance_matching(paths, NewtypeContainer::Vec);
            let converted = convert("value", inner, &nested, direction);
            format!("({expr}).into_iter().map(|value| {converted}).collect::<Vec<_>>()")
        }
        TypeRef::Map(key, value) => {
            let key_paths = advance_matching(paths, NewtypeContainer::MapKey);
            let value_paths = advance_matching(paths, NewtypeContainer::MapValue);
            let key = convert("key", key, &key_paths, direction);
            let value = convert("value", value, &value_paths, direction);
            format!("({expr}).into_iter().map(|(key, value)| ({key}, {value})).collect()")
        }
        _ => unreachable!("validated explicit newtype paths must end at their wrapped leaf"),
    }
}

fn advance_matching(paths: &[NewtypeWrapperMetadata], expected: NewtypeContainer) -> Vec<NewtypeWrapperMetadata> {
    paths
        .iter()
        .filter_map(|path| {
            let (first, rest) = path.containers.split_first()?;
            (*first == expected).then(|| NewtypeWrapperMetadata {
                rust_path: path.rust_path.clone(),
                conversion: path.conversion.clone(),
                containers: rest.to_vec(),
            })
        })
        .collect()
}

fn convert_wrapper(expr: &str, metadata: &NewtypeWrapperMetadata, direction: Direction) -> String {
    match (&metadata.conversion, direction) {
        (NewtypeConversion::TransparentString { from, .. }, Direction::ToCore) => {
            format!("{}::{from}({expr})", metadata.rust_path)
        }
        (NewtypeConversion::TransparentString { into, .. }, Direction::FromCore) => {
            format!("({expr}).{into}()")
        }
        (NewtypeConversion::TupleField, Direction::ToCore) => format!("{}({expr})", metadata.rust_path),
        (NewtypeConversion::TupleField, Direction::FromCore) => format!("({expr}).0"),
    }
}

fn fallback(expr: &str, ty: &TypeRef, direction: Direction) -> String {
    match (direction, ty) {
        (_, TypeRef::Optional(inner)) => {
            let converted = fallback("value", inner, direction);
            format!("({expr}).map(|value| {converted})")
        }
        (_, TypeRef::Vec(inner)) => {
            let converted = fallback("value", inner, direction);
            format!("({expr}).into_iter().map(|value| {converted}).collect::<Vec<_>>()")
        }
        (_, TypeRef::Map(key, value)) => {
            let key = fallback("key", key, direction);
            let value = fallback("value", value, direction);
            format!("({expr}).into_iter().map(|(key, value)| ({key}, {value})).collect()")
        }
        (Direction::FromCore, TypeRef::Named(name)) => format!("{name}::from({expr})"),
        (Direction::ToCore, TypeRef::Named(_)) => format!("({expr}).into()"),
        (Direction::FromCore, TypeRef::Json) => format!("serde_json::to_string(&({expr})).unwrap_or_default()"),
        (Direction::ToCore, TypeRef::Json) => format!("serde_json::from_str(&({expr})).unwrap_or_default()"),
        (Direction::FromCore, TypeRef::Char) => format!("({expr}).to_string()"),
        (Direction::ToCore, TypeRef::Char) => format!("({expr}).chars().next().unwrap_or_default()"),
        (Direction::FromCore, TypeRef::Path) => format!("({expr}).to_string_lossy().into_owned()"),
        (Direction::ToCore, TypeRef::Path) => format!("std::path::PathBuf::from({expr})"),
        (Direction::FromCore, TypeRef::Duration) => format!("({expr}).as_millis() as i64"),
        (Direction::ToCore, TypeRef::Duration) => format!("std::time::Duration::from_millis(({expr}) as u64)"),
        (_, TypeRef::Primitive(_)) => format!("({expr}) as _"),
        (_, TypeRef::Bytes) => format!("({expr}).into()"),
        (_, TypeRef::String) => expr.to_string(),
        (_, TypeRef::Unit) => "()".to_string(),
    }
}
