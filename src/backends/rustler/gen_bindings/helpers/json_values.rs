use crate::core::ir::{FieldDef, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;
use std::collections::HashMap;

/// Elixir built-in type names that must not be redefined with `@type`.
///
/// Emitting `@type list :: ...` shadows the built-in `list/0` and produces a
/// Dialyzer/Elixir compiler warning. Append `_variant` to any name that
/// collides with one of these identifiers.
const ELIXIR_BUILTIN_TYPES: &[&str] = &[
    "any",
    "as_boolean",
    "atom",
    "binary",
    "boolean",
    "byte",
    "char",
    "charlist",
    "float",
    "fun",
    "function",
    "identifier",
    "integer",
    "iodata",
    "iolist",
    "keyword",
    "list",
    "map",
    "mfa",
    "module",
    "no_return",
    "node",
    "none",
    "number",
    "pid",
    "port",
    "reference",
    "string",
    "struct",
    "term",
    "timeout",
    "tuple",
];

/// Return a `@type` name that does not collide with an Elixir built-in type.
///
/// If `name` matches one of the Elixir built-in type identifiers it is suffixed
/// with `_variant` so the generated `@type` declaration does not shadow the
/// built-in and trigger compiler or Dialyzer warnings.
pub(in crate::backends::rustler::gen_bindings) fn elixir_safe_type_name(name: &str) -> String {
    if ELIXIR_BUILTIN_TYPES.contains(&name) {
        format!("{name}_variant")
    } else {
        name.to_owned()
    }
}
/// Elixir built-in module attributes that cannot be used as custom `@attribute` names.
///
/// Emitting `@doc :doc` (for an enum variant named `Doc`) raises a compiler error because
/// `@doc` is a built-in module attribute. Append `_attr` when the snake_case variant name
/// collides with one of these identifiers.
const ELIXIR_RESERVED_MODULE_ATTRIBUTES: &[&str] = &[
    "after_compile",
    "before_compile",
    "behaviour",
    "callback",
    "compile",
    "deprecated",
    "derive",
    "dialyzer",
    "doc",
    "enforce_keys",
    "external_resource",
    "file",
    "impl",
    "moduledoc",
    "on_definition",
    "on_load",
    "opaque",
    "optional_callbacks",
    "spec",
    "type",
    "typedoc",
    "typep",
    "vsn",
];

/// Return a module attribute name that does not collide with an Elixir built-in attribute.
///
/// If `name` matches a reserved Elixir module attribute (e.g. `doc`, `type`, `spec`)
/// it is suffixed with `_attr` so the generated `@attribute` declaration does not
/// shadow the built-in and trigger a compiler error.
pub(in crate::backends::rustler::gen_bindings) fn elixir_safe_attr_name(name: &str) -> String {
    if ELIXIR_RESERVED_MODULE_ATTRIBUTES.contains(&name) {
        format!("{name}_attr")
    } else {
        name.to_owned()
    }
}

/// Elixir reserved words that cannot be used as parameter names.
const ELIXIR_RESERVED_WORDS: &[&str] = &[
    "after", "and", "catch", "cond", "do", "else", "end", "false", "fn", "for", "if", "in", "nil", "not", "or",
    "raise", "receive", "rescue", "true", "try", "unless", "when", "with",
];

/// Ensure a parameter name does not collide with an Elixir reserved word.
pub(in crate::backends::rustler::gen_bindings) fn elixir_safe_param_name(name: &str) -> String {
    let snake = name.to_snake_case();
    if ELIXIR_RESERVED_WORDS.contains(&snake.as_str()) {
        format!("{snake}_val")
    } else {
        snake
    }
}

/// Return an Elixir atom value (without leading `:`, as the template adds it).
/// If the atom contains non-identifier characters, it is quoted as `"atom:value"`.
///
/// Valid Elixir identifiers are: `[a-zA-Z_][a-zA-Z_0-9]*[?!]?`.
/// Atoms containing colons, dashes, or other special chars are wrapped as `"atom:value"`.
/// This is used for enum variant atom values that may contain `#[serde(rename)]` strings.
pub(in crate::backends::rustler::gen_bindings) fn elixir_safe_atom(atom_value: &str) -> String {
    fn is_valid_identifier(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let mut chars = s.chars();
        let first = chars.next().unwrap();
        if !first.is_ascii_alphabetic() && first != '_' {
            return false;
        }
        loop {
            match chars.next() {
                None => return true,
                Some(c) => {
                    if !c.is_ascii_alphanumeric() && c != '_' && c != '?' && c != '!' {
                        return false;
                    }
                    if (c == '?' || c == '!') && chars.as_str() != "" {
                        return false;
                    }
                }
            }
        }
    }

    if is_valid_identifier(atom_value) {
        atom_value.to_string()
    } else {
        format!(r#""{atom_value}""#)
    }
}

/// - If the field name is a struct field name (like `reason`), use it directly.
/// - For multiple tuple fields, use generic names: `value0`, `value1`, etc.
pub(in crate::backends::rustler::gen_bindings) fn elixir_field_name_with_type(
    field_name: &str,
    field_idx: usize,
    field_type_name: Option<&str>,
    variant_name: &str,
    total_fields: usize,
) -> String {
    let stripped = field_name.trim_start_matches('_');

    if !stripped.is_empty() && !stripped.chars().all(|c| c.is_ascii_digit()) {
        return stripped.to_snake_case();
    }

    if total_fields == 1 {
        if let Some(type_name) = field_type_name {
            if let Some(remainder) = type_name.strip_prefix(variant_name) {
                let derived = remainder.to_snake_case();
                if !derived.is_empty() {
                    return derived;
                }
            }

            if is_primitive_type(type_name) {
                return "value".to_string();
            }
        }
    }

    if total_fields > 1 {
        return format!("value{}", field_idx);
    }

    "value".to_string()
}

/// Check if a type name is a primitive type (String, bool, integers, floats, etc.).
fn is_primitive_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "bool"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "isize"
            | "f32"
            | "f64"
            | "char"
            | "byte"
            | "unit"
    )
}

