use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// These fields have no meaningful name and must be skipped in languages requiring named fields.
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Returns true if the Rust default value for a field is its type's inherent default,
/// meaning `.unwrap_or_default()` can be used instead of `.unwrap_or(value)`.
/// This avoids clippy::unwrap_or_default warnings.
pub(super) fn use_unwrap_or_default(field: &FieldDef) -> bool {
    if let Some(typed_default) = &field.typed_default {
        return matches!(typed_default, DefaultValue::Empty | DefaultValue::None);
    }
    field.default.is_none() && !matches!(&field.ty, TypeRef::Named(_))
}

pub(super) fn constructor_fields(typ: &TypeDef) -> impl Iterator<Item = &FieldDef> {
    typ.fields.iter().filter(|field| !field.binding_excluded)
}

pub fn default_value_for_field(field: &FieldDef, language: &str) -> String {
    if let Some(typed_default) = &field.typed_default {
        return match typed_default {
            DefaultValue::BoolLiteral(b) => match language {
                "python" => {
                    if *b {
                        "True".to_string()
                    } else {
                        "False".to_string()
                    }
                }
                "ruby" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "go" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "java" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "csharp" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "php" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "r" => {
                    if *b {
                        "TRUE".to_string()
                    } else {
                        "FALSE".to_string()
                    }
                }
                "rust" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                _ => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
            },
            DefaultValue::StringLiteral(s) => match language {
                "rust" => format!("\"{}\".to_string()", s.replace('"', "\\\"")),
                _ => format!("\"{}\"", s.replace('"', "\\\"")),
            },
            DefaultValue::IntLiteral(n) => n.to_string(),
            DefaultValue::FloatLiteral(f) => {
                let s = f.to_string();
                if !s.contains('.') { format!("{}.0", s) } else { s }
            }
            DefaultValue::EnumVariant(v) => {
                if matches!(field.ty, TypeRef::String) {
                    let snake = v.to_snake_case();
                    return match language {
                        "rust" => format!("\"{}\".to_string()", snake),
                        _ => format!("\"{}\"", snake),
                    };
                }
                match language {
                    "python" => format!("{}.{}", field.ty.type_name(), v.to_shouty_snake_case()),
                    "ruby" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                    "go" => format!("{}{}", field.ty.type_name(), v.to_pascal_case()),
                    "java" => format!("{}.{}", field.ty.type_name(), v.to_shouty_snake_case()),
                    "csharp" => format!("{}.{}", field.ty.type_name(), v.to_pascal_case()),
                    "php" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                    "r" => format!("{}${}", field.ty.type_name(), v.to_pascal_case()),
                    "rust" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                    _ => v.clone(),
                }
            }
            DefaultValue::Empty => match &field.ty {
                TypeRef::Vec(_) => match language {
                    "python" | "ruby" | "csharp" => "[]".to_string(),
                    "go" => "nil".to_string(),
                    "java" => "List.of()".to_string(),
                    "php" => "[]".to_string(),
                    "r" => "c()".to_string(),
                    "rust" => "vec![]".to_string(),
                    _ => "null".to_string(),
                },
                TypeRef::Map(_, _) => match language {
                    "python" => "{}".to_string(),
                    "go" => "nil".to_string(),
                    "java" => "Map.of()".to_string(),
                    "rust" => "Default::default()".to_string(),
                    _ => "null".to_string(),
                },
                TypeRef::Primitive(p) => match p {
                    PrimitiveType::Bool => match language {
                        "python" => "False".to_string(),
                        "ruby" => "false".to_string(),
                        _ => "false".to_string(),
                    },
                    PrimitiveType::F32 | PrimitiveType::F64 => "0.0".to_string(),
                    _ => "0".to_string(),
                },
                TypeRef::String | TypeRef::Char | TypeRef::Path => match language {
                    "rust" => "String::new()".to_string(),
                    _ => "\"\"".to_string(),
                },
                TypeRef::Json => match language {
                    "python" | "ruby" => "{}".to_string(),
                    "go" => "json.RawMessage(nil)".to_string(),
                    "java" => "new com.fasterxml.jackson.databind.node.ObjectNode(null)".to_string(),
                    "csharp" => "JObject.Parse(\"{}\")".to_string(),
                    "php" => "[]".to_string(),
                    "r" => "list()".to_string(),
                    "rust" => "serde_json::json!({})".to_string(),
                    _ => "{}".to_string(),
                },
                TypeRef::Duration => "0".to_string(),
                TypeRef::Bytes => match language {
                    "python" => "b\"\"".to_string(),
                    "go" => "[]byte{}".to_string(),
                    "rust" => "vec![]".to_string(),
                    _ => "\"\"".to_string(),
                },
                _ => match language {
                    "python" => "None".to_string(),
                    "ruby" => "nil".to_string(),
                    "go" => "nil".to_string(),
                    "rust" => "Default::default()".to_string(),
                    _ => "null".to_string(),
                },
            },
            DefaultValue::None => match language {
                "python" => "None".to_string(),
                "ruby" => "nil".to_string(),
                "go" => "nil".to_string(),
                "java" => "null".to_string(),
                "csharp" => "null".to_string(),
                "php" => "null".to_string(),
                "r" => "NULL".to_string(),
                "rust" => "None".to_string(),
                _ => "null".to_string(),
            },
        };
    }

    // `#[serde(default)]` as a "/* serde(default) */" placeholder and
    // `#[serde(default = "path")]` as a `serde(default = "path")` marker. Both are
    if let Some(default_str) = &field.default
        && default_str != "/* serde(default) */"
        && !default_str.starts_with("serde(default = \"")
    {
        return default_str.clone();
    }

    match &field.ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => match language {
                "python" => "False".to_string(),
                "ruby" => "false".to_string(),
                "csharp" => "false".to_string(),
                "java" => "false".to_string(),
                "php" => "false".to_string(),
                "r" => "FALSE".to_string(),
                _ => "false".to_string(),
            },
            crate::core::ir::PrimitiveType::U8
            | crate::core::ir::PrimitiveType::U16
            | crate::core::ir::PrimitiveType::U32
            | crate::core::ir::PrimitiveType::U64
            | crate::core::ir::PrimitiveType::I8
            | crate::core::ir::PrimitiveType::I16
            | crate::core::ir::PrimitiveType::I32
            | crate::core::ir::PrimitiveType::I64
            | crate::core::ir::PrimitiveType::Usize
            | crate::core::ir::PrimitiveType::Isize => "0".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
        },
        TypeRef::String | TypeRef::Char => match language {
            "python" => "\"\"".to_string(),
            "ruby" => "\"\"".to_string(),
            "go" => "\"\"".to_string(),
            "java" => "\"\"".to_string(),
            "csharp" => "\"\"".to_string(),
            "php" => "\"\"".to_string(),
            "r" => "\"\"".to_string(),
            "rust" => "String::new()".to_string(),
            _ => "\"\"".to_string(),
        },
        TypeRef::Bytes => match language {
            "python" => "b\"\"".to_string(),
            "ruby" => "\"\"".to_string(),
            "go" => "[]byte{}".to_string(),
            "java" => "new byte[]{}".to_string(),
            "csharp" => "new byte[]{}".to_string(),
            "php" => "\"\"".to_string(),
            "r" => "raw()".to_string(),
            "rust" => "vec![]".to_string(),
            _ => "[]".to_string(),
        },
        TypeRef::Optional(_) => match language {
            "python" => "None".to_string(),
            "ruby" => "nil".to_string(),
            "go" => "nil".to_string(),
            "java" => "null".to_string(),
            "csharp" => "null".to_string(),
            "php" => "null".to_string(),
            "r" => "NULL".to_string(),
            "rust" => "None".to_string(),
            _ => "null".to_string(),
        },
        TypeRef::Vec(_) => match language {
            "python" => "[]".to_string(),
            "ruby" => "[]".to_string(),
            "go" => "[]interface{}{}".to_string(),
            "java" => "new java.util.ArrayList<>()".to_string(),
            "csharp" => "[]".to_string(),
            "php" => "[]".to_string(),
            "r" => "c()".to_string(),
            "rust" => "vec![]".to_string(),
            _ => "[]".to_string(),
        },
        TypeRef::Map(_, _) => match language {
            "python" => "{}".to_string(),
            "ruby" => "{}".to_string(),
            "go" => "make(map[string]interface{})".to_string(),
            "java" => "new java.util.HashMap<>()".to_string(),
            "csharp" => "new Dictionary<string, object>()".to_string(),
            "php" => "[]".to_string(),
            "r" => "list()".to_string(),
            "rust" => "std::collections::HashMap::new()".to_string(),
            _ => "{}".to_string(),
        },
        TypeRef::Json => match language {
            "python" => "{}".to_string(),
            "ruby" => "{}".to_string(),
            "go" => "json.RawMessage(nil)".to_string(),
            "java" => "new com.fasterxml.jackson.databind.JsonNode()".to_string(),
            "csharp" => "JObject.Parse(\"{}\")".to_string(),
            "php" => "[]".to_string(),
            "r" => "list()".to_string(),
            "rust" => "serde_json::json!({})".to_string(),
            _ => "{}".to_string(),
        },
        TypeRef::Named(name) => match language {
            "rust" => format!("{name}::default()"),
            "python" => "None".to_string(),
            "ruby" => "nil".to_string(),
            "go" => "nil".to_string(),
            "java" => "null".to_string(),
            "csharp" => "null".to_string(),
            "php" => "null".to_string(),
            "r" => "NULL".to_string(),
            _ => "null".to_string(),
        },
        _ => match language {
            "python" => "None".to_string(),
            "ruby" => "nil".to_string(),
            "go" => "nil".to_string(),
            "java" => "null".to_string(),
            "csharp" => "null".to_string(),
            "php" => "null".to_string(),
            "r" => "NULL".to_string(),
            "rust" => "Default::default()".to_string(),
            _ => "null".to_string(),
        },
    }
}

trait TypeRefExt {
    fn type_name(&self) -> String;
}

impl TypeRefExt for TypeRef {
    fn type_name(&self) -> String {
        match self {
            TypeRef::Named(n) => n.clone(),
            TypeRef::Primitive(p) => format!("{:?}", p),
            TypeRef::String | TypeRef::Char => "String".to_string(),
            TypeRef::Bytes => "Bytes".to_string(),
            TypeRef::Optional(inner) => format!("Option<{}>", inner.type_name()),
            TypeRef::Vec(inner) => format!("Vec<{}>", inner.type_name()),
            TypeRef::Map(k, v) => format!("Map<{}, {}>", k.type_name(), v.type_name()),
            TypeRef::Path => "Path".to_string(),
            TypeRef::Unit => "()".to_string(),
            TypeRef::Json => "Json".to_string(),
            TypeRef::Duration => "Duration".to_string(),
        }
    }
}
