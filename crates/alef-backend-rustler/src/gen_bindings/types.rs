use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, FieldDef, TypeDef, TypeRef};

/// Generate a Rustler opaque resource wrapper for a type.
pub(super) fn gen_opaque_resource(typ: &TypeDef, core_import: &str, _opaque_types: &AHashSet<String>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("#[derive(Clone)]\n");
    out.push_str(&format!("pub struct {} {{\n", typ.name));
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    out.push_str(&format!("    inner: Arc<{}>,\n", core_path));
    out.push_str("}\n\n");
    // SAFETY: The inner value is behind Arc (immutable shared reference) and
    // Rustler's ResourceArc ensures thread-safe access.
    out.push_str(&format!(
        "// SAFETY: See gen_opaque_resource in alef-backend-rustler for rationale.\n\
         impl std::panic::RefUnwindSafe for {} {{}}\n\n\
         impl rustler::Resource for {} {{}}\n",
        typ.name, typ.name
    ));
    out
}

/// Generate a Rustler NIF struct definition using the shared TypeMapper.
/// Rustler 0.37: NifStruct is a derive macro with #[module = "..."] attribute.
///
/// Fields listed in `exclude_fields` are omitted from the generated struct —
/// used to skip bridge fields (e.g. visitor) that are handled at the Elixir layer
/// and cannot implement Rustler's Encoder/Decoder traits.
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &crate::type_map::RustlerMapper,
    module_prefix: &str,
    exclude_fields: &AHashSet<String>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);
    if typ.has_default {
        // Config types use NifMap so partial maps can be passed —
        // unspecified keys use Rust Default values instead of Elixir zero values.
        // Binding types always derive Default, Serialize, and Deserialize.
        writeln!(
            out,
            "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifMap)]"
        )
        .ok();
    } else {
        // Binding types always derive Serialize and Deserialize for FFI/type conversion.
        writeln!(
            out,
            "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifStruct)]"
        )
        .ok();
        writeln!(out, "#[module = \"{}.{}\"]", module_prefix, typ.name).ok();
    }
    writeln!(out, "pub struct {} {{", typ.name).ok();

    for field in &typ.fields {
        // Skip fields excluded by the caller (e.g. options_field bridge fields).
        if exclude_fields.contains(&field.name) {
            continue;
        }
        // When field.ty is already Optional(T) and field.optional is also true, the type is
        // a double-optional (Option<Option<T>>) in core — map_type already produces Option<T>,
        // so wrapping again would give Option<Option<T>> which is correct for the struct but
        // only when field.optional is acting as the outer wrapper. The shared structs.rs
        // gen_struct_with_per_field_attrs avoids double-wrapping by checking whether
        // field.ty is already Optional before applying the outer Option. We match that here.
        let field_type = if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        writeln!(out, "    pub {}: {},", field.name, field_type).ok();
    }

    write!(out, "}}").ok();
    out
}

/// Generate a Rustler config constructor impl for a type with `has_default`.
/// Fields in `exclude_fields` are skipped in both the struct and the constructor.
pub(super) fn gen_rustler_config_impl(
    typ: &TypeDef,
    mapper: &crate::type_map::RustlerMapper,
    exclude_fields: &AHashSet<String>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "impl {} {{", typ.name).ok();

    // Convert AHashSet to std HashSet for config_gen API
    let excl_std: std::collections::HashSet<String> = exclude_fields.iter().cloned().collect();
    let map_fn = |ty: &TypeRef| mapper.map_type(ty);
    let config_method = alef_codegen::config_gen::gen_rustler_kwargs_constructor_with_exclude(typ, &map_fn, &excl_std);
    write!(out, "    {}", config_method).ok();

    writeln!(out, "}}").ok();
    out
}

/// Map a field's TypeRef to a Rust type string suitable for use in a Rustler NifTaggedEnum variant.
/// Named types are emitted by short name (the binding-layer mirror struct).
/// Vec<Named> recurses so JSON arrays round-trip as actual arrays.
/// Map and complex types collapse to String for JSON round-trip.
fn field_type_for_rustler(field: &FieldDef) -> String {
    let base = field_type_for_rustler_inner(&field.ty);
    if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
        format!("Option<{base}>")
    } else {
        base
    }
}