/// Format an integer literal with underscore separators for Elixir conventions.
/// E.g. 5242880 → "5_242_880". Numbers < 1000 are returned unchanged.
fn elixir_format_integer(n: i64) -> String {
    let (neg, s) = if n < 0 {
        (true, (-n).to_string())
    } else {
        (false, n.to_string())
    };
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push('_');
        }
        result.push(c);
    }
    let formatted: String = result.chars().rev().collect();
    if neg { format!("-{formatted}") } else { formatted }
}

/// Derive an Elixir default expression for a struct field.
pub(in crate::backends::rustler::gen_bindings) fn elixir_field_default(
    field: &FieldDef,
    ty: &TypeRef,
    enum_defaults: &HashMap<String, String>,
    _opaque_types: &AHashSet<String>,
) -> String {
    use crate::core::ir::DefaultValue;

    let is_nilable = field.optional || matches!(ty, TypeRef::Optional(_));
    if is_nilable {
        return "nil".to_string();
    }

    if let Some(td) = &field.typed_default {
        return match td {
            DefaultValue::BoolLiteral(b) => (if *b { "true" } else { "false" }).to_string(),
            DefaultValue::StringLiteral(s) => format!("\"{}\"", s.replace('"', "\\\"")),
            DefaultValue::IntLiteral(i) => elixir_format_integer(*i),
            DefaultValue::FloatLiteral(f) => format!("{f}"),
            DefaultValue::EnumVariant(v) => format!(":{}", v.to_snake_case()),
            DefaultValue::Empty => elixir_zero_value(ty, enum_defaults),
            DefaultValue::None => "nil".to_string(),
        };
    }

    elixir_zero_value(ty, enum_defaults)
}

/// Generate a type-appropriate zero/default value for Elixir.
///
/// G7: Defaults align with @type specs:
/// - String-like values → `nil` unless an explicit default is present
/// - Non-nilable numbers → `0` or `0.0`
/// - Non-nilable booleans → `false`
/// - Non-nilable lists → `[]`
/// - Non-nilable maps → `%{}`
/// - Struct/Named types → first variant default (enum) or `nil`
fn elixir_zero_value(ty: &TypeRef, enum_defaults: &HashMap<String, String>) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "false".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "nil".to_string(),
        TypeRef::Bytes => "<<>>".to_string(),
        TypeRef::Duration => "0".to_string(),
        TypeRef::Vec(_) => "[]".to_string(),
        TypeRef::Map(_, _) => "%{}".to_string(),
        TypeRef::Optional(_) => "nil".to_string(),
        TypeRef::Unit => "nil".to_string(),
        TypeRef::Named(name) => {
            if let Some(variant) = enum_defaults.get(name) {
                format!(":{variant}")
            } else {
                "nil".to_string()
            }
        }
    }
}

