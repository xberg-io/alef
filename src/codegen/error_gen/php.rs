use super::shared::{error_variant_wildcard_pattern, millis_expr, millis_type, python_exception_name, to_snake_case};
use super::{
    ExceptionNaming, LastErrorFieldKind, additional_exception_families, exception_families, last_error_field_kind,
    same_exception_lineage,
};
use crate::codegen::naming::{internal_rust_identifier, public_field_name};
use crate::core::config::Language;
use crate::core::ir::{ApiSurface, CoreWrapper, ErrorDef, ErrorVariant, PrimitiveType, TypeRef};
use std::collections::{BTreeMap, HashSet};

type PayloadLayout = BTreeMap<String, (String, String, String)>;

#[derive(serde::Serialize, Clone)]
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
    /// The family marker interfaces this class implements beyond those its parent brings.
    interfaces: Vec<String>,
}

/// The marker interface of one error family. Every class of the family implements it, so
/// `catch (SchemaErrorInterface $e)` covers each of the family's variants, including a variant
/// class it shares with another family and therefore does not extend.
#[derive(serde::Serialize)]
pub(super) struct FamilyInterface {
    family: String,
    name: String,
    php_name_literal: String,
    /// The interface of the family's `exception_base` parent, which this one extends.
    parent: Option<String>,
}

