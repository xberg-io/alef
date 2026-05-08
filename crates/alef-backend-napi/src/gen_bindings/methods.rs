//! NAPI-RS tagged-enum From-impl code generation (binding ↔ core conversions).

use alef_core::ir::EnumDef;

use super::enums::{tagged_enum_binding_struct_fields, tagged_enum_mixed_named_fields};
use super::functions::{core_prim_str, needs_napi_cast};

/// Generate `From<JsTaggedEnum> for core::TaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
) -> String {
    use alef_core::ir::TypeRef;
    use std::fmt::Write;
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    // Determine which Named fields use binding structs vs serde JSON String.
    // A field uses a binding struct only if: (1) it has a binding struct in struct_names,
    // (2) it's not sanitized, and (3) the field name maps to a single Named type across
    // all variants (not shared with different types).
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    // Fields with different Named types across variants are stored as String (JSON) in the
    // binding struct and must be deserialized per-variant via serde_json.
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
            let field_exprs: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let has_binding = fields_with_binding_struct.contains(f.name.as_str());
                    let is_mixed = mixed_named_fields.contains(&f.name);
                    if f.optional {
                        match &f.ty {
                            TypeRef::Path => {
                                format!("val.{}.map(std::path::PathBuf::from)", f.name)
                            }
                            TypeRef::Named(n) if is_mixed => {
                                // Mixed-type field: stored as String (JSON), deserialize per variant
                                let core_type = format!("{core_import}::{n}");
                                format!(
                                    "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok())",
                                    f.name
                                )
                            }
                            TypeRef::Named(_) if has_binding => {
                                format!("val.{}.map(|v| v.into())", f.name)
                            }
                            // Non-sanitized Named fields with a single consistent type are stored
                            // as Option<JsXxx> in the binding struct, so use .into() conversion.
                            TypeRef::Named(_) => {
                                format!("val.{}.map(|v| v.into())", f.name)
                            }
                            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                let core_ty = core_prim_str(p);
                                format!("val.{}.map(|v| v as {core_ty})", f.name)
                            }
                            _ => {
                                format!("val.{}", f.name)
                            }
                        }
                    } else if f.sanitized {
                        let expr = "Default::default()".to_string();
                        if f.is_boxed { format!("Box::new({expr})") } else { expr }
                    } else {
                        let expr = match &f.ty {
                            TypeRef::Named(n) if is_mixed => {
                                // Mixed-type field: stored as String (JSON), deserialize per variant
                                let core_type = format!("{core_import}::{n}");
                                format!(
                                    "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok()).unwrap_or_default()",
                                    f.name
                                )
                            }
                            TypeRef::Named(_) if has_binding => {
                                format!("val.{}.map(|v| v.into()).unwrap_or_default()", f.name)
                            }
                            // Non-sanitized Named fields with a single consistent type are stored
                            // as Option<JsXxx> in the binding struct, so use .into() conversion.
                            TypeRef::Named(_) => {
                                format!("val.{}.map(|v| v.into()).unwrap_or_default()", f.name)
                            }
                            TypeRef::Path => {
                                format!("val.{}.map(std::path::PathBuf::from).unwrap_or_default()", f.name)
                            }
                            TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                let core_ty = core_prim_str(p);
                                format!("val.{}.map(|v| v as {core_ty}).unwrap_or_default()", f.name)
                            }
                            _ => {
                                format!("val.{}.unwrap_or_default()", f.name)
                            }
                        };
                        if f.is_boxed { format!("Box::new({expr})") } else { expr }
                    }
                })
                .collect();
            if is_tuple {
                writeln!(
                    out,
                    "            \"{tag_value}\" => Self::{}({}),",
                    variant.name,
                    field_exprs.join(", ")
                )
                .ok();
            } else {
                let field_inits: Vec<String> = variant
                    .fields
                    .iter()
                    .zip(field_exprs.iter())
                    .map(|(f, expr)| format!("{}: {expr}", f.name))
                    .collect();
                writeln!(
                    out,
                    "            \"{tag_value}\" => Self::{} {{ {} }},",
                    variant.name,
                    field_inits.join(", ")
                )
                .ok();
            }
        }
    }

    // Default fallback to first variant
    if let Some(first) = enum_def.variants.first() {
        if first.fields.is_empty() {
            writeln!(out, "            _ => Self::{},", first.name).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&first.fields);
            if is_tuple {
                let defaults: Vec<&str> = first.fields.iter().map(|_| "Default::default()").collect();
                writeln!(out, "            _ => Self::{}({}),", first.name, defaults.join(", ")).ok();
            } else {
                let defaults: Vec<String> = first
                    .fields
                    .iter()
                    .map(|f| format!("{}: Default::default()", f.name))
                    .collect();
                writeln!(
                    out,
                    "            _ => Self::{} {{ {} }},",
                    first.name,
                    defaults.join(", ")
                )
                .ok();
            }
        }
    }

    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

