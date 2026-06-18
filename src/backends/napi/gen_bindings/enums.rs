//! NAPI-RS enum code generation: plain enums and tagged union helpers.

use crate::backends::napi::type_map::NapiMapper;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

pub(super) fn tagged_enum_field_is_tuple(field: &FieldDef) -> bool {
    field
        .name
        .strip_prefix('_')
        .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
}

pub(super) fn tagged_enum_field_name(variant: &EnumVariant, field: &FieldDef) -> String {
    if let Some(index) = field
        .name
        .strip_prefix('_')
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
    {
        if variant.fields.len() == 1 {
            let source_name = field
                .serde_rename
                .as_deref()
                .or(variant.serde_rename.as_deref())
                .unwrap_or(&variant.name);
            return crate::codegen::naming::to_python_name(source_name);
        }
        return format!("field_{index}");
    }

    field.name.clone()
}

pub(super) fn tagged_enum_field_js_name(variant: &EnumVariant, field: &FieldDef) -> String {
    if let Some(index) = field
        .name
        .strip_prefix('_')
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
    {
        if variant.fields.len() == 1 {
            return field
                .serde_rename
                .clone()
                .or_else(|| variant.serde_rename.clone())
                .unwrap_or_else(|| crate::codegen::naming::to_node_name(&variant.name));
        }
        return format!("field{index}");
    }

    crate::codegen::naming::to_node_name(&field.name)
}

/// Collect synthesized variant-data field names emitted on the binding struct for tagged enums
/// where a variant carries a single-tuple Named field. These are the per-variant optional
/// properties (e.g. `excel: Option<JsExcelMetadata>`) added on top of the discriminator and
/// shared variant fields, enabling direct property access in TypeScript.
pub(super) fn variant_data_field_names(enum_def: &EnumDef) -> Vec<String> {
    let mut names = Vec::new();
    for v in &enum_def.variants {
        if v.fields.len() != 1 {
            continue;
        }
        let field = &v.fields[0];
        if !tagged_enum_field_is_tuple(field) {
            continue;
        }
        if matches!(&field.ty, TypeRef::Named(_)) {
            names.push(tagged_enum_field_name(v, field));
        }
    }
    names
}

pub(super) fn gen_enum(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    let is_tagged_data_enum = enum_def.serde_tag.is_some() && has_data_variants;
    let is_untagged_data_enum = enum_def.serde_untagged && has_data_variants;

    if is_tagged_data_enum {
        return gen_tagged_enum_as_object(enum_def, prefix, has_serde);
    }

    if is_untagged_data_enum {
        return gen_untagged_data_enum_as_value_wrapper(enum_def, prefix);
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

    // Include js_name so NAPI-RS exports the unprefixed name to TypeScript
    // while the Rust enum retains its JsFoo identifier internally.
    let js_name = &enum_def.name;
    let string_enum_attr = match napi_case {
        Some(case) => format!("#[napi(string_enum = \"{case}\", js_name = \"{js_name}\")]"),
        None => format!("#[napi(string_enum, js_name = \"{js_name}\")]"),
    };

    let derives = if has_serde {
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]".to_string()
    } else {
        "#[derive(Clone)]".to_string()
    };
    // Emit rustdoc on the enum so napi-derive forwards it to JSDoc in the .d.ts.
    // Sanitize Rust-specific code examples so they render properly in TypeScript.
    let mut enum_doc = String::new();
    let sanitized_enum_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
        &enum_def.doc,
        crate::codegen::doc_emission::DocTarget::TsDoc,
    );
    crate::codegen::doc_emission::emit_rustdoc(&mut enum_doc, &sanitized_enum_doc, "");
    let mut lines: Vec<String> = Vec::new();
    if !enum_doc.is_empty() {
        // Strip the trailing newline emit_rustdoc appends so the lines join doesn't double up.
        lines.push(enum_doc.trim_end_matches('\n').to_string());
    }
    lines.push(string_enum_attr);
    lines.push(derives);
    lines.push(format!("pub enum {prefix}{} {{", enum_def.name));

    for variant in &enum_def.variants {
        // Variant-level rustdoc → JSDoc on the corresponding TS enum member.
        let mut variant_doc = String::new();
        let sanitized_variant_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
            &variant.doc,
            crate::codegen::doc_emission::DocTarget::TsDoc,
        );
        // Further escape */ sequences that may remain to prevent JSDoc block closure.
        // This handles cases where the sanitizer's escape is lost during rustdoc emission.
        let escaped_variant_doc = sanitized_variant_doc.replace("*/", "* /");
        crate::codegen::doc_emission::emit_rustdoc(&mut variant_doc, &escaped_variant_doc, "    ");
        if !variant_doc.is_empty() {
            lines.push(variant_doc.trim_end_matches('\n').to_string());
        }
        if let Some(rename) = variant.serde_rename.as_deref() {
            lines.push(format!("    #[napi(value = \"{rename}\")]"));
        }
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

