use crate::codegen::naming::to_java_name;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{DefaultValue, PrimitiveType, TypeRef};
use heck::ToLowerCamelCase;
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

/// The Java literal that restores a non-optional field's Rust default, or `None` when this
/// backend has no primitive literal to restore.
///
/// Every emitter that touches defaults asks one of two questions — "must this component be
/// boxed?" and "what does the compact constructor assign?" — and they are the same question:
/// a component is boxed precisely so a literal can be restored into it. Both answers come from
/// here so they cannot drift apart, which is how the `float` default below was lost: the boxing
/// predicate and the compact-constructor match were separate lists and only the latter was ever
/// extended.
///
/// Only a default that differs from the Java type-zero qualifies. A default equal to the zero
/// needs no restore, and boxing for it would widen the record's public API for nothing.
///
/// Reference-typed defaults (`StringLiteral`, `EnumVariant`, `ListLiteral`, `Empty`) are excluded
/// deliberately, not overlooked: those components are already nullable, the record's
/// `@JsonInclude(NON_ABSENT)` drops a null key from the wire entirely, and Rust's own `Default`
/// then supplies the value — so the binding is correct with no initializer at all. An unboxed
/// primitive has no null to drop, which is the whole reason it is the one shape where a missing
/// restore ships the type-zero to Rust as though the caller had chosen it. ~keep
pub(crate) fn java_literal_default(ty: &TypeRef, typed_default: Option<&DefaultValue>) -> Option<String> {
    match typed_default? {
        DefaultValue::BoolLiteral(true) => Some("true".to_string()),
        DefaultValue::IntLiteral(n) if *n != 0 => Some(match ty {
            TypeRef::Duration
            | TypeRef::Primitive(
                PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize,
            ) => format!("{n}L"),
            _ => n.to_string(),
        }),
        // `float_literal_digits` rejects NaN and the infinities, which have no Java literal, so
        // those fall through to "no restore" rather than to source that does not parse. It also
        // keeps the decimal point on a whole-valued default: `Double d = 2` is an int literal
        // and does not compile in the boxing position the compact constructor assigns through. ~keep
        DefaultValue::FloatLiteral(f) if *f != 0.0 => {
            let digits = crate::codegen::shared::float_literal_digits(*f)?;
            Some(match ty {
                TypeRef::Primitive(PrimitiveType::F32) => format!("{digits}f"),
                _ => digits,
            })
        }
        _ => None,
    }
}

/// Whether a non-optional field must be boxed so `null` can mean "not supplied".
///
/// Java records have no per-component defaults, so a default is restored in the compact
/// constructor by testing the incoming value. That test needs a value meaning "nothing was
/// supplied". `Duration` and `#[serde(default)]` fields already box for exactly this reason;
/// a field carrying a plain literal default must join them, because on an unboxed primitive
/// the only available sentinel is the type-zero — and then a caller passing an explicit `0`,
/// `0.0` or `false` is silently given the default instead. Every other language emits per-field
/// initializers and needs no sentinel, which is why this is Java-local. ~keep
pub(crate) fn boxes_to_carry_literal_default(ty: &TypeRef, typed_default: Option<&DefaultValue>) -> bool {
    java_literal_default(ty, typed_default).is_some()
}

/// Restore a known collection default when serde may omit its wire key.
///
/// Named default functions can fold into a nonempty `ListLiteral`. Preserve supported string
/// lists rather than mistaking them for the collection type's zero. Unknown or unsupported
/// values remain nullable and are omitted from JSON so Rust supplies its own default. ~keep
pub(crate) fn serde_default_collection_literal(
    ty: &TypeRef,
    has_serde_default: bool,
    serde_skip_serializing_if: bool,
    typed_default: Option<&DefaultValue>,
) -> Option<String> {
    if !has_serde_default || !serde_skip_serializing_if {
        return None;
    }
    match (ty, typed_default) {
        (TypeRef::Vec(element), Some(DefaultValue::ListLiteral(items))) if **element == TypeRef::String => {
            let values: Option<Vec<String>> = items
                .iter()
                .map(|item| match item {
                    DefaultValue::StringLiteral(value) => Some(format!(
                        "\"{}\"",
                        crate::codegen::java_literal::escape_java_string_literal(value)
                    )),
                    _ => None,
                })
                .collect();
            Some(format!("List.of({})", values?.join(", ")))
        }
        (TypeRef::Vec(_), None | Some(DefaultValue::Empty)) => Some("List.of()".into()),
        (TypeRef::Map(_, _), None | Some(DefaultValue::Empty)) => Some("Map.of()".into()),
        _ => None,
    }
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
                    '\\' if code_iter.peek() == Some(&'u') => result.push_str("&#92;"),
                    '<' => result.push_str("&lt;"),
                    '>' => result.push_str("&gt;"),
                    '&' => result.push_str("&amp;"),
                    '*' if code_iter.peek() == Some(&'/') => {
                        code_iter.next();
                        result.push_str("*&#47;");
                    }
                    other => result.push(other),
                }
            }
            result.push('}');
        } else if ch == '\\' && chars.peek() == Some(&'u') {
            result.push_str("&#92;");
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

/// Thin wrapper over the canonical `rename_all` casing logic. Kept as a distinct name (rather
/// than calling `naming::apply_serde_rename_all` at each call site) because
/// `src/backends/java/gen_bindings/types/enums.rs` still calls it directly.
pub(crate) fn java_apply_rename_all(name: &str, rename_all: Option<&str>) -> String {
    crate::codegen::naming::apply_serde_rename_all(name, rename_all)
}

pub(crate) fn format_optional_value(ty: &TypeRef, default: &str) -> String {
    if default.contains("Optional.") {
        return default.to_string();
    }

    let inner_ty = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };

    let formatted_value = match inner_ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Isize | PrimitiveType::Usize => {
                if default.ends_with('L') || default.ends_with('l') {
                    default.to_string()
                } else if default.parse::<i64>().is_ok() {
                    format!("{}L", default)
                } else {
                    default.to_string()
                }
            }
            PrimitiveType::F32 => {
                if default.ends_with('f') || default.ends_with('F') {
                    default.to_string()
                } else if default.parse::<f32>().is_ok() {
                    format!("{}f", default)
                } else {
                    default.to_string()
                }
            }
            PrimitiveType::F64 => default.to_string(),
            _ => default.to_string(),
        },
        _ => default.to_string(),
    };

    format!("Optional.of({})", formatted_value)
}

