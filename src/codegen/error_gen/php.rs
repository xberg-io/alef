use super::shared::{error_variant_wildcard_pattern, millis_expr, millis_type, python_exception_name, to_snake_case};
use super::{ExceptionNaming, LastErrorFieldKind, exception_families, last_error_field_kind};
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

/// One introspection method of the error (`status_code`, `retry_after`, ...), stored on every
/// exception object when the converter builds it and read back through a PHP method of the same
/// name in lower camel case (`statusCode()`).
#[derive(serde::Serialize, Clone)]
pub(super) struct InfoField {
    pub(super) name: String,
    php_name: String,
    rust_type: String,
    php_type: String,
    /// Expression over the live core error `e`. Empty for the root exception class.
    pub(super) value: String,
    doc: String,
    /// True when the stored value is `Clone` but not `Copy`.
    clone: bool,
}

#[derive(serde::Serialize)]
pub(super) struct ExceptionClass {
    pub(super) name: String,
    rust_name: String,
    php_name_literal: String,
    pub(super) parent: Option<String>,
    fields: Vec<PayloadField>,
    pub(super) info: Vec<InfoField>,
    /// True for the project-level root class, which extends PHP's `RuntimeException`.
    root: bool,
}

fn info_field(method: &crate::core::ir::MethodDef) -> Option<InfoField> {
    // The same set the stub declares: the three named methods and every `Duration` method.
    if !(matches!(method.name.as_str(), "status_code" | "is_transient" | "error_type")
        || super::duration_shape(&method.return_type).is_some())
    {
        return None;
    }
    let kind = last_error_field_kind(method)?;
    let call = format!("e.{}()", method.name);
    let php_name = heck::ToLowerCamelCase::to_lower_camel_case(method.name.as_str());
    let (rust_type, php_type, value, clone, doc) = match kind {
        LastErrorFieldKind::Scalar(primitive) => {
            let rust = TypeRef::Primitive(primitive.clone()).rust_source_display();
            let php = scalar_type(&TypeRef::Primitive(primitive)).unwrap_or_else(|| "int".into());
            (rust, php, call, false, format!("`{}` of the error.", method.name))
        }
        LastErrorFieldKind::Text => (
            "String".to_string(),
            "string".to_string(),
            format!("{call}.to_string()"),
            true,
            format!("`{}` of the error.", method.name),
        ),
        LastErrorFieldKind::DurationMillis => {
            let optional = matches!(method.return_type, TypeRef::Optional(_));
            (
                millis_type(optional, "i64"),
                if optional { "?int".into() } else { "int".into() },
                millis_expr(&call, optional, "i64"),
                false,
                format!(
                    "`{}` in whole milliseconds{}.",
                    method.name,
                    if optional { " (null when absent)" } else { "" }
                ),
            )
        }
    };
    Some(InfoField {
        name: method.name.clone(),
        php_name,
        rust_type,
        php_type,
        value,
        doc,
        clone,
    })
}

/// The introspection fields of one error, in declaration order.
pub(super) fn error_info(error: &ErrorDef) -> Vec<InfoField> {
    error.methods.iter().filter_map(info_field).collect()
}

/// The union of the introspection fields over every error, for the root class.
fn root_info(api: &ApiSurface) -> Vec<InfoField> {
    let mut fields: Vec<InfoField> = Vec::new();
    for error in &api.errors {
        for field in error_info(error) {
            if !fields.iter().any(|existing| existing.name == field.name) {
                fields.push(InfoField {
                    value: String::new(),
                    ..field
                });
            }
        }
    }
    fields
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

pub(super) fn classes(api: &ApiSurface, namespace: &str, root: Option<&str>) -> anyhow::Result<Vec<ExceptionClass>> {
    let families = exception_families(api, ExceptionNaming::Python)?;
    validate_payload_layouts(api)?;
    // A root named like an error of the API would be two classes with one name.
    let root = root.filter(|name| !api.errors.is_empty() && api.errors.iter().all(|error| error.name != *name));
    let mut seen = HashSet::new();
    let mut classes = Vec::new();
    if let Some(name) = root {
        seen.insert(name.to_string());
        classes.push(ExceptionClass {
            rust_name: format!("Php{name}"),
            php_name_literal: format!("{:?}", format!("{namespace}\\{name}")),
            name: name.to_string(),
            parent: None,
            fields: Vec::new(),
            info: root_info(api),
            root: true,
        });
    }
    for family in families {
        let error = family.error;
        let info = error_info(&error);
        let mut add = |name: String, parent: Option<String>, fields: Vec<PayloadField>| {
            if seen.insert(name.clone()) {
                classes.push(ExceptionClass {
                    rust_name: format!("Php{name}"),
                    php_name_literal: format!("{:?}", format!("{namespace}\\{name}")),
                    name,
                    parent,
                    fields,
                    info: info.clone(),
                    root: false,
                });
            }
        };
        add(
            error.name.clone(),
            family.parent.or_else(|| root.map(str::to_string)),
            Vec::new(),
        );
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

/// The exception classes as Rust items. `root` names the project-level exception class that every
/// family extends (for example `LiterLlmException`).
pub fn gen_php_error_types(api: &ApiSurface, namespace: &str, root: Option<&str>) -> anyhow::Result<String> {
    Ok(crate::codegen::template_env::render(
        "error_gen/php_error_types.jinja",
        crate::alef_context! { classes => classes(api, namespace, root)? },
    ))
}

pub fn gen_php_error_stubs(api: &ApiSurface, root: Option<&str>) -> anyhow::Result<String> {
    Ok(crate::codegen::template_env::render(
        "error_gen/php_error_stubs.jinja",
        crate::alef_context! { classes => classes(api, "", root)? },
    ))
}

pub fn php_error_class_names(api: &ApiSurface, root: Option<&str>) -> anyhow::Result<Vec<String>> {
    Ok(classes(api, "", root)?
        .into_iter()
        .map(|class| class.rust_name)
        .collect())
}

/// `fn alef_php_err`: turns the error a core call returned into the typed exception of its family.
/// An error type that no converter handles falls back to a plain message exception.
pub fn gen_php_error_dispatch(api: &ApiSurface, core_import: &str) -> String {
    let arms = api
        .errors
        .iter()
        .map(|error| {
            let rust_path = if error.rust_path.is_empty() {
                format!("{core_import}::{}", error.name)
            } else {
                error.rust_path.replace('-', "_")
            };
            crate::alef_context! {
                rust_path => rust_path,
                fn_name => format!("{}_to_php_err", to_snake_case(&error.name)),
            }
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/php_error_dispatch.jinja",
        crate::alef_context! { arms => arms },
    )
}

pub(super) fn gen_converter(error: &ErrorDef, core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };
    let info = error_info(error);
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
                info => info,
            }
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/php_error_converter.jinja",
        crate::alef_context! {
            rust_path => rust_path,
            fn_name => format!("{}_to_php_err", to_snake_case(&error.name)),
            base_class => format!("Php{}", error.name),
            info => info,
            variants => variants,
        },
    )
}