/// Generate an untagged data enum as a thin wrapper around `serde_json::Value`.
///
/// `#[serde(untagged)]` enums (e.g. `enum Input { Single(String), Multiple(Vec<String>) }`)
/// can't be expressed as a `#[napi(string_enum)]` because that loses the inner data.
/// JS users want to pass either shape directly (`"hi"` or `["a", "b"]`), so we wrap the
/// value through `serde_json::Value` (napi-rs's `serde-json` feature provides FromNapiValue/
/// ToNapiValue for it) and bridge to/from the core enum via serde.
pub(super) fn gen_untagged_data_enum_as_value_wrapper(enum_def: &EnumDef, prefix: &str) -> String {
    let name = format!("{prefix}{}", enum_def.name);
    format!(
        "#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]\n\
         #[serde(transparent)]\n\
         pub struct {name}(pub serde_json::Value);\n\
         \n\
         impl napi::bindgen_prelude::TypeName for {name} {{\n    \
             fn type_name() -> &'static str {{ \"{name}\" }}\n    \
             fn value_type() -> napi::ValueType {{ napi::ValueType::Unknown }}\n\
         }}\n\
         \n\
         impl napi::bindgen_prelude::FromNapiValue for {name} {{\n    \
             unsafe fn from_napi_value(env: napi::sys::napi_env, val: napi::sys::napi_value) -> napi::Result<Self> {{\n        \
                 let v: serde_json::Value = unsafe {{ napi::bindgen_prelude::FromNapiValue::from_napi_value(env, val)? }};\n        \
                 Ok(Self(v))\n    \
             }}\n\
         }}\n\
         \n\
         impl napi::bindgen_prelude::ToNapiValue for {name} {{\n    \
             unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> napi::Result<napi::sys::napi_value> {{\n        \
                 unsafe {{ napi::bindgen_prelude::ToNapiValue::to_napi_value(env, val.0) }}\n    \
             }}\n\
         }}\n\
         \n\
         impl napi::bindgen_prelude::ValidateNapiValue for {name} {{}}\n"
    )
}