/// Fully-qualifies any Java type-name token that collides with a variant name of the
/// same sealed interface.
///
/// JLS §6.4.1: a member type declared inside a class/interface body shadows any
/// same-named top-level type for the rest of that body, including inside sibling
/// member declarations. Every `record <Variant>(...)` nested inside a
/// `sealed interface` is such a member, so a field whose Java type happens to share
/// a variant's simple name resolves to the nested record instead of the intended
/// sibling top-level type — silently making construction impossible and, on
/// deserialization, discarding the real payload. Qualifying with the package name
/// disambiguates in favor of the top-level type, which is always the one Rust meant. ~keep
pub(crate) fn qualify_shadowed_type(java_type: &str, package: &str, variant_names: &HashSet<&str>) -> String {
    let mut result = String::with_capacity(java_type.len());
    let mut chars = java_type.char_indices().peekable();
    let mut prev_was_dot = false;
    while let Some((start, ch)) = chars.next() {
        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut end = start + ch.len_utf8();
            while let Some(&(_, next_ch)) = chars.peek() {
                if !(next_ch.is_ascii_alphanumeric() || next_ch == '_') {
                    break;
                }
                end += next_ch.len_utf8();
                chars.next();
            }
            let ident = &java_type[start..end];
            if !prev_was_dot && variant_names.contains(ident) {
                result.push_str(package);
                result.push('.');
            }
            result.push_str(ident);
            prev_was_dot = false;
        } else {
            result.push(ch);
            prev_was_dot = ch == '.';
        }
    }
    result
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

#[cfg(test)]
mod tests {
    use super::{escape_javadoc_line, qualify_shadowed_type};
    use std::collections::HashSet;

    #[test]
    fn escapes_unicode_sequences_before_java_lexing() {
        let escaped = escape_javadoc_line(r"Rust chars use `\u{1F600}` or \u{00A0}");

        assert!(!escaped.contains(r"\u"));
        assert!(escaped.contains("&#92;u{1F600}"));
        assert!(escaped.contains("&#92;u{00A0}"));
    }

    #[test]
    fn qualify_shadowed_type_qualifies_a_bare_name_matching_a_variant() {
        let variants: HashSet<&str> = ["ImageUrl", "Text"].into_iter().collect();
        assert_eq!(
            qualify_shadowed_type("ImageUrl", "io.xberg.literllm", &variants),
            "io.xberg.literllm.ImageUrl"
        );
    }

    #[test]
    fn qualify_shadowed_type_leaves_non_colliding_names_untouched() {
        let variants: HashSet<&str> = ["ImageUrl", "Text"].into_iter().collect();
        assert_eq!(
            qualify_shadowed_type("DocumentContent", "io.xberg.literllm", &variants),
            "DocumentContent"
        );
    }

    #[test]
    fn qualify_shadowed_type_qualifies_inside_generics() {
        let variants: HashSet<&str> = ["ImageUrl"].into_iter().collect();
        assert_eq!(
            qualify_shadowed_type("List<ImageUrl>", "io.xberg.literllm", &variants),
            "List<io.xberg.literllm.ImageUrl>"
        );
    }

    #[test]
    fn qualify_shadowed_type_does_not_double_qualify_already_qualified_names() {
        let variants: HashSet<&str> = ["ImageUrl"].into_iter().collect();
        assert_eq!(
            qualify_shadowed_type("io.xberg.literllm.ImageUrl", "io.xberg.literllm", &variants),
            "io.xberg.literllm.ImageUrl"
        );
    }

    #[test]
    fn qualify_shadowed_type_ignores_names_with_no_collision() {
        let variants: HashSet<&str> = HashSet::new();
        assert_eq!(
            qualify_shadowed_type("ImageUrl", "io.xberg.literllm", &variants),
            "ImageUrl"
        );
    }
}
