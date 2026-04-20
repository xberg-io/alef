use alef_core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToPascalCase, ToShoutySnakeCase};

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// These fields have no meaningful name and must be skipped in languages requiring named fields.
fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Returns true if the Rust default value for a field is its type's inherent default,
/// meaning `.unwrap_or_default()` can be used instead of `.unwrap_or(value)`.
/// This avoids clippy::unwrap_or_default warnings.
fn use_unwrap_or_default(field: &FieldDef) -> bool {
    if let Some(typed_default) = &field.typed_default {
        return matches!(typed_default, DefaultValue::Empty | DefaultValue::None);
    }
    // No typed_default — the fallback default_value_for_field generates type-based zero values
    // which are the same as Default::default() for the type.
    // Named types may not implement Default in some bindings (e.g. Magnus), so they
    // fall through to the explicit default path.
    field.default.is_none() && !matches!(&field.ty, TypeRef::Named(_))
}

/// Generate a PyO3 `#[new]` constructor with kwargs for a type with `has_default`.
/// All fields become keyword args with their defaults in `#[pyo3(signature = (...))]`.
pub fn gen_pyo3_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let mut lines = Vec::new();
    lines.push("#[new]".to_string());

    // Build the signature line with defaults
    let mut sig_parts = Vec::new();
    for field in &typ.fields {
        let default_str = default_value_for_field(field, "python");
        sig_parts.push(format!("{}={}", field.name, default_str));
    }
    let signature = format!("#[pyo3(signature = ({}))]", sig_parts.join(", "));
    lines.push(signature);

    // Function signature
    lines.push("fn new(".to_string());
    for (i, field) in typ.fields.iter().enumerate() {
        let type_str = type_mapper(&field.ty);
        let comma = if i < typ.fields.len() - 1 { "," } else { "" };
        lines.push(format!("    {}: {}{}", field.name, type_str, comma));
    }
    lines.push(") -> Self {".to_string());

    // Body
    lines.push("    Self {".to_string());
    for field in &typ.fields {
        lines.push(format!("        {},", field.name));
    }
    lines.push("    }".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate NAPI constructor that applies defaults for missing optional fields.
pub fn gen_napi_defaults_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let mut lines = Vec::new();
    lines.push("pub fn new(mut env: napi::Env, obj: napi::Object) -> napi::Result<Self> {".to_string());

    // Field assignments with defaults
    for field in &typ.fields {
        let type_str = type_mapper(&field.ty);
        let default_str = default_value_for_field(field, "rust");
        lines.push(format!(
            "    let {}: {} = obj.get(\"{}\").unwrap_or({})?;",
            field.name, type_str, field.name, default_str
        ));
    }

    lines.push("    Ok(Self {".to_string());
    for field in &typ.fields {
        lines.push(format!("        {},", field.name));
    }
    lines.push("    })".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate Go functional options pattern for a type with `has_default`.
/// Returns: type definition + Option type + WithField functions + NewConfig constructor
pub fn gen_go_functional_options(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let mut lines = Vec::new();

    // Type definition
    lines.push(format!("// {} is a configuration type.", typ.name));
    lines.push(format!("type {} struct {{", typ.name));
    for field in &typ.fields {
        if is_tuple_field(field) {
            continue;
        }
        let go_type = type_mapper(&field.ty);
        lines.push(format!("    {} {}", field.name.to_pascal_case(), go_type));
    }
    lines.push("}".to_string());
    lines.push("".to_string());

    // Option function type
    lines.push(format!(
        "// {}Option is a functional option for {}.",
        typ.name, typ.name
    ));
    lines.push(format!("type {}Option func(*{})", typ.name, typ.name));
    lines.push("".to_string());

    // WithField functions
    for field in &typ.fields {
        if is_tuple_field(field) {
            continue;
        }
        let option_name = format!("With{}{}", typ.name, field.name.to_pascal_case());
        let go_type = type_mapper(&field.ty);
        lines.push(format!("// {} sets the {}.", option_name, field.name));
        lines.push(format!("func {}(val {}) {}Option {{", option_name, go_type, typ.name));
        lines.push(format!("    return func(c *{}) {{", typ.name));
        lines.push(format!("        c.{} = val", field.name.to_pascal_case()));
        lines.push("    }".to_string());
        lines.push("}".to_string());
        lines.push("".to_string());
    }

    // New constructor
    lines.push(format!(
        "// New{} creates a new {} with default values and applies options.",
        typ.name, typ.name
    ));
    lines.push(format!(
        "func New{}(opts ...{}Option) *{} {{",
        typ.name, typ.name, typ.name
    ));
    lines.push(format!("    c := &{} {{", typ.name));
    for field in &typ.fields {
        if is_tuple_field(field) {
            continue;
        }
        let default_str = default_value_for_field(field, "go");
        lines.push(format!("        {}: {},", field.name.to_pascal_case(), default_str));
    }
    lines.push("    }".to_string());
    lines.push("    for _, opt := range opts {".to_string());
    lines.push("        opt(c)".to_string());
    lines.push("    }".to_string());
    lines.push("    return c".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate Java builder pattern for a type with `has_default`.
/// Returns: Builder inner class with withField methods + build() method
pub fn gen_java_builder(typ: &TypeDef, package: &str, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "// DO NOT EDIT - auto-generated by alef\npackage {};\n",
        package
    ));
    lines.push("/// Builder for creating instances of {} with sensible defaults".to_string());
    lines.push(format!("public class {}Builder {{", typ.name));

    // Fields
    for field in &typ.fields {
        let java_type = type_mapper(&field.ty);
        lines.push(format!("    private {} {};", java_type, field.name.to_lowercase()));
    }
    lines.push("".to_string());

    // Constructor
    lines.push(format!("    public {}Builder() {{", typ.name));
    for field in &typ.fields {
        let default_str = default_value_for_field(field, "java");
        lines.push(format!("        this.{} = {};", field.name.to_lowercase(), default_str));
    }
    lines.push("    }".to_string());
    lines.push("".to_string());

    // withField methods
    for field in &typ.fields {
        let java_type = type_mapper(&field.ty);
        let method_name = format!("with{}", field.name.to_pascal_case());
        lines.push(format!(
            "    public {}Builder {}({} value) {{",
            typ.name, method_name, java_type
        ));
        lines.push(format!("        this.{} = value;", field.name.to_lowercase()));
        lines.push("        return this;".to_string());
        lines.push("    }".to_string());
        lines.push("".to_string());
    }

    // build() method
    lines.push(format!("    public {} build() {{", typ.name));
    lines.push(format!("        return new {}(", typ.name));
    for (i, field) in typ.fields.iter().enumerate() {
        let comma = if i < typ.fields.len() - 1 { "," } else { "" };
        lines.push(format!("            this.{}{}", field.name.to_lowercase(), comma));
    }
    lines.push("        );".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate C# record with init properties for a type with `has_default`.
pub fn gen_csharp_record(typ: &TypeDef, namespace: &str, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    let mut lines = Vec::new();

    lines.push("// This file is auto-generated by alef. DO NOT EDIT.".to_string());
    lines.push("using System;".to_string());
    lines.push("".to_string());
    lines.push(format!("namespace {};\n", namespace));

    lines.push(format!("/// Configuration record: {}", typ.name));
    lines.push(format!("public record {} {{", typ.name));

    for field in &typ.fields {
        // Skip tuple struct internals (e.g., _0, _1, etc.)
        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        let cs_type = type_mapper(&field.ty);
        let default_str = default_value_for_field(field, "csharp");
        lines.push(format!(
            "    public {} {} {{ get; init; }} = {};",
            cs_type,
            field.name.to_pascal_case(),
            default_str
        ));
    }

    lines.push("}".to_string());

    lines.join("\n")
}

/// Get a language-appropriate default value string for a field.
/// Uses `typed_default` if available, falls back to `default` string, or type-based zero value.
pub fn default_value_for_field(field: &FieldDef, language: &str) -> String {
    // First try typed_default if it exists
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
            DefaultValue::EnumVariant(v) => match language {
                "python" => format!("{}.{}", field.ty.type_name(), v.to_shouty_snake_case()),
                "ruby" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                "go" => format!("{}{}", field.ty.type_name(), v.to_pascal_case()),
                "java" => format!("{}.{}", field.ty.type_name(), v.to_shouty_snake_case()),
                "csharp" => format!("{}.{}", field.ty.type_name(), v.to_pascal_case()),
                "php" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                "r" => format!("{}${}", field.ty.type_name(), v.to_pascal_case()),
                "rust" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                _ => v.clone(),
            },
            DefaultValue::Empty => {
                // Empty means "type's default" — check field type to pick the right zero value
                match &field.ty {
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
                        "go" => "map[string]interface{}{}".to_string(),
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
                }
            }
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

    // Fall back to string default if it exists
    if let Some(default_str) = &field.default {
        return default_str.clone();
    }

    // Final fallback: type-based zero value
    match &field.ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => match language {
                "python" => "False".to_string(),
                "ruby" => "false".to_string(),
                "csharp" => "false".to_string(),
                "java" => "false".to_string(),
                "php" => "false".to_string(),
                "r" => "FALSE".to_string(),
                _ => "false".to_string(),
            },
            alef_core::ir::PrimitiveType::U8
            | alef_core::ir::PrimitiveType::U16
            | alef_core::ir::PrimitiveType::U32
            | alef_core::ir::PrimitiveType::U64
            | alef_core::ir::PrimitiveType::I8
            | alef_core::ir::PrimitiveType::I16
            | alef_core::ir::PrimitiveType::I32
            | alef_core::ir::PrimitiveType::I64
            | alef_core::ir::PrimitiveType::Usize
            | alef_core::ir::PrimitiveType::Isize => "0".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
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
            "go" => "make(map[string]interface{})".to_string(),
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

// Helper trait extension for TypeRef to get type name
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

/// The maximum arity supported by Magnus `function!` macro.
const MAGNUS_MAX_ARITY: usize = 15;

/// Generate a Magnus (Ruby) kwargs constructor for a type with `has_default`.
///
/// For types with <=15 fields, generates a positional `Option<T>` parameter constructor.
/// For types with >15 fields (exceeding Magnus arity limit), generates a hash-based constructor
/// using `RHash` that extracts fields by name, applying defaults for missing keys.
pub fn gen_magnus_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    if typ.fields.len() > MAGNUS_MAX_ARITY {
        gen_magnus_hash_constructor(typ, type_mapper)
    } else {
        gen_magnus_positional_constructor(typ, type_mapper)
    }
}

/// Wrap a type string for use as a type-path prefix in Rust.
///
/// Types containing `<` (generics like `Vec<String>`, `Option<T>`) cannot be used as
/// `Vec<String>::try_convert(v)` — that's a parse error. They must use the UFCS form
/// `<Vec<String>>::try_convert(v)` instead. Simple names like `String`, `bool` can use
/// `String::try_convert(v)` directly.
fn as_type_path_prefix(type_str: &str) -> String {
    if type_str.contains('<') {
        format!("<{type_str}>")
    } else {
        type_str.to_string()
    }
}

/// Generate a hash-based Magnus constructor for types with many fields.
/// Accepts `(kwargs: RHash)` and extracts each field by symbol name, applying defaults.
fn gen_magnus_hash_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(1024);

    writeln!(out, "fn new(kwargs: magnus::RHash) -> Result<Self, magnus::Error> {{").ok();
    writeln!(out, "    let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").ok();
    writeln!(out, "    Ok(Self {{").ok();

    for field in &typ.fields {
        let is_optional = field_is_optional_in_rust(field);
        // Use inner type for try_convert, since the hash value is the inner T, not Option<T>.
        let inner_type = type_mapper(&field.ty);
        let type_prefix = as_type_path_prefix(&inner_type);
        if is_optional {
            // Field is Option<T>: extract from hash, wrap in Some, default to None
            writeln!(
                out,
                "        {name}: kwargs.get(ruby.to_symbol(\"{name}\")).and_then(|v| {type_prefix}::try_convert(v).ok()),",
                name = field.name,
                type_prefix = type_prefix,
            ).ok();
        } else if use_unwrap_or_default(field) {
            writeln!(
                out,
                "        {name}: kwargs.get(ruby.to_symbol(\"{name}\")).and_then(|v| {type_prefix}::try_convert(v).ok()).unwrap_or_default(),",
                name = field.name,
                type_prefix = type_prefix,
            ).ok();
        } else {
            let default_str = default_value_for_field(field, "rust");
            writeln!(
                out,
                "        {name}: kwargs.get(ruby.to_symbol(\"{name}\")).and_then(|v| {type_prefix}::try_convert(v).ok()).unwrap_or({default}),",
                name = field.name,
                type_prefix = type_prefix,
                default = default_str,
            ).ok();
        }
    }

    writeln!(out, "    }})").ok();
    writeln!(out, "}}").ok();

    out
}

/// Returns true if the generated Rust field type is already `Option<T>`.
/// This covers both:
/// - Fields with `optional: true` (the Rust field type becomes `Option<inner_type>`)
/// - Fields whose `TypeRef` is explicitly `Optional(_)` (rare, for nested Option types)
fn field_is_optional_in_rust(field: &FieldDef) -> bool {
    field.optional || matches!(&field.ty, TypeRef::Optional(_))
}

/// Generate a positional Magnus constructor for types with <=15 fields.
/// Uses `Option<T>` parameters and applies defaults in the body.
fn gen_magnus_positional_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "fn new(").ok();

    // All params are Option<T> so Ruby users can pass nil for any field.
    // If the Rust field type is already Option<T> (via optional:true or TypeRef::Optional),
    // use that type directly (avoids Option<Option<T>>).
    for (i, field) in typ.fields.iter().enumerate() {
        let comma = if i < typ.fields.len() - 1 { "," } else { "" };
        let is_optional = field_is_optional_in_rust(field);
        if is_optional {
            // field.ty is the inner type; the mapper maps inner type, we wrap in Option<>
            // BUT the type_mapper call site already wraps when field.optional==true.
            // Here we call type_mapper on the field's inner type directly to get the param type.
            let inner_type = type_mapper(&field.ty);
            writeln!(out, "    {}: Option<{}>{}", field.name, inner_type, comma).ok();
        } else {
            let field_type = type_mapper(&field.ty);
            writeln!(out, "    {}: Option<{}>{}", field.name, field_type, comma).ok();
        }
    }

    writeln!(out, ") -> Self {{").ok();
    writeln!(out, "    Self {{").ok();

    for field in &typ.fields {
        let is_optional = field_is_optional_in_rust(field);
        if is_optional {
            // The Rust field is Option<T>; param is Option<T>; assign directly.
            writeln!(out, "        {},", field.name).ok();
        } else if use_unwrap_or_default(field) {
            writeln!(out, "        {}: {}.unwrap_or_default(),", field.name, field.name).ok();
        } else {
            let default_str = default_value_for_field(field, "rust");
            writeln!(
                out,
                "        {}: {}.unwrap_or({}),",
                field.name, field.name, default_str
            )
            .ok();
        }
    }

    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    out
}

/// Generate a PHP kwargs constructor for a type with `has_default`.
/// All fields become `Option<T>` parameters so PHP users can omit any field.
/// Assignments wrap non-Optional fields in `Some()` and apply defaults.
pub fn gen_php_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "pub fn __construct(").ok();

    // All params are Option<MappedType> — PHP users can omit any field
    for (i, field) in typ.fields.iter().enumerate() {
        let mapped = type_mapper(&field.ty);
        let comma = if i < typ.fields.len() - 1 { "," } else { "" };
        writeln!(out, "    {}: Option<{}>{}", field.name, mapped, comma).ok();
    }

    writeln!(out, ") -> Self {{").ok();
    writeln!(out, "    Self {{").ok();

    for field in &typ.fields {
        let is_optional_field = field.optional || matches!(&field.ty, TypeRef::Optional(_));
        if is_optional_field {
            // Struct field is Option<T>, param is Option<T> — pass through directly
            writeln!(out, "        {},", field.name).ok();
        } else if use_unwrap_or_default(field) {
            // Struct field is T, param is Option<T> — unwrap with type's default
            writeln!(out, "        {}: {}.unwrap_or_default(),", field.name, field.name).ok();
        } else {
            // Struct field is T, param is Option<T> — unwrap with explicit default
            let default_str = default_value_for_field(field, "rust");
            writeln!(
                out,
                "        {}: {}.unwrap_or({}),",
                field.name, field.name, default_str
            )
            .ok();
        }
    }

    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    out
}