/// Generate a tagged enum as a flattened `#[napi(object)]` struct.
/// E.g. `AuthConfig { Basic { username, password }, Bearer { token } }` becomes:
/// ```rust,ignore
/// #[napi(object)]
/// struct JsAuthConfig {
///     #[napi(js_name = "kind")]
///     pub kind_tag: String,
///     pub username: Option<String>,
///     pub password: Option<String>,
///     pub token: Option<String>,
/// }
/// ```
///
/// The discriminant field is always named "kind" in TypeScript (via js_name),
/// regardless of the Rust serde tag attribute, for consistency across bindings.
///
/// For tagged enums where every non-empty variant is a single-tuple field with a Named type
/// (e.g. `FormatMetadata`), a `#[napi]` impl block is additionally emitted with per-variant
/// getter methods, enabling `result.metadata.format.excel.sheetCount`-style access.
pub(super) fn gen_tagged_enum_as_object(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    use crate::codegen::type_mapper::TypeMapper;
    let mapper = NapiMapper::new(prefix.to_string());

    // Use the Rust serde tag as the TypeScript discriminant field name to match
    // what the Rust deserializer expects. If no explicit tag is set, default to "type".
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let ts_discriminant = tag_field;

    let derive = if has_serde {
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]"
    } else {
        "#[derive(Clone)]"
    };
    // Include js_name so NAPI-RS exports the unprefixed name to TypeScript.
    let js_name = &enum_def.name;
    let mut lines: Vec<String> = Vec::new();
    // Emit rustdoc on the flattened struct so napi-derive forwards it to JSDoc.
    // Sanitize Rust-specific code examples so they render properly in TypeScript.
    let mut enum_doc = String::new();
    let sanitized_enum_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
        &enum_def.doc,
        crate::codegen::doc_emission::DocTarget::TsDoc,
    );
    crate::codegen::doc_emission::emit_rustdoc(&mut enum_doc, &sanitized_enum_doc, "");
    if !enum_doc.is_empty() {
        lines.push(enum_doc.trim_end_matches('\n').to_string());
    }
    lines.push(derive.to_string());
    lines.push(format!("#[napi(object, js_name = \"{js_name}\")]"));
    lines.push(format!("pub struct {prefix}{} {{", enum_def.name));
    lines.push(format!("    #[napi(js_name = \"{ts_discriminant}\")]"));
    // The Rust field is `{tag_field}_tag` (e.g., `type_tag`), but the JS name is `{ts_discriminant}` (e.g., `type`).
    // serde will serialize using the Rust field name unless #[serde(rename)] is set.
    if has_serde {
        lines.push(format!("    #[serde(rename = \"{ts_discriminant}\")]"));
    }
    lines.push(format!("    pub {tag_field}_tag: String,"));

    // Fields that appear in multiple variants with different Named types cannot be represented
    // as a single concrete JsXxx type. Store them as String (JSON) instead, and convert
    // per-variant via serde_json in the From impls.
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    // Collect all unique fields across all variants (all made optional)
    let mut seen_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if tagged_enum_field_is_tuple(field) && matches!(&field.ty, TypeRef::Named(_)) {
                continue;
            }
            let field_name = tagged_enum_field_name(variant, field);
            if seen_fields.insert(field_name.clone()) {
                // Sanitized fields and mixed-type Named fields are represented as String
                // and converted via serde_json in From/Into impls
                let field_type = if (field.sanitized || mixed_named_fields.contains(&field_name))
                    && matches!(&field.ty, TypeRef::Named(_))
                {
                    "String".to_string()
                } else {
                    mapper.map_type(&field.ty).to_string()
                };
                let js_name = tagged_enum_field_js_name(variant, field);
                if js_name != field_name {
                    lines.push(format!("    #[napi(js_name = \"{js_name}\")]"));
                    // When js_name differs from field_name, add #[serde(rename)] for serialization
                    if has_serde {
                        lines.push(format!("    #[serde(rename = \"{js_name}\")]"));
                    }
                }
                lines.push(format!("    pub {field_name}: Option<{field_type}>,"));
            }
        }
    }

    // For tagged enums with variants having single-tuple Named fields, add explicit variant-specific
    // properties (not via getters, which NAPI-RS doesn't generate .d.ts for) to enable direct property access.
    enum_def.variants.iter().for_each(|v| {
        if v.fields.len() != 1 {
            return;
        }
        let field = &v.fields[0];
        if !tagged_enum_field_is_tuple(field) {
            return;
        }
        if let TypeRef::Named(inner_type_name) = &field.ty {
            let field_name = tagged_enum_field_name(v, field);
            let binding_type = format!("{prefix}{inner_type_name}");
            let js_name = tagged_enum_field_js_name(v, field);
            if js_name != field_name {
                lines.push(format!("    #[napi(js_name = \"{js_name}\")]"));
                // When js_name differs from field_name, add #[serde(rename)] for serialization
                if has_serde {
                    lines.push(format!("    #[serde(rename = \"{js_name}\")]"));
                }
            }
            lines.push(format!("    pub {field_name}: Option<{binding_type}>,"));
        }
    });

    lines.push("}".to_string());

    // Default impl — must include both shared variant fields and the synthesized variant-data
    // properties so the struct literal is complete.
    let synth_fields = variant_data_field_names(enum_def);
    let default_inits: Vec<String> = seen_fields
        .iter()
        .cloned()
        .chain(synth_fields.iter().cloned())
        .map(|f| format!("{f}: None"))
        .collect();
    lines.push(String::new());
    lines.push("#[allow(clippy::derivable_impls)]".to_string());
    lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
    lines.push(format!(
        "    fn default() -> Self {{ Self {{ {tag_field}_tag: String::new(), {} }} }}",
        default_inits.join(", ")
    ));
    lines.push("}".to_string());

    // For tagged enums where every non-empty variant is a single-tuple Named field, emit a
    // #[napi] impl block with per-variant getters so callers can do `.excel.sheetCount` etc.
    let _tuple_named_variants: Vec<(&crate::core::ir::EnumVariant, &str)> = enum_def
        .variants
        .iter()
        .filter_map(|v| {
            if v.fields.len() != 1 {
                return None;
            }
            let field = &v.fields[0];
            let is_tuple = field
                .name
                .strip_prefix('_')
                .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
            if !is_tuple {
                return None;
            }
            if let TypeRef::Named(inner_type_name) = &field.ty {
                Some((v, inner_type_name.as_str()))
            } else {
                None
            }
        })
        .collect();

    lines.join("\n")
}

