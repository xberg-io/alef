use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_codegen::conversions::ConversionConfig;
use alef_codegen::naming::to_php_name;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, PrimitiveType, TypeDef, TypeRef};
use minijinja::context;

/// Return true if any field of the type (recursively through Optional/Vec) is a Named type
/// that is an enum. PHP maps enum Named types to String, so From/Into impls would need
/// From<String> for the core enum which doesn't exist -- skip generation for such types.
/// Check if a TypeRef references any type in the given set (transitively through containers).
pub(crate) fn references_named_type(ty: &alef_core::ir::TypeRef, names: &AHashSet<String>) -> bool {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => names.contains(name.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => references_named_type(inner, names),
        TypeRef::Map(k, v) => references_named_type(k, names) || references_named_type(v, names),
        _ => false,
    }
}

pub(crate) fn has_enum_named_field(typ: &alef_core::ir::TypeDef, enum_names: &AHashSet<String>) -> bool {
    fn type_ref_has_enum_named(ty: &alef_core::ir::TypeRef, enum_names: &AHashSet<String>) -> bool {
        use alef_core::ir::TypeRef;
        match ty {
            TypeRef::Named(name) => enum_names.contains(name.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_has_enum_named(inner, enum_names),
            TypeRef::Map(k, v) => type_ref_has_enum_named(k, enum_names) || type_ref_has_enum_named(v, enum_names),
            _ => false,
        }
    }
    typ.fields.iter().any(|f| type_ref_has_enum_named(&f.ty, enum_names))
}

/// Generate PHP-specific function parameter list.
/// Non-opaque Named types use `&T` (ext-php-rs only provides `FromZvalMut` for `&mut T`/`&T`,
/// not owned `T`, when `T` is a `#[php_class]`).
/// Vec<NonOpaqueCustomType> also needs &Vec<T> since the elements are php_class types.
/// Bridge type aliases (like VisitorHandle) are mapped to raw PHP object types `&mut ZendObject`.
pub(crate) fn gen_php_function_params(
    params: &[alef_core::ir::ParamDef],
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
pub(crate) fn gen_php_call_args(params: &[alef_core::ir::ParamDef], opaque_types: &AHashSet<String>) -> String {
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
                TypeRef::Duration => {
                    if p.optional {
                        format!("{php_name}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))")
                    } else {
                        format!("std::time::Duration::from_millis({php_name}.max(0) as u64)")
                    }
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
pub(crate) fn gen_php_named_let_bindings(
    params: &[alef_core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut out = String::new();

    for p in params {
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                out.push_str(&crate::template_env::render(
                    "php_named_let_binding.jinja",
                    context! {
                        php_name => &p.name,
                        core_import => core_import,
                        type_name => name.as_str(),
                        is_optional => p.optional,
                    },
                ));
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        // Vec<NonOpaqueCustomType> (php_class struct): manually iterate PHP array
                        out.push_str(&crate::template_env::render(
                            "php_vec_named_struct_let_binding.jinja",
                            context! {
                                php_name => &p.name,
                                core_import => core_import,
                                struct_name => name,
                                is_optional => p.optional,
                            },
                        ));
                    }
                } else if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() {
                    // Sanitized Vec<tuple>: each item is a JSON-encoded tuple string.
                    // Deserialize so the core function can be called with its native signature.
                    out.push_str(&crate::template_env::render(
                        "php_sanitized_vec_let_binding.jinja",
                        context! {
                            param_name => &p.name,
                            is_optional => p.optional,
                        },
                    ));
                } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref {
                    // Vec<String> with is_ref=true: core expects &[&str].
                    out.push_str(&crate::template_env::render(
                        "php_vec_string_refs_let_binding.jinja",
                        context! {
                            param_name => &p.name,
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
    params: &[alef_core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
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
                    if p.optional {
                        format!("{php_name}.as_ref().map(|v| &v.inner)")
                    } else {
                        format!("&{php_name}.inner")
                    }
                }
                TypeRef::Named(_) => {
                    // Non-opaque Named: use the _core binding.
                    // If core expects a reference (is_ref=true), add & for optional or &val for non-optional.
                    if p.is_ref {
                        if p.optional {
                            format!("{php_name}_core.as_ref()")
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
                    } else if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref {
                        // Vec<String> with is_ref: convert via _refs binding to &[&str]
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
                _ => php_name,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns true if a primitive type needs i64->core casting in PHP.
fn needs_i64_cast(p: &PrimitiveType) -> bool {
    matches!(p, PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize)
}

/// Returns the core primitive type string for i64-cast primitives.
pub(crate) fn core_prim_str(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::U64 => "u64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
        _ => unreachable!(),
    }
}

/// PHP-specific return wrapping that handles i64 casts for u64/usize/isize primitives.
/// Extends the shared `wrap_return` with type conversions for primitives that are i64 in PHP.
pub(crate) fn php_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &ahash::AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    mutex_types: &ahash::AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Bytes => {
            // Core returns Vec<u8> or Bytes; PHP binding expects Vec<u8>.
            if returns_ref {
                format!("{expr}.to_vec()")
            } else {
                format!("Vec::<u8>::from({expr})")
            }
        }
        TypeRef::Primitive(p) if needs_i64_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        // Opaque Named returns need Arc wrapper (and Mutex for mutex types)
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            let wrapper = if mutex_types.contains(type_name) {
                |v: String| format!("Arc::new(std::sync::Mutex::new({v}))")
            } else {
                |v: String| format!("Arc::new({v})")
            };
            if returns_cow {
                format!("Self {{ inner: {} }}", wrapper(format!("{expr}.into_owned()")))
            } else if returns_ref {
                format!("Self {{ inner: {} }}", wrapper(format!("{expr}.clone()")))
            } else {
                format!("Self {{ inner: {} }}", wrapper(expr.to_string()))
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            let wrapper = if mutex_types.contains(n) {
                |v: String| format!("Arc::new(std::sync::Mutex::new({v}))")
            } else {
                |v: String| format!("Arc::new({v})")
            };
            if returns_cow {
                format!("{n} {{ inner: {} }}", wrapper(format!("{expr}.into_owned()")))
            } else if returns_ref {
                format!("{n} {{ inner: {} }}", wrapper(format!("{expr}.clone()")))
            } else {
                format!("{n} {{ inner: {} }}", wrapper(expr.to_string()))
            }
        }
        TypeRef::Named(_) => {
            // Non-opaque Named return type — use .into() for core→binding From conversion.
            if returns_cow {
                format!("{expr}.into_owned().into()")
            } else if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_i64_cast(p) => {
                format!("{expr}.map(|v| v as i64)")
            }
            TypeRef::Duration => format!("{expr}.map(|d| d.as_millis() as i64)"),
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if mutex_types.contains(n) {
                    if returns_ref {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(std::sync::Mutex::new(v.clone())) }})")
                    } else {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(std::sync::Mutex::new(v)) }})")
                    }
                } else {
                    if returns_ref {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(v.clone()) }})")
                    } else {
                        format!("{expr}.map(|v| {n} {{ inner: Arc::new(v) }})")
                    }
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            _ => {
                // Fall back to shared wrap_return for other Option types
                use alef_codegen::generators;
                generators::wrap_return(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    self_is_opaque,
                    returns_ref,
                    returns_cow,
                )
            }
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) if needs_i64_cast(p) => {
                format!("{expr}.into_iter().map(|v| v as i64).collect()")
            }
            // Vec<Vec<T>> where the inner primitive needs widening (e.g. Vec<Vec<usize>> → Vec<Vec<i64>>)
            TypeRef::Vec(inner2) => {
                if let TypeRef::Primitive(p) = inner2.as_ref() {
                    if needs_i64_cast(p) {
                        return format!(
                            "{expr}.into_iter().map(|row| row.into_iter().map(|x| x as i64).collect::<Vec<_>>()).collect::<Vec<_>>()"
                        );
                    }
                }
                // Fall back to shared wrap_return for nested Vec types that don't need casting
                use alef_codegen::generators;
                generators::wrap_return(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    self_is_opaque,
                    returns_ref,
                    returns_cow,
                )
            }
            _ => {
                // Fall back to shared wrap_return for other Vec types
                use alef_codegen::generators;
                generators::wrap_return(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    self_is_opaque,
                    returns_ref,
                    returns_cow,
                )
            }
        },
        _ => {
            // Fall back to shared wrap_return for all other types
            use alef_codegen::generators;
            generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            )
        }
    }
}

