use crate::template_env;
use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, FieldDef, TypeDef, TypeRef};

/// Generate a Rustler opaque resource wrapper for a type.
pub(super) fn gen_opaque_resource(typ: &TypeDef, core_import: &str, _opaque_types: &AHashSet<String>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("#[derive(Clone)]\n");
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    out.push_str(&template_env::render(
        "rust_opaque_struct.jinja",
        minijinja::context! {
            struct_name => &typ.name,
            core_path => &core_path,
        },
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
    let mut out = String::with_capacity(512);
    if typ.has_default {
        // Config types use NifMap so partial maps can be passed —
        // unspecified keys use Rust Default values instead of Elixir zero values.
        // Binding types always derive Default, Serialize, and Deserialize.
        out.push_str("#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifMap)]\n");
    } else {
        // Binding types always derive Serialize and Deserialize for FFI/type conversion.
        out.push_str("#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifStruct)]\n");
        out.push_str(&template_env::render(
            "rust_module_attr.jinja",
            minijinja::context! {
                module_prefix => module_prefix,
                struct_name => &typ.name,
            },
        ));
    }
    out.push_str(&template_env::render(
        "rust_struct_header.jinja",
        minijinja::context! {
            struct_name => &typ.name,
        },
    ));

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
        out.push_str(&template_env::render(
            "rust_struct_field.jinja",
            minijinja::context! {
                name => &field.name,
                type => &field_type,
            },
        ));
    }

    out.push_str("}\n");
    out
}

/// Generate a Rustler config constructor impl for a type with `has_default`.
/// Fields in `exclude_fields` are skipped in both the struct and the constructor.
pub(super) fn gen_rustler_config_impl(
    typ: &TypeDef,
    mapper: &crate::type_map::RustlerMapper,
    exclude_fields: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "rust_impl_header.jinja",
        minijinja::context! {
            struct_name => &typ.name,
        },
    ));

    // Convert AHashSet to std HashSet for config_gen API
    let excl_std: std::collections::HashSet<String> = exclude_fields.iter().cloned().collect();
    let map_fn = |ty: &TypeRef| mapper.map_type(ty);
    let config_method = alef_codegen::config_gen::gen_rustler_kwargs_constructor_with_exclude(typ, &map_fn, &excl_std);
    out.push_str(config_method.trim_start());

    out.push_str("}\n");
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
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            field_type_for_rustler_inner(k),
            field_type_for_rustler_inner(v)
        ),
        TypeRef::Optional(inner) => format!("Option<{}>", field_type_for_rustler_inner(inner)),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Return the serde wire name for a variant, applying serde_rename_all if set.
fn variant_wire_name(variant: &alef_core::ir::EnumVariant, enum_def: &EnumDef) -> String {
    if let Some(rename) = &variant.serde_rename {
        return rename.clone();
    }
    match enum_def.serde_rename_all.as_deref() {
        Some("snake_case") => heck::AsSnakeCase(variant.name.as_str()).to_string(),
        Some("camelCase") => heck::AsLowerCamelCase(variant.name.as_str()).to_string(),
        Some("PascalCase") | Some("UpperCamelCase") => variant.name.clone(),
        Some("SCREAMING_SNAKE_CASE") => heck::AsShoutySnakeCase(variant.name.as_str()).to_string(),
        Some("kebab-case") => heck::AsKebabCase(variant.name.as_str()).to_string(),
        _ => variant.name.clone(),
    }
}