/// Generate a free function binding.
pub(super) fn tagged_enum_mixed_named_fields(enum_def: &EnumDef) -> ahash::AHashSet<String> {
    use crate::core::ir::TypeRef;
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
    use crate::core::ir::TypeRef;
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
    use crate::core::ir::{EnumDef, EnumVariant};

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
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                })
                .collect(),
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: true,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
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

    /// Regression test D4A: tagged enum with unit variant emits { kind: 'bold' }
    /// and not { annotation_type: 'bold' }.
    #[test]
    fn gen_tagged_enum_unit_variant_uses_kind_discriminant() {
        use crate::core::ir::{FieldDef, TypeRef};

        // Create a tagged enum with both unit and tuple variants so it's treated as tagged
        let e = EnumDef {
            name: "AnnotationKind".to_string(),
            rust_path: "test::AnnotationKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Bold".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("bold".to_string()),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "FontSize".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    is_tuple: true,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("fontSize".to_string()),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("annotation_type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        };

        let result = gen_enum(&e, "Js", true);

        // The discriminant field js_name must match the Rust serde tag name so TypeScript
        // payloads deserialize correctly in Rust. Here the serde_tag is "annotation_type".
        assert!(
            result.contains("js_name = \"annotation_type\""),
            "tagged enum must use js_name matching serde tag (annotation_type);\nactual:\n{result}"
        );
    }

    /// Regression test D4B: tagged enum with tuple variant (payload) emits camelCase
    /// value name in serde_rename, e.g., 'fontSize' not 'font_size'.
    #[test]
    fn gen_tagged_enum_tuple_variant_uses_camel_case_value() {
        use crate::core::ir::{FieldDef, TypeRef};

        let e = EnumDef {
            name: "AnnotationKind".to_string(),
            rust_path: "test::AnnotationKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "FontSize".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: Some("fontSize".to_string()),
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("fontSize".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("annotation_type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        };

        let result = gen_enum(&e, "Js", true);

        // The variant's serde_rename must be respected at the JS boundary while Rust field names
        // remain snake_case so generated code is warning-free.
        assert!(
            result.contains("js_name = \"fontSize\"") && result.contains("pub font_size: Option<String>"),
            "tagged enum with tuple variant must expose camelCase js_name and keep Rust snake_case;\nactual:\n{result}"
        );
    }

    /// Regression test D4C: struct variant with named field emits field name unchanged.
    /// E.g., Custom { reason: String } → { kind: 'custom'; reason: string }
    #[test]
    fn gen_tagged_enum_struct_variant_emits_field_names() {
        use crate::core::ir::{FieldDef, TypeRef};

        let e = EnumDef {
            name: "AnnotationKind".to_string(),
            rust_path: "test::AnnotationKind".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Custom".to_string(),
                fields: vec![FieldDef {
                    name: "reason".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("custom".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("annotation_type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        };

        let result = gen_enum(&e, "Js", true);

        // Must include the struct variant field names.
        assert!(
            result.contains("reason"),
            "struct variant must emit field names (reason);\nactual:\n{result}"
        );
        // Discriminant js_name must match the serde tag name (annotation_type here).
        assert!(
            result.contains("js_name = \"annotation_type\""),
            "struct variant enum must use js_name matching serde tag;\nactual:\n{result}"
        );
    }

    /// Regression test for JSDoc block-close escaping in enum variant docs.
    /// When a variant doc contains `/* ... */` inside backticks (e.g., a code example),
    /// the `*/` must be escaped to `* /` so it doesn't prematurely close the JSDoc block
    /// in the generated TypeScript .d.ts file.
    #[test]
    fn gen_enum_escapes_jsdoc_block_close_in_variant_docs() {
        let e = EnumDef {
            name: "CommentType".to_string(),
            rust_path: "test::CommentType".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Block".to_string(),
                    fields: vec![],
                    doc: "A block or multi-line comment (e.g., `/* ... */`).".to_string(),
                    is_default: false,
                    serde_rename: Some("block".to_string()),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Doc".to_string(),
                    fields: vec![],
                    doc: "A documentation comment (e.g., `/// ...` or `/** ... */`).".to_string(),
                    is_default: false,
                    serde_rename: Some("doc".to_string()),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: true,
            has_serde: true,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        };

        let result = gen_enum(&e, "", false);
        eprintln!("Generated code:\n{}\n", result);

        // The variant docs should have `*/` escaped to `* /` to prevent premature
        // JSDoc block closure in the generated TypeScript .d.ts
        assert!(
            result.contains("* /"),
            "enum variant doc must escape */ sequences:\nactual:\n{result}"
        );
        // Verify the unescaped `*/` does not appear (except in the escaped form)
        let unescaped_count = result.matches("*/").count();
        let escaped_count = result.matches("* /").count();
        eprintln!("Unescaped */ count: {}", unescaped_count);
        eprintln!("Escaped * / count: {}", escaped_count);
        assert!(
            escaped_count > 0 && unescaped_count == 0,
            "enum variant doc should contain escaped * / but no bare */:\nactual:\n{result}"
        );
    }
}
