use crate::codegen::conversions::helpers;
use crate::core::ir::{
    FunctionDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, ParamDef, TypeRef,
};

pub(super) fn validate(function: &FunctionDef) -> anyhow::Result<()> {
    if let Some(param) = function
        .params
        .iter()
        .find(|p| p.is_ref && p.is_mut && p.newtype_wrapper.is_some())
    {
        anyhow::bail!(
            "Dart function `{}` has a mutable newtype reference `{}` requiring writeback; take and return the updated value by move",
            function.name,
            param.name
        );
    }
    if function.returns_ref
        && let Some(wrapper) = &function.return_newtype_wrapper
    {
        for path in paths(wrapper) {
            if matches!(
                path.conversion,
                NewtypeConversion::TransparentString {
                    wrapper_is_clone: None | Some(false),
                    ..
                }
            ) {
                anyhow::bail!(
                    "Dart function `{}` returns a borrowed transparent newtype without a proven Clone implementation",
                    function.name
                );
            }
        }
    }
    Ok(())
}

pub(super) fn argument(param: &ParamDef, checked: bool) -> String {
    let mut ty = param.ty.clone();
    if param.optional && !matches!(ty, TypeRef::Optional(_)) {
        ty = TypeRef::Optional(Box::new(ty));
    }
    let wrapper = param
        .newtype_wrapper
        .as_deref()
        .expect("caller checked newtype metadata");
    let collection = param.map_is_btree.then_some("std::collections::BTreeMap<_, _>");
    let wrapped = rewrite(&param.name, &ty, &paths(wrapper), Mode::Argument(param), collection);
    let wrapped = if checked {
        let checks = unsigned_checks(&format!("&{}", param.name), &ty, &param.name);
        format!("{{ {checks} {wrapped} }}")
    } else {
        wrapped
    };
    match (param.is_ref, param.optional) {
        (true, true) => format!("({wrapped}).as_ref()"),
        (true, false) => format!("&({wrapped})"),
        _ => wrapped,
    }
}

pub(super) fn result(expr: &str, ty: &TypeRef, wrapper: &str, borrowed: bool) -> String {
    let unwrapped = if borrowed {
        rewrite(expr, ty, &paths(wrapper), Mode::Borrowed, None)
    } else {
        helpers::apply_field_newtype_from_core(expr, ty, false, wrapper)
    };
    let converted = super::super::newtype_conversions::from_core_value("__alef_unwrapped", ty);
    format!("{{ let __alef_unwrapped = {unwrapped}; {converted} }}")
}

fn paths(wrapper: &str) -> Vec<NewtypeWrapperMetadata> {
    match NewtypeWrapper::decode(wrapper).expect("validated newtype metadata") {
        NewtypeWrapper::Tuple(rust_path) => vec![NewtypeWrapperMetadata {
            rust_path,
            conversion: NewtypeConversion::TupleField,
            containers: vec![],
        }],
        NewtypeWrapper::Explicit(paths) => paths,
    }
}

#[derive(Clone, Copy)]
enum Mode<'a> {
    Argument(&'a ParamDef),
    Borrowed,
}

fn descend(paths: &[NewtypeWrapperMetadata], container: NewtypeContainer) -> Vec<NewtypeWrapperMetadata> {
    paths
        .iter()
        .filter_map(|path| {
            let (head, tail) = path.containers.split_first()?;
            (*head == container).then(|| NewtypeWrapperMetadata {
                containers: tail.to_vec(),
                ..path.clone()
            })
        })
        .collect()
}

fn rewrite(
    expr: &str,
    ty: &TypeRef,
    paths: &[NewtypeWrapperMetadata],
    mode: Mode<'_>,
    collection: Option<&str>,
) -> String {
    if let Some(path) = paths.iter().find(|path| path.containers.is_empty()) {
        return leaf(expr, ty, Some(path), mode);
    }
    match ty {
        TypeRef::Optional(inner) => {
            let value = rewrite("value", inner, &descend(paths, NewtypeContainer::Optional), mode, None);
            let borrow = if matches!(mode, Mode::Borrowed) {
                ".as_ref()"
            } else {
                ""
            };
            format!("({expr}){borrow}.map(|value| {value})")
        }
        TypeRef::Vec(inner) => {
            let value = rewrite("value", inner, &descend(paths, NewtypeContainer::Vec), mode, None);
            let iter = if matches!(mode, Mode::Borrowed) {
                "iter"
            } else {
                "into_iter"
            };
            format!("({expr}).{iter}().map(|value| {value}).collect::<Vec<_>>()")
        }
        TypeRef::Map(key, value) => {
            let key = rewrite("key", key, &descend(paths, NewtypeContainer::MapKey), mode, None);
            let value = rewrite("value", value, &descend(paths, NewtypeContainer::MapValue), mode, None);
            let iter = if matches!(mode, Mode::Borrowed) {
                "iter"
            } else {
                "into_iter"
            };
            let collection = collection.unwrap_or("std::collections::HashMap<_, _>");
            format!("({expr}).{iter}().map(|(key, value)| ({key}, {value})).collect::<{collection}>()")
        }
        _ => leaf(expr, ty, None, mode),
    }
}

fn leaf(expr: &str, ty: &TypeRef, path: Option<&NewtypeWrapperMetadata>, mode: Mode<'_>) -> String {
    match mode {
        Mode::Borrowed => match path.map(|path| &path.conversion) {
            Some(NewtypeConversion::TupleField) => format!("({expr}).0.to_owned()"),
            Some(NewtypeConversion::TransparentString { into, .. }) => {
                let path = &path.expect("matched metadata").rust_path;
                format!("<{path} as Clone>::clone({expr}).{into}()")
            }
            None => format!("({expr}).to_owned()"),
        },
        Mode::Argument(param) => {
            let mut primitive = param.clone();
            primitive.name = expr.to_string();
            primitive.ty = ty.clone();
            primitive.optional = false;
            primitive.is_ref = false;
            primitive.is_mut = false;
            primitive.newtype_wrapper = None;
            let value = if matches!(ty, TypeRef::Primitive(_)) {
                super::super::conversions::dart_call_arg_checked(&primitive, false)
            } else {
                super::super::newtype_conversions::to_core_value(expr, ty)
            };
            path.map_or(value.clone(), |path| {
                helpers::apply_field_newtype_to_core(
                    &value,
                    ty,
                    false,
                    &NewtypeWrapper::encode_explicit(std::slice::from_ref(path)),
                )
            })
        }
    }
}

fn unsigned_checks(expr: &str, ty: &TypeRef, parameter: &str) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::Primitive(
            p @ (PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::Usize),
        ) => {
            let target = super::super::conversions::primitive_name(p);
            format!(
                "let _ = {target}::try_from(*({expr})).map_err(|_| \"parameter '{parameter}' must be between 0 and {target}::MAX\".to_string())?;"
            )
        }
        TypeRef::Optional(inner) => {
            let check = unsigned_checks("__alef_value", inner, parameter);
            format!("if let Some(__alef_value) = ({expr}).as_ref() {{ {check} }}")
        }
        TypeRef::Vec(inner) => {
            let check = unsigned_checks("__alef_value", inner, parameter);
            format!("for __alef_value in ({expr}).iter() {{ {check} }}")
        }
        TypeRef::Map(key, value) => {
            let key = unsigned_checks("__alef_key", key, parameter);
            let value = unsigned_checks("__alef_value", value, parameter);
            format!("for (__alef_key, __alef_value) in ({expr}).iter() {{ {key} {value} }}")
        }
        _ => String::new(),
    }
}
