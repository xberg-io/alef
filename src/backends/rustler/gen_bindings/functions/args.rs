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
            // Default-typed Named params are passed as Option<String> JSON and decoded
            // by the caller into a `{name}_core` local. Reference that local here.
            TypeRef::Named(name) if default_types.contains(name.as_str()) => {
                if p.optional {
                    format!("{}_core", p.name)
                } else if p.is_ref && p.is_mut {
                    // Core expects &mut T → reference the mutable binding created by preamble
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
            // Optional<String> where core expects Option<Cow<'_, str>>: wrap owned values
            // via Cow::Owned. Without this the binding's Option<String> doesn't satisfy
            // the core's Option<Cow<'_, str>> parameter signature.
            TypeRef::String | TypeRef::Char if p.optional && p.core_wrapper == CoreWrapper::Cow => {
                format!("{}.map(std::borrow::Cow::Owned)", p.name)
            }
            TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
            TypeRef::String | TypeRef::Char if p.is_ref && p.is_mut => format!("&mut {}", p.name),
            TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
            // String where core expects Cow<'_, str>: String implements Into<Cow<str>>,
            // so `.into()` performs the coercion.
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
            // Json params: String (from NIF) is converted to serde_json::Value in the preamble.
            // The caller builds a `{name}_json` local via JSON deserialization preamble.
            TypeRef::Json => {
                if p.optional {
                    // Option<serde_json::Value> for optional params
                    format!("{}_json", p.name)
                } else if p.is_ref && p.is_mut {
                    // &mut serde_json::Value for mutable references
                    format!("&mut {}_json", p.name)
                } else if p.is_ref {
                    // &serde_json::Value for references
                    format!("&{}_json", p.name)
                } else {
                    // serde_json::Value for owned params
                    format!("{}_json", p.name)
                }
            }
            TypeRef::Vec(_) => {
                if p.is_ref && p.is_mut {
                    // `&mut Vec<T>` derefs to `&mut [T]`. When the preamble creates a mutable
                    // binding (e.g., `let mut handles_mut = ...`), pass the mutable reference.
                    format!("&mut {}_mut", p.name)
                } else if p.is_ref {
                    // `&Vec<T>` derefs to `&[T]`, which matches sample_core core for `&[String]`.
                    // For `&[&str]` signatures (Vec<String> inner), a refs intermediate is
                    // emitted in the caller body (gen_nif_function deser_lines) instead.
                    format!("&{}", p.name)
                } else {
                    p.name.to_string()
                }
            }
            // Map: when the core fn expects BTreeMap but the binding receives a
            // HashMap (Rustler decodes BEAM maps as HashMap), collect into a BTreeMap.
            TypeRef::Map(_, _) if p.map_is_btree => {
                if p.is_ref {
                    // Pre-bound let binding emitted by the caller body would be needed for
                    // borrows; we emit the inline collect for the owned case below. For the
                    // method-receiver path, refs to maps are rare; fall through to owned-collect.
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
