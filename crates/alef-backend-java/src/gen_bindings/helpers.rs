use alef_codegen::naming::to_java_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{PrimitiveType, TypeRef};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::HashSet;
use std::fmt::Write;

/// Names that conflict with methods on `java.lang.Object` and are therefore
/// illegal as record component names or method names in generated Java code.
const JAVA_OBJECT_METHOD_NAMES: &[&str] = &[
    "wait",
    "notify",
    "notifyAll",
    "getClass",
    "hashCode",
    "equals",
    "toString",
    "clone",
    "finalize",
];

/// Returns true if `name` is a tuple/unnamed field index such as `"0"`, `"1"`, `"_0"`, `"_1"`.
/// Serde represents tuple and newtype variant fields with these numeric names. They are not
/// real JSON keys and must not be used as Java identifiers.
/// Escape a string for use inside a Javadoc comment.
/// Replaces `*/` (which would close the comment) and `@` (which starts a tag).
pub(crate) fn escape_javadoc_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut code = String::new();
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                code.push(c);
            }
            result.push_str("{@code ");
            result.push_str(&code);
            result.push('}');
        } else if ch == '<' {
            result.push_str("&lt;");
        } else if ch == '>' {
            result.push_str("&gt;");
        } else if ch == '&' {
            result.push_str("&amp;");
        } else if ch == '*' && chars.peek() == Some(&'/') {
            chars.next();
            result.push_str("* /");
        } else if ch == '@' {
            result.push_str("{@literal @}");
        } else {
            result.push(ch);
        }
    }
    result
}

pub(crate) fn is_tuple_field_name(name: &str) -> bool {
    let stripped = name.trim_start_matches('_');
    !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit())
}

/// Sanitise a field/parameter name that would conflict with `java.lang.Object`
/// methods.  Conflicting names get a `_` suffix (e.g. `wait` -> `wait_`), which
/// is then converted to camelCase by `to_java_name`.
pub(crate) fn safe_java_field_name(name: &str) -> String {
    let java_name = to_java_name(name);
    if JAVA_OBJECT_METHOD_NAMES.contains(&java_name.as_str()) {
        format!("{}Value", java_name)
    } else {
        java_name
    }
}

pub(crate) fn is_bridge_param_java(
    param: &alef_core::ir::ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    if bridge_param_names.contains(param.name.as_str()) {
        return true;
    }
    let type_name = match &param.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                Some(n.as_str())
            } else {
                None
            }
        }
        _ => None,
    };
    type_name.is_some_and(|n| bridge_type_aliases.contains(n))
}

pub(crate) fn gen_exception_class(package: &str, class_name: &str) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();

    writeln!(out, "/** Exception thrown by {}. */", class_name).ok();
    writeln!(out, "public class {}Exception extends Exception {{", class_name).ok();
    writeln!(out, "    /** The error code. */").ok();
    writeln!(out, "    private final int code;").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Creates a new {}Exception. */", class_name).ok();
    writeln!(
        out,
        "    public {}Exception(final int code, final String message) {{",
        class_name
    )
    .ok();
    writeln!(out, "        super(message);").ok();
    writeln!(out, "        this.code = code;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Creates a new {}Exception with a cause. */", class_name).ok();
    writeln!(
        out,
        "    public {}Exception(final String message, final Throwable cause) {{",
        class_name
    )
    .ok();
    writeln!(out, "        super(message, cause);").ok();
    writeln!(out, "        this.code = -1;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Returns the error code. */").ok();
    writeln!(out, "    public int getCode() {{").ok();
    writeln!(out, "        return code;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    out
}

// ---------------------------------------------------------------------------
// High-level facade class (public API)
// ---------------------------------------------------------------------------

pub(crate) fn emit_javadoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    writeln!(out, "{indent}/**").ok();
    for line in doc.lines() {
        if line.is_empty() {
            writeln!(out, "{indent} *").ok();
        } else {
            let escaped = escape_javadoc_line(line);
            writeln!(out, "{indent} * {escaped}").ok();
        }
    }
    writeln!(out, "{indent} */").ok();
}

/// Maximum line length before splitting record fields across multiple lines.
/// Checkstyle enforces 120 chars; we split at 100 to leave headroom for indentation.
const RECORD_LINE_WRAP_THRESHOLD: usize = 100;

pub(crate) fn java_apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") => name.to_snake_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_pascal_case(),
        Some("SCREAMING_SNAKE_CASE") => name.to_snake_case().to_uppercase(),
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        _ => name.to_lowercase(),
    }
}

pub(crate) fn is_ffi_string_return(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
        TypeRef::Optional(inner) => is_ffi_string_return(inner),
        _ => false,
    }
}

/// Returns the appropriate Java cast type for non-string FFI return values.
pub(crate) fn java_ffi_return_cast(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "boolean",
            PrimitiveType::U8 | PrimitiveType::I8 => "byte",
            PrimitiveType::U16 | PrimitiveType::I16 => "short",
            PrimitiveType::U32 | PrimitiveType::I32 => "int",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "long",
            PrimitiveType::F32 => "float",
            PrimitiveType::F64 => "double",
        },
        TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) => "MemorySegment",
        _ => "MemorySegment",
    }
}

pub(crate) fn format_optional_value(ty: &TypeRef, default: &str) -> String {
    // Check if the default is already wrapped (e.g., "Optional.of(...)" or "Optional.empty()")
    if default.contains("Optional.") {
        return default.to_string();
    }

    // Unwrap Optional types to get the inner type
    let inner_ty = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };

    // Determine the proper literal suffix based on type
    let formatted_value = match inner_ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Isize | PrimitiveType::Usize => {
                // Add 'L' suffix for long values if not already present
                if default.ends_with('L') || default.ends_with('l') {
                    default.to_string()
                } else if default.parse::<i64>().is_ok() {
                    format!("{}L", default)
                } else {
                    default.to_string()
                }
            }
            PrimitiveType::F32 => {
                // Add 'f' suffix for float values if not already present
                if default.ends_with('f') || default.ends_with('F') {
                    default.to_string()
                } else if default.parse::<f32>().is_ok() {
                    format!("{}f", default)
                } else {
                    default.to_string()
                }
            }
            PrimitiveType::F64 => {
                // Double defaults can have optional 'd' suffix, but 0.0 is fine
                default.to_string()
            }
            _ => default.to_string(),
        },
        _ => default.to_string(),
    };

    format!("Optional.of({})", formatted_value)
}
