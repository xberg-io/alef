use crate::core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeRef};

pub(super) fn enum_variant_field_conv_to_core(binding: &str, field: &FieldDef) -> String {
    if field.sanitized {
        return "Default::default()".to_string();
    }
    if let Some(converted) = super::newtype_conversions::enum_to_core(binding, field) {
        return converted;
    }
    if let Some((inner, nested)) = optional_inner(field) {
        let converted = enum_variant_field_conv_to_core("value", &inner);
        let mapped = format!("{binding}.map(|value| {converted})");
        return if nested { format!("{mapped}.map(Some)") } else { mapped };
    }
    match &field.ty {
        TypeRef::Named(_) => match field.core_wrapper {
            CoreWrapper::Arc | CoreWrapper::ArcMutex => format!("std::sync::Arc::new({binding}.into())"),
            _ if field.is_boxed => format!("Box::new({binding}.into())"),
            _ => format!("{binding}.into()"),
        },
        TypeRef::String if matches!(field.core_wrapper, CoreWrapper::Cow) => format!("{binding}.into()"),
        TypeRef::String => binding.to_string(),
        TypeRef::Char => format!("{binding}.chars().next().unwrap_or_default()"),
        TypeRef::Path => format!("std::path::PathBuf::from({binding})"),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("{binding}.into_iter().map(Into::into).collect()"),
            TypeRef::String => binding.to_string(),
            _ => format!("{binding}.into_iter().map(|x| x as _).collect::<Vec<_>>()"),
        },
        TypeRef::Primitive(PrimitiveType::Bool) => binding.to_string(),
        TypeRef::Primitive(_) => match &field.newtype_wrapper {
            Some(wrapper) => format!("{wrapper}({binding} as _)"),
            None => format!("{binding} as _"),
        },
        _ => format!("{binding}.into()"),
    }
}

pub(super) fn enum_variant_field_conv(binding: &str, field: &FieldDef, source_crate_name: &str) -> String {
    let _ = source_crate_name;
    if let Some(converted) = super::newtype_conversions::enum_from_core(binding, field) {
        return converted;
    }
    if let Some((inner, nested)) = optional_inner(field) {
        let converted = enum_variant_field_conv("value", &inner, source_crate_name);
        let flatten = if nested { ".flatten()" } else { "" };
        return format!("{binding}{flatten}.map(|value| {converted})");
    }
    if field.sanitized {
        return sanitized_field_expression(binding, &field.ty);
    }
    match &field.ty {
        TypeRef::Named(name) if field.is_boxed => format!("{name}::from(*{binding})"),
        TypeRef::Named(name) => format!("{name}::from({binding})"),
        TypeRef::Vec(inner) => vector_field_expression(binding, inner),
        TypeRef::String if matches!(field.core_wrapper, CoreWrapper::Cow) => format!("{binding}.into()"),
        TypeRef::String => binding.to_string(),
        TypeRef::Char => format!("{binding}.to_string()"),
        TypeRef::Path => format!("{binding}.to_string_lossy().into_owned()"),
        TypeRef::Json => format!("serde_json::to_string(&{binding}).unwrap_or_default()"),
        TypeRef::Primitive(_) if field.newtype_wrapper.is_some() => format!("{binding}.0 as _"),
        TypeRef::Primitive(_) => format!("{binding} as _"),
        TypeRef::Map(_, value) if matches!(value.as_ref(), TypeRef::Json | TypeRef::Named(_)) => format!(
            "{binding}.into_iter().map(|(k, v)| (k.into(), serde_json::to_string(&v).unwrap_or_default())).collect()"
        ),
        TypeRef::Map(_, _) => format!("{binding}.into_iter().map(|(k, v)| (k.into(), v.into())).collect()"),
        _ => binding.to_string(),
    }
}

fn vector_field_expression(binding: &str, inner: &TypeRef) -> String {
    match inner {
        TypeRef::Named(name) => format!("{binding}.into_iter().map({name}::from).collect()"),
        TypeRef::Primitive(_) => format!("{binding}.into_iter().map(|x| x as _).collect()"),
        TypeRef::String => binding.to_string(),
        _ => format!("{binding}.into_iter().map(|s| s.into()).collect()"),
    }
}

fn sanitized_field_expression(binding: &str, ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(_) => format!("{binding} as _"),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Vec(nested) if matches!(nested.as_ref(), TypeRef::String)) =>
        {
            format!("{binding}.into_iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect()")
        }
        TypeRef::Vec(_) => {
            format!("{binding}.into_iter().map(|e| serde_json::to_string(&e).unwrap_or_default()).collect()")
        }
        _ => format!("serde_json::to_string(&{binding}).unwrap_or_default()"),
    }
}

fn optional_inner(field: &FieldDef) -> Option<(FieldDef, bool)> {
    let (ty, nested) = match &field.ty {
        TypeRef::Optional(inner) => (*inner.clone(), field.optional),
        _ if field.optional => (field.ty.clone(), false),
        _ => return None,
    };
    Some((
        FieldDef {
            ty,
            optional: false,
            ..field.clone()
        },
        nested,
    ))
}