/// PHP-specific lossy binding->core struct literal.
/// Like `gen_lossy_binding_to_core_fields` but adds i64->usize casts for large-int primitives.
pub(crate) fn gen_php_lossy_binding_to_core_fields(
    typ: &TypeDef,
    core_import: &str,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
) -> String {
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    let mut out = crate::template_env::render(
        "php_lossy_binding_struct_begin.jinja",
        context! {
            core_type => &core_path,
            has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
        },
    );
    for field in &typ.fields {
        // Skip cfg-gated fields — they are absent from the binding struct.
        // The ..Default::default() spread below fills them when the feature is enabled.
        if field.cfg.is_some() {
            continue;
        }
        let name = &field.name;
        if field.sanitized {
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => "Default::default()",
                },
            ));
        } else {
            // Check if this Named field is an enum (PHP maps enums to String).
            // If so, use string->enum parsing instead of .into().
            let expr = if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
                gen_string_to_enum_expr(&format!("self.{name}"), &enum_name, field.optional, enums, core_import)
            } else if let Some(enum_name) = get_vec_enum_named(&field.ty, enum_names) {
                let elem_conv = gen_string_to_enum_expr("s", &enum_name, false, enums, core_import);
                if field.optional {
                    format!("self.{name}.clone().map(|v| v.into_iter().map(|s| {elem_conv}).collect())")
                } else {
                    format!("self.{name}.clone().into_iter().map(|s| {elem_conv}).collect()")
                }
            } else {
                match &field.ty {
                    TypeRef::Primitive(p) if needs_i64_cast(p) => {
                        let core_ty = core_prim_str(p);
                        if field.optional {
                            format!("self.{name}.map(|v| v as {core_ty})")
                        } else {
                            format!("self.{name} as {core_ty}")
                        }
                    }
                    TypeRef::Primitive(_) => format!("self.{name}"),
                    TypeRef::Duration => {
                        if field.optional {
                            format!("self.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
                        } else if typ.has_default {
                            // Duration stored as Option<i64> (option_duration_on_defaults).
                            // Use the core type's default rather than Duration::default() (0s)
                            // so that e.g. BrowserConfig.timeout preserves its 30s default.
                            crate::template_env::render(
                                "php_duration_default_expr.jinja",
                                context! {
                                    value_expr => &format!("self.{name}"),
                                    cast => " as u64",
                                    core_type => &core_path,
                                    field_name => name.as_str(),
                                },
                            )
                        } else {
                            format!("std::time::Duration::from_millis(self.{name} as u64)")
                        }
                    }
                    TypeRef::String | TypeRef::Char => format!("self.{name}.clone()"),
                    TypeRef::Bytes => format!("self.{name}.clone().into()"),
                    TypeRef::Path => {
                        if field.optional {
                            format!("self.{name}.clone().map(Into::into)")
                        } else {
                            format!("self.{name}.clone().into()")
                        }
                    }
                    TypeRef::Named(_) => {
                        if field.optional {
                            format!("self.{name}.clone().map(Into::into)")
                        } else {
                            format!("self.{name}.clone().into()")
                        }
                    }
                    TypeRef::Vec(inner) => match inner.as_ref() {
                        TypeRef::Named(_) => {
                            if field.optional {
                                format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                            } else {
                                format!("self.{name}.clone().into_iter().map(Into::into).collect()")
                            }
                        }
                        TypeRef::Primitive(p) if needs_i64_cast(p) => {
                            let core_ty = core_prim_str(p);
                            if field.optional {
                                format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                            } else {
                                format!("self.{name}.clone().into_iter().map(|v| v as {core_ty}).collect()")
                            }
                        }
                        _ => format!("self.{name}.clone()"),
                    },
                    TypeRef::Optional(inner) => match inner.as_ref() {
                        TypeRef::Primitive(p) if needs_i64_cast(p) => {
                            let core_ty = core_prim_str(p);
                            format!("self.{name}.map(|v| v as {core_ty})")
                        }
                        TypeRef::Duration => {
                            format!("self.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
                        }
                        TypeRef::Named(_) => {
                            format!("self.{name}.clone().map(Into::into)")
                        }
                        TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                            format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                        }
                        TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) => {
                            if let TypeRef::Primitive(p) = vi.as_ref() {
                                let core_ty = core_prim_str(p);
                                format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                            } else {
                                format!("self.{name}.clone()")
                            }
                        }
                        _ => format!("self.{name}.clone()"),
                    },
                    // Map with Json values: PHP stores String but core expects serde_json::Value.
                    // Can't recover original Values, so fall back to an empty map.
                    TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Json) => "Default::default()".to_string(),
                    // Map<K, Named>: each value needs Into conversion to bridge the binding wrapper
                    // type into the core type (e.g. PhpExtractionPattern → ExtractionPattern).
                    TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Named(_)) => {
                        if field.optional {
                            format!("self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())")
                        } else {
                            format!("self.{name}.clone().into_iter().map(|(k, v)| (k, v.into())).collect()")
                        }
                    }
                    // Map<K, V> where V is not Json/Named: PHP uses HashMap but core may use BTreeMap.
                    // Use into_iter().collect() to allow coercion to the target map type.
                    TypeRef::Map(_, _) => {
                        if field.optional {
                            format!("self.{name}.clone().map(|m| m.into_iter().collect())")
                        } else {
                            format!("self.{name}.clone().into_iter().collect()")
                        }
                    }
                    TypeRef::Unit => format!("self.{name}.clone()"),
                    // Json maps to String in PHP -- can't directly assign to serde_json::Value
                    TypeRef::Json => "Default::default()".to_string(),
                }
            };
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &expr,
                },
            ));
        }
    }
    // Use ..Default::default() to fill cfg-gated fields stripped from the IR
    if typ.has_stripped_cfg_fields {
        out.push_str(&crate::template_env::render(
            "php_default_update.jinja",
            minijinja::Value::default(),
        ));
    }
    out.push_str(&crate::template_env::render(
        "php_lossy_binding_struct_end.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Compute the set of enum-tainted types for which binding->core From CAN be generated.
/// A type is excluded if it references (directly or transitively) an enum with data variants,
/// because data-variant fields may reference types that don't implement Default.
#[allow(dead_code)]
pub(crate) fn gen_convertible_enum_tainted(
    types: &[TypeDef],
    enum_tainted: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
) -> AHashSet<String> {
    // First, find which enum-tainted types directly reference data-variant enums
    let mut unconvertible: AHashSet<String> = AHashSet::new();
    for typ in types {
        if !enum_tainted.contains(&typ.name) {
            continue;
        }
        for field in &typ.fields {
            if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
                if let Some(enum_def) = enums.iter().find(|e| e.name == enum_name) {
                    if enum_def.variants.iter().any(|v| !v.fields.is_empty()) {
                        unconvertible.insert(typ.name.clone());
                    }
                }
            }
        }
    }
    // Transitively exclude types that reference unconvertible types
    let mut changed = true;
    while changed {
        changed = false;
        for typ in types {
            if !enum_tainted.contains(&typ.name) || unconvertible.contains(&typ.name) {
                continue;
            }
            if typ.fields.iter().any(|f| references_named_type(&f.ty, &unconvertible)) {
                unconvertible.insert(typ.name.clone());
                changed = true;
            }
        }
    }
    // Return the set of enum-tainted types that CAN be converted
    enum_tainted
        .iter()
        .filter(|name| !unconvertible.contains(name.as_str()))
        .cloned()
        .collect()
}

/// Generate `impl From<BindingType> for core::Type` for enum-tainted types.
/// Enum-Named fields use string->enum parsing (match on variant names, first variant as fallback).
/// Fields referencing other enum-tainted struct types use `.into()` (their own From is also generated).
/// Non-enum fields use the normal conversion with i64 casts.
pub(crate) fn gen_enum_tainted_from_binding_to_core(
    typ: &TypeDef,
    core_import: &str,
    enum_names: &AHashSet<String>,
    _enum_tainted: &AHashSet<String>,
    config: &ConversionConfig,
    enums: &[EnumDef],
    bridge_type_aliases: &AHashSet<String>,
) -> String {
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    let mut out = String::with_capacity(512);
    out.push_str(&crate::template_env::render(
        "php_impl_from_begin.jinja",
        context! {
            binding_type => &typ.name,
            core_type => &core_path,
            has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
        },
    ));
    for field in &typ.fields {
        // cfg-gated fields are absent from the binding struct and must not appear in the
        // From impl field list — they are filled by the ..Default::default() spread.
        if field.cfg.is_some() {
            continue;
        }
        let name = &field.name;
        // Bridge type alias fields (e.g. VisitorHandle) are NOT sanitized but are in
        // from_binding_skip_types, so field_conversion_to_core_cfg would emit Default::default().
        // Handle them here first: the PHP struct wraps opaque Named types in Option<T> even
        // when field.optional=false, so always use the map(|v| (*v.inner).clone()) form.
        let is_bridge_named = match &field.ty {
            alef_core::ir::TypeRef::Named(n) => bridge_type_aliases.contains(n.as_str()),
            alef_core::ir::TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), alef_core::ir::TypeRef::Named(n) if bridge_type_aliases.contains(n.as_str()))
            }
            _ => false,
        };
        if is_bridge_named {
            // PHP opaque structs wrap the core handle in Arc<VisitorHandle>; extract via deref.
            // The PHP binding struct stores the field as Option<T> (opaque naming convention),
            // so map over the option rather than direct deref.
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &format!("val.{name}.map(|v| (*v.inner).clone())"),
                },
            ));
        } else if field.sanitized {
            // Sanitized fields (e.g. Duration→u64, Vec<T>→Vec<String>) use Default::default()
            // since they can't be round-tripped from the PHP binding representation.
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => "Default::default()",
                },
            ));
        } else if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
            // Direct enum-Named field: generate string->enum match
            let conversion =
                gen_string_to_enum_expr(&format!("val.{name}"), &enum_name, field.optional, enums, core_import);
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &conversion,
                },
            ));
        } else if let Some(enum_name) = get_vec_enum_named(&field.ty, enum_names) {
            // Vec<Enum-Named> field: element-wise string->enum parsing
            let elem_conversion = gen_string_to_enum_expr("s", &enum_name, false, enums, core_import);
            if field.optional {
                let conversion = format!("val.{name}.map(|v| v.into_iter().map(|s| {elem_conversion}).collect())");
                out.push_str(&crate::template_env::render(
                    "php_struct_field_assignment.jinja",
                    context! {
                        field_name => name.as_str(),
                        field_expr => &conversion,
                    },
                ));
            } else {
                let conversion = format!("val.{name}.into_iter().map(|s| {elem_conversion}).collect()");
                out.push_str(&crate::template_env::render(
                    "php_struct_field_assignment.jinja",
                    context! {
                        field_name => name.as_str(),
                        field_expr => &conversion,
                    },
                ));
            }
        } else if !field.optional
            && matches!(field.ty, TypeRef::Duration)
            && config.option_duration_on_defaults
            && typ.has_default
        {
            // Non-optional Duration stored as Option<i64> (option_duration_on_defaults).
            // field_conversion_to_core_cfg doesn't know about this optionalization and would
            // generate `val.{name} as u64` which fails to compile on Option<i64>.
            // Use the core type's default when None to preserve intended defaults (e.g. 30s timeout).
            let cast = if config.cast_large_ints_to_i64 { " as u64" } else { "" };
            let conversion = crate::template_env::render(
                "php_duration_default_expr.jinja",
                context! {
                    value_expr => &format!("val.{name}"),
                    cast => cast,
                    core_type => &core_path,
                    field_name => name.as_str(),
                },
            );
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &conversion,
                },
            ));
        } else if matches!(field.ty, TypeRef::Bytes)
            || matches!(&field.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes))
        {
            // PHP binding Bytes fields are Vec<u8>. Convert via .into() to core Bytes type.
            let conversion = if field.optional {
                format!("val.{name}.map(|v| v.into())")
            } else {
                format!("val.{name}.into()")
            };
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &conversion,
                },
            ));
        } else {
            // Non-enum field (may reference other tainted types, which have their own From)
            let conversion =
                alef_codegen::conversions::field_conversion_to_core_cfg(name, &field.ty, field.optional, config);
            // Newtype wrapping: when field was resolved from a newtype (e.g. NodeIndex → String),
            // wrap the binding value back into the newtype for the core struct.
            let conversion = if let Some(newtype_path) = &field.newtype_wrapper {
                if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                    match &field.ty {
                        TypeRef::Optional(_) => format!("{name}: ({expr}).map({newtype_path})"),
                        TypeRef::Vec(_) => format!("{name}: ({expr}).into_iter().map({newtype_path}).collect()"),
                        _ if field.optional => format!("{name}: ({expr}).map({newtype_path})"),
                        _ => format!("{name}: {newtype_path}({expr})"),
                    }
                } else {
                    conversion
                }
            } else {
                conversion
            };
            // Box<T> fields: wrap the converted value in Box::new().
            let conversion = if field.is_boxed && matches!(&field.ty, TypeRef::Named(_)) {
                if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                    if field.optional {
                        format!("{name}: {expr}.map(Box::new)")
                    } else {
                        format!("{name}: Box::new({expr})")
                    }
                } else {
                    conversion
                }
            } else {
                conversion
            };
            // Apply core wrapper handling (Cow/Arc/Bytes; vec_inner_core_wrapper for Vec<Arc<T>>)
            let conversion = alef_codegen::conversions::apply_core_wrapper_to_core(
                &conversion,
                name,
                &field.core_wrapper,
                &field.vec_inner_core_wrapper,
                field.optional,
            );
            // field_conversion_to_core_cfg returns "name: expr" (with the field name prefix).
            // php_struct_field_assignment.jinja already adds "{{ field_name }}: " so we strip
            // the prefix here to avoid "name: name: expr" duplication.
            let field_expr = conversion.strip_prefix(&format!("{name}: ")).unwrap_or(&conversion);
            out.push_str(&crate::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => field_expr,
                },
            ));
        }
    }
    out.push_str(&crate::template_env::render(
        "php_impl_from_end.jinja",
        context! {
            has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
        },
    ));
    out
}

