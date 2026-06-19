use crate::backends::php::type_map::PhpMapper;
use crate::codegen::naming::to_php_name;
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::TypeRef;
use ahash::AHashSet;
use minijinja::context;

use super::primitives::{core_prim_str, needs_i64_cast};

/// Return true if any field of the type (recursively through Optional/Vec) is a Named type
/// that is an enum. PHP maps enum Named types to String, so From/Into impls would need
/// From<String> for the core enum which doesn't exist -- skip generation for such types.
/// Check if a TypeRef references any type in the given set (transitively through containers).
pub(crate) fn references_named_type(ty: &crate::core::ir::TypeRef, names: &AHashSet<String>) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => names.contains(name.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => references_named_type(inner, names),
        TypeRef::Map(k, v) => references_named_type(k, names) || references_named_type(v, names),
        _ => false,
    }
}

/// True when a param's binding→core conversion can fail (emits `return Err(...)`), which forces
/// the enclosing function to return `PhpResult<T>` even if the core function is infallible.
///
/// This is the case for `Vec<StructType>` params: the PHP array is decoded element-by-element via
/// `FromZval`, and a non-convertible element triggers `return Err(...)` (see
/// `php_vec_named_struct_let_binding.jinja`). `Vec<enum>` (string round-trip) and `Vec<opaque>` do
/// not fail this way.
pub(crate) fn param_conversion_is_fallible(
    p: &crate::core::ir::ParamDef,
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
) -> bool {
    use crate::core::ir::TypeRef;
    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Named(name) = inner.as_ref() {
            return !opaque_types.contains(name.as_str()) && !enum_names.contains(name.as_str());
        }
    }
    false
}