/// Generate a Rustler flat struct enum for data enums with tuple fields containing Named types.
///
/// Instead of NifTaggedEnum with `{:Variant, inner_data}` tuples, this generates:
/// - A flat NifStruct with all variant inner types as optional fields
/// - A discriminator string field for the variant type
/// - From impls that populate the appropriate field and set the discriminator
///
/// Example: FormatMetadata enum with Excel(ExcelMetadata) → struct with
/// `format_type: String` and `excel: Option<ExcelMetadata>`, with other optional fields.
fn gen_rustler_flat_data_enum(enum_def: &EnumDef, module_prefix: &str) -> String {
    let name = &enum_def.name;
    let mut out = String::with_capacity(1024);

    // Derive line for the struct — no Default here; explicit impl Default below
    out.push_str(&template_env::render("flat_enum_derive.jinja", minijinja::context! {}));
    out.push_str(&template_env::render(
        "flat_enum_struct_header.jinja",
        minijinja::context! {
            module_prefix => module_prefix,
            name => name,
        },
    ));

    // Add discriminator field (use serde tag name if available, else "type")
    let discriminator_field = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    out.push_str(&template_env::render(
        "flat_enum_discriminator_field.jinja",
        minijinja::context! {
            discriminator_field => discriminator_field,
        },
    ));

    // For each variant with tuple data, add an Optional field with that type.
    // Use snake_case field names based on variant names.
    for variant in &enum_def.variants {
        if !variant.fields.is_empty() && variant.is_tuple {
            // Tuple variant: field is the first (and typically only) inner type
            if let Some(first_field) = variant.fields.first() {
                let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
                let field_type = field_type_for_rustler(first_field);
                out.push_str(&template_env::render(
                    "flat_enum_variant_field.jinja",
                    minijinja::context! {
                        field_name => &field_name,
                        field_type => &field_type,
                    },
                ));
            }
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_struct_footer.jinja",
        minijinja::context! {},
    ));

    // Add Default impl
    out.push_str(&template_env::render(
        "flat_enum_default_impl.jinja",
        minijinja::context! {
            name => name,
            discriminator_field => discriminator_field,
        },
    ));

    for variant in &enum_def.variants {
        if !variant.fields.is_empty() && variant.is_tuple {
            let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
            out.push_str(&template_env::render(
                "flat_enum_default_variant_field.jinja",
                minijinja::context! {
                    field_name => &field_name,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_default_impl_footer.jinja",
        minijinja::context! {},
    ));

    out
}

/// Generate a `From<core::EnumName> for FlatStruct` impl for flat data enums.
///
/// The generic `gen_enum_from_core_to_binding_cfg` generates enum→enum arm matching, which
/// does not apply to flat structs. This function generates the correct struct-init form:
///
/// ```text
/// impl From<core::FormatMetadata> for FormatMetadata {
///     fn from(val: core::FormatMetadata) -> Self {
///         match val {
///             core::FormatMetadata::Excel(_0) => Self {
///                 format_type: "Excel".to_string(), excel: Some(_0.into()), ..Default::default()
///             },
///             ...
///         }
///     }
/// }
/// ```
pub(super) fn gen_rustler_flat_data_enum_from_core(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl.jinja",
        minijinja::context! {
            core_path => &core_path,
            name => name,
        },
    ));

    for variant in &enum_def.variants {
        let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
        let wire_name = variant_wire_name(variant, enum_def);

        if variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_unit.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                },
            ));
        } else if variant.is_tuple {
            let first_field = variant.fields.first().unwrap();
            let is_boxed = first_field.is_boxed;
            let is_sanitized_to_string = first_field.sanitized && matches!(first_field.ty, TypeRef::String);
            // Vec<Named>: blanket From<Vec<core::T>> for Vec<T> doesn't exist; map element-wise.
            let is_vec_of_named =
                matches!(&first_field.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)));
            let data_expr: String = if is_sanitized_to_string {
                if is_boxed {
                    "format!(\"{:?}\", *_0)".to_string()
                } else {
                    "format!(\"{:?}\", _0)".to_string()
                }
            } else if is_vec_of_named {
                if is_boxed {
                    "(*_0).into_iter().map(Into::into).collect()".to_string()
                } else {
                    "_0.into_iter().map(Into::into).collect()".to_string()
                }
            } else if is_boxed {
                "(*_0).into()".to_string()
            } else {
                "_0.into()".to_string()
            };
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_tuple.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                    fname => &field_name,
                    expr => &data_expr,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl_footer.jinja",
        minijinja::context! {},
    ));

    out
}

