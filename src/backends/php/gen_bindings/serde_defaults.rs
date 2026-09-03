use crate::codegen::naming::{pascal_to_snake, wire_variant_value};
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;

/// Where the body of a `crate::serde_defaults::*` function comes from, once
/// [`serde_default_source`] has decided the function exists at all.
enum SerdeDefaultSource<'a> {
    /// `#[serde(default = "…::function")]` on a field whose type is a named (struct/enum) type.
    /// The return type depends on whether that type is mirrored into the binding crate, which is
    /// why resolving it needs the whole [`ApiSurface`].
    NamedFunctionPath { type_name: &'a str, function_name: &'a str },
    /// A default recovered from the core type's `Default` impl as a literal value.
    Literal { return_type: &'static str, body: String },
    /// `#[serde(default = "…::function")]` on a scalar field, answered by the fully-qualified
    /// core function the extractor already resolved for this field's `Default`-impl initializer.
    /// [`SerdeDefaultSource::NamedFunctionPath`] cannot serve these: it derives the call from the
    /// field's own `TypeRef::Named`, which a scalar field does not have. ~keep
    ResolvedFunctionPath { return_type: &'static str, call: String },
}

fn default_fn_ident(type_name: &str, field_name: &str) -> String {
    format!("{}_{}", pascal_to_snake(type_name), pascal_to_snake(field_name))
}

fn serde_default_path(default: Option<&str>) -> Option<&str> {
    let default = default?;
    let marker = "serde(default = \"";
    let start = default.find(marker)? + marker.len();
    let rest = &default[start..];
    let end = rest.find('"')?;
    let path = rest[..end].trim();
    (!path.is_empty()).then_some(path)
}

/// The PHP binding crate's Rust type for a core primitive. One table, shared by every arm that
/// needs to name a `serde_defaults` return type, so the widths cannot drift between them. ~keep
fn primitive_return_type(primitive: &PrimitiveType) -> &'static str {
    match primitive {
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "i64",
        PrimitiveType::Bool => "bool",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
    }
}

fn typed_default_fn(default: &DefaultValue, ty: &TypeRef) -> Option<(&'static str, String)> {
    match (default, ty) {
        (DefaultValue::BoolLiteral(value), TypeRef::Primitive(PrimitiveType::Bool)) => {
            Some(("bool", value.to_string()))
        }
        (DefaultValue::StringLiteral(value) | DefaultValue::EnumVariant(value), TypeRef::String) => {
            Some(("String", format!("{value:?}.to_string()")))
        }
        (DefaultValue::IntLiteral(value), TypeRef::Primitive(primitive)) => {
            if matches!(primitive, PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64) {
                return None;
            }
            Some((primitive_return_type(primitive), value.to_string()))
        }
        (
            DefaultValue::FloatLiteral(value),
            TypeRef::Primitive(primitive @ (PrimitiveType::F32 | PrimitiveType::F64)),
        ) => {
            let rendered = format!("{value}");
            let body = if rendered.contains('.') || rendered.contains('e') {
                rendered
            } else {
                format!("{rendered}.0")
            };
            Some((primitive_return_type(primitive), body))
        }
        _ => None,
    }
}

/// A scalar field's `#[serde(default = "…")]` answered by the fully-qualified core function the
/// extractor resolved. The core function returns the *core* width (`usize`), while the binding
/// field carries the PHP-facing width (`i64`), so numeric returns need the same `as` cast the
/// generated static-method wrappers use.
fn resolved_function_call(path: &str, ty: &TypeRef) -> Option<(&'static str, String)> {
    match ty {
        TypeRef::String => Some(("String", format!("{path}().into()"))),
        TypeRef::Primitive(PrimitiveType::Bool) => Some(("bool", format!("{path}()"))),
        TypeRef::Primitive(primitive) => {
            let return_type = primitive_return_type(primitive);
            Some((return_type, format!("{path}() as {return_type}")))
        }
        _ => None,
    }
}