pub(crate) fn has_enum_named_field(typ: &crate::core::ir::TypeDef, enum_names: &AHashSet<String>) -> bool {
    fn type_ref_has_enum_named(ty: &crate::core::ir::TypeRef, enum_names: &AHashSet<String>) -> bool {
        use crate::core::ir::TypeRef;
        match ty {
            TypeRef::Named(name) => enum_names.contains(name.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_has_enum_named(inner, enum_names),
            TypeRef::Map(k, v) => type_ref_has_enum_named(k, enum_names) || type_ref_has_enum_named(v, enum_names),
            _ => false,
        }
    }
    binding_fields(&typ.fields).any(|f| type_ref_has_enum_named(&f.ty, enum_names))
}

/// Generate PHP-specific function parameter list.
/// Non-opaque Named types use `&T` (ext-php-rs only provides `FromZvalMut` for `&mut T`/`&T`,
/// not owned `T`, when `T` is a `#[php_class]`).
/// Vec<NonOpaqueCustomType> also needs &Vec<T> since the elements are php_class types.
/// Bridge type aliases (like VisitorHandle) are mapped to raw PHP object types `&mut ZendObject`.
pub(crate) fn gen_php_function_params(
    params: &[crate::core::ir::ParamDef],
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    bridge_type_aliases: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| {
            let base_ty = mapper.map_type(&p.ty);
            let ty = match &p.ty {
                TypeRef::Named(name) => {
                    // Bridge type aliases: map to &mut ZendObject for direct PHP object access
                    if bridge_type_aliases.contains(name.as_str()) {
                        if p.optional {
                            "Option<&mut ext_php_rs::types::ZendObject>".to_string()
                        } else {
                            "&mut ext_php_rs::types::ZendObject".to_string()
                        }
                    } else if mapper.enum_names.contains(name.as_str()) {
                        // Enum types are mapped to String in PHP — use owned String, not &String.
                        // Only php_class struct types need &T (ext-php-rs only provides
                        // FromZvalMut for &T/&mut T, not owned T, for php_class types).
                        if p.optional {
                            format!("Option<{base_ty}>")
                        } else {
                            base_ty
                        }
                    } else if p.optional {
                        format!("Option<&{base_ty}>")
                    } else {
                        format!("&{base_ty}")
                    }
                }
                TypeRef::Vec(inner) => {
                    // Vec<NonOpaqueCustomType>: ext-php-rs cannot implement FromZvalMut for
                    // Vec<T> when T is a #[php_class] type. Use &ZendHashTable instead and
                    // convert element-by-element in a let binding via php_vec_named_struct_let_binding.
                    if let TypeRef::Named(name) = inner.as_ref() {
                        if !opaque_types.contains(name.as_str()) && !mapper.enum_names.contains(name.as_str()) {
                            if p.optional {
                                "Option<&ext_php_rs::types::ZendHashTable>".to_string()
                            } else {
                                "&ext_php_rs::types::ZendHashTable".to_string()
                            }
                        } else {
                            // Opaque or enum named type inside Vec: use owned Vec.
                            if p.optional {
                                format!("Option<{base_ty}>")
                            } else {
                                base_ty
                            }
                        }
                    } else {
                        // Primitive types inside Vec: use owned Vec.
                        if p.optional {
                            format!("Option<{base_ty}>")
                        } else {
                            base_ty
                        }
                    }
                }
                TypeRef::Bytes => {
                    // PhpBytes is the local wrapper that accepts PHP binary strings without
                    // UTF-8 validation (ext-php-rs's String FromZval rejects non-UTF-8 bytes).
                    if p.optional {
                        "Option<PhpBytes>".to_string()
                    } else {
                        "PhpBytes".to_string()
                    }
                }
                TypeRef::Map(_, _) if p.map_is_ahash && p.map_key_is_cow => {
                    // AHashMap<Cow<'static, str>, Value> params: PHP cannot provide FromZvalMut
                    // for this type directly. Receive as HashMap<String, String> (PHP hash with
                    // string values) and convert in the pre-call binding in the function body.
                    if p.optional {
                        "Option<std::collections::HashMap<String, String>>".to_string()
                    } else {
                        "std::collections::HashMap<String, String>".to_string()
                    }
                }
                TypeRef::Json => {
                    // serde_json::Value params: ext-php-rs cannot implement FromZvalMut for serde_json::Value.
                    // Receive as String (JSON-encoded) and parse in the function body binding.
                    if p.optional {
                        "Option<String>".to_string()
                    } else {
                        "String".to_string()
                    }
                }
                _ => {
                    if p.optional {
                        format!("Option<{base_ty}>")
                    } else {
                        base_ty
                    }
                }
            };
            format!("{}: {}", to_php_name(&p.name), ty)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate PHP-specific call arguments.
/// Non-opaque Named types are passed as `&T`, so we clone before `.into()`.
/// Handles i64->usize/u64 casts for primitive types that need conversion.
pub(crate) fn gen_php_call_args(params: &[crate::core::ir::ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .map(|p| {
            let php_name = to_php_name(&p.name);
            // Newtype params (e.g. NodeIndex(u32)→u32): re-wrap the raw binding value.
            if let Some(newtype_path) = &p.newtype_wrapper {
                return if p.optional {
                    format!("{php_name}.map({newtype_path})")
                } else {
                    format!("{newtype_path}({php_name})")
                };
            }
            match &p.ty {
                TypeRef::Primitive(prim) if needs_i64_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        format!("{php_name}.map(|v| v as {})", core_ty)
                    } else {
                        format!("{php_name} as {}", core_ty)
                    }
                }
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    if p.optional {
                        format!("{php_name}.as_ref().map(|v| &v.inner)")
                    } else {
                        format!("&{php_name}.inner")
                    }
                }
                TypeRef::Named(_) => {
                    // Non-opaque: param is &T, clone then convert
                    if p.optional {
                        format!("{php_name}.map(|v| v.clone().into())")
                    } else {
                        format!("{php_name}.clone().into()")
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    // For optional params, only use as_deref() when core expects &str (is_ref=true).
                    // When is_ref=false, core takes Option<String> — pass owned.
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_deref()")
                        } else {
                            php_name
                        }
                    } else if p.is_ref {
                        format!("&{php_name}")
                    } else {
                        php_name
                    }
                }
                TypeRef::Path => {
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_deref().map(std::path::Path::new)")
                        } else {
                            format!("{php_name}.map(std::path::PathBuf::from)")
                        }
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{php_name})")
                    } else {
                        format!("std::path::PathBuf::from({php_name})")
                    }
                }
                TypeRef::Bytes => {
                    // PHP-side param is PhpBytes (binary-safe wrapper). Convert to
                    // &[u8] / Vec<u8> when passing to core.
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_ref().map(|s| &s.0[..])")
                        } else {
                            format!("{php_name}.map(|b| b.0)")
                        }
                    } else if p.is_ref {
                        format!("&{php_name}.0[..]")
                    } else {
                        format!("{php_name}.0")
                    }
                }
                TypeRef::Vec(inner) => {
                    // Vec<NonOpaqueCustomType> is passed as &Vec when inner is non-opaque Named.
                    // Use let bindings for conversion (handled in gen_php_named_let_bindings).
                    if let TypeRef::Named(name) = inner.as_ref() {
                        if !opaque_types.contains(name.as_str()) {
                            // Non-opaque named type inside Vec: use the _core binding
                            if p.is_ref {
                                if p.optional {
                                    format!("{php_name}_core.as_ref().map(|v| &v[..])")
                                } else {
                                    format!("&{php_name}_core[..]")
                                }
                            } else {
                                format!("{php_name}_core")
                            }
                        } else {
                            // Opaque or enum named type inside Vec
                            if p.optional {
                                if p.is_ref {
                                    format!("{php_name}.as_deref()")
                                } else {
                                    php_name
                                }
                            } else if p.is_ref {
                                format!("&{php_name}[..]")
                            } else {
                                php_name
                            }
                        }
                    } else {
                        // Primitive types inside Vec
                        if p.optional {
                            if p.is_ref {
                                format!("{php_name}.as_deref()")
                            } else {
                                php_name
                            }
                        } else if p.is_ref {
                            format!("&{php_name}[..]")
                        } else {
                            php_name
                        }
                    }
                }
                TypeRef::Map(_, _) if p.map_is_ahash && p.map_key_is_cow => {
                    // AHashMap<Cow, Value>: use the __<name>_ahash pre-bound variable.
                    let bound_name = format!("__{}_ahash", p.name);
                    if p.optional && p.is_ref {
                        format!("{bound_name}.as_ref()")
                    } else if p.is_ref {
                        format!("{bound_name}.as_ref().unwrap()")
                    } else {
                        bound_name
                    }
                }
                TypeRef::Map(_, _) if p.map_is_btree => {
                    // Core expects a `BTreeMap`; PHP receives the hash as `HashMap`. Collect
                    // into a `BTreeMap` at the call site to match the core signature.
                    let collect = "iter().map(|(k, v)| (k.clone(), v.clone()))\
                        .collect::<std::collections::BTreeMap<_, _>>()";
                    if p.optional {
                        format!("{php_name}.as_ref().map(|m| m.{collect})")
                    } else if p.is_ref {
                        format!("&{php_name}.{collect}")
                    } else {
                        format!("{php_name}.{collect}")
                    }
                }
                TypeRef::Duration => {
                    if p.optional {
                        format!("{php_name}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))")
                    } else {
                        format!("std::time::Duration::from_millis({php_name}.max(0) as u64)")
                    }
                }
                TypeRef::Json => {
                    // JSON param is received as String from PHP, use the _json binding for parsing.
                    let bound_name = format!("{php_name}_json");
                    bound_name
                }
                _ => php_name,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate let bindings for non-opaque Named params in free functions.