fn field_type_for_rustler_inner(ty: &TypeRef) -> String {
    use alef_core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "u8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "u16".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "u32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "u64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "usize".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "i8".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "i16".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "i32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "i64".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "isize".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "f32".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "f64".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Vec(inner) => format!("Vec<{}>", field_type_for_rustler_inner(inner)),
        TypeRef::Map(_, _) => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", field_type_for_rustler_inner(inner)),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Generate a Rustler NIF enum definition.
///
/// Unit enums (all variants have no fields) use `NifUnitEnum` — they encode as atoms.
/// Data enums (one or more variants have fields) use `NifTaggedEnum` — they encode as
/// `{:VariantName, field_map}` tagged tuples, preserving all inner fields faithfully.
pub(super) fn gen_enum(enum_def: &EnumDef) -> String {
    use std::fmt::Write;
    let name = &enum_def.name;
    let mut out = String::with_capacity(512);

    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if has_data {
        // NifTaggedEnum: supports unit variants (atoms) and struct variants (tagged tuples).
        // Cannot be Copy when variants have fields.
        writeln!(
            out,
            "#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rustler::NifTaggedEnum)]"
        )
        .ok();
        if let Some(tag) = &enum_def.serde_tag {
            writeln!(out, r#"#[serde(tag = "{tag}")]"#).ok();
        }
        writeln!(out, "pub enum {name} {{").ok();
        for variant in &enum_def.variants {
            if variant.fields.is_empty() {
                writeln!(out, "    {},", variant.name).ok();
            } else {
                let fields: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| format!("    {}: {},", f.name, field_type_for_rustler(f)))
                    .collect();
                writeln!(out, "    {} {{", variant.name).ok();
                for field_line in &fields {
                    writeln!(out, "    {field_line}").ok();
                }
                writeln!(out, "    }},").ok();
            }
        }
        writeln!(out, "}}").ok();
    } else {
        // All unit variants: NifUnitEnum encodes as atoms.
        writeln!(
            out,
            "#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rustler::NifUnitEnum)]"
        )
        .ok();
        writeln!(out, "pub enum {name} {{").ok();
        for variant in &enum_def.variants {
            writeln!(out, "    {},", variant.name).ok();
        }
        writeln!(out, "}}").ok();
    }

    // Default impl: use the variant marked is_default=true; fall back to the first variant.
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.is_default)
        .or_else(|| enum_def.variants.first());
    if let Some(dv) = default_variant {
        write!(
            out,
            "\n#[allow(clippy::derivable_impls)]\nimpl Default for {name} {{\n    fn default() -> Self {{"
        )
        .ok();
        if has_data && !dv.fields.is_empty() {
            let field_defaults: Vec<String> = dv
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            write!(out, " Self::{} {{ {} }} }}\n}}", dv.name, field_defaults.join(", ")).ok();
        } else {
            write!(out, " Self::{} }}\n}}", dv.name).ok();
        }
    }

    out
}