/// The PHP name of `family`'s marker interface.
pub(super) fn family_interface_name(family: &str) -> String {
    format!("{family}Interface")
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
                || !matches!(field.core_wrapper, CoreWrapper::None | CoreWrapper::StaticStr)
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

/// One variant that maps to a class: its family, the variant and its payload fields.
type Declaration<'a> = (&'a ErrorDef, &'a ErrorVariant, Vec<PayloadField>);

fn layout_of(fields: &[PayloadField]) -> PayloadLayout {
    fields
        .iter()
        .map(|field| {
            (
                field.name.clone(),
                (field.php_name.clone(), field.rust_type.clone(), field.php_type.clone()),
            )
        })
        .collect()
}

fn optional(mut field: PayloadField) -> PayloadField {
    if !field.php_type.starts_with('?') {
        field.rust_type = format!("Option<{}>", field.rust_type);
        field.php_type = format!("?{}", field.php_type);
    }
    field
}

/// The payload fields of every variant class, by class name.
///
/// A class shared by unrelated families carries the union of their fields; a field some family
/// lacks becomes nullable, and a field two families declare with different types is rejected.
/// Families of one `exception_base` lineage, and a variant named like an error class, must agree
/// exactly, as one class has always served them.
fn payload_layouts(errors: &[ErrorDef]) -> anyhow::Result<BTreeMap<String, Vec<PayloadField>>> {
    let mut declarations: BTreeMap<String, Vec<Declaration<'_>>> = BTreeMap::new();
    for error in errors {
        for variant in &error.variants {
            declarations
                .entry(python_exception_name(&variant.name, &error.name))
                .or_default()
                .push((error, variant, payload_fields(variant)));
        }
    }
    let mut layouts = BTreeMap::new();
    for (class, declared) in declarations {
        let owner = |error: &ErrorDef, variant: &ErrorVariant| format!("{}::{}", error.name, variant.name);
        if let Some(base) = errors.iter().find(|error| error.name == class) {
            for (error, variant, fields) in &declared {
                anyhow::ensure!(
                    fields.is_empty(),
                    "incompatible PHP payload layouts for {}: {} has {:?}; {} has {:?}",
                    class,
                    base.name,
                    PayloadLayout::new(),
                    owner(error, variant),
                    layout_of(fields)
                );
            }
        }
        for (index, (error, variant, fields)) in declared.iter().enumerate() {
            for (other_error, other_variant, other_fields) in &declared[..index] {
                if same_exception_lineage(errors, &error.name, &other_error.name) {
                    anyhow::ensure!(
                        layout_of(fields) == layout_of(other_fields),
                        "incompatible PHP payload layouts for {}: {} has {:?}; {} has {:?}",
                        class,
                        owner(other_error, other_variant),
                        layout_of(other_fields),
                        owner(error, variant),
                        layout_of(fields)
                    );
                }
            }
        }
        let mut union: Vec<(PayloadField, String)> = Vec::new();
        for (error, variant, fields) in &declared {
            for field in fields {
                match union.iter().find(|(existing, _)| existing.name == field.name) {
                    Some((existing, first)) => anyhow::ensure!(
                        (&existing.php_name, &existing.rust_type, &existing.php_type)
                            == (&field.php_name, &field.rust_type, &field.php_type),
                        "incompatible PHP payload field {} of {}: {} has {} {}; {} has {} {}",
                        field.name,
                        class,
                        first,
                        existing.php_name,
                        existing.rust_type,
                        owner(error, variant),
                        field.php_name,
                        field.rust_type
                    ),
                    None => union.push((field.clone(), owner(error, variant))),
                }
            }
        }
        let fields = union
            .into_iter()
            .map(|(field, _)| {
                if declared
                    .iter()
                    .all(|(_, _, fields)| fields.iter().any(|declared| declared.name == field.name))
                {
                    field
                } else {
                    optional(field)
                }
            })
            .collect();
        layouts.insert(class, fields);
    }
    Ok(layouts)
}

/// The introspection fields of every class, by class name.
///
/// A class several families raise -- a variant they share, or a family's base class that is also
/// another family's variant -- carries every one of their introspection values. A family without
/// one of the methods fills its default rather than `null`: PHP return types are covariant, and
/// the root exception and the class's parent declare these getters non-nullable, so a nullable
/// override would not load. One method two of them type differently is rejected.
fn info_layouts(errors: &[ErrorDef]) -> anyhow::Result<BTreeMap<String, Vec<InfoField>>> {
    let mut contributors: BTreeMap<String, Vec<&ErrorDef>> = BTreeMap::new();
    for error in errors {
        let classes = std::iter::once(error.name.clone()).chain(
            error
                .variants
                .iter()
                .map(|variant| python_exception_name(&variant.name, &error.name)),
        );
        for class in classes {
            let entry = contributors.entry(class).or_default();
            if !entry.iter().any(|existing| existing.name == error.name) {
                entry.push(error);
            }
        }
    }
    let mut layouts = BTreeMap::new();
    for (class, families) in contributors {
        let per_family: Vec<(&ErrorDef, Vec<InfoField>)> =
            families.iter().map(|error| (*error, error_info(error))).collect();
        let mut union: Vec<(InfoField, &str)> = Vec::new();
        for (error, fields) in &per_family {
            for field in fields {
                match union.iter().find(|(existing, _)| existing.name == field.name) {
                    Some((existing, first)) => anyhow::ensure!(
                        existing.rust_type == field.rust_type,
                        "incompatible PHP introspection {} of {class}: {first} returns {}; {} returns {}",
                        field.name,
                        existing.rust_type,
                        error.name,
                        field.rust_type
                    ),
                    None => union.push((field.clone(), &error.name)),
                }
            }
        }
        let fields = union
            .into_iter()
            .map(|(mut field, _)| {
                let everywhere = per_family
                    .iter()
                    .all(|(_, fields)| fields.iter().any(|declared| declared.name == field.name));
                if !everywhere {
                    field.doc.push_str(
                        " The type's default (0, '' or false) when the family that raised it has no such value.",
                    );
                }
                field
            })
            .collect();
        layouts.insert(class, fields);
    }
    Ok(layouts)
}

/// `(field, value)` for every introspection field of a class one family raises: the family's own
/// values, and the type's default for the values only other families report.
fn info_values(own: &[InfoField], class_fields: Option<&Vec<InfoField>>) -> Vec<(String, String)> {
    let Some(class_fields) = class_fields else {
        return own
            .iter()
            .map(|field| (field.name.clone(), field.value.clone()))
            .collect();
    };
    class_fields
        .iter()
        .map(|class_field| {
            let value = own
                .iter()
                .find(|field| field.name == class_field.name)
                .map_or_else(|| "Default::default()".to_string(), |field| field.value.clone());
            (class_field.name.clone(), value)
        })
        .collect()
}

/// The marker interface of every error family, parents before children.
pub(super) fn family_interfaces(api: &ApiSurface, namespace: &str) -> anyhow::Result<Vec<FamilyInterface>> {
    super::validate_family_interface_names(api, "PHP", family_interface_name)?;
    Ok(exception_families(api, ExceptionNaming::Python)?
        .into_iter()
        .map(|family| {
            let name = family_interface_name(&family.error.name);
            FamilyInterface {
                family: family.error.name.clone(),
                php_name_literal: format!("{:?}", format!("{namespace}\\{name}")),
                name,
                parent: family.parent.as_deref().map(family_interface_name),
            }
        })
        .collect())
}

pub(super) fn classes(api: &ApiSurface, namespace: &str, root: Option<&str>) -> anyhow::Result<Vec<ExceptionClass>> {
    let families = exception_families(api, ExceptionNaming::Python)?;
    let mut layouts = payload_layouts(&api.errors)?;
    let info_layouts = info_layouts(&api.errors)?;
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
            interfaces: Vec::new(),
        });
    }
    for family in families {
        let error = family.error;
        let info = error_info(&error);
        let mut add = |name: String, parent: Option<String>, fields: Vec<PayloadField>, interfaces: Vec<String>| {
            if seen.insert(name.clone()) {
                let info = info_layouts.get(&name).cloned().unwrap_or_else(|| info.clone());
                classes.push(ExceptionClass {
                    rust_name: format!("Php{name}"),
                    php_name_literal: format!("{:?}", format!("{namespace}\\{name}")),
                    name,
                    parent,
                    fields,
                    info,
                    root: false,
                    interfaces,
                });
            } else if let Some(class) = classes.iter_mut().find(|class| class.name == name) {
                // A family's base class can be another family's variant class (`AppError::GraphQL`
                // is `GraphQLError`); the class declared first must still carry this family's
                // marker interface. ~keep
                for interface in interfaces {
                    if !class.interfaces.contains(&interface) {
                        class.interfaces.push(interface);
                    }
                }
            }
        };
        add(
            error.name.clone(),
            family.parent.or_else(|| root.map(str::to_string)),
            Vec::new(),
            vec![family_interface_name(&error.name)],
        );
        for variant in &error.variants {
            let name = python_exception_name(&variant.name, &error.name);
            let fields = layouts.remove(&name).unwrap_or_default();
            let interfaces =
                additional_exception_families(&api.errors, &error.name, &variant.name, ExceptionNaming::Python)
                    .iter()
                    .map(|family| family_interface_name(family))
                    .collect();
            add(name, Some(error.name.clone()), fields, interfaces);
        }
    }
    Ok(classes)
}

