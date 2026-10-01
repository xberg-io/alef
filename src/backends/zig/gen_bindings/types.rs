use crate::codegen::naming::{
    PublicIdentifierKind, pascal_to_snake, public_field_name, public_host_identifier, wire_variant_value,
};
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{DefaultValue, EnumDef, TypeDef, TypeRef};

use crate::backends::zig::type_map::ZigMapper;

use super::helpers::emit_cleaned_zig_doc;

/// Emit a Zig struct for an IR struct type.
///
/// `config` is threaded in only so the field identifiers can honor `[crates.zig] rename_fields`.
/// Zig struct members are the public field surface a consumer both constructs and reads by name,
/// so a configured rename that this emitter ignored would leave the consumer's `alef.toml` a
/// silent no-op rather than an error. ~keep
pub(crate) fn emit_type(ty: &TypeDef, enums: &[EnumDef], config: &ResolvedCrateConfig, out: &mut String) {
    emit_cleaned_zig_doc(out, &ty.doc, "");
    out.push_str(&crate::backends::zig::template_env::render(
        "type_header.jinja",
        minijinja::context! {
            type_name => &ty.name,
        },
    ));
    for field in binding_fields(&ty.fields) {
        emit_cleaned_zig_doc(out, &field.doc, "    ");
        let ty_str = zig_field_type(&field.ty, field.optional);
        let field_default = zig_field_default(field, enums);
        out.push_str(&crate::backends::zig::template_env::render(
            "type_field.jinja",
            minijinja::context! {
                field_name => zig_field_identifier(ty, field, config),
                field_type => ty_str,
                field_default => field_default,
            },
        ));
    }
    out.push_str("};\n");
}

fn zig_enum_default(enum_def: &EnumDef, variant_ref: &str) -> Option<String> {
    let variant_name = variant_ref.rsplit_once("::").map_or(variant_ref, |(_, name)| name);
    let variant = enum_def
        .variants
        .iter()
        .find(|candidate| candidate.name == variant_name)?;
    let wire_value = wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    );
    Some(format!(
        ".{}",
        public_host_identifier(Language::Zig, PublicIdentifierKind::EnumVariant, &wire_value)
    ))
}

fn zig_field_default(field: &crate::core::ir::FieldDef, enums: &[EnumDef]) -> String {
    if !field.has_bare_serde_enum_default() {
        return String::new();
    }
    let Some(DefaultValue::EnumVariant(variant_ref)) = field.typed_default.as_ref() else {
        return String::new();
    };
    let TypeRef::Named(enum_name) = &field.ty else {
        return String::new();
    };
    let Some(enum_def) = enums.iter().find(|candidate| candidate.name == *enum_name) else {
        return String::new();
    };
    zig_enum_default(enum_def, variant_ref).map_or_else(String::new, |value| format!(" = {value}"))
}