/// Creates `let {name}_core: {core_import}::{TypeName} = {name}.clone().into();`
/// so the function body can pass `&{name}_core` instead of `{name}.clone().into()`.
/// Also handles Vec<NonOpaqueCustomType> by iterating PHP arrays and extracting each element.
/// Also handles AHashMap<Cow, Value> params by converting from HashMap<String, String>.
pub(crate) fn gen_php_named_let_bindings(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut out = String::new();

    for p in params {
        // AHashMap<Cow<'static, str>, Value> params: PHP receives as HashMap<String, String>.
        // Convert to ahash::AHashMap<Cow, Value> so core can be called with correct types.
        if let TypeRef::Map(_, _) = &p.ty {
            if p.map_is_ahash && p.map_key_is_cow {
                let php_name = to_php_name(&p.name);
                let bound_name = format!("__{}_ahash", p.name);
                out.push_str(&crate::backends::php::template_env::render(
                    "php_ahash_cow_value_let_binding.jinja",
                    context! {
                        bound_name => &bound_name,
                        php_name => &php_name,
                    },
                ));
                continue;
            }
        }
        // PHP parameter names are camelCase (via to_php_name). Let bindings must
        // use the same PHP-side name as the parameter so the generated code compiles.
        let php_param_name = to_php_name(&p.name);
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                out.push_str(&crate::backends::php::template_env::render(
                    "php_named_let_binding.jinja",
                    context! {
                        php_name => &php_param_name,
                        core_import => core_import,
                        type_name => name.as_str(),
                        is_optional => p.optional,
                        is_mut => p.is_mut,
                    },
                ));
                // For optional+is_mut params: unwrap into a mutable intermediate so
                // &mut {name}_unwrapped can be passed to functions taking &mut T.
                if p.optional && p.is_mut {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_optional_mut_unwrap_binding.jinja",
                        context! { php_name => &php_param_name },
                    ));
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        if enum_names.contains(name.as_str()) {
                            // Vec<EnumType>: the PHP param is Vec<String>; convert each element
                            // via `From<String>` (enum discriminant round-trip).
                            out.push_str(&crate::backends::php::template_env::render(
                                "php_let_binding_vec_named.jinja",
                                context! {
                                    pname => php_param_name.as_str(),
                                    core_import => core_import,
                                    name => name.as_str(),
                                },
                            ));
                        } else {
                            // Vec<StructType>: the PHP param is &ZendHashTable; iterate and convert
                            // each element via FromZval.
                            out.push_str(&crate::backends::php::template_env::render(
                                "php_vec_named_struct_let_binding.jinja",
                                context! {
                                    php_name => &php_param_name,
                                    core_import => core_import,
                                    struct_name => name,
                                    is_optional => p.optional,
                                },
                            ));
                        }
                    }
                } else if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() {
                    // Sanitized Vec<tuple>: each item is a JSON-encoded tuple string.
                    // Deserialize so the core function can be called with its native signature.
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_sanitized_vec_let_binding.jinja",
                        context! {
                            param_name => &php_param_name,
                            is_optional => p.optional,
                        },
                    ));
                } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref && p.vec_inner_is_ref {
                    // Vec<String> with is_ref=true AND vec_inner_is_ref=true: core expects &[&str].
                    // When vec_inner_is_ref=false the core expects &[String], which Vec<String>
                    // already satisfies via &[..] coercion — no _refs binding needed.
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_vec_string_refs_let_binding.jinja",
                        context! {
                            param_name => &php_param_name,
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// Generate call args using pre-bound let bindings for non-opaque Named params.
pub(crate) fn gen_php_call_args_with_let_bindings(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| {
            let php_name = to_php_name(&p.name);
            match &p.ty {
                TypeRef::Primitive(prim) if needs_i64_cast(prim) => {
                    let core_ty = core_prim_str(prim);
                    if p.optional {
                        format!("{php_name}.map(|v| v as {})", core_ty)
                    } else {
                        format!("{php_name} as {}", core_ty)
                    }
                }
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    let is_mutex = mutex_types.contains(name.as_str());
                    if p.optional {
                        if p.is_mut && is_mutex {
                            format!("{php_name}.as_ref().map(|v| &mut *v.inner.lock().unwrap())")
                        } else {
                            format!("{php_name}.as_ref().map(|v| &v.inner)")
                        }
                    } else if p.is_mut && is_mutex {
                        format!("&mut *{php_name}.inner.lock().unwrap()")
                    } else {
                        format!("&{php_name}.inner")
                    }
                }
                TypeRef::Named(_) => {
                    // Non-opaque Named: use the _core binding.
                    // If core expects a reference (is_ref=true), add & for optional or &val for non-optional.
                    if p.is_ref {
                        if p.optional {
                            if p.is_mut {
                                // gen_php_named_let_bindings emits an extra
                                // `let mut {name}_unwrapped = {name}_core.unwrap_or_default();`
                                // for optional+is_mut params, so we can borrow it mutably here.
                                format!("&mut {php_name}_unwrapped")
                            } else {
                                format!("{php_name}_core.as_ref()")
                            }
                        } else if p.is_mut {
                            format!("&mut {php_name}_core")
                        } else {
                            format!("&{php_name}_core")
                        }
                    } else {
                        format!("{php_name}_core")
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_deref()")
                        } else {
                            php_name
                        }
                    } else if p.is_ref {
                        format!("&{php_name}")
                    } else {
                        php_name
                    }
                }
                TypeRef::Path => {
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_deref().map(std::path::Path::new)")
                        } else {
                            format!("{php_name}.map(std::path::PathBuf::from)")
                        }
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{php_name})")
                    } else {
                        format!("std::path::PathBuf::from({php_name})")
                    }
                }
                TypeRef::Bytes => {
                    // PHP-side param is PhpBytes (binary-safe wrapper). Convert to
                    // &[u8] / Vec<u8> when passing to core.
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_ref().map(|s| &s.0[..])")
                        } else {
                            format!("{php_name}.map(|b| b.0)")
                        }
                    } else if p.is_ref {
                        format!("&{php_name}.0[..]")
                    } else {
                        format!("{php_name}.0")
                    }
                }
                TypeRef::Vec(inner) => {
                    // Check if inner is a non-opaque Named type that needs let binding
                    let uses_binding = if let TypeRef::Named(name) = inner.as_ref() {
                        !opaque_types.contains(name.as_str())
                    } else {
                        false
                    };
                    // Sanitized Vec<String> originating from a tuple type also has a `_core` binding
                    // (JSON-decoded items). Treat it like the named case.
                    let uses_sanitized_binding =
                        matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some();

                    if uses_binding || uses_sanitized_binding {
                        // Use the _core binding
                        if p.is_ref {
                            if p.optional {
                                format!("{php_name}_core.as_ref().map(|v| &v[..])")
                            } else {
                                format!("&{php_name}_core[..]")
                            }
                        } else {
                            format!("{php_name}_core")
                        }
                    } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                        && p.is_ref
                        && p.vec_inner_is_ref
                    {
                        // Vec<String> with is_ref and vec_inner_is_ref: core expects &[&str].
                        // Use the pre-built _refs binding (Vec<&str>).
                        format!("&{php_name}_refs")
                    } else {
                        // Opaque or primitive types: no binding needed
                        if p.optional {
                            if p.is_ref {
                                format!("{php_name}.as_deref()")
                            } else {
                                php_name
                            }
                        } else if p.is_ref {
                            // Core expects &[T], so convert Vec<T> to &[T]
                            format!("&{php_name}[..]")
                        } else {
                            php_name
                        }
                    }
                }
                TypeRef::Map(_, _) if p.map_is_ahash && p.map_key_is_cow => {
                    // AHashMap<Cow, Value> param: use the pre-bound __<name>_ahash variable
                    // from gen_php_named_let_bindings.
                    let bound_name = format!("__{}_ahash", p.name);
                    if p.optional && p.is_ref {
                        format!("{bound_name}.as_ref()")
                    } else if p.is_ref {
                        format!("{bound_name}.as_ref().unwrap()")
                    } else {
                        bound_name
                    }
                }
                TypeRef::Map(_, _) if p.map_is_btree => {
                    // Core expects a `BTreeMap`, but PHP receives the hash as `HashMap`.
                    // Collect into a `BTreeMap` at the call site so the key ordering and
                    // concrete type match the core signature.
                    let collect = "iter().map(|(k, v)| (k.clone(), v.clone()))\
                        .collect::<std::collections::BTreeMap<_, _>>()";
                    if p.optional {
                        format!("{php_name}.as_ref().map(|m| m.{collect})")
                    } else if p.is_ref {
                        format!("&{php_name}.{collect}")
                    } else {
                        format!("{php_name}.{collect}")
                    }
                }
                TypeRef::Map(_, _) => {
                    if p.optional {
                        if p.is_ref {
                            format!("{php_name}.as_ref()")
                        } else {
                            php_name
                        }
                    } else if p.is_ref {
                        format!("&{php_name}")
                    } else {
                        php_name
                    }
                }
                TypeRef::Duration => {
                    if p.optional {
                        format!("{php_name}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))")
                    } else {
                        format!("std::time::Duration::from_millis({php_name}.max(0) as u64)")
                    }
                }
                TypeRef::Json => {
                    // JSON param is received as String from PHP, use the _json binding for parsing.
                    let bound_name = format!("{php_name}_json");
                    bound_name
                }
                _ => php_name,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
