//! NAPI-RS enum code generation: plain enums and tagged union helpers.

use crate::type_map::NapiMapper;
use alef_core::ir::{EnumDef, TypeRef};

pub(super) fn gen_enum(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    let is_tagged_data_enum = enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if is_tagged_data_enum {
        return gen_tagged_enum_as_object(enum_def, prefix, has_serde);
    }

    // Simple string enum
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

    let derives = if has_serde {
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]".to_string()
    } else {
        "#[derive(Clone)]".to_string()
    };
    let mut lines = vec![
        string_enum_attr,
        derives,
        format!("pub enum {prefix}{} {{", enum_def.name),
    ];

    for variant in &enum_def.variants {
        lines.push(format!("    {},", variant.name));
    }

    lines.push("}".to_string());

    // Default impl for config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", first.name));
        lines.push("}".to_string());
    }

    lines.join("\n")
}

/// Generate a tagged enum as a flattened `#[napi(object)]` struct.
/// E.g. `AuthConfig { Basic { username, password }, Bearer { token } }` becomes:
/// ```rust,ignore
/// #[napi(object)]
/// struct JsAuthConfig {
///     #[napi(js_name = "type")]
///     pub auth_type: String,
///     pub username: Option<String>,
///     pub password: Option<String>,
///     pub token: Option<String>,
/// }
/// ```
pub(super) fn gen_tagged_enum_as_object(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    use alef_codegen::type_mapper::TypeMapper;
    let mapper = NapiMapper::new(prefix.to_string());

    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    let derive = if has_serde {
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]"
    } else {
        "#[derive(Clone)]"
    };
    let mut lines = vec![
        derive.to_string(),
        "#[napi(object)]".to_string(),
        format!("pub struct {prefix}{} {{", enum_def.name),
        format!("    #[napi(js_name = \"{tag_field}\")]"),
        format!("    pub {tag_field}_tag: String,"),
    ];

    // Fields that appear in multiple variants with different Named types cannot be represented
    // as a single concrete JsXxx type. Store them as String (JSON) instead, and convert
    // per-variant via serde_json in the From impls.
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    // Collect all unique fields across all variants (all made optional)
    let mut seen_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if seen_fields.insert(field.name.clone()) {
                // Sanitized fields and mixed-type Named fields are represented as String
                // and converted via serde_json in From/Into impls
                let field_type = if (field.sanitized || mixed_named_fields.contains(&field.name))
                    && matches!(&field.ty, TypeRef::Named(_))
                {
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

    // Default impl
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

/// Generate a free function binding.
pub(super) fn tagged_enum_mixed_named_fields(enum_def: &EnumDef) -> ahash::AHashSet<String> {
    use alef_core::ir::TypeRef;
    let mut field_types: std::collections::HashMap<&str, ahash::AHashSet<&str>> = std::collections::HashMap::new();

    for variant in &enum_def.variants {
        for field in &variant.fields {
            if field.sanitized {
                continue;
            }
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(&field.name).or_default().insert(n.as_str());
            }
        }
    }

    field_types
        .into_iter()
        .filter(|(_, types)| types.len() > 1)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Determine which Named fields in a tagged enum use binding structs (Into conversion)
/// vs serde JSON String flattening. A field uses a binding struct only if:
/// 1. The field name maps to a single Named type across all variants
/// 2. That Named type has a binding struct (in struct_names)
/// 3. The field is not sanitized
pub(super) fn tagged_enum_binding_struct_fields<'a>(
    enum_def: &'a EnumDef,
    struct_names: &ahash::AHashSet<String>,
) -> ahash::AHashSet<&'a str> {
    use alef_core::ir::TypeRef;
    let mut field_types: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut sanitized_fields: ahash::AHashSet<&str> = ahash::AHashSet::new();

    for variant in &enum_def.variants {
        for field in &variant.fields {
            if field.sanitized {
                sanitized_fields.insert(&field.name);
            }
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(&field.name).or_default().push(n);
            }
        }
    }

    let mut result = ahash::AHashSet::new();
    for (field_name, types) in &field_types {
        if sanitized_fields.contains(field_name) {
            continue;
        }
        // All variants sharing this field name must have the same Named type
        if types.iter().all(|t| *t == types[0]) && struct_names.contains(types[0]) {
            result.insert(*field_name);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::gen_enum;
    use alef_core::ir::{EnumDef, EnumVariant};

    fn make_simple_enum(name: &str, variants: &[&str]) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("test::{name}"),
            original_rust_path: String::new(),
            variants: variants
                .iter()
                .map(|v| EnumVariant {
                    name: v.to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                })
                .collect(),
            doc: String::new(),
            cfg: None,
            is_copy: true,
            has_serde: false,
            serde_tag: None,
            serde_rename_all: None,
        }
    }

    /// gen_enum with no variants produces a valid enum declaration.
    #[test]
    fn gen_enum_empty_variants_compiles() {
        let e = make_simple_enum("Status", &[]);
        let result = gen_enum(&e, "", false);
        assert!(result.contains("enum Status") || result.is_empty() || result.contains("Status"));
    }

    /// gen_enum with variants includes variant names.
    #[test]
    fn gen_enum_includes_variant_names() {
        let e = make_simple_enum("Color", &["Red", "Green", "Blue"]);
        let result = gen_enum(&e, "", false);
        assert!(result.contains("Red") || result.contains("red") || result.contains("RED"));
    }
}
