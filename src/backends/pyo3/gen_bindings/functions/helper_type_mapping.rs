use crate::backends::pyo3::gen_bindings::enums::Wrapping;

pub(in crate::backends::pyo3::gen_bindings) fn classify_param_type(
    ty: &crate::core::ir::TypeRef,
) -> Option<(&str, Wrapping)> {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) => Some((n.as_str(), Wrapping::Plain)),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some((n.as_str(), Wrapping::Optional)),
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) => Some((n.as_str(), Wrapping::OptionalVec)),
                _ => None,
            },
            _ => None,
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some((n.as_str(), Wrapping::Vec)),
            _ => None,
        },
        _ => None,
    }
}
