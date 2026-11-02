use crate::core::ir::{ParamDef, TypeRef};
use ahash::AHashSet;

pub(super) fn binding(
    param: &ParamDef,
    fallible_named: &AHashSet<String>,
    core_import: &str,
    unwrap_optional: bool,
) -> Option<String> {
    let (core_type, item_conversion) = conversion(&param.ty, "value", fallible_named, core_import)?;
    let name = &param.name;
    let (core_type, expression) = if param.optional {
        let expression = format!("{name}.map(|value| {item_conversion}).transpose()?");
        if unwrap_optional {
            (core_type, format!("{expression}.unwrap_or_default()"))
        } else {
            (format!("Option<{core_type}>"), expression)
        }
    } else {
        let (_, expression) = conversion(&param.ty, name, fallible_named, core_import)?;
        (core_type, format!("{expression}?"))
    };
    let mutability = if param.is_mut { "mut " } else { "" };
    if param.optional && param.is_ref && !unwrap_optional && matches!(param.ty, TypeRef::Named(_)) {
        let borrow = if param.is_mut { "as_mut" } else { "as_ref" };
        Some(format!(
            "let {mutability}{name}_owned: {core_type} = {expression};\n    \
             let {name}_core = {name}_owned.{borrow}();\n    "
        ))
    } else {
        Some(format!(
            "let {mutability}{name}_core: {core_type} = {expression};\n    "
        ))
    }
}

fn conversion(
    ty: &TypeRef,
    value: &str,
    fallible_named: &AHashSet<String>,
    core_import: &str,
) -> Option<(String, String)> {
    match ty {
        TypeRef::Named(name) if fallible_named.contains(name) => {
            Some((format!("{core_import}::{name}"), format!("{value}.try_into_core()")))
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            let (core_type, item) = conversion(inner, "item", fallible_named, core_import)?;
            Some((
                format!("Vec<{core_type}>"),
                format!("{value}.into_iter().map(|item| {item}).collect::<napi::Result<Vec<_>>>()"),
            ))
        }
        _ => None,
    }
}