/// Resolve the wire value (post `rename`/`rename_all`) for a field's `DefaultValue::EnumVariant`.
///
/// `variant_ref` is the raw text `extract_field` recorded, which is sometimes the bare variant
/// name and sometimes qualified as `"Enum::Variant"` -- strip any such prefix before the lookup.
/// Returns `None` when the enum or variant cannot be found in `api`, which happens only if the
/// extractor and this backend have disagreed about the surface (a bug elsewhere, not a case to
/// paper over with a fabricated value). ~keep
fn enum_variant_wire_value(api: &ApiSurface, enum_name: &str, variant_ref: &str) -> Option<String> {
    let variant_name = variant_ref.rsplit_once("::").map_or(variant_ref, |(_, v)| v);
    let enum_def = api.enums.iter().find(|e| e.name == enum_name)?;
    let variant = enum_def.variants.iter().find(|v| v.name == variant_name)?;
    Some(wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    ))
}

/// A field's bare `#[serde(default)]` where the core type is a named enum the PHP backend lowers
/// to `String` (`enum_names`, from `PhpMapper::enum_names` -- see `rust_bindings::generate_bindings`).
/// The core `Default` impl supplies a *variant name* (`DefaultValue::EnumVariant`), but the mirror
/// field is a bare `String`, so the generated fn must return the enum's *wire* value (e.g.
/// `"plain"`, not `"Plain"`) or `from_json`/absent-field deserialization produces a string the
/// enum's own `TryFrom`/match at deserialize time would reject. ~keep
fn enum_default_wire_source(field: &FieldDef, enum_names: &AHashSet<String>, api: &ApiSurface) -> Option<String> {
    // Only the bare-default sentinel: a field with no `#[serde(default...)]` on the core type at
    // all must stay required on the mirror, same as every other arm in this module. ~keep
    if field.default.as_deref() != Some("/* serde(default) */") {
        return None;
    }
    let TypeRef::Named(enum_name) = &field.ty else {
        return None;
    };
    if !enum_names.contains(enum_name) {
        return None;
    }
    let DefaultValue::EnumVariant(variant_ref) = field.typed_default.as_ref()? else {
        return None;
    };
    enum_variant_wire_value(api, enum_name, variant_ref)
}

/// The single decision point for "does `typ.field` get a `crate::serde_defaults::*` function?".
///
/// Both emitters go through it: [`gen_serde_defaults_module`] to write the `pub fn`, and
/// `gen_php_struct`'s field-attribute pass (via [`serde_default_fn_name`]) to write the
/// `#[serde(default = "crate::serde_defaults::…")]` that references it. They used to decide
/// separately, from two hand-mirrored type tables, and a field the reference side accepted but
/// the definition side dropped produced a generated crate that could not compile. Rendering is
/// infallible once this returns `Some`, so the two sides cannot disagree. ~keep
fn serde_default_source<'a>(
    typ: &TypeDef,
    field: &'a FieldDef,
    enum_names: &AHashSet<String>,
    api: &ApiSurface,
) -> Option<SerdeDefaultSource<'a>> {
    if !typ.has_default || field.optional || field.binding_excluded {
        return None;
    }
    let serde_path = serde_default_path(field.default.as_deref());

    if let Some(path) = serde_path
        && let TypeRef::Named(type_name) = &field.ty
        && let Some((_, function_name)) = path.rsplit_once("::")
    {
        return Some(SerdeDefaultSource::NamedFunctionPath {
            type_name,
            function_name,
        });
    }

    if let Some(default) = &field.typed_default
        && let Some((return_type, body)) = typed_default_fn(default, &field.ty)
    {
        return Some(SerdeDefaultSource::Literal { return_type, body });
    }

    if serde_path.is_some()
        && let Some(DefaultValue::PublicFunctionCall(path)) = &field.typed_default
        && let Some((return_type, call)) = resolved_function_call(path, &field.ty)
    {
        return Some(SerdeDefaultSource::ResolvedFunctionPath { return_type, call });
    }

    if let Some(wire_value) = enum_default_wire_source(field, enum_names, api) {
        return Some(SerdeDefaultSource::Literal {
            return_type: "String",
            body: format!("{wire_value:?}.to_string()"),
        });
    }

    None
}