/// Wrap a return expression for Rustler (opaque types get ResourceArc wrapping).
pub(super) fn gen_rustler_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    _type_name: &str,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
) -> String {
    match return_type {
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("ResourceArc::new({n} {{ inner: Arc::new({expr}.clone()) }})")
            } else {
                format!("ResourceArc::new({n} {{ inner: Arc::new({expr}) }})")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        // String and Char: only apply .into() if the core returns a reference (&str, &char).
        // If returns_ref is false, the core returns owned String/Char, so no conversion needed.
        TypeRef::String | TypeRef::Char => {
            if returns_ref {
                // Core returns &str/&char, need to convert to String/Char
                format!("{expr}.into()")
            } else {
                // Core already returns String/Char, no conversion needed
                expr.to_string()
            }
        }
        // Bytes (Vec<u8>): only apply .into() if the core returns a reference (&[u8]).
        // If returns_ref is false, the core returns owned Vec<u8>, so no conversion needed.
        TypeRef::Bytes => {
            if returns_ref {
                // Core returns &[u8], need to convert to Vec<u8>
                format!("{expr}.into()")
            } else {
                // Core already returns Vec<u8>, no conversion needed
                expr.to_string()
            }
        }
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Duration => format!("{expr}.as_millis() as u64"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                format!("{expr}.into_iter().map(|v| ResourceArc::new({n} {{ inner: Arc::new(v) }})).collect()")
            }
            TypeRef::Named(_) => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            _ => expr.to_string(),
        },
        // Optional<T>: when the core returns a reference (&str, &T) wrapped in Option,
        // we must convert each value with `.map(...)`. Without this, Option<&str> is
        // returned where the wrapper signature expects Option<String>.
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Bytes if returns_ref => {
                format!("{expr}.map(|v| v.into())")
            }
            TypeRef::Path => format!("{expr}.map(|v| v.to_string_lossy().to_string())"),
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if returns_ref {
                    format!("{expr}.map(|v| ResourceArc::new({n} {{ inner: Arc::new(v.clone()) }}))")
                } else {
                    format!("{expr}.map(|v| ResourceArc::new({n} {{ inner: Arc::new(v) }}))")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(|v| v.into())")
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{EnumVariant, FieldDef, PrimitiveType};

    fn unit_enum() -> EnumDef {
        EnumDef {
            name: "Color".to_string(),
            rust_path: "my_crate::Color".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Red".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Blue".into(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_rename_all: None,
        }
    }

    fn data_enum() -> EnumDef {
        EnumDef {
            name: "SecuritySchemeInfo".to_string(),
            rust_path: "my_crate::SecuritySchemeInfo".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Http".into(),
                    fields: vec![
                        FieldDef {
                            name: "scheme".into(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef_core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                        },
                        FieldDef {
                            name: "bearer_format".into(),
                            ty: TypeRef::Optional(Box::new(TypeRef::String)),
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef_core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                        },
                    ],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "ApiKey".into(),
                    fields: vec![
                        FieldDef {
                            name: "location".into(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef_core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                        },
                        FieldDef {
                            name: "name".into(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef_core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                        },
                    ],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_rename_all: None,
        }
    }

    /// Unit enums must still lower to NifUnitEnum (atoms on the Elixir side).
    #[test]
    fn test_gen_enum_unit_uses_nif_unit_enum() {
        let result = gen_enum(&unit_enum());
        assert!(
            result.contains("NifUnitEnum"),
            "unit enum should use NifUnitEnum; got:\n{result}"
        );
        assert!(
            !result.contains("NifTaggedEnum"),
            "unit enum must not use NifTaggedEnum; got:\n{result}"
        );
        assert!(result.contains("Red,"), "should contain Red variant; got:\n{result}");
        assert!(result.contains("Blue,"), "should contain Blue variant; got:\n{result}");
    }

    /// Data enums must lower to NifTaggedEnum and preserve all variant fields.
    #[test]
    fn test_gen_enum_data_uses_nif_tagged_enum() {
        let result = gen_enum(&data_enum());
        assert!(
            result.contains("NifTaggedEnum"),
            "data enum should use NifTaggedEnum; got:\n{result}"
        );
        assert!(
            !result.contains("NifUnitEnum"),
            "data enum must not use NifUnitEnum; got:\n{result}"
        );
        // Http variant must have its fields
        assert!(
            result.contains("scheme"),
            "Http variant must preserve `scheme` field; got:\n{result}"
        );
        assert!(
            result.contains("bearer_format"),
            "Http variant must preserve `bearer_format` field; got:\n{result}"
        );
        // ApiKey variant must have its fields
        assert!(
            result.contains("location"),
            "ApiKey variant must preserve `location` field; got:\n{result}"
        );
        assert!(
            result.contains("name"),
            "ApiKey variant must preserve `name` field; got:\n{result}"
        );
    }

    /// Data enum From impls must destructure fields, not use Default::default().
    #[test]
    fn test_data_enum_from_impls_destructure_fields() {
        let e = data_enum();
        let cfg = alef_codegen::conversions::ConversionConfig {
            binding_enums_have_data: true,
            ..Default::default()
        };
        let binding_to_core = alef_codegen::conversions::gen_enum_from_binding_to_core_cfg(&e, "my_crate", &cfg);
        // Must destructure, not Default::default()
        assert!(
            !binding_to_core.contains("Default::default()"),
            "binding->core From must not use Default::default() for data enum fields; got:\n{binding_to_core}"
        );
        assert!(
            binding_to_core.contains("scheme"),
            "binding->core From must destructure `scheme`; got:\n{binding_to_core}"
        );
        assert!(
            binding_to_core.contains("bearer_format"),
            "binding->core From must destructure `bearer_format`; got:\n{binding_to_core}"
        );

        let core_to_binding = alef_codegen::conversions::gen_enum_from_core_to_binding_cfg(&e, "my_crate", &cfg);
        assert!(
            core_to_binding.contains("scheme"),
            "core->binding From must destructure `scheme`; got:\n{core_to_binding}"
        );
        assert!(
            !core_to_binding.contains(".."),
            "core->binding From must not discard fields with `..`; got:\n{core_to_binding}"
        );
    }

    /// Primitive field type mapping for NifTaggedEnum variants.
    #[test]
    fn test_field_type_for_rustler_primitives() {
        let bool_field = FieldDef {
            name: "flag".into(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(field_type_for_rustler(&bool_field), "bool");
        let str_field = FieldDef {
            name: "s".into(),
            ty: TypeRef::String,
            ..bool_field.clone()
        };
        assert_eq!(field_type_for_rustler(&str_field), "String");
        let opt_field = FieldDef {
            name: "o".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            ..bool_field
        };
        assert_eq!(field_type_for_rustler(&opt_field), "Option<String>");
    }
}