/// Generate a Rustler (Elixir) kwargs constructor for a type with `has_default`.
/// Accepts keyword list or map, applies defaults for missing fields.
pub fn gen_rustler_kwargs_constructor(typ: &TypeDef, _type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    // NifStruct already handles keyword list conversion, but we generate
    // an explicit constructor wrapper that applies defaults.
    writeln!(
        out,
        "pub fn new(opts: std::collections::HashMap<String, rustler::Term>) -> Self {{"
    )
    .ok();
    writeln!(out, "    Self {{").ok();

    // Field assignments with defaults from opts.
    // Optional fields (Option<T>) need special handling: decode the inner type
    // directly so we get Option<T> from and_then, with no unwrap_or needed.
    for field in &typ.fields {
        if field.optional {
            // Field type is Option<T>. Decode inner T from the Term, yielding Option<T>.
            writeln!(
                out,
                "        {}: opts.get(\"{}\").and_then(|t| t.decode().ok()),",
                field.name, field.name
            )
            .ok();
        } else if use_unwrap_or_default(field) {
            writeln!(
                out,
                "        {}: opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                field.name, field.name
            )
            .ok();
        } else {
            let default_str = default_value_for_field(field, "rust");
            // Check if the default value looks like an enum variant (e.g., "OutputFormat::Plain")
            // which wouldn't work for String types. If so, use unwrap_or_default() instead.
            let is_enum_variant_default = default_str.contains("::") || default_str.starts_with("\"");

            if is_enum_variant_default && matches!(&field.ty, TypeRef::String | TypeRef::Char) {
                // Use default for String types with enum-like defaults
                writeln!(
                    out,
                    "        {}: opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                    field.name, field.name
                )
                .ok();
            } else if matches!(&field.ty, TypeRef::Named(_)) {
                // For other Named types, use unwrap_or_default() since the binding type may differ
                // from the core type (e.g., excluded enums become String).
                writeln!(
                    out,
                    "        {}: opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or_default(),",
                    field.name, field.name
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "        {}: opts.get(\"{}\").and_then(|t| t.decode().ok()).unwrap_or({}),",
                    field.name, field.name, default_str
                )
                .ok();
            }
        }
    }

    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    out
}