/// Map a TypeRef to an Elixir typespec string for `@spec` annotations.
///
/// `default_types` lists types that are passed as JSON strings at the NIF boundary
/// (types with `has_default = true`).  Their typespec is `String.t() | nil` rather
/// than `map()` because callers encode them with `Jason.encode!/1`.
pub(in crate::backends::rustler::gen_bindings) fn elixir_typespec(
    ty: &TypeRef,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String.t()".to_string(),
        TypeRef::Bytes => "binary()".to_string(),
        TypeRef::Unit => "nil".to_string(),
        TypeRef::Duration => "non_neg_integer()".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "boolean()".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "float()".to_string(),
            crate::core::ir::PrimitiveType::U8
            | crate::core::ir::PrimitiveType::U16
            | crate::core::ir::PrimitiveType::U32
            | crate::core::ir::PrimitiveType::U64
            | crate::core::ir::PrimitiveType::Usize => "non_neg_integer()".to_string(),
            crate::core::ir::PrimitiveType::I8
            | crate::core::ir::PrimitiveType::I16
            | crate::core::ir::PrimitiveType::I32
            | crate::core::ir::PrimitiveType::I64
            | crate::core::ir::PrimitiveType::Isize => "integer()".to_string(),
        },
        TypeRef::Named(name) => {
            if opaque_types.contains(name) {
                "reference()".to_string()
            } else if default_types.contains(name) {
                "String.t() | nil".to_string()
            } else {
                "map()".to_string()
            }
        }
        TypeRef::Optional(inner) => {
            let inner_spec = elixir_typespec(inner, opaque_types, default_types);
            if inner_spec.ends_with("| nil") {
                inner_spec
            } else {
                format!("{} | nil", inner_spec)
            }
        }
        TypeRef::Vec(inner) => {
            format!("[{}]", elixir_typespec(inner, opaque_types, default_types))
        }
        TypeRef::Map(_, _) => "map()".to_string(),
    }
}

/// Map a TypeRef to an Elixir struct-field typespec for generated public DTO modules.
///
/// Unlike NIF-boundary specs, known generated DTO names can reference their public
/// Elixir module directly. Unknown named types still fall back to `map()`.
pub(in crate::backends::rustler::gen_bindings) fn elixir_struct_field_typespec(
    ty: &TypeRef,
    app_module: &str,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    known_struct_types: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(name) if known_struct_types.contains(name) && !opaque_types.contains(name) => {
            format!("{app_module}.{}.t()", elixir_safe_type_name(name))
        }
        TypeRef::Optional(inner) => {
            let inner_spec =
                elixir_struct_field_typespec(inner, app_module, opaque_types, default_types, known_struct_types);
            if inner_spec.ends_with("| nil") {
                inner_spec
            } else {
                format!("{inner_spec} | nil")
            }
        }
        TypeRef::Vec(inner) => {
            let inner_spec =
                elixir_struct_field_typespec(inner, app_module, opaque_types, default_types, known_struct_types);
            format!("[{inner_spec}]")
        }
        _ => elixir_typespec(ty, opaque_types, default_types),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_elixir_typespec_optional_default_type_no_double_nil() {
        let mut default_types = AHashSet::new();
        default_types.insert("SomeType".to_string());

        let opaque_types = AHashSet::new();

        let ty = TypeRef::Optional(Box::new(TypeRef::Named("SomeType".to_string())));
        let result = elixir_typespec(&ty, &opaque_types, &default_types);

        assert_eq!(
            result, "String.t() | nil",
            "Optional default_type should not produce double nil: got {}",
            result
        );
    }

    #[test]
    fn test_elixir_typespec_named_default_type() {
        let mut default_types = AHashSet::new();
        default_types.insert("Options".to_string());

        let opaque_types = AHashSet::new();

        let ty = TypeRef::Named("Options".to_string());
        let result = elixir_typespec(&ty, &opaque_types, &default_types);

        assert_eq!(result, "String.t() | nil");
    }

    #[test]
    fn test_elixir_typespec_optional_non_default_type() {
        let default_types = AHashSet::new();
        let opaque_types = AHashSet::new();

        let ty = TypeRef::Optional(Box::new(TypeRef::Named("RegularType".to_string())));
        let result = elixir_typespec(&ty, &opaque_types, &default_types);

        assert_eq!(result, "map() | nil");
    }

    #[test]
    fn test_elixir_typespec_optional_string() {
        let default_types = AHashSet::new();
        let opaque_types = AHashSet::new();

        let ty = TypeRef::Optional(Box::new(TypeRef::String));
        let result = elixir_typespec(&ty, &opaque_types, &default_types);

        assert_eq!(result, "String.t() | nil");
    }
}
