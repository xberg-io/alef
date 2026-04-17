use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_codegen::conversions::ConversionConfig;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, PrimitiveType, TypeDef, TypeRef};
use std::fmt::Write;

/// Generate a serde JSON bridge `impl From<BindingType> for core::Type`.
/// Used for enum-tainted types where field-by-field From can't work (no From<String> for core enums),
/// but serde can round-trip through JSON since the binding type derives Serialize and the core type
/// derives Deserialize.
pub(crate) fn gen_serde_bridge_from(typ: &TypeDef, core_import: &str) -> String {
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    format!(
        "impl From<{}> for {} {{\n    \
         fn from(val: {}) -> Self {{\n        \
         let json = serde_json::to_string(&val).expect(\"alef: serialize binding type\");\n        \
         serde_json::from_str(&json).expect(\"alef: deserialize to core type\")\n    \
         }}\n\
         }}",
        typ.name, core_path, typ.name
    )
}

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
pub(crate) fn gen_php_function_params(
    params: &[alef_core::ir::ParamDef],
    mapper: &PhpMapper,
    _opaque_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| {
            let base_ty = mapper.map_type(&p.ty);
            let ty = match &p.ty {
                TypeRef::Named(name) => {
                    // Enum types are mapped to String in PHP — use owned String, not &String.
                    // Only php_class struct types need &T (ext-php-rs only provides
                    // FromZvalMut for &T/&mut T, not owned T, for php_class types).
                    if mapper.enum_names.contains(name.as_str()) {
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
                _ => {
                    if p.optional {
                        format!("Option<{base_ty}>")
                    } else {
                        base_ty
                    }
                }
            };
            format!("{}: {}", p.name, ty)
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
        .map(|p| match &p.ty {
            TypeRef::Primitive(prim) if needs_i64_cast(prim) => {
                let core_ty = core_prim_str(prim);
                if p.optional {
                    format!("{}.map(|v| v as {})", p.name, core_ty)
                } else {
                    format!("{} as {}", p.name, core_ty)
                }
            }
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if p.optional {
                    format!("{}.as_ref().map(|v| &v.inner)", p.name)
                } else {
                    format!("&{}.inner", p.name)
                }
            }
            TypeRef::Named(_) => {
                // Non-opaque: param is &T, clone then convert
                if p.optional {
                    format!("{}.map(|v| v.clone().into())", p.name)
                } else {
                    format!("{}.clone().into()", p.name)
                }
            }
            TypeRef::String | TypeRef::Char => {
                // For optional params, only use as_deref() when core expects &str (is_ref=true).
                // When is_ref=false, core takes Option<String> — pass owned.
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
            TypeRef::Vec(_) => {
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
            TypeRef::Duration => {
                if p.optional {
                    format!("{}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))", p.name)
                } else {
                    format!("std::time::Duration::from_millis({}.max(0) as u64)", p.name)
                }
            }
            _ => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate let bindings for non-opaque Named params in free functions.
/// Creates `let {name}_core: {core_import}::{TypeName} = {name}.clone().into();`
/// so the function body can pass `&{name}_core` instead of `{name}.clone().into()`.
pub(crate) fn gen_php_named_let_bindings(
    params: &[alef_core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut out = String::new();
    for p in params {
        if let TypeRef::Named(name) = &p.ty {
            if !opaque_types.contains(name.as_str()) {
                if p.optional {
                    writeln!(
                        out,
                        "let {}_core: Option<{core_import}::{name}> = {}.map(|v| v.clone().into());",
                        p.name, p.name
                    )
                    .ok();
                } else {
                    writeln!(
                        out,
                        "let {}_core: {core_import}::{name} = {}.clone().into();",
                        p.name, p.name
                    )
                    .ok();
                }
            }
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
        .map(|p| match &p.ty {
            TypeRef::Primitive(prim) if needs_i64_cast(prim) => {
                let core_ty = core_prim_str(prim);
                if p.optional {
                    format!("{}.map(|v| v as {})", p.name, core_ty)
                } else {
                    format!("{} as {}", p.name, core_ty)
                }
            }
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if p.optional {
                    format!("{}.as_ref().map(|v| &v.inner)", p.name)
                } else {
                    format!("&{}.inner", p.name)
                }
            }
            TypeRef::Named(_) => {
                format!("{}_core", p.name)
            }
            TypeRef::String | TypeRef::Char => {
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
            TypeRef::Vec(_) => {
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
            TypeRef::Map(_, _) => {
                if p.optional {
                    if p.is_ref {
                        format!("{}.as_ref()", p.name)
                    } else {
                        p.name.clone()
                    }
                } else if p.is_ref {
                    format!("&{}", p.name)
                } else {
                    p.name.clone()
                }
            }
            TypeRef::Duration => {
                if p.optional {
                    format!("{}.map(|v| std::time::Duration::from_millis(v.max(0) as u64))", p.name)
                } else {
                    format!("std::time::Duration::from_millis({}.max(0) as u64)", p.name)
                }
            }
            _ => p.name.clone(),
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
) -> String {
    match return_type {
        TypeRef::Primitive(p) if needs_i64_cast(p) => {
            format!("{expr} as i64")
        }
        TypeRef::Duration => format!("{expr}.as_millis() as i64"),
        // Opaque Named returns need Arc wrapper
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if returns_cow {
                format!("Self {{ inner: Arc::new({expr}.into_owned()) }}")
            } else if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_cow {
                format!("{n} {{ inner: Arc::new({expr}.into_owned()) }}")
            } else if returns_ref {
                format!("{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{n} {{ inner: Arc::new({expr}) }}")
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
                if returns_ref {
                    format!("{expr}.map(|v| {n} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {n} {{ inner: Arc::new(v) }})")
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
    let mut out = format!("let core_self = {core_path} {{\n");
    for field in &typ.fields {
        let name = &field.name;
        if field.sanitized {
            writeln!(out, "            {name}: Default::default(),").ok();
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
                        } else {
                            format!("std::time::Duration::from_millis(self.{name} as u64)")
                        }
                    }
                    TypeRef::String | TypeRef::Char | TypeRef::Bytes => format!("self.{name}.clone()"),
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
                    TypeRef::Map(_, _) => format!("self.{name}.clone()"),
                    TypeRef::Unit => format!("self.{name}.clone()"),
                    // Json maps to String in PHP -- can't directly assign to serde_json::Value
                    TypeRef::Json => "Default::default()".to_string(),
                }
            };
            writeln!(out, "            {name}: {expr},").ok();
        }
    }
    // Use ..Default::default() to fill cfg-gated fields stripped from the IR
    if typ.has_stripped_cfg_fields {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str("        };\n        ");
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
) -> String {
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    let mut out = String::with_capacity(512);
    writeln!(out, "impl From<{}> for {core_path} {{", typ.name).ok();
    writeln!(out, "    fn from(val: {}) -> Self {{", typ.name).ok();
    writeln!(out, "        Self {{").ok();
    for field in &typ.fields {
        let name = &field.name;
        if field.sanitized {
            writeln!(out, "            {name}: Default::default(),").ok();
        } else if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
            // Direct enum-Named field: generate string->enum match
            let conversion =
                gen_string_to_enum_expr(&format!("val.{name}"), &enum_name, field.optional, enums, core_import);
            writeln!(out, "            {name}: {conversion},").ok();
        } else if let Some(enum_name) = get_vec_enum_named(&field.ty, enum_names) {
            // Vec<Enum-Named> field: element-wise string->enum parsing
            let elem_conversion = gen_string_to_enum_expr("s", &enum_name, false, enums, core_import);
            if field.optional {
                writeln!(
                    out,
                    "            {name}: val.{name}.map(|v| v.into_iter().map(|s| {elem_conversion}).collect()),"
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "            {name}: val.{name}.into_iter().map(|s| {elem_conversion}).collect(),"
                )
                .ok();
            }
        } else {
            // Non-enum field (may reference other tainted types, which have their own From)
            let conversion =
                alef_codegen::conversions::field_conversion_to_core_cfg(name, &field.ty, field.optional, config);
            writeln!(out, "            {conversion},").ok();
        }
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
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
            format!("{core_path}::{}", variant.name)
        } else if alef_codegen::conversions::is_tuple_variant(&variant.fields) {
            let defaults: Vec<&str> = variant.fields.iter().map(|_| "Default::default()").collect();
            format!("{core_path}::{}({})", variant.name, defaults.join(", "))
        } else {
            let defaults: Vec<String> = variant
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            format!("{core_path}::{} {{ {} }}", variant.name, defaults.join(", "))
        }
    }

    let first_expr = variant_expr(&core_enum_path, &enum_def.variants[0]);
    let mut match_arms = String::new();
    for variant in &enum_def.variants {
        let expr = variant_expr(&core_enum_path, variant);
        write!(match_arms, "\"{}\" => {expr}, ", variant.name).ok();
    }
    write!(match_arms, "_ => {first_expr}").ok();

    if optional {
        format!("{val_expr}.as_deref().map(|s| match s {{ {match_arms} }})")
    } else {
        format!("match {val_expr}.as_str() {{ {match_arms} }}")
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
