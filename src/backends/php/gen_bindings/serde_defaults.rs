use crate::codegen::naming::pascal_to_snake;
use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, PrimitiveType, TypeRef};

fn serde_default_fn_name(type_name: &str, field_name: &str) -> String {
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

fn function_path_default_fn(field: &FieldDef, api: &ApiSurface) -> Option<(String, String)> {
    let path = serde_default_path(field.default.as_deref())?;
    let TypeRef::Named(type_name) = &field.ty else {
        return None;
    };
    let (_, function_name) = path.rsplit_once("::")?;
    // The `serde_defaults` module has no imports, so the type must be absolutely
    // qualified. `type_rust_path` is the core type's absolute path (e.g.
    // `core_crate::Policy`); when absent the type is itself a binding DTO at the
    // crate root, whose default fn is an associated function we call directly.
    //
    // When the core type is *also* mirrored into a crate-root binding DTO, the
    // field is rendered as the mirror (`crate::Policy`), not the core type — so the
    // helper must return the mirror and convert the core value via `.into()` (the
    // mirror derives `From<core>`). Returning the core type there mismatches the
    // field type and fails to compile.
    match field.type_rust_path.as_deref() {
        Some(core_path) if api.types.iter().any(|typ| &typ.name == type_name) => Some((
            format!("crate::{type_name}"),
            format!("{core_path}::{function_name}().into()"),
        )),
        Some(core_path) => Some((core_path.to_string(), format!("{core_path}::{function_name}()"))),
        None => Some((
            format!("crate::{type_name}"),
            format!("crate::{type_name}::{function_name}()"),
        )),
    }
}

fn typed_default_fn(default: &DefaultValue, ty: &TypeRef) -> Option<(&'static str, String)> {
    match (default, ty) {
        (DefaultValue::BoolLiteral(value), TypeRef::Primitive(PrimitiveType::Bool)) => {
            Some(("bool", value.to_string()))
        }
        // Only emit String-returning fn for actual String fields. Named (enum-backed struct
        // wrapper) fields would mismatch the wrapped Named return type at compile time, so
        // skip emission and let serde fall back to Default for the wrapped type.
        (DefaultValue::StringLiteral(value) | DefaultValue::EnumVariant(value), TypeRef::String) => {
            Some(("String", format!("{value:?}.to_string()")))
        }
        (DefaultValue::IntLiteral(value), TypeRef::Primitive(primitive)) => {
            let return_type = match primitive {
                PrimitiveType::U8 => "u8",
                PrimitiveType::U16 => "u16",
                PrimitiveType::U32 => "u32",
                PrimitiveType::U64 => "i64",
                PrimitiveType::I8 => "i8",
                PrimitiveType::I16 => "i16",
                PrimitiveType::I32 => "i32",
                PrimitiveType::I64 => "i64",
                PrimitiveType::Usize | PrimitiveType::Isize => "i64",
                PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64 => return None,
            };
            Some((return_type, value.to_string()))
        }
        (DefaultValue::FloatLiteral(value), TypeRef::Primitive(PrimitiveType::F32)) => {
            let s = format!("{value}");
            let body = if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            };
            Some(("f32", body))
        }
        (DefaultValue::FloatLiteral(value), TypeRef::Primitive(PrimitiveType::F64)) => {
            let s = format!("{value}");
            let body = if s.contains('.') || s.contains('e') {
                s
            } else {
                format!("{s}.0")
            };
            Some(("f64", body))
        }
        _ => None,
    }
}

pub(super) fn gen_serde_defaults_module(api: &ApiSurface) -> Option<String> {
    let mut functions = Vec::new();
    for typ in api.types.iter().filter(|typ| typ.has_default) {
        for field in typ.fields.iter().filter(|field| !field.optional) {
            let fn_name = serde_default_fn_name(&typ.name, &field.name);

            if let Some((return_type, body)) = function_path_default_fn(field, api) {
                functions.push(format!("    pub fn {fn_name}() -> {return_type} {{ {body} }}"));
                continue;
            }

            if let Some(default) = &field.typed_default {
                let Some((return_type, body)) = typed_default_fn(default, &field.ty) else {
                    continue;
                };
                functions.push(format!("    pub fn {fn_name}() -> {return_type} {{ {body} }}"));
            }
        }
    }

    if functions.is_empty() {
        None
    } else {
        Some(format!("mod serde_defaults {{\n{}\n}}", functions.join("\n")))
    }
}

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

    // A core type that is also mirrored into a crate-root DTO: the field renders as
    // the mirror, so the helper must return the mirror and `.into()`-convert the core
    // value. Regression: a mirrored core type must not emit the core path as the return
    // type (would cause a type mismatch compile error).
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

        let module = gen_serde_defaults_module(&api).expect("module generated");
        assert!(
            module.contains("pub fn fetch_config_ssrf() -> crate::SsrfPolicy { mylib::SsrfPolicy::from_env().into() }"),
            "expected mirror return type with `.into()` conversion, got:\n{module}"
        );
    }

    // The same core type used directly (no mirror DTO): return the core type as-is,
    // no conversion.
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

        let module = gen_serde_defaults_module(&api).expect("module generated");
        assert!(
            module.contains("pub fn fetch_config_ssrf() -> mylib::SsrfPolicy { mylib::SsrfPolicy::from_env() }"),
            "expected core return type without conversion, got:\n{module}"
        );
    }
}