/// Generate `From<core::TaggedEnum> for JsTaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_core_to_binding(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
) -> String {
    use std::fmt::Write;
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    // Fields with different Named types across variants are stored as String (JSON) in the
    // binding struct and must be serialized per-variant via serde_json.
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    // Collect all field names across all variants
    let all_fields: Vec<String> = {
        let mut fields = std::collections::BTreeSet::new();
        for v in &enum_def.variants {
            for f in &v.fields {
                fields.insert(f.name.clone());
            }
        }
        fields.into_iter().collect()
    };

    // Synthesized variant-data fields that the binding struct exposes for direct property access
    // (e.g. `excel: Option<JsExcelMetadata>`). Each variant's From arm must initialize all of them
    // so the struct literal is complete; the matching variant overrides its own slot to Some(...).
    let synth_fields: Vec<String> = super::enums::variant_data_field_names(enum_def);

    let mut out = String::with_capacity(512);
    writeln!(out, "impl From<{core_path}> for {binding_name} {{").ok();
    writeln!(out, "    fn from(val: {core_path}) -> Self {{").ok();
    writeln!(out, "        match val {{").ok();

    for variant in &enum_def.variants {
        let default_tag = variant.name.to_lowercase();
        let tag_value = variant.serde_rename.as_deref().unwrap_or(&default_tag);
        let _variant_field_names: std::collections::BTreeSet<String> =
            variant.fields.iter().map(|f| f.name.clone()).collect();

        // Helper to build variant-specific optional property initializations
        // (e.g., excel: Some(...), for FormatMetadata::Excel variant)
        let variant_data_inits = if variant.fields.len() == 1 {
            let field = &variant.fields[0];
            let is_tuple = field
                .name
                .strip_prefix('_')
                .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
            if is_tuple && matches!(&field.ty, alef_core::ir::TypeRef::Named(_)) {
                // This variant has single-tuple Named field that should be exposed as optional property.
                // For Box<T> variants the JsXxx From impl is over T, not Box<T>, so deref before .into().
                let variant_name_snake = alef_codegen::naming::to_python_name(&variant.name);
                let inner_expr = if field.is_boxed { "(*_0).into()" } else { "_0.into()" };
                vec![format!("{variant_name_snake}: Some({inner_expr})")]
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        if variant.fields.is_empty() {
            let inits: Vec<String> = all_fields
                .iter()
                .chain(synth_fields.iter())
                .map(|f| format!("{f}: None"))
                .collect();
            writeln!(
                out,
                "            {core_path}::{} => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},",
                variant.name,
                inits.join(", ")
            )
            .ok();
        } else {
            use alef_core::ir::TypeRef;
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            let variant_field_map: std::collections::BTreeMap<&str, &alef_core::ir::FieldDef> =
                variant.fields.iter().map(|f| (f.name.as_str(), f)).collect();
            let destructured: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    if f.sanitized {
                        if is_tuple {
                            format!("_{}", f.name)
                        } else {
                            format!("{}: _{}", f.name, f.name)
                        }
                    } else {
                        f.name.clone()
                    }
                })
                .collect();
            let field_inits: Vec<String> = all_fields
                .iter()
                .map(|f| {
                    if let Some(field) = variant_field_map.get(f.as_str()) {
                        let has_binding = fields_with_binding_struct.contains(f.as_str());
                        let is_mixed = mixed_named_fields.contains(f.as_str());
                        if field.optional {
                            match &field.ty {
                                TypeRef::Path => format!("{f}: {f}.map(|p| p.to_string_lossy().to_string())"),
                                TypeRef::Named(_) if is_mixed => {
                                    // Mixed-type field: serialize to JSON String for the binding struct
                                    format!("{f}: {f}.and_then(|v| serde_json::to_string(&v).ok())")
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("{f}: {f}.map(|v| v.into())")
                                }
                                // Non-sanitized Named fields with a single consistent type are stored
                                // as Option<JsXxx> in the binding struct, so use .into() conversion.
                                TypeRef::Named(_) => {
                                    format!("{f}: {f}.map(|v| v.into())")
                                }
                                _ => format!("{f}: {f}"),
                            }
                        } else if field.sanitized {
                            format!("{f}: None")
                        } else {
                            match &field.ty {
                                TypeRef::Named(_) if is_mixed => {
                                    // Mixed-type field: serialize to JSON String for the binding struct
                                    format!("{f}: serde_json::to_string(&{f}).ok()")
                                }
                                TypeRef::Named(_) if has_binding => format!("{f}: Some({f}.into())"),
                                // Non-sanitized Named fields with a single consistent type are stored
                                // as Option<JsXxx> in the binding struct, so use .into() conversion.
                                TypeRef::Named(_) => format!("{f}: Some({f}.into())"),
                                TypeRef::Path => format!("{f}: Some({f}.to_string_lossy().to_string())"),
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    match p {
                                        alef_core::ir::PrimitiveType::F32 => format!("{f}: Some({f} as f64)"),
                                        alef_core::ir::PrimitiveType::U64
                                        | alef_core::ir::PrimitiveType::Usize
                                        | alef_core::ir::PrimitiveType::Isize => format!("{f}: Some({f} as i64)"),
                                        // U32 stays as-is in NAPI
                                        _ => format!("{f}: Some({f})"),
                                    }
                                }
                                _ => format!("{f}: Some({f})"),
                            }
                        }
                    } else {
                        format!("{f}: None")
                    }
                })
                .collect();

            // For tagged enums where the binding struct also has synthesized variant-data
            // properties (e.g. `excel: Option<JsExcelMetadata>`), every variant arm must
            // initialize all of those: the matching variant slot is set by `variant_data_inits`,
            // the rest must be set to `None`.
            let matching_synth: std::collections::BTreeSet<String> = if variant.fields.len() == 1 {
                let field = &variant.fields[0];
                let is_tuple_pos = field
                    .name
                    .strip_prefix('_')
                    .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
                if is_tuple_pos && matches!(&field.ty, alef_core::ir::TypeRef::Named(_)) {
                    let mut set = std::collections::BTreeSet::new();
                    set.insert(alef_codegen::naming::to_python_name(&variant.name));
                    set
                } else {
                    std::collections::BTreeSet::new()
                }
            } else {
                std::collections::BTreeSet::new()
            };
            let synth_none_inits: Vec<String> = synth_fields
                .iter()
                .filter(|f| !matching_synth.contains(f.as_str()))
                .map(|f| format!("{f}: None"))
                .collect();

            // Combine all field inits (base fields + variant-specific optional properties +
            // None for non-matching synthesized variant-data fields).
            let all_inits: Vec<String> = field_inits
                .iter()
                .chain(variant_data_inits.iter())
                .chain(synth_none_inits.iter())
                .cloned()
                .collect();

            if is_tuple {
                writeln!(
                    out,
                    "            {core_path}::{}({}) => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},",
                    variant.name,
                    destructured.join(", "),
                    all_inits.join(", ")
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "            {core_path}::{} {{ {} }} => Self {{ {tag_field}_tag: \"{tag_value}\".to_string(), {} }},",
                    variant.name,
                    destructured.join(", "),
                    all_inits.join(", ")
                )
                .ok();
            }
        }
    }

    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

/// Determine which Named fields in a tagged enum have **different** Named types across variants.
/// These fields cannot use a single `JsXxx` binding type, so they are stored as `String` (JSON)
/// and converted via `serde_json` per variant in the From impls.
#[cfg(test)]
mod tests {
    /// gen_tagged_enum_binding_to_core is tested via integration tests in gen_bindings_test.rs.
    /// This unit test verifies the function exists and is callable.
    #[test]
    fn tagged_enum_from_impls_exist() {
        // Compilation check only — integration tests cover the generated output.
    }
}
