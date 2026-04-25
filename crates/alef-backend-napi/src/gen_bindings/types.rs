use crate::type_map::NapiMapper;
use alef_codegen::builder::StructBuilder;
use alef_codegen::naming::to_node_name;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, TypeDef, TypeRef};
use std::fmt::Write;
use super::helpers::needs_napi_cast;

pub(super) fn gen_struct(typ: &TypeDef, mapper: &NapiMapper, prefix: &str, has_serde: bool) -> String {
    let mut struct_builder = StructBuilder::new(&format!("{prefix}{}", typ.name));
    struct_builder.add_attr("napi(object)");
    struct_builder.add_derive("Clone");
    struct_builder.add_derive("Default");
    if has_serde {
        struct_builder.add_derive("serde::Serialize");
        struct_builder.add_derive("serde::Deserialize");
    }
    for field in &typ.fields {
        let mapped_type = mapper.map_type(&field.ty);
        let field_type = if (field.optional || typ.has_default) && !matches!(field.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapped_type)
        } else {
            mapped_type
        };
        let js_name = to_node_name(&field.name);
        let attrs = if js_name != field.name {
            vec![format!("napi(js_name = \"{}\")", js_name)]
        } else {
            vec![]
        };
        struct_builder.add_field(&field.name, &field_type, attrs);
    }
    struct_builder.build()
}

pub(super) fn gen_enum(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    let is_tagged_data_enum = enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty());
    if is_tagged_data_enum {
        return gen_tagged_enum_as_object(enum_def, prefix, has_serde);
    }
    let napi_case = enum_def.serde_rename_all.as_deref().and_then(|s| match s {
        "snake_case" => Some("snake_case"),
        "camelCase" => Some("camelCase"),
        "kebab-case" => Some("kebab-case"),
        "SCREAMING_SNAKE_CASE" => Some("UPPER_SNAKE"),
        "lowercase" => Some("lowercase"),
        "UPPERCASE" => Some("UPPERCASE"),
        "PascalCase" => Some("PascalCase"),
        _ => None,
    });
    let string_enum_attr = match napi_case {
        Some(case) => format!("#[napi(string_enum = \"{case}\")]"),
        None => "#[napi(string_enum)]".to_string(),
    };
    let derives = if has_serde { "#[derive(Clone, serde::Serialize, serde::Deserialize)]".to_string() } else { "#[derive(Clone)]".to_string() };
    let mut lines = vec![
        string_enum_attr,
        derives,
        format!("pub enum {prefix}{} {{", enum_def.name),
    ];
    for variant in &enum_def.variants {
        lines.push(format!("    {},", variant.name));
    }
    lines.push("}".to_string());
    if let Some(first) = enum_def.variants.first() {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", first.name));
        lines.push("}".to_string());
    }
    lines.join("\n")
}