/// If the TypeRef is a Named type referencing an enum, return the enum name.
/// Handles Named(enum) and Optional(Named(enum)).
fn get_direct_enum_named(ty: &TypeRef, enum_names: &AHashSet<String>) -> Option<String> {
    match ty {
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(name.clone()),
        TypeRef::Optional(inner) => get_direct_enum_named(inner, enum_names),
        _ => None,
    }
}

/// If the TypeRef is a Vec<Named(enum)>, return the enum name.
/// Handles Vec(Named(enum)) and Optional(Vec(Named(enum))).
fn get_vec_enum_named(ty: &TypeRef, enum_names: &AHashSet<String>) -> Option<String> {
    match ty {
        TypeRef::Vec(inner) => get_direct_enum_named(inner, enum_names),
        TypeRef::Optional(inner) => get_vec_enum_named(inner, enum_names),
        _ => None,
    }
}

/// Generate an expression that converts a String to a core enum type via matching.
/// Falls back to the first variant if no match found.
/// Data variants (with fields) use `Default::default()` for each field.
fn gen_string_to_enum_expr(
    val_expr: &str,
    enum_name: &str,
    optional: bool,
    enums: &[EnumDef],
    core_import: &str,
) -> String {
    let enum_def = match enums.iter().find(|e| e.name == enum_name) {
        Some(e) => e,
        None => return "Default::default()".to_string(),
    };
    let core_enum_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);

    if enum_def.variants.is_empty() {
        return "Default::default()".to_string();
    }

    /// Build the variant constructor expression, filling data variant fields with defaults.
    fn variant_expr(core_path: &str, variant: &alef_core::ir::EnumVariant) -> String {
        if variant.fields.is_empty() {
            crate::template_env::render(
                "php_enum_variant_unit_expr.jinja",
                context! {
                    core_path => core_path,
                    variant_name => &variant.name,
                },
            )
        } else if alef_codegen::conversions::is_tuple_variant(&variant.fields) {
            let defaults: Vec<&str> = variant.fields.iter().map(|_| "Default::default()").collect();
            crate::template_env::render(
                "php_enum_variant_tuple_expr.jinja",
                context! {
                    core_path => core_path,
                    variant_name => &variant.name,
                    defaults => defaults.join(", "),
                },
            )
        } else {
            let fields: Vec<String> = variant
                .fields
                .iter()
                .map(|field| {
                    crate::template_env::render(
                        "php_enum_variant_default_field_expr.jinja",
                        context! {
                            field_name => &field.name,
                        },
                    )
                })
                .collect();
            crate::template_env::render(
                "php_enum_variant_struct_expr.jinja",
                context! {
                    core_path => core_path,
                    variant_name => &variant.name,
                    fields => fields.join(", "),
                },
            )
        }
    }

    let has_default_variant = enum_def.variants.iter().any(|v| v.is_default);
    let fallback_expr = if has_default_variant {
        "Default::default()".to_string()
    } else {
        variant_expr(&core_enum_path, &enum_def.variants[0])
    };
    let mut match_arms = String::new();
    for variant in &enum_def.variants {
        let expr = variant_expr(&core_enum_path, variant);
        // The wire value the PHP user supplies (in JSON or via the binding's String
        // mirror of a Rust enum) follows the core enum's serde rename strategy. Match
        // against `#[serde(rename)]` first, then `#[serde(rename_all = "...")]`, then
        // the variant's raw Rust name as a fallback.
        let wire_name = if let Some(rename) = &variant.serde_rename {
            rename.clone()
        } else if let Some(rename_all) = &enum_def.serde_rename_all {
            crate::gen_bindings::types::apply_rename_all_public(&variant.name, rename_all)
        } else {
            variant.name.clone()
        };
        match_arms.push_str(&crate::template_env::render(
            "php_enum_string_match_arm.jinja",
            context! {
                variant_name => &wire_name,
                expr => &expr,
            },
        ));
    }
    match_arms.push_str(&crate::template_env::render(
        "php_enum_string_match_fallback_arm.jinja",
        context! {
            fallback_expr => &fallback_expr,
        },
    ));

    if optional {
        crate::template_env::render(
            "php_enum_string_optional_match_expr.jinja",
            context! {
                val_expr => val_expr,
                match_arms => &match_arms,
            },
        )
    } else {
        crate::template_env::render(
            "php_enum_string_match_expr.jinja",
            context! {
                val_expr => val_expr,
                match_arms => &match_arms,
            },
        )
    }
}

/// Generate a global Tokio runtime for PHP async support.
pub(crate) fn gen_tokio_runtime() -> String {
    "static WORKER_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect(\"Failed to create Tokio runtime\")
});"
    .to_string()
}
