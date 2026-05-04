use alef_core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};

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
            DefaultValue::EnumVariant(v) => {
                // When the field's original enum type was excluded/sanitized and mapped to
                // String, we must emit a string literal rather than an enum type path.
                // Example: OutputFormat::Plain → "plain".to_string() (Rust), "plain" (others).
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

    // Hash-based constructor accepts 0 or 1 arguments using scan_args
    writeln!(out, "fn new(args: &[magnus::Value]) -> Result<Self, magnus::Error> {{").ok();
    writeln!(out, "    let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").ok();
    writeln!(
        out,
        "    let args = magnus::scan_args::scan_args::<(), (Option<magnus::RHash>,), (), (), (), ()>(args)?;"
    )
    .ok();
    writeln!(out, "    let (kwargs_opt,) = args.optional;").ok();
    writeln!(out, "    let kwargs = kwargs_opt.unwrap_or_else(|| ruby.hash_new());").ok();
    writeln!(out, "    Ok(Self {{").ok();

    for field in &typ.fields {
        let is_optional = field_is_optional_in_rust(field);
        // Use inner type for try_convert, since the hash value is T, not Option<T>.
        // When field.ty is already Optional(T) and field.optional is true, strip one layer so we
        // call <T>::try_convert, not <Option<T>>::try_convert (which would yield Option<Option<T>>).
        let effective_inner_ty = match &field.ty {
            TypeRef::Optional(inner) if is_optional => inner.as_ref(),
            ty => ty,
        };
        let inner_type = type_mapper(effective_inner_ty);
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
            // When the binding maps the field type to String (e.g. an excluded enum), but the
            // original default is an EnumVariant, `default_value_for_field` would emit
            // `TypeName::Variant` which is invalid for a `String` field. Fall back to the
            // string-literal form in that case.
            let default_str = if inner_type == "String" {
                if let Some(DefaultValue::EnumVariant(variant)) = &field.typed_default {
                    use heck::ToSnakeCase;
                    format!("\"{}\".to_string()", variant.to_snake_case())
                } else {
                    default_value_for_field(field, "rust")
                }
            } else {
                default_value_for_field(field, "rust")
            };
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
            // Strip one Optional wrapper when ty is Optional(T) AND field is marked optional,
            // to avoid emitting Option<Option<T>>. The param represents Option<inner>, not
            // Option<Option<inner>>.
            let effective_inner_ty = match &field.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                ty => ty,
            };
            let inner_type = type_mapper(effective_inner_ty);
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
/// Fields in `exclude_fields` are skipped (used for bridge fields that cannot implement Encoder/Decoder).
pub fn gen_rustler_kwargs_constructor_with_exclude(
    typ: &TypeDef,
    _type_mapper: &dyn Fn(&TypeRef) -> String,
    exclude_fields: &std::collections::HashSet<String>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(
        out,
        "pub fn new(opts: std::collections::HashMap<String, rustler::Term>) -> Self {{"
    )
    .ok();
    writeln!(out, "    Self {{").ok();

    for field in &typ.fields {
        if exclude_fields.contains(&field.name) {
            continue;
        }
        if field.optional {
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
            let is_enum_variant_default = default_str.contains("::") || default_str.starts_with("\"");

            if (is_enum_variant_default && matches!(&field.ty, TypeRef::String | TypeRef::Char))
                || matches!(&field.ty, TypeRef::Named(_))
            {
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
///
/// Rust does not support function-parameter defaults, and extendr 0.9 only allows
/// defaults via the per-parameter `#[extendr(default = "...")]` attribute (not via
/// `param: T = expr` syntax).  Rather than encode every default in attribute form,
/// we accept each field as `Option<T>` and unwrap it via `T::default()` (or via the
/// type's own `Default::default()` for the whole struct as the base) inside the body.
/// The R-side wrapper generated in `generate_public_api` already supplies named
/// arguments with `NULL` defaults, so callers see ergonomic kwargs at the R level.
///
/// `enum_names` is the set of type names that are enums in this API surface.  For
/// fields whose type resolves to a Named enum, the parameter is widened to
/// `Option<String>` (extendr has no `TryFrom<&Robj>` for binding enums) and the body
/// deserialises the string back to the enum via `serde_json::from_str`.
pub fn gen_extendr_kwargs_constructor(
    typ: &TypeDef,
    type_mapper: &dyn Fn(&TypeRef) -> String,
    enum_names: &ahash::AHashSet<String>,
) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(512);

    writeln!(out, "#[extendr]").ok();
    writeln!(out, "pub fn new_{}(", typ.name.to_lowercase()).ok();

    // Add all fields as Option<T> parameters — extendr passes NULL → None which lets
    // each field fall through to the struct's Default value when omitted.
    // Fields whose inner type is a binding enum use Option<String> because extendr has
    // no TryFrom<&Robj> impl for enum types; the string is parsed back in the body.
    // Fields that are non-opaque, non-enum named types (structs) are mapped to Robj
    // since extendr cannot convert Option<SomeStruct> from R automatically.
    let is_named_enum = |ty: &TypeRef| -> bool { matches!(ty, TypeRef::Named(n) if enum_names.contains(n.as_str())) };
    let is_named_struct = |ty: &TypeRef| -> bool {
        // A bare Named type that is not an enum — treated as Robj in params since
        // extendr only generates TryFrom<&Robj> for &Foo (reference), not for Foo (owned).
        matches!(ty, TypeRef::Named(n) if !enum_names.contains(n.as_str()))
    };
    let is_optional_named_struct = |ty: &TypeRef| -> bool {
        if let TypeRef::Optional(inner) = ty {
            is_named_struct(inner)
        } else {
            false
        }
    };
    // Returns true if the field type is already Optional (TypeRef::Optional or has field.optional=true
    // with a type that the mapper wraps in Option<>). Used to prevent double-wrapping.
    let ty_is_optional = |ty: &TypeRef| -> bool { matches!(ty, TypeRef::Optional(_)) };

    // Pre-collect emittable fields (skip struct-typed fields that extendr cannot convert).
    let emittable_fields: Vec<&FieldDef> = typ
        .fields
        .iter()
        .filter(|f| !is_named_struct(&f.ty) && !is_optional_named_struct(&f.ty))
        .collect();

    for (i, field) in emittable_fields.iter().enumerate() {
        let comma = if i < emittable_fields.len() - 1 { "," } else { "" };
        if is_named_enum(&field.ty) {
            // Enum fields: use Option<String> — parsed back via serde_json in the body.
            writeln!(out, "    {}: Option<String>{}", field.name, comma).ok();
        } else if ty_is_optional(&field.ty) {
            // Already Optional type: type_mapper emits "Option<T>", so don't double-wrap.
            // The param is the same type as the field (no extra Option wrapper needed for kwargs).
            let param_type = type_mapper(&field.ty);
            writeln!(out, "    {}: {}{}", field.name, param_type, comma).ok();
        } else {
            let param_type = type_mapper(&field.ty);
            writeln!(out, "    {}: Option<{}>{}", field.name, param_type, comma).ok();
        }
    }

    writeln!(out, ") -> {} {{", typ.name).ok();
    // Use the type's Default impl as a base so unspecified fields keep their natural
    // defaults rather than `Default::default()` for each individual field type (which
    // would, for example, give an empty String for fields whose true default is
    // `"utf-8"`).  Then overlay any caller-provided values.
    writeln!(out, "    let mut __out = <{}>::default();", typ.name).ok();
    for field in &typ.fields {
        // Skip struct-typed fields — they were omitted from the parameter list and
        // will keep their Default value from __out.
        if is_named_struct(&field.ty) || is_optional_named_struct(&field.ty) {
            continue;
        }
        if is_named_enum(&field.ty) {
            // Enum field via String: parse with serde_json, fall back to Default on error.
            if field.optional {
                writeln!(
                    out,
                    "    if let Some(v) = {name} {{ __out.{name} = serde_json::from_str(&format!(\"\\\"{{v}}\\\"\")).ok(); }}",
                    name = field.name
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "    if let Some(v) = {name} {{ if let Ok(parsed) = serde_json::from_str(&format!(\"\\\"{{v}}\\\"\")) {{ __out.{name} = parsed; }} }}",
                    name = field.name
                )
                .ok();
            }
        } else if ty_is_optional(&field.ty) {
            // Already Optional: param IS Option<inner>, assign through Some on match.
            writeln!(
                out,
                "    if let Some(v) = {name} {{ __out.{name} = Some(v); }}",
                name = field.name
            )
            .ok();
        } else if field.optional {
            // Optional flag set but type is plain T: wrap with Some.
            writeln!(
                out,
                "    if let Some(v) = {name} {{ __out.{name} = Some(v); }}",
                name = field.name
            )
            .ok();
        } else {
            writeln!(
                out,
                "    if let Some(v) = {name} {{ __out.{name} = v; }}",
                name = field.name
            )
            .ok();
        }
    }
    writeln!(out, "    __out").ok();
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
            original_rust_path: String::new(),
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
            is_copy: false,
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

    fn make_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
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
        }
    }

    fn simple_type_mapper(tr: &TypeRef) -> String {
        match tr {
            TypeRef::Primitive(p) => match p {
                PrimitiveType::U64 => "u64".to_string(),
                PrimitiveType::Bool => "bool".to_string(),
                PrimitiveType::U32 => "u32".to_string(),
                _ => "i64".to_string(),
            },
            TypeRef::String | TypeRef::Char => "String".to_string(),
            TypeRef::Optional(inner) => format!("Option<{}>", simple_type_mapper(inner)),
            TypeRef::Vec(inner) => format!("Vec<{}>", simple_type_mapper(inner)),
            TypeRef::Named(n) => n.clone(),
            _ => "Value".to_string(),
        }
    }

    // -------------------------------------------------------------------------
    // default_value_for_field — untested branches
    // -------------------------------------------------------------------------

    #[test]
    fn test_default_value_bool_literal_ruby() {
        let field = FieldDef {
            name: "flag".to_string(),
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
        assert_eq!(default_value_for_field(&field, "ruby"), "true");
        assert_eq!(default_value_for_field(&field, "php"), "true");
        assert_eq!(default_value_for_field(&field, "csharp"), "true");
        assert_eq!(default_value_for_field(&field, "java"), "true");
        assert_eq!(default_value_for_field(&field, "rust"), "true");
    }

    #[test]
    fn test_default_value_bool_literal_r() {
        let field = FieldDef {
            name: "flag".to_string(),
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
        assert_eq!(default_value_for_field(&field, "r"), "FALSE");
    }

    #[test]
    fn test_default_value_string_literal_rust() {
        let field = FieldDef {
            name: "label".to_string(),
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
        assert_eq!(default_value_for_field(&field, "rust"), "\"hello\".to_string()");
    }

    #[test]
    fn test_default_value_string_literal_escapes_quotes() {
        let field = FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::StringLiteral("say \"hi\"".to_string())),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn test_default_value_float_literal_whole_number() {
        // A whole-number float should be rendered with ".0" suffix.
        let field = FieldDef {
            name: "scale".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::F32),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::FloatLiteral(2.0)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        let result = default_value_for_field(&field, "python");
        assert!(result.contains('.'), "whole-number float should contain '.': {result}");
    }

    #[test]
    fn test_default_value_enum_variant_per_language() {
        let field = FieldDef {
            name: "format".to_string(),
            ty: TypeRef::Named("OutputFormat".to_string()),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::EnumVariant("JsonOutput".to_string())),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "OutputFormat.JSON_OUTPUT");
        assert_eq!(default_value_for_field(&field, "ruby"), "OutputFormat::JsonOutput");
        assert_eq!(default_value_for_field(&field, "go"), "OutputFormatJsonOutput");
        assert_eq!(default_value_for_field(&field, "java"), "OutputFormat.JSON_OUTPUT");
        assert_eq!(default_value_for_field(&field, "csharp"), "OutputFormat.JsonOutput");
        assert_eq!(default_value_for_field(&field, "php"), "OutputFormat::JsonOutput");
        assert_eq!(default_value_for_field(&field, "r"), "OutputFormat$JsonOutput");
        assert_eq!(default_value_for_field(&field, "rust"), "OutputFormat::JsonOutput");
    }

    #[test]
    fn test_default_value_empty_vec_per_language() {
        let field = FieldDef {
            name: "items".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "[]");
        assert_eq!(default_value_for_field(&field, "ruby"), "[]");
        assert_eq!(default_value_for_field(&field, "csharp"), "[]");
        assert_eq!(default_value_for_field(&field, "go"), "nil");
        assert_eq!(default_value_for_field(&field, "java"), "List.of()");
        assert_eq!(default_value_for_field(&field, "php"), "[]");
        assert_eq!(default_value_for_field(&field, "r"), "c()");
        assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
    }

    #[test]
    fn test_default_value_empty_map_per_language() {
        let field = FieldDef {
            name: "meta".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "{}");
        assert_eq!(default_value_for_field(&field, "go"), "nil");
        assert_eq!(default_value_for_field(&field, "java"), "Map.of()");
        assert_eq!(default_value_for_field(&field, "rust"), "Default::default()");
    }

    #[test]
    fn test_default_value_empty_bool_primitive() {
        let field = FieldDef {
            name: "flag".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "False");
        assert_eq!(default_value_for_field(&field, "ruby"), "false");
        assert_eq!(default_value_for_field(&field, "go"), "false");
    }

    #[test]
    fn test_default_value_empty_float_primitive() {
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
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "0.0");
    }

    #[test]
    fn test_default_value_empty_string_type() {
        let field = FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "rust"), "String::new()");
        assert_eq!(default_value_for_field(&field, "python"), "\"\"");
    }

    #[test]
    fn test_default_value_empty_bytes_type() {
        let field = FieldDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "b\"\"");
        assert_eq!(default_value_for_field(&field, "go"), "[]byte{}");
        assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
    }

    #[test]
    fn test_default_value_empty_json_type() {
        let field = FieldDef {
            name: "payload".to_string(),
            ty: TypeRef::Json,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        };
        assert_eq!(default_value_for_field(&field, "python"), "{}");
        assert_eq!(default_value_for_field(&field, "ruby"), "{}");
        assert_eq!(default_value_for_field(&field, "go"), "json.RawMessage(nil)");
        assert_eq!(default_value_for_field(&field, "r"), "list()");
        assert_eq!(default_value_for_field(&field, "rust"), "serde_json::json!({})");
    }

    #[test]
    fn test_default_value_none_ruby_php_r() {
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
        assert_eq!(default_value_for_field(&field, "ruby"), "nil");
        assert_eq!(default_value_for_field(&field, "php"), "null");
        assert_eq!(default_value_for_field(&field, "r"), "NULL");
        assert_eq!(default_value_for_field(&field, "rust"), "None");
    }

    // -------------------------------------------------------------------------
    // Fallback (no typed_default, no default) — type-based zero values
    // -------------------------------------------------------------------------

    #[test]
    fn test_default_value_fallback_bool_all_languages() {
        let field = make_field("flag", TypeRef::Primitive(PrimitiveType::Bool));
        assert_eq!(default_value_for_field(&field, "python"), "False");
        assert_eq!(default_value_for_field(&field, "ruby"), "false");
        assert_eq!(default_value_for_field(&field, "csharp"), "false");
        assert_eq!(default_value_for_field(&field, "java"), "false");
        assert_eq!(default_value_for_field(&field, "php"), "false");
        assert_eq!(default_value_for_field(&field, "r"), "FALSE");
        assert_eq!(default_value_for_field(&field, "rust"), "false");
    }

    #[test]
    fn test_default_value_fallback_float() {
        let field = make_field("ratio", TypeRef::Primitive(PrimitiveType::F64));
        assert_eq!(default_value_for_field(&field, "python"), "0.0");
        assert_eq!(default_value_for_field(&field, "rust"), "0.0");
    }

    #[test]
    fn test_default_value_fallback_string_all_languages() {
        let field = make_field("name", TypeRef::String);
        assert_eq!(default_value_for_field(&field, "python"), "\"\"");
        assert_eq!(default_value_for_field(&field, "ruby"), "\"\"");
        assert_eq!(default_value_for_field(&field, "go"), "\"\"");
        assert_eq!(default_value_for_field(&field, "java"), "\"\"");
        assert_eq!(default_value_for_field(&field, "csharp"), "\"\"");
        assert_eq!(default_value_for_field(&field, "php"), "\"\"");
        assert_eq!(default_value_for_field(&field, "r"), "\"\"");
        assert_eq!(default_value_for_field(&field, "rust"), "String::new()");
    }

    #[test]
    fn test_default_value_fallback_bytes_all_languages() {
        let field = make_field("data", TypeRef::Bytes);
        assert_eq!(default_value_for_field(&field, "python"), "b\"\"");
        assert_eq!(default_value_for_field(&field, "ruby"), "\"\"");
        assert_eq!(default_value_for_field(&field, "go"), "[]byte{}");
        assert_eq!(default_value_for_field(&field, "java"), "new byte[]{}");
        assert_eq!(default_value_for_field(&field, "csharp"), "new byte[]{}");
        assert_eq!(default_value_for_field(&field, "php"), "\"\"");
        assert_eq!(default_value_for_field(&field, "r"), "raw()");
        assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
    }

    #[test]
    fn test_default_value_fallback_optional() {
        let field = make_field("maybe", TypeRef::Optional(Box::new(TypeRef::String)));
        assert_eq!(default_value_for_field(&field, "python"), "None");
        assert_eq!(default_value_for_field(&field, "ruby"), "nil");
        assert_eq!(default_value_for_field(&field, "go"), "nil");
        assert_eq!(default_value_for_field(&field, "java"), "null");
        assert_eq!(default_value_for_field(&field, "csharp"), "null");
        assert_eq!(default_value_for_field(&field, "php"), "null");
        assert_eq!(default_value_for_field(&field, "r"), "NULL");
        assert_eq!(default_value_for_field(&field, "rust"), "None");
    }

    #[test]
    fn test_default_value_fallback_vec_all_languages() {
        let field = make_field("items", TypeRef::Vec(Box::new(TypeRef::String)));
        assert_eq!(default_value_for_field(&field, "python"), "[]");
        assert_eq!(default_value_for_field(&field, "ruby"), "[]");
        assert_eq!(default_value_for_field(&field, "go"), "[]interface{}{}");
        assert_eq!(default_value_for_field(&field, "java"), "new java.util.ArrayList<>()");
        assert_eq!(default_value_for_field(&field, "csharp"), "[]");
        assert_eq!(default_value_for_field(&field, "php"), "[]");
        assert_eq!(default_value_for_field(&field, "r"), "c()");
        assert_eq!(default_value_for_field(&field, "rust"), "vec![]");
    }

    #[test]
    fn test_default_value_fallback_map_all_languages() {
        let field = make_field(
            "meta",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        );
        assert_eq!(default_value_for_field(&field, "python"), "{}");
        assert_eq!(default_value_for_field(&field, "ruby"), "{}");
        assert_eq!(default_value_for_field(&field, "go"), "make(map[string]interface{})");
        assert_eq!(default_value_for_field(&field, "java"), "new java.util.HashMap<>()");
        assert_eq!(
            default_value_for_field(&field, "csharp"),
            "new Dictionary<string, object>()"
        );
        assert_eq!(default_value_for_field(&field, "php"), "[]");
        assert_eq!(default_value_for_field(&field, "r"), "list()");
        assert_eq!(
            default_value_for_field(&field, "rust"),
            "std::collections::HashMap::new()"
        );
    }

    #[test]
    fn test_default_value_fallback_json_all_languages() {
        let field = make_field("payload", TypeRef::Json);
        assert_eq!(default_value_for_field(&field, "python"), "{}");
        assert_eq!(default_value_for_field(&field, "ruby"), "{}");
        assert_eq!(default_value_for_field(&field, "go"), "json.RawMessage(nil)");
        assert_eq!(default_value_for_field(&field, "r"), "list()");
        assert_eq!(default_value_for_field(&field, "rust"), "serde_json::json!({})");
    }

    #[test]
    fn test_default_value_fallback_named_type() {
        let field = make_field("config", TypeRef::Named("MyConfig".to_string()));
        assert_eq!(default_value_for_field(&field, "rust"), "MyConfig::default()");
        assert_eq!(default_value_for_field(&field, "python"), "None");
        assert_eq!(default_value_for_field(&field, "ruby"), "nil");
        assert_eq!(default_value_for_field(&field, "go"), "nil");
        assert_eq!(default_value_for_field(&field, "java"), "null");
        assert_eq!(default_value_for_field(&field, "csharp"), "null");
        assert_eq!(default_value_for_field(&field, "php"), "null");
        assert_eq!(default_value_for_field(&field, "r"), "NULL");
    }

    #[test]
    fn test_default_value_fallback_duration() {
        // Duration falls through to the wildcard arm
        let field = make_field("timeout", TypeRef::Duration);
        assert_eq!(default_value_for_field(&field, "python"), "None");
        assert_eq!(default_value_for_field(&field, "rust"), "Default::default()");
    }

    // -------------------------------------------------------------------------
    // gen_magnus_kwargs_constructor — positional (≤15 fields)
    // -------------------------------------------------------------------------

    #[test]
    fn test_gen_magnus_kwargs_constructor_positional_basic() {
        let typ = make_test_type();
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

        assert!(output.contains("fn new("), "should have fn new");
        // All params are Option<T>
        assert!(output.contains("Option<u64>"), "timeout should be Option<u64>");
        assert!(output.contains("Option<bool>"), "enabled should be Option<bool>");
        assert!(output.contains("Option<String>"), "name should be Option<String>");
        assert!(output.contains("-> Self {"), "should return Self");
        // timeout has IntLiteral(30), use_unwrap_or_default is false for Named → uses unwrap_or
        assert!(
            output.contains("timeout: timeout.unwrap_or(30),"),
            "should apply int default"
        );
        // enabled has BoolLiteral(true), not unwrap_or_default
        assert!(
            output.contains("enabled: enabled.unwrap_or(true),"),
            "should apply bool default"
        );
        // name has StringLiteral, not unwrap_or_default
        assert!(
            output.contains("name: name.unwrap_or(\"default\".to_string()),"),
            "should apply string default"
        );
    }

    #[test]
    fn test_gen_magnus_kwargs_constructor_positional_optional_field() {
        // A field with optional=true should be assigned directly (no unwrap)
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
            name: "extra".to_string(),
            ty: TypeRef::String,
            optional: true,
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
        });
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
        // Optional field param is Option<String> and assigned directly
        assert!(output.contains("extra,"), "optional field should be assigned directly");
        assert!(!output.contains("extra.unwrap"), "optional field should not use unwrap");
    }

    #[test]
    fn test_gen_magnus_kwargs_constructor_unwrap_or_default() {
        // A primitive field with no typed_default and no default should use unwrap_or_default()
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
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
        });
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output.contains("count: count.unwrap_or_default(),"),
            "plain primitive with no default should use unwrap_or_default"
        );
    }

    #[test]
    fn test_gen_magnus_kwargs_constructor_hash_path_for_many_fields() {
        // Build a type with 16 fields (> MAGNUS_MAX_ARITY = 15) to force hash path
        let mut fields: Vec<FieldDef> = (0..16)
            .map(|i| FieldDef {
                name: format!("field_{i}"),
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
            })
            .collect();
        // Make one field optional to exercise that branch in the hash constructor
        fields[0].optional = true;

        let typ = TypeDef {
            name: "BigConfig".to_string(),
            rust_path: "crate::BigConfig".to_string(),
            original_rust_path: String::new(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        };
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);

        assert!(output.contains("kwargs: magnus::RHash"), "should accept RHash");
        assert!(output.contains("ruby.to_symbol("), "should use symbol lookup");
        // Optional field uses and_then without unwrap_or
        assert!(
            output.contains("field_0: kwargs.get(ruby.to_symbol(\"field_0\")).and_then(|v|"),
            "optional field should use and_then"
        );
        assert!(
            output.contains("field_0:").then_some(()).is_some(),
            "field_0 should appear in output"
        );
    }

    // -------------------------------------------------------------------------
    // gen_php_kwargs_constructor
    // -------------------------------------------------------------------------

    #[test]
    fn test_gen_php_kwargs_constructor_basic() {
        let typ = make_test_type();
        let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);

        assert!(
            output.contains("pub fn __construct("),
            "should use PHP constructor name"
        );
        // All params are Option<T>
        assert!(
            output.contains("timeout: Option<u64>"),
            "timeout param should be Option<u64>"
        );
        assert!(
            output.contains("enabled: Option<bool>"),
            "enabled param should be Option<bool>"
        );
        assert!(
            output.contains("name: Option<String>"),
            "name param should be Option<String>"
        );
        assert!(output.contains("-> Self {"), "should return Self");
        assert!(
            output.contains("timeout: timeout.unwrap_or(30),"),
            "should apply int default for timeout"
        );
        assert!(
            output.contains("enabled: enabled.unwrap_or(true),"),
            "should apply bool default for enabled"
        );
        assert!(
            output.contains("name: name.unwrap_or(\"default\".to_string()),"),
            "should apply string default for name"
        );
    }

    #[test]
    fn test_gen_php_kwargs_constructor_optional_field_passthrough() {
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
            name: "tag".to_string(),
            ty: TypeRef::String,
            optional: true,
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
        });
        let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output.contains("tag,"),
            "optional field should be passed through directly"
        );
        assert!(!output.contains("tag.unwrap"), "optional field should not call unwrap");
    }

    #[test]
    fn test_gen_php_kwargs_constructor_unwrap_or_default_for_primitive() {
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
            name: "retries".to_string(),
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
        });
        let output = gen_php_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output.contains("retries: retries.unwrap_or_default(),"),
            "primitive with no default should use unwrap_or_default"
        );
    }

    // -------------------------------------------------------------------------
    // gen_rustler_kwargs_constructor
    // -------------------------------------------------------------------------

    #[test]
    fn test_gen_rustler_kwargs_constructor_basic() {
        let typ = make_test_type();
        let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);

        assert!(
            output.contains("pub fn new(opts: std::collections::HashMap<String, rustler::Term>)"),
            "should accept HashMap of Terms"
        );
        assert!(output.contains("Self {"), "should construct Self");
        // timeout has IntLiteral(30) — explicit unwrap_or
        assert!(
            output.contains("timeout: opts.get(\"timeout\").and_then(|t| t.decode().ok()).unwrap_or(30),"),
            "should apply int default for timeout"
        );
        // enabled has BoolLiteral(true) — explicit unwrap_or
        assert!(
            output.contains("enabled: opts.get(\"enabled\").and_then(|t| t.decode().ok()).unwrap_or(true),"),
            "should apply bool default for enabled"
        );
    }

    #[test]
    fn test_gen_rustler_kwargs_constructor_optional_field() {
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
            name: "extra".to_string(),
            ty: TypeRef::String,
            optional: true,
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
        });
        let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output.contains("extra: opts.get(\"extra\").and_then(|t| t.decode().ok()),"),
            "optional field should decode without unwrap"
        );
    }

    #[test]
    fn test_gen_rustler_kwargs_constructor_named_type_uses_unwrap_or_default() {
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
            name: "inner".to_string(),
            ty: TypeRef::Named("InnerConfig".to_string()),
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
        });
        let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output.contains("inner: opts.get(\"inner\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
            "Named type with no default should use unwrap_or_default"
        );
    }

    #[test]
    fn test_gen_rustler_kwargs_constructor_string_field_uses_unwrap_or_default() {
        // A String field with a StringLiteral default contains "::", triggering the
        // is_enum_variant_default check — should fall back to unwrap_or_default().
        let mut typ = make_test_type();
        // 'name' field in make_test_type() has StringLiteral("default") — verify it
        let output = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output.contains("name: opts.get(\"name\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
            "String field with quoted default should use unwrap_or_default"
        );
        // Also verify a plain string field (no default) also falls through to unwrap_or_default
        typ.fields.push(FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
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
        });
        let output2 = gen_rustler_kwargs_constructor(&typ, &simple_type_mapper);
        assert!(
            output2.contains("label: opts.get(\"label\").and_then(|t| t.decode().ok()).unwrap_or_default(),"),
            "String field with no default should use unwrap_or_default"
        );
    }

    // -------------------------------------------------------------------------
    // gen_extendr_kwargs_constructor
    // -------------------------------------------------------------------------

    #[test]
    fn test_gen_extendr_kwargs_constructor_basic() {
        let typ = make_test_type();
        let empty_enums = ahash::AHashSet::new();
        let output = gen_extendr_kwargs_constructor(&typ, &simple_type_mapper, &empty_enums);

        assert!(output.contains("#[extendr]"), "should have extendr attribute");
        assert!(
            output.contains("pub fn new_config("),
            "function name should be lowercase type name"
        );
        // Fields appear as Option<T> parameters — Rust does not support param defaults.
        assert!(
            output.contains("timeout: Option<u64>"),
            "should accept timeout as Option<u64>: {output}"
        );
        assert!(
            output.contains("enabled: Option<bool>"),
            "should accept enabled as Option<bool>: {output}"
        );
        assert!(
            output.contains("name: Option<String>"),
            "should accept name as Option<String>: {output}"
        );
        assert!(output.contains("-> Config {"), "should return Config");
        assert!(
            output.contains("let mut __out = <Config>::default();"),
            "should base on Default impl: {output}"
        );
        assert!(
            output.contains("if let Some(v) = timeout { __out.timeout = v; }"),
            "should overlay caller-provided timeout"
        );
        assert!(
            output.contains("if let Some(v) = enabled { __out.enabled = v; }"),
            "should overlay caller-provided enabled"
        );
        assert!(
            output.contains("if let Some(v) = name { __out.name = v; }"),
            "should overlay caller-provided name"
        );
    }

    #[test]
    fn test_gen_extendr_kwargs_constructor_uses_option_for_all_fields() {
        // Rust function-parameter defaults (`x: T = expr`) are a syntax error and
        // extendr 0.9 only supports defaults via the `#[extendr(default = "...")]`
        // attribute.  Verify that no field is emitted with a Rust-syntax default.
        let typ = make_test_type();
        let empty_enums = ahash::AHashSet::new();
        let output = gen_extendr_kwargs_constructor(&typ, &simple_type_mapper, &empty_enums);
        assert!(
            !output.contains("= TRUE") && !output.contains("= FALSE") && !output.contains("= \"default\""),
            "constructor must not use Rust-syntax param defaults: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // gen_go_functional_options — tuple-field filtering
    // -------------------------------------------------------------------------

    #[test]
    fn test_gen_go_functional_options_skips_tuple_fields() {
        let mut typ = make_test_type();
        typ.fields.push(FieldDef {
            name: "_0".to_string(),
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
        });
        let output = gen_go_functional_options(&typ, &simple_type_mapper);
        assert!(
            !output.contains("_0"),
            "tuple field _0 should be filtered out from Go output"
        );
    }

    // -------------------------------------------------------------------------
    // as_type_path_prefix — tested indirectly through hash constructor
    // -------------------------------------------------------------------------

    #[test]
    fn test_gen_magnus_hash_constructor_generic_type_prefix() {
        // A field with a Vec type should use <Vec<...>>::try_convert UFCS form
        let fields: Vec<FieldDef> = (0..16)
            .map(|i| FieldDef {
                name: format!("field_{i}"),
                ty: if i == 0 {
                    TypeRef::Vec(Box::new(TypeRef::String))
                } else {
                    TypeRef::Primitive(PrimitiveType::U32)
                },
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
            })
            .collect();
        let typ = TypeDef {
            name: "WideConfig".to_string(),
            rust_path: "crate::WideConfig".to_string(),
            original_rust_path: String::new(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        };
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
        // Vec<String> is a generic type; must use <Vec<String>>::try_convert
        assert!(
            output.contains("<Vec<String>>::try_convert"),
            "generic types should use UFCS angle-bracket prefix: {output}"
        );
    }

    // -------------------------------------------------------------------------
    // Bug B regression: Option<Option<T>> must not appear when field.optional==true
    // and field.ty==Optional(T). This happens for "Update" structs where the core
    // field is Option<Option<T>> — the binding flattens to Option<T>.
    // -------------------------------------------------------------------------

    #[test]
    fn test_magnus_hash_constructor_no_double_option_when_ty_is_optional() {
        // field with optional=true AND ty=Optional(Usize) — represents a core Option<Option<usize>>
        // that should flatten to Option<usize> in the binding constructor.
        // simple_type_mapper maps Usize → "i64" (catch-all primitive arm).
        let field = FieldDef {
            name: "max_depth".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
            optional: true,
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
        // Build a large type (>15 fields) so the hash constructor is used
        let mut fields: Vec<FieldDef> = (0..15)
            .map(|i| FieldDef {
                name: format!("field_{i}"),
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
            })
            .collect();
        fields.push(field);
        let typ = TypeDef {
            name: "UpdateConfig".to_string(),
            rust_path: "crate::UpdateConfig".to_string(),
            original_rust_path: String::new(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        };
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
        // The try_convert call must be for the inner type (i64, as mapped by simple_type_mapper),
        // not Option<i64> (which would yield Option<Option<i64>>).
        assert!(
            !output.contains("Option<Option<"),
            "hash constructor must not emit double Option: {output}"
        );
        assert!(
            output.contains("i64::try_convert"),
            "hash constructor should call inner-type::try_convert, not Option<T>::try_convert: {output}"
        );
    }

    #[test]
    fn test_magnus_positional_constructor_no_double_option_when_ty_is_optional() {
        // field with optional=true AND ty=Optional(Usize) — small type uses positional constructor
        // simple_type_mapper maps Usize → "i64"
        let field = FieldDef {
            name: "max_depth".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
            optional: true,
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
        let typ = TypeDef {
            name: "SmallUpdate".to_string(),
            rust_path: "crate::SmallUpdate".to_string(),
            original_rust_path: String::new(),
            fields: vec![field],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        };
        let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
        // simple_type_mapper maps Usize → "i64", so Optional(Usize) → "Option<i64>"
        // The param must be Option<i64>, never Option<Option<i64>>.
        assert!(
            !output.contains("Option<Option<"),
            "positional constructor must not emit double Option: {output}"
        );
        assert!(
            output.contains("Option<i64>"),
            "positional constructor should emit Option<inner> for optional Optional(T): {output}"
        );
    }
}
