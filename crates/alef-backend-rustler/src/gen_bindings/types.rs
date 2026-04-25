use ahash::AHashSet;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, TypeDef, TypeRef};

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
pub(super) fn gen_struct(typ: &TypeDef, mapper: &crate::type_map::RustlerMapper, module_prefix: &str) -> String {
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
pub(super) fn gen_rustler_config_impl(typ: &TypeDef, mapper: &crate::type_map::RustlerMapper) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "impl {} {{", typ.name).ok();

    // Generate kwargs constructor using config_gen helper
    let map_fn = |ty: &TypeRef| mapper.map_type(ty);
    let config_method = alef_codegen::config_gen::gen_rustler_kwargs_constructor(typ, &map_fn);
    write!(out, "    {}", config_method).ok();

    writeln!(out, "}}").ok();
    out
}

/// Generate a Rustler NIF enum definition (unit enum).
pub(super) fn gen_enum(enum_def: &EnumDef) -> String {
    let mut lines = vec![
        "#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rustler::NifUnitEnum)]".to_string(),
        format!("pub enum {} {{", enum_def.name),
    ];

    for variant in &enum_def.variants {
        lines.push(format!("    {},", variant.name));
    }

    lines.push("}".to_string());

    // Default impl for config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {} {{", enum_def.name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", first.name));
        lines.push("}".to_string());
    }

    lines.join("\n")
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
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => format!("{expr}.into()"),
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
        _ => expr.to_string(),
    }
}
