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
pub(crate) fn elixir_safe_param_name(name: &str) -> String {
    let snake = name.to_snake_case();
    if ELIXIR_RESERVED_WORDS.contains(&snake.as_str()) {
        format!("{snake}_val")
    } else {
        snake
    }
}

/// The Elixir atom an enum variant carries at runtime, ready to follow a `:`.
///
/// One expression, used by every surface of the generated enum module that names a variant: the
/// `@type t` union, the `@variant` attribute's value, and `wire_value/1`'s clause heads. They must
/// agree, and the value they must agree ON is fixed by Rustler, not by us -- a `NifUnitEnum`
/// decodes to `pascal_to_snake(variant.name)`, because serde and rustler are independent proc
/// macros over the same variant and `serde_rename` never reaches the rustler one.
///
/// Spelling any of these surfaces from `serde_rename` instead makes the module contradict itself.
/// It did: for `#[serde(rename = "og:image")] OgImage`, the accessor returned `:"og:image"` while
/// the only `wire_value/1` clause matched `:og_image`, so `Enum.wire_value(Enum.og_image())` --
/// the module's own two public functions, composed -- raised `FunctionClauseError`, and the
/// `@type t` advertised an atom the NIF never produces. Confirmed on Elixir 1.20.4. The clause
/// was not merely wrong, it was unreachable through the module's public surface, and there is no
/// fallback clause behind it to catch the value that actually arrives. Per the repo's
/// `centralized-naming` rule, `serde_rename` defines wire names only; `wire_value/1` is where the
/// wire name is exposed, and it maps FROM this atom TO that string. ~keep
pub(in crate::backends::rustler::gen_bindings) fn elixir_variant_atom(rust_variant_name: &str) -> String {
    elixir_safe_atom(&crate::codegen::naming::pascal_to_snake(rust_variant_name))
}

/// Return an Elixir atom value (without leading `:`, as the template adds it).
/// If the atom contains non-identifier characters, it is quoted as `"atom:value"`.
///
/// Delegates to [`crate::backends::rustler::elixir_escape::elixir_atom_body`], which owns both
/// halves of the decision — bare-versus-quoted, and what escaping the quoted body needs. This
/// used to inline `format!(r#""{atom_value}""#)`, which quoted the body without escaping it, so a
/// `#[serde(rename)]` carrying `"`, `\` or `#{` produced either a broken module or, for `#{`, one
/// that executed the rename's contents when it compiled. ~keep
pub(in crate::backends::rustler::gen_bindings) fn elixir_safe_atom(atom_value: &str) -> String {
    crate::backends::rustler::elixir_escape::elixir_atom_body(atom_value)
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

    if total_fields == 1
        && let Some(type_name) = field_type_name
    {
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
            DefaultValue::ListLiteral(items) => {
                let rendered: Option<Vec<String>> = items.iter().map(elixir_scalar_default).collect();
                // A nested or non-scalar element falls back to the empty collection rather than
                // a partial list, matching the extractor's all-or-nothing rule. ~keep
                match rendered {
                    Some(values) => format!("[{}]", values.join(", ")),
                    None => elixir_zero_value(ty, enum_defaults),
                }
            }
            DefaultValue::Empty => elixir_zero_value(ty, enum_defaults),
            // `Unresolved`: alef could not read the real default out of `impl Default`.
            // `TupleVariant`/`StructVariant`: alef read the value, but this renderer has no
            // Elixir expression for "construct enum variant X with these field values" the way
            // it does for a bare `EnumVariant`. Falling through to `elixir_zero_value` (as this
            // used to, for `Unresolved`) would ship the *type's* zero underneath a doc comment
            // quoting the real (unrendered) Rust default — a value the source never actually
            // specified. `nil` is the same "a default exists, this renderer cannot spell it"
            // signal already used for `FunctionCall`/`PublicFunctionCall` below. ~keep
            DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..) => {
                "nil".to_string()
            }
            DefaultValue::None => "nil".to_string(),
            DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => "nil".to_string(),
        };
    }

    elixir_zero_value(ty, enum_defaults)
}

/// Render one element of a collection-literal default as Elixir source.
///
/// Deliberately scalar-only: a nested list has no element type to resolve against here, and a
/// function-call default cannot be evaluated at generation time, so both return `None` and let
/// the caller fall back to the empty collection. ~keep
fn elixir_scalar_default(item: &crate::core::ir::DefaultValue) -> Option<String> {
    use crate::core::ir::DefaultValue;
    match item {
        DefaultValue::BoolLiteral(b) => Some((if *b { "true" } else { "false" }).to_string()),
        DefaultValue::StringLiteral(s) => Some(format!("\"{}\"", s.replace('"', "\\\""))),
        DefaultValue::IntLiteral(i) => Some(elixir_format_integer(*i)),
        DefaultValue::FloatLiteral(f) => Some(format!("{f}")),
        DefaultValue::EnumVariant(v) => Some(format!(":{}", v.to_snake_case())),
        DefaultValue::ListLiteral(_)
        | DefaultValue::Empty
        | DefaultValue::Unresolved(_)
        | DefaultValue::TupleVariant(..)
        | DefaultValue::StructVariant(..)
        | DefaultValue::None
        | DefaultValue::FunctionCall(_)
        | DefaultValue::PublicFunctionCall(_) => None,
    }
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

    #[test]
    fn struct_field_vector_prefers_known_public_enum_module() {
        let default_types = AHashSet::from_iter(["HostMatcher".to_string()]);
        let known_types = AHashSet::from_iter(["HostMatcher".to_string()]);
        let result = elixir_struct_field_typespec(
            &TypeRef::Vec(Box::new(TypeRef::Named("HostMatcher".into()))),
            "Sample",
            &AHashSet::new(),
            &default_types,
            &known_types,
        );

        assert_eq!(result, "[Sample.HostMatcher.t()]");
    }
}