fn default_fn_signature(source: &SerdeDefaultSource<'_>, field: &FieldDef, api: &ApiSurface) -> (String, String) {
    match source {
        SerdeDefaultSource::NamedFunctionPath {
            type_name,
            function_name,
        } => match field.type_rust_path.as_deref() {
            Some(core_path) if api.types.iter().any(|typ| typ.name == **type_name) => (
                format!("crate::{type_name}"),
                format!("{core_path}::{function_name}().into()"),
            ),
            Some(core_path) => (core_path.to_string(), format!("{core_path}::{function_name}()")),
            None => (
                format!("crate::{type_name}"),
                format!("crate::{type_name}::{function_name}()"),
            ),
        },
        SerdeDefaultSource::Literal { return_type, body } => ((*return_type).to_string(), body.clone()),
        SerdeDefaultSource::ResolvedFunctionPath { return_type, call } => ((*return_type).to_string(), call.clone()),
    }
}

/// The `crate::serde_defaults::…` function name for a field, or `None` when no function will be
/// generated for it. The reference side must not emit an attribute when this returns `None`.
pub(super) fn serde_default_fn_name(
    typ: &TypeDef,
    field: &FieldDef,
    enum_names: &AHashSet<String>,
    api: &ApiSurface,
) -> Option<String> {
    serde_default_source(typ, field, enum_names, api).map(|_| default_fn_ident(&typ.name, &field.name))
}

pub(super) fn gen_serde_defaults_module(api: &ApiSurface, enum_names: &AHashSet<String>) -> Option<String> {
    let functions: Vec<minijinja::Value> = api
        .types
        .iter()
        .flat_map(|typ| typ.fields.iter().map(move |field| (typ, field)))
        .filter_map(|(typ, field)| {
            let source = serde_default_source(typ, field, enum_names, api)?;
            let (return_type, body) = default_fn_signature(&source, field, api);
            Some(minijinja::context! {
                name => default_fn_ident(&typ.name, &field.name),
                return_type,
                body,
            })
        })
        .collect();

    if functions.is_empty() {
        return None;
    }
    Some(
        crate::backends::php::template_env::render("serde_defaults_module.jinja", minijinja::context! { functions })
            .trim_end()
            .to_string(),
    )
}

#[cfg(test)]
#[path = "serde_defaults/pairing_tests.rs"]
mod pairing_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeDef;

    fn foreign_default_field(name: &str, type_name: &str, core_path: &str, default_fn: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::Named(type_name.to_string()),
            optional: false,
            default: Some(format!("serde(default = \"{default_fn}\")")),
            type_rust_path: Some(core_path.to_string()),
            ..Default::default()
        }
    }

    fn config_with_field(field: FieldDef) -> TypeDef {
        TypeDef {
            name: "FetchConfig".to_string(),
            has_default: true,
            fields: vec![field],
            ..Default::default()
        }
    }

    #[test]
    fn mirrored_core_type_default_returns_mirror_and_converts() {
        let config = config_with_field(foreign_default_field(
            "ssrf",
            "SsrfPolicy",
            "mylib::SsrfPolicy",
            "mylib::SsrfPolicy::from_env",
        ));
        let mirror = TypeDef {
            name: "SsrfPolicy".to_string(),
            ..Default::default()
        };
        let api = ApiSurface {
            types: vec![config, mirror],
            ..Default::default()
        };

        let module = gen_serde_defaults_module(&api, &AHashSet::new()).expect("module generated");
        assert!(
            module.contains("pub fn fetch_config_ssrf() -> crate::SsrfPolicy { mylib::SsrfPolicy::from_env().into() }"),
            "expected mirror return type with `.into()` conversion, got:\n{module}"
        );
    }

    #[test]
    fn unmirrored_core_type_default_returns_core_type() {
        let config = config_with_field(foreign_default_field(
            "ssrf",
            "SsrfPolicy",
            "mylib::SsrfPolicy",
            "mylib::SsrfPolicy::from_env",
        ));
        let api = ApiSurface {
            types: vec![config],
            ..Default::default()
        };

        let module = gen_serde_defaults_module(&api, &AHashSet::new()).expect("module generated");
        assert!(
            module.contains("pub fn fetch_config_ssrf() -> mylib::SsrfPolicy { mylib::SsrfPolicy::from_env() }"),
            "expected core return type without conversion, got:\n{module}"
        );
    }
}
