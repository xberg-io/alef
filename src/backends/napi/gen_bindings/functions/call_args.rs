use crate::core::ir::{CoreWrapper, ParamDef, TypeRef};
use ahash::AHashSet;

/// Split a comma-joined call-argument list on top-level commas only, ignoring commas nested inside
/// angle brackets (`<...>`), parentheses or square brackets. A naive `split(',')` would break an
/// argument such as `x.into_iter().collect::<BTreeMap<_, _>>()` into pieces at the inner comma.
fn split_top_level_commas(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (idx, ch) in args.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(args[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = args[start..].trim();
    if !tail.is_empty() || !out.is_empty() {
        out.push(tail);
    }
    out
}

pub(in crate::backends::napi::gen_bindings) fn napi_apply_primitive_casts_to_call_args(
    generic_args: &str,
    params: &[ParamDef],
) -> String {
    let args_list: Vec<&str> = split_top_level_commas(generic_args);
    args_list
        .iter()
        .zip(params.iter())
        .map(|(arg, p)| {
            if needs_vec_f32_conversion(&p.ty) && p.is_ref {
                return format!("&{}_f32", p.name);
            }
            let arg_is_bare_name = *arg == p.name;
            match &p.ty {
                TypeRef::Primitive(prim) if needs_napi_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        if arg.contains(".map(") || arg.contains(".as_") {
                            arg.to_string()
                        } else {
                            format!("{}.map(|v| v as {})", arg, core_ty)
                        }
                    } else {
                        format!("{} as {}", arg, core_ty)
                    }
                }
                TypeRef::String | TypeRef::Char
                    if arg_is_bare_name && !p.is_ref && p.core_wrapper == CoreWrapper::Cow =>
                {
                    if p.optional {
                        format!("{}.map(std::borrow::Cow::Owned)", arg)
                    } else {
                        format!("{}.into()", arg)
                    }
                }
                _ => arg.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate let bindings for Vec<f32> parameters that need f64→f32 conversion.
pub(in crate::backends::napi::gen_bindings) fn napi_gen_call_args(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| {
            if needs_vec_f32_conversion(&p.ty) && p.is_ref {
                return format!("&{}_f32", p.name);
            }
            match &p.ty {
                TypeRef::Primitive(prim) if needs_napi_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        format!("{}.map(|v| v as {})", p.name, core_ty)
                    } else {
                        format!("{} as {}", p.name, core_ty)
                    }
                }
                TypeRef::Duration => {
                    if p.optional {
                        format!("{}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))", p.name)
                    } else {
                        format!("std::time::Duration::from_millis({}.max(0) as u64)", p.name)
                    }
                }
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    if p.is_ref {
                        if p.optional {
                            format!("{}.as_ref().map(|v| v.inner.as_ref())", p.name)
                        } else {
                            format!("{}.inner.as_ref()", p.name)
                        }
                    } else if p.optional {
                        format!("{}.as_ref().map(|v| (*v.inner).clone())", p.name)
                    } else {
                        format!("(*{}.inner).clone()", p.name)
                    }
                }
                TypeRef::Named(_) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else {
                            format!("{}.map(Into::into)", p.name)
                        }
                    } else {
                        format!("{}.into()", p.name)
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else if p.core_wrapper == CoreWrapper::Cow {
                            format!("{}.map(std::borrow::Cow::Owned)", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{}.into()", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Path => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref().map(std::path::Path::new)", p.name)
                        } else {
                            format!("{}.map(std::path::PathBuf::from)", p.name)
                        }
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{})", p.name)
                    } else {
                        format!("std::path::PathBuf::from({})", p.name)
                    }
                }
                TypeRef::Bytes => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Vec(inner) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_deref()", p.name)
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref
                        && p.vec_inner_is_ref
                        && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                    {
                        format!("&{}_refs", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::Map(_, _) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{}.as_ref()", p.name)
                        } else if p.map_is_btree {
                            format!(
                                "{}.map(|m| m.into_iter().collect::<std::collections::BTreeMap<_, _>>())",
                                p.name
                            )
                        } else {
                            p.name.clone()
                        }
                    } else if p.is_ref && p.map_is_btree {
                        format!("&{}.into_iter().collect::<std::collections::BTreeMap<_, _>>()", p.name)
                    } else if p.is_ref {
                        format!("&{}", p.name)
                    } else if p.map_is_btree {
                        format!("{}.into_iter().collect::<std::collections::BTreeMap<_, _>>()", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn needs_vec_f32_conversion(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::F32)))
}

pub(in crate::backends::napi::gen_bindings) fn needs_napi_cast(p: &crate::core::ir::PrimitiveType) -> bool {
    matches!(
        p,
        crate::core::ir::PrimitiveType::U64
            | crate::core::ir::PrimitiveType::Usize
            | crate::core::ir::PrimitiveType::Isize
            | crate::core::ir::PrimitiveType::F32
    )
}

pub(in crate::backends::napi::gen_bindings) fn core_prim_str(p: &crate::core::ir::PrimitiveType) -> &'static str {
    match p {
        crate::core::ir::PrimitiveType::U64 => "u64",
        crate::core::ir::PrimitiveType::Usize => "usize",
        crate::core::ir::PrimitiveType::Isize => "isize",
        crate::core::ir::PrimitiveType::F32 => "f32",
        _ => unreachable!(),
    }
}

/// Check if a type is Vec<u8> or Bytes (which becomes napi::Buffer).
pub(super) fn is_bytes_param(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Bytes)
        || matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)))
}

#[cfg(test)]
mod tests {
    use super::split_top_level_commas;

    #[test]
    fn split_ignores_commas_nested_in_angle_brackets() {
        let args = "&preset_core, custom_schema, &context.into_iter().collect::<std::collections::BTreeMap<_, _>>()";
        assert_eq!(
            split_top_level_commas(args),
            vec![
                "&preset_core",
                "custom_schema",
                "&context.into_iter().collect::<std::collections::BTreeMap<_, _>>()",
            ],
        );
    }

    #[test]
    fn split_handles_simple_and_empty_arglists() {
        assert_eq!(split_top_level_commas("a, b, c"), vec!["a", "b", "c"]);
        assert!(split_top_level_commas("").is_empty());
    }
}
