use crate::codegen::naming::to_java_name;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{PrimitiveType, TypeRef};
use heck::{ToKebabCase, ToLowerCamelCase, ToPascalCase};
use std::collections::HashSet;

/// Placeholder the extractor stores in `FieldDef::default` for a bare
/// `#[serde(default)]` attribute.
const SERDE_DEFAULT_PLACEHOLDER: &str = "/* serde(default) */";

/// Returns `true` when a field's stored default marks a serde default — either the
/// bare `#[serde(default)]` placeholder or the named `#[serde(default = "path")]`
/// form, which the extractor stores verbatim as `serde(default = "...")` so other
/// backends (e.g. PHP's generated Rust crate) can re-emit the Rust path.
///
/// The Java backend treats both forms identically: the field is boxed/nullable and
/// omitted from JSON via `@JsonInclude(NON_ABSENT)`, letting Rust's serde apply its
/// own default. The raw marker must never be emitted as a Java initializer — doing so
/// produces uncompilable source like `boolean denyPrivate = serde(default = "...");`.
pub(crate) fn is_serde_default_marker(default: Option<&str>) -> bool {
    matches!(default, Some(s) if s == SERDE_DEFAULT_PLACEHOLDER || s.starts_with("serde(default = \""))
}

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

/// Types automatically imported by the Java Language Specification as members
/// of `java.lang.*`. These types must never be emitted as explicit import statements,
/// as checkstyle's `UnusedImports` rule will flag them as redundant.
/// This allowlist is provided for future use by code generation functions that
/// emit imports; all generated facades should consult it before emitting imports.
#[allow(dead_code)]
pub(crate) const JAVA_LANG_AUTO_IMPORTED: &[&str] = &[
    "Iterable",
    "Object",
    "String",
    "Throwable",
    "Number",
    "Boolean",
    "Integer",
    "Long",
    "Double",
    "Float",
    "Character",
    "Byte",
    "Short",
    "Math",
    "Runnable",
    "Thread",
    "Exception",
    "RuntimeException",
    "Error",
    "Class",
    "Comparable",
];

/// Returns true if `name` is a tuple/unnamed field index such as `"0"`, `"1"`, `"_0"`, `"_1"`.
/// Serde represents tuple and newtype variant fields with these numeric names. They are not
/// real JSON keys and must not be used as Java identifiers.
/// Escape a string for use inside a Javadoc comment.
/// Replaces `*/` (which would close the comment) and `@` (which starts a tag).
///
/// HTML entities (`<`, `>`, `&`) are also escaped *inside* `{@code …}` blocks.
/// Leaving them raw lets Eclipse-formatter Spotless interpret content like
/// `<pre>` as a block-level HTML element and shatter the line across
/// multiple `* ` rows, which then breaks `alef-verify`'s embedded hash.
///
/// Also sanitizes Rust-specific syntax that leaks into Javadoc:
/// - `::` (namespace separator) → `.` (Java package separator)
/// - `.unwrap()` / `.expect()` → removed (Rust idioms with no Java equivalent)
pub(crate) fn escape_javadoc_line(s: &str) -> String {
    // First pass: sanitize Rust-specific syntax outside backticks
    let sanitized = sanitize_rust_syntax(s);

    let mut result = String::with_capacity(sanitized.len());
    let mut chars = sanitized.chars().peekable();
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
            let mut code_iter = code.chars().peekable();
            while let Some(code_ch) = code_iter.next() {
                match code_ch {
                    '<' => result.push_str("&lt;"),
                    '>' => result.push_str("&gt;"),
                    '&' => result.push_str("&amp;"),
                    // Literal `*/` inside {@code …} would prematurely close the
                    // surrounding /** … */ javadoc. Escape the slash to keep the
                    // span intact (parser still renders the original glyph).
                    '*' if code_iter.peek() == Some(&'/') => {
                        code_iter.next();
                        result.push_str("*&#47;");
                    }
                    other => result.push(other),
                }
            }
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

/// Sanitize Rust-specific syntax in docstrings.
///
/// Delegates to the shared [`crate::codegen::doc_emission::sanitize_rust_idioms`]
/// implementation with the [`crate::codegen::doc_emission::DocTarget::JavaDoc`] target.
fn sanitize_rust_syntax(s: &str) -> String {
    crate::codegen::doc_emission::sanitize_rust_idioms(s, crate::codegen::doc_emission::DocTarget::JavaDoc)
}

pub(crate) fn is_tuple_field_name(name: &str) -> bool {
    let stripped = name.trim_start_matches('_');
    !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit())
}