fn gen_tagged_enum_as_object(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    let mapper = NapiMapper::new(prefix.to_string());
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let derive = if has_serde { "#[derive(Clone, serde::Serialize, serde::Deserialize)]" } else { "#[derive(Clone)]" };
    let mut lines = vec![
        derive.to_string(),
        "#[napi(object)]".to_string(),
        format!("pub struct {prefix}{} {{", enum_def.name),
        format!("    #[napi(js_name = \"{tag_field}\")]"),
        format!("    pub {tag_field}_tag: String,"),
    ];
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);
    let mut seen_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if seen_fields.insert(field.name.clone()) {
                let field_type = if (field.sanitized || mixed_named_fields.contains(&field.name)) && matches!(&field.ty, TypeRef::Named(_)) {
                    "String".to_string()
                } else {
                    mapper.map_type(&field.ty).to_string()
                };
                let js_name = alef_codegen::naming::to_node_name(&field.name);
                if js_name != field.name {
                    lines.push(format!("    #[napi(js_name = \"{js_name}\")]"));
                }
                lines.push(format!("    pub {}: Option<{field_type}>,", field.name));
            }
        }
    }
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("#[allow(clippy::derivable_impls)]".to_string());
    lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
    lines.push(format!(
        "    fn default() -> Self {{ Self {{ {tag_field}_tag: String::new(), {} }} }}",
        seen_fields
            .iter()
            .map(|f| format!("{f}: None"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.push("}".to_string());
    lines.join("\n")
}

pub(super) fn gen_tagged_enum_binding_to_core(enum_def: &EnumDef, core_import: &str, prefix: &str, struct_names: &ahash::AHashSet<String>) -> String {
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);
    let mut out = String::with_capacity(512);
    writeln!(out, "impl From<{binding_name}> for {core_path} {{").ok();
    writeln!(out, "    fn from(val: {binding_name}) -> Self {{").ok();
    writeln!(out, "        match val.{tag_field}_tag.as_str() {{").ok();
    for variant in &enum_def.variants {
        let default_tag = variant.name.to_lowercase();
        let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
        if variant.fields.is_empty() {
            writeln!(out, "            \"{tag_value}\" => Self::{},", variant.name).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            let field_exprs: Vec<String> = variant.fields.iter().map(|f| {
                let has_binding = fields_with_binding_struct.contains(f.name.as_str());
                let is_mixed = mixed_named_fields.contains(&f.name);
                if f.optional {
                    match &f.ty {
                        TypeRef::Path => format!("val.{}.map(std::path::PathBuf::from)", f.name),
                        TypeRef::Named(n) if is_mixed => { let core_type = format!("{core_import}::{n}"); format!("val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok())", f.name) },
                        TypeRef::Named(_) if has_binding => format!("val.{}.map(|v| v.into())", f.name),
                        TypeRef::Named(_) => format!("val.{}.map(|v| v.into())", f.name),
                        TypeRef::Primitive(p) if needs_napi_cast(p) => { let core_ty = super::helpers::core_prim_str(p); format!("val.{}.map(|v| v as {core_ty})", f.name) },
                        _ => format!("val.{}", f.name),
                    }
                } else if f.sanitized {
                    let expr = "Default::default()".to_string();
                    if f.is_boxed { format!("Box::new({expr})") } else { expr }
                } else {
                    let expr = match &f.ty {
                        TypeRef::Named(n) if is_mixed => { let core_type = format!("{core_import}::{n}"); format!("val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok()).unwrap_or_default()", f.name) },
                        TypeRef::Named(_) if has_binding => format!("val.{}.map(|v| v.into()).unwrap_or_default()", f.name),
                        TypeRef::Named(_) => format!("val.{}.map(|v| v.into()).unwrap_or_default()", f.name),
                        TypeRef::Path => format!("val.{}.map(std::path::PathBuf::from).unwrap_or_default()", f.name),
                        TypeRef::Primitive(p) if needs_napi_cast(p) => { let core_ty = super::helpers::core_prim_str(p); format!("val.{}.map(|v| v as {core_ty}).unwrap_or_default()", f.name) },
                        _ => format!("val.{}.unwrap_or_default()", f.name),
                    };
                    if f.is_boxed { format!("Box::new({expr})") } else { expr }
                }
            }).collect();
            if is_tuple {
                writeln!(out, "            \"{tag_value}\" => Self::{}({}),", variant.name, field_exprs.join(", ")).ok();
            } else {
                let field_inits: Vec<String> = variant.fields.iter().zip(field_exprs.iter()).map(|(f, expr)| format!("{}: {expr}", f.name)).collect();
                writeln!(out, "            \"{tag_value}\" => Self::{} {{ {} }},", variant.name, field_inits.join(", ")).ok();
            }
        }
    }
    if let Some(first) = enum_def.variants.first() {
        if first.fields.is_empty() {
            writeln!(out, "            _ => Self::{},", first.name).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&first.fields);
            if is_tuple {
                let defaults: Vec<&str> = first.fields.iter().map(|_| "Default::default()").collect();
                writeln!(out, "            _ => Self::{}({}),", first.name, defaults.join(", ")).ok();
            } else {
                let defaults: Vec<String> = first.fields.iter().map(|f| format!("{}: Default::default()", f.name)).collect();
                writeln!(out, "            _ => Self::{} {{ {} }},", first.name, defaults.join(", ")).ok();
            }
        }
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

pub(super) fn gen_tagged_enum_core_to_binding(enum_def: &EnumDef, core_import: &str, prefix: &str, struct_names: &ahash::AHashSet<String>) -> String {
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);
    let all_fields: Vec<String> = {
        let mut fields = std::collections::BTreeSet::new();
        for v in &enum_def.variants {
            for f in &v.fields {
                fields.insert(f.name.clone());
            }
        }
        fields.into_iter().collect()
    };
    let mut out = String::with_capacity(512);
    writeln!(out, "impl From<{core_path}> for {binding_name} {{").ok();
    writeln!(out, "    fn from(val: {core_path}) -> Self {{").ok();
    writeln!(out, "        match val {{").ok();
    for variant in &enum_def.variants {
        let default_tag = variant.name.to_lowercase();
        let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
        if variant.fields.is_empty() {
            writeln!(out, "            {core_path}::{} => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},", variant.name, all_fields.iter().map(|f| format!("{f}: None")).collect::<Vec<_>>().join(", ")).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            let variant_field_map: std::collections::BTreeMap<&str, &alef_core::ir::FieldDef> = variant.fields.iter().map(|f| (f.name.as_str(), f)).collect();
            let destructured: Vec<String> = variant.fields.iter().map(|f| { if f.sanitized { if is_tuple { format!("_{}", f.name) } else { format!("{}: _{}", f.name, f.name) } } else { f.name.clone() } }).collect();
            let field_inits: Vec<String> = all_fields.iter().map(|f| {
                if let Some(field) = variant_field_map.get(f.as_str()) {
                    let has_binding = fields_with_binding_struct.contains(f.as_str());
                    let is_mixed = mixed_named_fields.contains(f.as_str());
                    if field.optional {
                        match &field.ty {
                            TypeRef::Path => format!("{f}: {f}.map(|p| p.to_string_lossy().to_string())"),
                            TypeRef::Named(_) if is_mixed => format!("{f}: {f}.and_then(|v| serde_json::to_string(&v).ok())"),
                            TypeRef::Named(_) if has_binding => format!("{f}: {f}.map(|v| v.into())"),
                            TypeRef::Named(_) => format!("{f}: {f}.map(|v| v.into())"),
                            _ => format!("{f}: {f}"),
                        }
                    } else if field.sanitized {
                        format!("{f}: None")
                    } else {
                        match &field.ty {
                            TypeRef::Named(_) if is_mixed => format!("{f}: serde_json::to_string(&{f}).ok()"),
                            TypeRef::Named(_) if has_binding => format!("{f}: Some({f}.into())"),
                            TypeRef::Named(_) => format!("{f}: Some({f}.into())"),
                            TypeRef::Path => format!("{f}: Some({f}.to_string_lossy().to_string())"),
                            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                match p {
                                    alef_core::ir::PrimitiveType::F32 => format!("{f}: Some({f} as f64)"),
                                    alef_core::ir::PrimitiveType::U64 | alef_core::ir::PrimitiveType::Usize | alef_core::ir::PrimitiveType::Isize => format!("{f}: Some({f} as i64)"),
                                    _ => format!("{f}: Some({f})"),
                                }
                            },
                            _ => format!("{f}: Some({f})"),
                        }
                    }
                } else {
                    format!("{f}: None")
                }
            }).collect();
            if is_tuple {
                writeln!(out, "            {core_path}::{}({}) => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},", variant.name, destructured.join(", "), field_inits.join(", ")).ok();
            } else {
                writeln!(out, "            {core_path}::{} {{ {} }} => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},", variant.name, destructured.join(", "), field_inits.join(", ")).ok();
            }
        }
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

fn tagged_enum_mixed_named_fields(enum_def: &EnumDef) -> ahash::AHashSet<String> {
    let mut field_types: std::collections::HashMap<&str, ahash::AHashSet<&str>> = std::collections::HashMap::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if field.sanitized { continue; }
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(&field.name).or_default().insert(n.as_str());
            }
        }
    }
    field_types.into_iter().filter(|(_, types)| types.len() > 1).map(|(name, _)| name.to_string()).collect()
}

fn tagged_enum_binding_struct_fields<'a>(enum_def: &'a EnumDef, struct_names: &ahash::AHashSet<String>) -> ahash::AHashSet<&'a str> {
    let mut field_types: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut sanitized_fields: ahash::AHashSet<&str> = ahash::AHashSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if field.sanitized { sanitized_fields.insert(&field.name); }
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(&field.name).or_default().push(n);
            }
        }
    }
    let mut result = ahash::AHashSet::new();
    for (field_name, types) in &field_types {
        if sanitized_fields.contains(field_name) { continue; }
        if types.iter().all(|t| *t == types[0]) && struct_names.contains(types[0]) {
            result.insert(*field_name);
        }
    }
    result
}