/// Generate the binding-to-core direction for a flat data enum.
///
/// Local representation is a struct with a discriminator field plus one optional payload
/// field per variant. Dispatches on the discriminator string to produce the matching core
/// enum variant, threading per-payload conversions (`.into()`, or iter-map for `Vec<Named>`).
pub(super) fn gen_rustler_flat_data_enum_to_core(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_to_core_impl_header.jinja",
        minijinja::context! {
            name => name,
            core_path => &core_path,
            discriminator => discriminator,
        },
    ));

    for variant in &enum_def.variants {
        let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
        let wire_name = variant_wire_name(variant, enum_def);

        if variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "flat_enum_to_core_variant_unit.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    core_path => &core_path,
                    variant_name => &variant.name,
                },
            ));
        } else if variant.is_tuple {
            let first_field = variant.fields.first().unwrap();
            let is_boxed = first_field.is_boxed;
            let is_sanitized_to_string = first_field.sanitized && matches!(first_field.ty, TypeRef::String);
            let is_vec_of_named =
                matches!(&first_field.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)));
            // Each variant's payload is stored as `Option<T>` on the local struct;
            // `.unwrap_or_default()` falls back to T::default() when the discriminator matches
            // but the payload is missing (defensive against malformed input).
            let payload_expr = if is_sanitized_to_string {
                "Default::default()".to_string()
            } else if is_vec_of_named {
                format!("val.{field_name}.unwrap_or_default().into_iter().map(Into::into).collect()")
            } else {
                format!("val.{field_name}.unwrap_or_default().into()")
            };
            let payload_expr = if is_boxed {
                format!("Box::new({payload_expr})")
            } else {
                payload_expr
            };
            out.push_str(&template_env::render(
                "flat_enum_to_core_variant_tuple.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    core_path => &core_path,
                    variant_name => &variant.name,
                    payload_expr => &payload_expr,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_to_core_impl_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Generate a Rustler NIF enum definition.
///
/// Unit enums (all variants have no fields) use `NifUnitEnum` — they encode as atoms.
/// Data enums where all tuple variants contain Named types (structs) use flat NifStruct
/// with optional fields. Other data enums use `NifTaggedEnum`.
pub(super) fn gen_enum(enum_def: &EnumDef, module_prefix: &str) -> String {
    let name = &enum_def.name;
    let mut out = String::with_capacity(512);

    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Use the flat struct approach for any data enum where every data variant is a
    // tuple variant (single unnamed field). This covers Named-inner types (e.g.
    // Excel(ExcelMetadata)) as well as primitive-inner types (e.g. Pdf(String)).
    let use_flat_struct = has_data
        && enum_def
            .variants
            .iter()
            .filter(|v| !v.fields.is_empty())
            .all(|v| v.is_tuple);

    if use_flat_struct {
        // Use flat struct approach for better Elixir field access
        return gen_rustler_flat_data_enum(enum_def, module_prefix);
    }

    if has_data {
        // NifTaggedEnum: supports unit variants (atoms) and struct variants (tagged tuples).
        // Cannot be Copy when variants have fields.
        out.push_str("#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rustler::NifTaggedEnum)]\n");
        if let Some(tag) = &enum_def.serde_tag {
            out.push_str(&template_env::render(
                "nif_tagged_enum_serde_tag.jinja",
                minijinja::context! {
                    tag => tag,
                },
            ));
        }
        out.push_str(&template_env::render(
            "nif_tagged_enum_header.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        for variant in &enum_def.variants {
            if variant.fields.is_empty() {
                out.push_str(&template_env::render(
                    "nif_tagged_enum_variant_unit.jinja",
                    minijinja::context! {
                        variant_name => &variant.name,
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "nif_tagged_enum_variant_struct_header.jinja",
                    minijinja::context! {
                        variant_name => &variant.name,
                    },
                ));
                let fields: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| format!("{}: {},", f.name, field_type_for_rustler(f)))
                    .collect();
                for field_line in &fields {
                    out.push_str(&template_env::render(
                        "nif_tagged_enum_variant_field_line.jinja",
                        minijinja::context! {
                            field_line => field_line,
                        },
                    ));
                }
                out.push_str(&template_env::render(
                    "nif_tagged_enum_variant_struct_footer.jinja",
                    minijinja::context! {},
                ));
            }
        }
        out.push_str("}\n");
    } else {
        // All unit variants: NifUnitEnum encodes as atoms.
        out.push_str(&template_env::render(
            "nif_unit_enum_header.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        for variant in &enum_def.variants {
            out.push_str(&template_env::render(
                "nif_enum_variant.jinja",
                minijinja::context! {
                    variant_name => &variant.name,
                },
            ));
        }
        out.push_str("}\n");
    }

    // Default impl: use the variant marked is_default=true; fall back to the first variant.
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.is_default)
        .or_else(|| enum_def.variants.first());
    if let Some(dv) = default_variant {
        out.push_str(&template_env::render(
            "nif_enum_default_header.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        if has_data && !dv.fields.is_empty() {
            let field_defaults: Vec<String> = dv
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            out.push_str(&template_env::render(
                "nif_enum_default_with_fields.jinja",
                minijinja::context! {
                    variant_name => &dv.name,
                    field_defaults => field_defaults.join(", "),
                },
            ));
        } else {
            out.push_str(&template_env::render(
                "nif_enum_default_value.jinja",
                minijinja::context! {
                    variant_name => &dv.name,
                },
            ));
        }
        out.push_str(&template_env::render(
            "nif_enum_default_footer.jinja",
            minijinja::context! {},
        ));
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
        // Bytes (Vec<u8>): core may return `bytes::Bytes` or `Vec<u8>` or `&[u8]`.
        // `.to_vec()` works for all three: owned Bytes→Vec<u8>, owned Vec<u8>→no-op clone,
        // and &[u8]→Vec<u8>. This is safe to apply unconditionally.
        TypeRef::Bytes => format!("{expr}.to_vec()"),
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
            // Core returns &[&str]/&[&char] — materialize to owned Vec<String>/Vec<char>.
            TypeRef::String | TypeRef::Char if returns_ref => {
                format!("{expr}.iter().map(|s| s.to_string()).collect()")
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
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
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
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
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
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
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
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
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
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    /// Unit enums must still lower to NifUnitEnum (atoms on the Elixir side).
    #[test]
    fn test_gen_enum_unit_uses_nif_unit_enum() {
        let result = gen_enum(&unit_enum(), "Kreuzberg");
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
        let result = gen_enum(&data_enum(), "Kreuzberg");
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

    /// Data enums with tuple variants containing Named types should use flat NifStruct.
    #[test]
    fn test_gen_enum_tuple_named_uses_nif_struct() {
        // Create a data enum with tuple variants containing Named types (like FormatMetadata)
        let format_enum = EnumDef {
            name: "FormatMetadata".to_string(),
            rust_path: "my_crate::FormatMetadata".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Excel".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Named("ExcelMetadata".into()),
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
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Pdf".into(),
                    fields: vec![FieldDef {
                        name: "_0".into(),
                        ty: TypeRef::Named("String".into()),
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
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: Some("format_type".into()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let result = gen_enum(&format_enum, "Kreuzberg");
        // Should use NifStruct, not NifTaggedEnum
        assert!(
            result.contains("NifStruct"),
            "tuple data enum with named types should use NifStruct; got:\n{result}"
        );
        assert!(
            !result.contains("NifTaggedEnum"),
            "tuple data enum with named types must not use NifTaggedEnum; got:\n{result}"
        );
        // Should have format_type discriminator field
        assert!(
            result.contains("format_type: String"),
            "should have format_type discriminator; got:\n{result}"
        );
        // Should have optional fields for each variant
        assert!(
            result.contains("excel: Option<ExcelMetadata>"),
            "should have optional excel field; got:\n{result}"
        );
        assert!(
            result.contains("pdf: Option<String>"),
            "should have optional pdf field; got:\n{result}"
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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
