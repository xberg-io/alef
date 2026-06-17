use crate::core::ir::{MethodDef, TypeRef};

pub(super) fn non_opaque_method_result_wrap(method: &MethodDef) -> String {
    match &method.return_type {
        TypeRef::Named(_) | TypeRef::String | TypeRef::Char | TypeRef::Path => ".into()".to_string(),
        TypeRef::Bytes => wrap_bytes(method),
        TypeRef::Optional(inner) => wrap_optional_method_result(method, inner),
        TypeRef::Map(_, _) => wrap_map(method),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => wrap_vec_named(method),
        _ => String::new(),
    }
}

fn wrap_bytes(method: &MethodDef) -> String {
    if method.returns_ref {
        ".to_vec()".to_string()
    } else {
        ".into()".to_string()
    }
}

fn wrap_optional_method_result(method: &MethodDef, inner: &TypeRef) -> String {
    match inner {
        TypeRef::String | TypeRef::Char if method.returns_ref || method.returns_cow => {
            ".map(|v| v.to_owned())".to_string()
        }
        TypeRef::Path => ".map(|v| v.to_string_lossy().to_string())".to_string(),
        TypeRef::Bytes if method.returns_ref => ".map(|v| v.to_vec())".to_string(),
        TypeRef::Named(_) => wrap_optional_named(method),
        TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Named(_)) => wrap_optional_vec_named(method),
        _ => String::new(),
    }
}

fn wrap_map(method: &MethodDef) -> String {
    if method.returns_ref || method.returns_cow {
        ".iter().map(|(k, v)| (k.clone(), v.clone())).collect()".to_string()
    } else {
        String::new()
    }
}

fn wrap_optional_named(method: &MethodDef) -> String {
    if method.returns_ref {
        ".map(|v| v.clone().into())".to_string()
    } else {
        ".map(Into::into)".to_string()
    }
}

fn wrap_vec_named(method: &MethodDef) -> String {
    if method.returns_ref {
        ".iter().map(|v| v.clone().into()).collect()".to_string()
    } else {
        ".into_iter().map(Into::into).collect()".to_string()
    }
}

fn wrap_optional_vec_named(method: &MethodDef) -> String {
    if method.returns_ref {
        ".as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect())".to_string()
    } else {
        ".map(|v| v.into_iter().map(Into::into).collect())".to_string()
    }
}