/// `root` is the shared parent exception class for every emitted family. ~keep
pub fn gen_php_error_types(api: &ApiSurface, namespace: &str, root: Option<&str>) -> anyhow::Result<String> {
    Ok(crate::codegen::template_env::render(
        "error_gen/php_error_types.jinja",
        crate::alef_context! {
            classes => classes(api, namespace, root)?,
            interfaces => family_interfaces(api, namespace)?,
        },
    ))
}

pub fn gen_php_error_stubs(api: &ApiSurface, root: Option<&str>) -> anyhow::Result<String> {
    Ok(crate::codegen::template_env::render(
        "error_gen/php_error_stubs.jinja",
        crate::alef_context! {
            classes => classes(api, "", root)?,
            interfaces => family_interfaces(api, "")?,
        },
    ))
}

/// The Rust names of the family marker interfaces, which the module registers before any class.
pub fn php_error_interface_names(api: &ApiSurface) -> anyhow::Result<Vec<String>> {
    Ok(family_interfaces(api, "")?
        .into_iter()
        .map(|interface| format!("PhpInterface{}", interface.name))
        .collect())
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

pub(super) fn gen_converter(error: &ErrorDef, errors: &[ErrorDef], core_import: &str) -> String {
    let rust_path = if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    };
    let info = error_info(error);
    let layouts = payload_layouts(errors).unwrap_or_default();
    let info_layouts = info_layouts(errors).unwrap_or_default();
    let variants = error
        .variants
        .iter()
        .map(|variant| {
            let fields = payload_fields(variant);
            let class = python_exception_name(&variant.name, &error.name);
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
                values => class_values(&fields, layouts.get(&class)),
                info => info_values(&info, info_layouts.get(&class)),
                class_name => format!("Php{class}"),
            }
        })
        .collect::<Vec<_>>();
    crate::codegen::template_env::render(
        "error_gen/php_error_converter.jinja",
        crate::alef_context! {
            rust_path => rust_path,
            fn_name => format!("{}_to_php_err", to_snake_case(&error.name)),
            base_class => format!("Php{}", error.name),
            info => info_values(&info, info_layouts.get(&error.name)),
            variants => variants,
        },
    )
}

/// `(field, value)` for every field of the variant's class: the variant's own values, wrapped in
/// `Some` where the shared class made the field nullable, and `None` for the fields only other
/// families declare.
fn class_values(own: &[PayloadField], class_fields: Option<&Vec<PayloadField>>) -> Vec<(String, String)> {
    let Some(class_fields) = class_fields else {
        return own
            .iter()
            .map(|field| (field.name.clone(), field.value.clone()))
            .collect();
    };
    class_fields
        .iter()
        .map(|class_field| {
            let value = match own.iter().find(|field| field.name == class_field.name) {
                Some(field) if field.rust_type == class_field.rust_type => field.value.clone(),
                Some(field) => format!("Some({})", field.value),
                None => "None".to_string(),
            };
            (class_field.name.clone(), value)
        })
        .collect()
}