/// Generate an extendr (R) kwargs constructor for a type with `has_default`.
/// Generates an R-callable function accepting named parameters with defaults.
pub fn gen_extendr_kwargs_constructor(typ: &TypeDef, type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "#[extendr]").ok();
    writeln!(out, "pub fn new_{}(", typ.name.to_lowercase()).ok();

    // Add all fields as named parameters with defaults
    for (i, field) in typ.fields.iter().enumerate() {
        let field_type = type_mapper(&field.ty);
        let default_str = default_value_for_field(field, "r");
        let comma = if i < typ.fields.len() - 1 { "," } else { "" };
        writeln!(out, "    {}: {} = {}{}", field.name, field_type, default_str, comma).ok();
    }

    writeln!(out, ") -> {} {{", typ.name).ok();
    writeln!(out, "    {} {{", typ.name).ok();

    // Field assignments
    for field in &typ.fields {
        writeln!(out, "        {},", field.name).ok();
    }

    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeRef};

    fn make_test_type() -> TypeDef {
        TypeDef {
            name: "Config".to_string(),
            rust_path: "my_crate::Config".to_string(),
            fields: vec![
                FieldDef {
                    name: "timeout".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U64),
                    optional: false,
                    default: Some("30".to_string()),
                    doc: "Timeout in seconds".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: Some(DefaultValue::IntLiteral(30)),
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                },
                FieldDef {
                    name: "enabled".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    optional: false,
                    default: None,
                    doc: "Enable feature".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: Some(DefaultValue::BoolLiteral(true)),
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                },
                FieldDef {
                    name: "name".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: "Config name".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: Some(DefaultValue::StringLiteral("default".to_string())),
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            doc: "Configuration type".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        }
    }

    #[test]
    fn test_default_value_bool_true_python() {
        let field = FieldDef {
            name: "enabled".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::BoolLiteral(true)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "True");
    }

    #[test]
    fn test_default_value_bool_false_go() {
        let field = FieldDef {
            name: "enabled".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::BoolLiteral(false)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "go"), "false");
    }

    #[test]
    fn test_default_value_string_literal() {
        let field = FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::StringLiteral("hello".to_string())),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "\"hello\"");
        assert_eq!(default_value_for_field(&field, "java"), "\"hello\"");
    }

    #[test]
    fn test_default_value_int_literal() {
        let field = FieldDef {
            name: "timeout".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::IntLiteral(42)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        let result = default_value_for_field(&field, "python");
        assert_eq!(result, "42");
    }

    #[test]
    fn test_default_value_none() {
        let field = FieldDef {
            name: "maybe".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::None),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "None");
        assert_eq!(default_value_for_field(&field, "go"), "nil");
        assert_eq!(default_value_for_field(&field, "java"), "null");
        assert_eq!(default_value_for_field(&field, "csharp"), "null");
    }

    #[test]
    fn test_default_value_fallback_string() {
        let field = FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: Some("\"custom\"".to_string()),
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "\"custom\"");
    }

    #[test]
    fn test_gen_pyo3_kwargs_constructor() {
        let typ = make_test_type();
        let output = gen_pyo3_kwargs_constructor(&typ, &|tr: &TypeRef| match tr {
            TypeRef::Primitive(p) => format!("{:?}", p),
            TypeRef::String | TypeRef::Char => "str".to_string(),
            _ => "Any".to_string(),
        });

        assert!(output.contains("#[new]"));
        assert!(output.contains("#[pyo3(signature = ("));
        assert!(output.contains("timeout=30"));
        assert!(output.contains("enabled=True"));
        assert!(output.contains("name=\"default\""));
        assert!(output.contains("fn new("));
    }

    #[test]
    fn test_gen_napi_defaults_constructor() {
        let typ = make_test_type();
        let output = gen_napi_defaults_constructor(&typ, &|tr: &TypeRef| match tr {
            TypeRef::Primitive(p) => format!("{:?}", p),
            TypeRef::String | TypeRef::Char => "String".to_string(),
            _ => "Value".to_string(),
        });

        assert!(output.contains("pub fn new(mut env: napi::Env, obj: napi::Object)"));
        assert!(output.contains("timeout"));
        assert!(output.contains("enabled"));
        assert!(output.contains("name"));
    }

    #[test]
    fn test_gen_go_functional_options() {
        let typ = make_test_type();
        let output = gen_go_functional_options(&typ, &|tr: &TypeRef| match tr {
            TypeRef::Primitive(p) => match p {
                PrimitiveType::U64 => "uint64".to_string(),
                PrimitiveType::Bool => "bool".to_string(),
                _ => "interface{}".to_string(),
            },
            TypeRef::String | TypeRef::Char => "string".to_string(),
            _ => "interface{}".to_string(),
        });

        assert!(output.contains("type Config struct {"));
        assert!(output.contains("type ConfigOption func(*Config)"));
        assert!(output.contains("func WithConfigTimeout(val uint64) ConfigOption"));
        assert!(output.contains("func WithConfigEnabled(val bool) ConfigOption"));
        assert!(output.contains("func WithConfigName(val string) ConfigOption"));
        assert!(output.contains("func NewConfig(opts ...ConfigOption) *Config"));
    }

    #[test]
    fn test_gen_java_builder() {
        let typ = make_test_type();
        let output = gen_java_builder(&typ, "dev.test", &|tr: &TypeRef| match tr {
            TypeRef::Primitive(p) => match p {
                PrimitiveType::U64 => "long".to_string(),
                PrimitiveType::Bool => "boolean".to_string(),
                _ => "int".to_string(),
            },
            TypeRef::String | TypeRef::Char => "String".to_string(),
            _ => "Object".to_string(),
        });

        assert!(output.contains("package dev.test;"));
        assert!(output.contains("public class ConfigBuilder"));
        assert!(output.contains("withTimeout"));
        assert!(output.contains("withEnabled"));
        assert!(output.contains("withName"));
        assert!(output.contains("public Config build()"));
    }

    #[test]
    fn test_gen_csharp_record() {
        let typ = make_test_type();
        let output = gen_csharp_record(&typ, "MyNamespace", &|tr: &TypeRef| match tr {
            TypeRef::Primitive(p) => match p {
                PrimitiveType::U64 => "ulong".to_string(),
                PrimitiveType::Bool => "bool".to_string(),
                _ => "int".to_string(),
            },
            TypeRef::String | TypeRef::Char => "string".to_string(),
            _ => "object".to_string(),
        });

        assert!(output.contains("namespace MyNamespace;"));
        assert!(output.contains("public record Config"));
        assert!(output.contains("public ulong Timeout"));
        assert!(output.contains("public bool Enabled"));
        assert!(output.contains("public string Name"));
        assert!(output.contains("init;"));
    }

    #[test]
    fn test_default_value_float_literal() {
        let field = FieldDef {
            name: "ratio".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::F64),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::FloatLiteral(1.5)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        let result = default_value_for_field(&field, "python");
        assert!(result.contains("1.5"));
    }

    #[test]
    fn test_default_value_no_typed_no_default() {
        let field = FieldDef {
            name: "count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        // Should fall back to type-based zero value
        assert_eq!(default_value_for_field(&field, "python"), "0");
        assert_eq!(default_value_for_field(&field, "go"), "0");
    }
}
