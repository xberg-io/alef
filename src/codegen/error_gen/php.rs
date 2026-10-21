use super::shared::{error_variant_wildcard_pattern, python_exception_name, to_snake_case};
use super::{ExceptionNaming, exception_families};
use crate::codegen::naming::{internal_rust_identifier, public_field_name};
use crate::core::config::Language;
use crate::core::ir::{ApiSurface, CoreWrapper, ErrorDef, ErrorVariant, PrimitiveType, TypeRef};
use std::collections::{BTreeMap, HashSet};

type PayloadLayout = BTreeMap<String, (String, String, String)>;
type ClassLayouts = BTreeMap<String, (String, PayloadLayout)>;

#[derive(serde::Serialize)]
struct PayloadField {
    name: String,
    php_name: String,
    rust_type: String,
    php_type: String,
    value: String,
}

#[derive(serde::Serialize)]
struct ExceptionClass {
    name: String,
    rust_name: String,
    php_name_literal: String,
    parent: Option<String>,
    fields: Vec<PayloadField>,
}

fn scalar_type(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::String => Some("string".into()),
        TypeRef::Primitive(PrimitiveType::Bool) => Some("bool".into()),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => Some("float".into()),
        TypeRef::Primitive(_) => Some("int".into()),
        TypeRef::Optional(inner) => scalar_type(inner).map(|ty| format!("?{ty}")),
        _ => None,
    }
}

fn scalar_value(binding: &str, ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => format!("ToString::to_string({binding})"),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
            format!("{binding}.as_ref().map(ToString::to_string)")
        }
        TypeRef::Optional(inner) if !matches!(inner.as_ref(), TypeRef::Primitive(_)) => {
            format!("{binding}.as_ref().map(|value| {})", scalar_value("value", inner))
        }
        _ => format!("*{binding}"),
    }
}

fn payload_fields(variant: &ErrorVariant) -> Vec<PayloadField> {
    if variant.is_tuple || variant.is_unit {
        return Vec::new();
    }
    variant
        .fields
        .iter()
        .filter_map(|field| {
            if field.binding_excluded
                || field.cfg.is_some()
                || field.sensitive
                || field.sanitized
                || field.is_boxed
                || field.core_wrapper != CoreWrapper::None
                || field.newtype_wrapper.is_some()
                || field.name.starts_with('_')
                || matches!(
                    field.name.as_str(),
                    "message" | "code" | "file" | "line" | "trace" | "previous"
                )
            {
                return None;
            }
            let mut rust_type = field.ty.rust_source_display();
            let mut php_type = scalar_type(&field.ty)?;
            let mut effective_type = field.ty.clone();
            if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
                rust_type = format!("Option<{rust_type}>");
                php_type = format!("?{php_type}");
                effective_type = TypeRef::Optional(Box::new(effective_type));
            }
            let name = internal_rust_identifier(&field.name);
            Some(PayloadField {
                value: scalar_value(&name, &effective_type),
                name,
                php_name: public_field_name(Language::Php, &field.name, None),
                rust_type,
                php_type,
            })
        })
        .collect()
}

fn record_payload_layout(
    layouts: &mut ClassLayouts,
    class: &str,
    owner: String,
    fields: Vec<PayloadField>,
) -> anyhow::Result<()> {
    let layout = fields
        .into_iter()
        .map(|field| (field.name, (field.php_name, field.rust_type, field.php_type)))
        .collect::<PayloadLayout>();
    if let Some((previous_owner, previous_layout)) = layouts.get(class) {
        anyhow::ensure!(
            previous_layout == &layout,
            "incompatible PHP payload layouts for {}: {} has {:?}; {} has {:?}",
            class,
            previous_owner,
            previous_layout,
            owner,
            layout
        );
    } else {
        layouts.insert(class.into(), (owner, layout));
    }
    Ok(())
}

fn validate_payload_layouts(api: &ApiSurface) -> anyhow::Result<()> {
    let mut layouts = ClassLayouts::new();
    for error in &api.errors {
        record_payload_layout(&mut layouts, &error.name, error.name.clone(), Vec::new())?;
    }
    for error in &api.errors {
        for variant in &error.variants {
            record_payload_layout(
                &mut layouts,
                &python_exception_name(&variant.name, &error.name),
                format!("{}::{}", error.name, variant.name),
                payload_fields(variant),
            )?;
        }
    }
    Ok(())
}

fn classes(api: &ApiSurface, namespace: &str) -> anyhow::Result<Vec<ExceptionClass>> {
    let families = exception_families(api, ExceptionNaming::Python)?;
    validate_payload_layouts(api)?;
    let mut seen = HashSet::new();
    let mut classes = Vec::new();
    for family in families {
        let error = family.error;
        let mut add = |name: String, parent: Option<String>, fields: Vec<PayloadField>| {
            if seen.insert(name.clone()) {
                classes.push(ExceptionClass {
                    rust_name: format!("Php{name}"),
                    php_name_literal: format!("{:?}", format!("{namespace}\\{name}")),
                    name,
                    parent,
                    fields,
                });
            }
        };
        add(error.name.clone(), family.parent, Vec::new());
        for variant in &error.variants {
            add(
                python_exception_name(&variant.name, &error.name),
                Some(error.name.clone()),
                payload_fields(variant),
            );
        }
    }
    Ok(classes)
}

pub fn gen_php_error_types(api: &ApiSurface, namespace: &str) -> anyhow::Result<String> {
    Ok(crate::codegen::template_env::render(
        "error_gen/php_error_types.jinja",
        crate::alef_context! { classes => classes(api, namespace)? },
    ))
}

pub fn gen_php_error_stubs(api: &ApiSurface) -> anyhow::Result<String> {
    Ok(crate::codegen::template_env::render(
        "error_gen/php_error_stubs.jinja",
        crate::alef_context! { classes => classes(api, "")? },
    ))
}

pub fn php_error_class_names(api: &ApiSurface) -> anyhow::Result<Vec<String>> {
    Ok(classes(api, "")?.into_iter().map(|class| class.rust_name).collect())
}

pub(super) fn gen_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };
    let variants = error
        .variants
        .iter()
        .map(|variant| {
            let fields = payload_fields(variant);
            let pattern = if fields.is_empty() {
                error_variant_wildcard_pattern(&rust_path, variant)
            } else {
                format!(
                    "{rust_path}::{} {{ {}, .. }}",
                    variant.name,
                    fields
                        .iter()
                        .map(|field| field.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            crate::alef_context! {
                pattern => pattern,
                class_name => format!("Php{}", python_exception_name(&variant.name, &error.name)),
                fields => fields,
            }
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/php_error_converter.jinja",
        crate::alef_context! {
            rust_path => rust_path,
            fn_name => format!("{}_to_php_err", to_snake_case(&error.name)),
            base_class => format!("Php{}", error.name),
            variants => variants,
        },
    )
}
