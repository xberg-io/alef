use crate::core::ir::{CoreWrapper, ParamDef, TypeRef};
use ahash::AHashSet;

/// Build call argument expressions for Rustler opaque method (receiver is `resource`).
pub(super) fn gen_rustler_method_call_args(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                format!("&{}.inner.read().unwrap_or_else(|e| e.into_inner()).clone()", p.name)
            }
            TypeRef::Named(name) if default_types.contains(name.as_str()) => {
                if p.optional {
                    format!("{}_core", p.name)
                } else if p.is_ref && p.is_mut {
                    format!("&mut {}_mut", p.name)
                } else if p.is_ref {
                    format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name)
                } else {
                    format!("{}_core.unwrap_or_default()", p.name)
                }
            }
            TypeRef::Named(_) => {
                if p.optional {
                    if p.is_ref {
                        format!("{}.as_ref().map(Into::into)", p.name)
                    } else {
                        format!("{}.map(Into::into)", p.name)
                    }
                } else if p.is_ref {
                    format!("&{}.clone().into()", p.name)
                } else {
                    format!("{}.into()", p.name)
                }
            }
            TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                format!("{}.as_deref()", p.name)
            }
            TypeRef::String | TypeRef::Char if p.optional && p.core_wrapper == CoreWrapper::Cow => {
                format!("{}.map(std::borrow::Cow::Owned)", p.name)
            }
            TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
            TypeRef::String | TypeRef::Char if p.is_ref && p.is_mut => format!("&mut {}", p.name),
            TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
            TypeRef::String | TypeRef::Char if p.core_wrapper == CoreWrapper::Cow => {
                format!("{}.into()", p.name)
            }
            TypeRef::String | TypeRef::Char => p.name.clone(),
            TypeRef::Path => {
                if p.is_ref && p.is_mut {
                    format!("&mut std::path::PathBuf::from({})", p.name)
                } else if p.is_ref {
                    format!("&std::path::PathBuf::from({})", p.name)
                } else {
                    format!("std::path::PathBuf::from({})", p.name)
                }
            }
            TypeRef::Bytes => {
                if p.is_ref {
                    format!("{}.as_slice()", p.name)
                } else {
                    format!("{}.as_slice().to_vec()", p.name)
                }
            }
            TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
            TypeRef::Json => {
                if p.optional {
                    format!("{}_json", p.name)
                } else if p.is_ref && p.is_mut {
                    format!("&mut {}_json", p.name)
                } else if p.is_ref {
                    format!("&{}_json", p.name)
                } else {
                    format!("{}_json", p.name)
                }
            }
            TypeRef::Vec(_) => {
                if p.is_ref && p.is_mut {
                    format!("&mut {}_mut", p.name)
                } else if p.is_ref {
                    format!("&{}", p.name)
                } else {
                    p.name.to_string()
                }
            }
            TypeRef::Map(_, _) if p.map_is_btree => {
                if p.is_ref {
                    format!("{}.into_iter().collect::<std::collections::BTreeMap<_, _>>()", p.name)
                } else {
                    format!("{}.into_iter().collect::<std::collections::BTreeMap<_, _>>()", p.name)
                }
            }
            _ => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
