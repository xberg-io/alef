use crate::codegen::conversions::helpers;
use crate::core::ir::{ParamDef, TypeRef};

pub(super) fn argument(param: &ParamDef, inner: &str) -> String {
    let wrapped = helpers::apply_param_newtype_to_core(inner, param).expect("caller checked newtype metadata");
    let wrapped = if matches!(param.ty, TypeRef::Vec(_)) && !param.optional {
        helpers::type_intermediate_vec_collect(&wrapped)
    } else {
        wrapped
    };
    match (param.is_ref || param.is_mut, param.optional, param.is_mut) {
        (true, true, true) => format!("({wrapped}).as_mut()"),
        (true, true, false) => format!("({wrapped}).as_ref()"),
        (true, false, true) => format!("&mut ({wrapped})"),
        (true, false, false) => format!("&({wrapped})"),
        _ => wrapped,
    }
}

pub(super) fn result(expr: &str, ty: &TypeRef, wrapper: &str) -> String {
    let unwrapped = helpers::apply_field_newtype_from_core(expr, ty, false, wrapper);
    let core_type = helpers::core_container_type(ty, None);
    let converted = super::super::newtype_conversions::from_core_value("__alef_unwrapped", ty);
    format!("{{ let __alef_unwrapped: {core_type} = {unwrapped}; {converted} }}")
}