/// Resolve a Zig struct member identifier, applying `[crates.zig] rename_fields` before casing.
///
/// Enum payload struct members deliberately do not go through here: `rename_fields` is keyed
/// `"TypeName.field_name"`, and an enum variant's payload has no unambiguous `TypeName` to key
/// on, so a lookup would silently miss rather than rename. ~keep
fn zig_field_identifier(ty: &TypeDef, field: &crate::core::ir::FieldDef, config: &ResolvedCrateConfig) -> String {
    let renamed = config.resolve_field_name(Language::Zig, &ty.name, &field.name);
    public_field_name(Language::Zig, &field.name, renamed.as_deref())
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String) {
    emit_cleaned_zig_doc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::backends::zig::template_env::render(
            "enum_unit_header.jinja",
            minijinja::context! {
                enum_name => &en.name,
            },
        ));
        for variant in &en.variants {
            emit_cleaned_zig_doc(out, &variant.doc, "    ");
            let tag_value = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            out.push_str(&crate::backends::zig::template_env::render(
                "enum_unit_variant.jinja",
                minijinja::context! {
                    variant_name => public_host_identifier(Language::Zig, PublicIdentifierKind::EnumVariant, &tag_value),
                },
            ));
        }
        out.push_str("};\n");
    } else {
        out.push_str(&crate::backends::zig::template_env::render(
            "enum_tagged_header.jinja",
            minijinja::context! {
                enum_name => &en.name,
            },
        ));
        for variant in &en.variants {
            emit_cleaned_zig_doc(out, &variant.doc, "    ");
            let tag_value = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let tag = public_host_identifier(Language::Zig, PublicIdentifierKind::EnumVariant, &tag_value);
            if variant.fields.is_empty() {
                out.push_str(&crate::backends::zig::template_env::render(
                    "enum_variant_void.jinja",
                    minijinja::context! {
                        tag => &tag,
                    },
                ));
            } else if variant.fields.len() == 1 {
                let ty_str = zig_field_type(&variant.fields[0].ty, variant.fields[0].optional);
                out.push_str(&crate::backends::zig::template_env::render(
                    "enum_variant_single.jinja",
                    minijinja::context! {
                        tag => &tag,
                        type_str => ty_str,
                    },
                ));
            } else {
                out.push_str(&crate::backends::zig::template_env::render(
                    "enum_variant_struct_header.jinja",
                    minijinja::context! {
                        tag => &tag,
                    },
                ));
                for f in &variant.fields {
                    let name = if f.name.is_empty() {
                        "value".into()
                    } else {
                        public_host_identifier(Language::Zig, PublicIdentifierKind::Field, &f.name)
                    };
                    let ty_str = zig_field_type(&f.ty, f.optional);
                    out.push_str(&crate::backends::zig::template_env::render(
                        "enum_variant_struct_field.jinja",
                        minijinja::context! {
                            field_name => name,
                            field_type => ty_str,
                        },
                    ));
                }
                out.push_str("    },\n");
            }
        }
        out.push_str("};\n");
    }
}

pub(crate) fn zig_field_type(ty: &TypeRef, optional: bool) -> String {
    let mapper = ZigMapper;
    let inner = mapper.map_type(ty);
    if optional && !inner.starts_with('?') {
        format!("?{inner}")
    } else {
        inner
    }
}

pub(crate) fn c_symbol_component(name: &str) -> String {
    pascal_to_snake(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumVariant, FieldDef};

    #[test]
    fn bare_enum_default_is_emitted_without_defaulting_required_sibling() {
        let typ = TypeDef {
            name: "CaptioningConfig".to_string(),
            fields: vec![
                FieldDef {
                    name: "llm".to_string(),
                    ty: TypeRef::Named("LlmConfig".to_string()),
                    ..Default::default()
                },
                FieldDef {
                    name: "alt_text".to_string(),
                    ty: TypeRef::Named("CaptionAltTextMode".to_string()),
                    default: Some("/* serde(default) */".to_string()),
                    typed_default: Some(DefaultValue::EnumVariant("Preserve".to_string())),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let enums = [EnumDef {
            name: "CaptionAltTextMode".to_string(),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![EnumVariant {
                name: "Preserve".to_string(),
                is_default: true,
                ..Default::default()
            }],
            ..Default::default()
        }];
        let mut out = String::new();
        emit_type(&typ, &enums, &ResolvedCrateConfig::default(), &mut out);

        assert!(out.contains("llm: LlmConfig,"), "{out}");
        assert!(!out.contains("llm: LlmConfig ="), "{out}");
        assert!(out.contains("alt_text: CaptionAltTextMode = .preserve,"), "{out}");
    }

    #[test]
    fn non_enum_bare_default_preserves_required_field_shape() {
        let typ = TypeDef {
            name: "OcrPipelineConfig".to_string(),
            fields: vec![FieldDef {
                name: "quality_thresholds".to_string(),
                ty: TypeRef::Named("OcrQualityThresholds".to_string()),
                default: Some("/* serde(default) */".to_string()),
                typed_default: Some(DefaultValue::Empty),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut out = String::new();
        emit_type(&typ, &[], &ResolvedCrateConfig::default(), &mut out);

        let control = TypeDef {
            name: "OcrPipelineConfig".to_string(),
            fields: vec![FieldDef {
                name: "quality_thresholds".to_string(),
                ty: TypeRef::Named("OcrQualityThresholds".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut control_out = String::new();
        emit_type(&control, &[], &ResolvedCrateConfig::default(), &mut control_out);

        assert_eq!(out, control_out);
        assert!(out.contains("quality_thresholds: OcrQualityThresholds,"), "{out}");
        assert!(!out.contains("quality_thresholds: OcrQualityThresholds ="), "{out}");
    }
}