/// Render a Java type with an optional `@Nullable` type-use annotation in the
/// position required by JLS for qualified types.
///
/// For unqualified type names (`Path`, `String`, `MyType`), the annotation
/// appears in leading position: `@Nullable Path`.
///
/// For fully-qualified type names (`java.nio.file.Path`, `com.example.Foo`),
/// the annotation must appear between the package prefix and the simple type
/// name: `java.nio.file.@Nullable Path`. Emitting the annotation in leading
/// position on a qualified type is a `javac` error (`type annotation is not
/// expected here`).
///
/// Same logic as in `types.rs:265` for record/Builder fields, lifted here so
/// `ffi_class.rs` parameter-list emitters can reuse it.
pub(crate) fn render_nullable_type(ftype: &str, is_nullable: bool) -> String {
    if !is_nullable {
        return ftype.to_string();
    }
    if let Some(idx) = ftype.rfind('.') {
        let (pkg, simple) = ftype.split_at(idx);
        let simple = simple.trim_start_matches('.');
        format!("{pkg}.@Nullable {simple}")
    } else {
        format!("@Nullable {ftype}")
    }
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

/// Sanitise a Rust impl-method name for use as a Java method identifier.
///
/// Rust impl blocks routinely use names that are Java reserved keywords —
/// `default`, `new`, `class`, `int`, etc. Emitting them verbatim produces
/// non-compiling Java (`public static Parser default()`).
///
/// Strategy: the two common Rust conventions get meaningful renames
/// (`default` → `defaultInstance`, `new` → `create`); any other keyword
/// collision falls back to a trailing-underscore suffix (e.g. `class`
/// → `class_`), matching the field-name convention.
pub(crate) fn safe_java_method_name(name: &str) -> String {
    let camel = name.to_lower_camel_case();
    match camel.as_str() {
        "default" => "defaultInstance".to_string(),
        "new" => "create".to_string(),
        other if crate::core::keywords::JAVA_KEYWORDS.contains(&other) => format!("{other}_"),
        _ => camel,
    }
}

pub(crate) fn is_bridge_param_java(
    param: &crate::core::ir::ParamDef,
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

/// Generate a named infrastructure exception class that extends `{main_class}Exception`.
///
/// Used for the two fixed FFI infrastructure error codes that are always dispatched
/// from `checkLastError()`:
/// - code 1 → `InvalidInputException` (null pointer / invalid UTF-8 in input args)
/// - code 2 → `ConversionErrorException` (JSON serialisation/deserialisation failure)
pub(crate) fn gen_infrastructure_exception_class(
    package: &str,
    main_class: &str,
    class_name: &str,
    code: i32,
    doc: &str,
) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::backends::java::template_env::render(
        "infrastructure_exception.jinja",
        minijinja::context! {
            header => header,
            package => package,
            class_name => class_name,
            main_class => main_class,
            code => code,
            doc => doc,
        },
    )
}

pub(crate) fn gen_exception_class(package: &str, class_name: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::backends::java::template_env::render(
        "exception_class.jinja",
        minijinja::context! {
            header => header,
            package => package,
            class_name => class_name,
        },
    )
}

// ---------------------------------------------------------------------------
// High-level facade class (public API)
// ---------------------------------------------------------------------------

/// Transform Rust intra-doc rustdoc into JavaDoc-compatible prose with
/// JavaDoc tags (`@param`, `@return`, `@throws`).
///
/// Uses the shared section parser from `crate::codegen::doc_emission` so the
/// behaviour is identical across all bindings that translate rustdoc into
/// host-native tag conventions.
///
/// `# Example` blocks are dropped here — they are handled separately by
/// `emit_javadoc`, which would need to wrap code in `<pre>{@code ...}</pre>`.
/// The current Java emitter does not yet emit examples; doing so safely
/// requires a JavaDoc-specific HTML escape that's not done here.
fn transform_rustdoc_for_java(doc: &str, throws_class: &str) -> String {
    let sections = crate::codegen::doc_emission::parse_rustdoc_sections(doc);
    let rendered = crate::codegen::doc_emission::render_javadoc_sections(&sections, throws_class);
    if rendered.trim().is_empty() {
        // Fallback: when no recognised sections present, sanitize Rust idioms and remove intra-doc links
        // to preserve backward compatibility for prose that has no Markdown headings.
        let sanitized = sanitize_rust_syntax(doc);
        return sanitized.replace("[`", "").replace("`]", "").trim().to_string();
    }
    rendered.replace("[`", "").replace("`]", "")
}

pub(crate) fn emit_javadoc(out: &mut String, doc: &str, indent: &str) {
    emit_javadoc_with_throws(out, doc, indent, "Exception");
}

pub(crate) fn emit_javadoc_with_throws(out: &mut String, doc: &str, indent: &str, throws_class: &str) {
    if doc.is_empty() {
        return;
    }
    let transformed = transform_rustdoc_for_java(doc, throws_class);
    if transformed.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    let lines: Vec<String> = transformed
        .lines()
        .map(|line| escape_javadoc_line(line).trim_end().to_string())
        .collect();
    out.push_str(&crate::backends::java::template_env::render(
        "javadoc_lines.jinja",
        minijinja::context! {
            indent => indent,
            lines => lines,
        },
    ));
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Maximum line length before splitting record fields across multiple lines.
/// Checkstyle enforces 120 chars; we split at 100 to leave headroom for indentation.
pub(crate) const RECORD_LINE_WRAP_THRESHOLD: usize = 100;

pub(crate) fn java_apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") => crate::codegen::naming::pascal_to_snake(name),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("PascalCase") => name.to_pascal_case(),
        Some("SCREAMING_SNAKE_CASE") => crate::codegen::naming::pascal_to_screaming_snake(name),
        Some("kebab-case") => name.to_kebab_case(),
        Some("SCREAMING-KEBAB-CASE") => name.to_kebab_case().to_uppercase(),
        Some("lowercase") => name.to_lowercase(),
        Some("UPPERCASE") => name.to_uppercase(),
        // Serde's default for enums (no #[serde(rename_all)]) is the variant name
        // unchanged (title-case/PascalCase). Match that behavior.
        _ => name.to_string(),
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

/// Generate the JsonUtil class for centralized JSON deserialization.
pub(crate) fn gen_json_util_class(package: &str, main_class: &str) -> String {
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    crate::backends::java::template_env::render(
        "json_util.jinja",
        minijinja::context! {
            header => header,
            package => package,
            main_class => main_class,
        },
    )
}
