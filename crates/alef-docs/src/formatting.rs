use crate::type_mapping::java_boxed_type;
use crate::{doc_type, enum_variant_name, type_name};
use alef_core::config::Language;
use alef_core::ir::{ApiSurface, DefaultValue, FieldDef, TypeRef};
use heck::{ToPascalCase, ToShoutySnakeCase};

pub(crate) fn format_field_default(field: &FieldDef, lang: Language, api: &ApiSurface, ffi_prefix: &str) -> String {
    if let Some(typed) = &field.typed_default {
        return format_typed_default(typed, &field.ty, lang, api, ffi_prefix, field.optional);
    }
    if let Some(raw) = &field.default {
        if !raw.is_empty() {
            return format!("`{raw}`");
        }
    }
    if field.optional {
        return match lang {
            Language::Python => "`None`".to_string(),
            Language::Node | Language::Wasm => "`null`".to_string(),
            Language::Go => "`nil`".to_string(),
            Language::Java => "`null`".to_string(),
            Language::Csharp => "`null`".to_string(),
            Language::Ruby => "`nil`".to_string(),
            Language::Php => "`null`".to_string(),
            Language::Elixir => "`nil`".to_string(),
            Language::R => "`NULL`".to_string(),
            Language::Rust => "`None`".to_string(),
            Language::Ffi => "`NULL`".to_string(),
            Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                "`null`".to_string()
            }
        };
    }
    "—".to_string()
}

pub(crate) fn format_typed_default(
    val: &DefaultValue,
    field_ty: &TypeRef,
    lang: Language,
    api: &ApiSurface,
    ffi_prefix: &str,
    optional: bool,
) -> String {
    match val {
        DefaultValue::BoolLiteral(b) => match lang {
            Language::Python => format!("`{}`", if *b { "True" } else { "False" }),
            _ => format!("`{b}`"),
        },
        DefaultValue::StringLiteral(s) => format!("`\"{s}\"`"),
        DefaultValue::IntLiteral(n) => {
            // Duration fields store defaults as milliseconds; show with unit label
            if matches!(field_ty, TypeRef::Duration)
                || matches!(field_ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Duration))
            {
                return format!("`{n}ms`");
            }
            format!("`{n}`")
        }
        DefaultValue::FloatLiteral(f) => format!("`{f}`"),
        DefaultValue::EnumVariant(v) => {
            // v is something like "HeadingStyle::Atx" or just "Atx"
            let parts: Vec<&str> = v.splitn(2, "::").collect();
            if parts.len() == 2 {
                let enum_type = type_name(parts[0], lang, ffi_prefix);
                let variant = enum_variant_name(parts[1], lang, ffi_prefix);
                format!("`{}`", format_enum_variant_ref(&enum_type, &variant, lang, ffi_prefix))
            } else {
                // Bare variant name — resolve the enum type from the field type
                let enum_type_name_str = match field_ty {
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
                if let Some(type_str) = enum_type_name_str {
                    let etype = type_name(type_str, lang, ffi_prefix);
                    let variant = enum_variant_name(v, lang, ffi_prefix);
                    format!("`{}`", format_enum_variant_ref(&etype, &variant, lang, ffi_prefix))
                } else {
                    format!("`{v}`")
                }
            }
        }
        DefaultValue::Empty => {
            // Duration fields with Empty default: the actual value could not be parsed.
            // Show a language-neutral placeholder rather than None/null.
            let inner_for_dur = match field_ty {
                TypeRef::Optional(inner) => Some(inner.as_ref()),
                other => Some(other),
            };
            if matches!(inner_for_dur, Some(TypeRef::Duration)) {
                return match lang {
                    Language::Rust => "`Duration::default()`".to_string(),
                    _ => "`0ms`".to_string(),
                };
            }

            // If the field type is a Named enum, resolve to its default (or first) variant.
            // But only for non-optional fields — optional enum fields default to None/null.
            if !optional {
                if let TypeRef::Named(type_name_str) = field_ty {
                    if let Some(enum_def) = api.enums.iter().find(|e| &e.name == type_name_str) {
                        let variant = enum_def
                            .variants
                            .iter()
                            .find(|v| v.is_default)
                            .or_else(|| enum_def.variants.first());
                        if let Some(v) = variant {
                            let etype = type_name(type_name_str, lang, ffi_prefix);
                            let vname = enum_variant_name(&v.name, lang, ffi_prefix);
                            return format!("`{}`", format_enum_variant_ref(&etype, &vname, lang, ffi_prefix));
                        }
                    }
                }
            }
            // Non-enum Empty: depends on field type
            // Unwrap Optional wrapper to get inner type for collection/map detection
            let inner_ty = match field_ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            if matches!(inner_ty, TypeRef::Vec(_)) {
                return match lang {
                    Language::Python => "`[]`".to_string(),
                    Language::Node | Language::Wasm => "`[]`".to_string(),
                    Language::Go => "`nil`".to_string(),
                    Language::Java => "`Collections.emptyList()`".to_string(),
                    Language::Csharp => {
                        let elem_ty = if let TypeRef::Vec(elem) = inner_ty {
                            doc_type(elem, lang, ffi_prefix)
                        } else {
                            String::new()
                        };
                        format!("`new List<{elem_ty}>()`")
                    }
                    Language::Ruby | Language::Elixir => "`[]`".to_string(),
                    Language::Php => "`[]`".to_string(),
                    Language::Rust => "`vec![]`".to_string(),
                    Language::Ffi => "`NULL`".to_string(),
                    Language::R => "`list()`".to_string(),
                    Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                        "`[]`".to_string()
                    }
                };
            }
            if matches!(inner_ty, TypeRef::Map(_, _)) {
                return match lang {
                    Language::Python | Language::Ruby | Language::Php => "`{}`".to_string(),
                    Language::Node | Language::Wasm => "`{}`".to_string(),
                    Language::Go => "`nil`".to_string(),
                    Language::Elixir => "`%{}`".to_string(),
                    Language::Java => "`Collections.emptyMap()`".to_string(),
                    Language::Csharp => {
                        if let TypeRef::Map(k, v) = inner_ty {
                            let kty = doc_type(k, lang, ffi_prefix);
                            let vty = doc_type(v, lang, ffi_prefix);
                            format!("`new Dictionary<{kty}, {vty}>()`")
                        } else {
                            "`new Dictionary<>()`".to_string()
                        }
                    }
                    Language::Rust => "`HashMap::new()`".to_string(),
                    Language::Ffi => "`NULL`".to_string(),
                    Language::R => "`list()`".to_string(),
                    Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                        "`{}`".to_string()
                    }
                };
            }
            // Non-collection Empty: only show null for optional fields
            if !optional {
                return "—".to_string();
            }
            match lang {
                Language::Python => "`None`".to_string(),
                Language::Node | Language::Wasm => "`null`".to_string(),
                Language::Go => "`nil`".to_string(),
                Language::Java => "`null`".to_string(),
                Language::Csharp => "`null`".to_string(),
                Language::Ruby => "`nil`".to_string(),
                Language::Php => "`null`".to_string(),
                Language::Elixir => "`nil`".to_string(),
                Language::R => "`NULL`".to_string(),
                Language::Rust => "`Default::default()`".to_string(),
                Language::Ffi => "`NULL`".to_string(),
                Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                    "`null`".to_string()
                }
            }
        }
        DefaultValue::None => {
            if !optional {
                return "—".to_string();
            }
            match lang {
                Language::Python => "`None`".to_string(),
                Language::Node | Language::Wasm => "`null`".to_string(),
                Language::Go => "`nil`".to_string(),
                Language::Java => "`null`".to_string(),
                Language::Csharp => "`null`".to_string(),
                Language::Ruby => "`nil`".to_string(),
                Language::Php => "`null`".to_string(),
                Language::Elixir => "`nil`".to_string(),
                Language::R => "`NULL`".to_string(),
                Language::Rust => "`None`".to_string(),
                Language::Ffi => "`NULL`".to_string(),
                Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                    "`null`".to_string()
                }
            }
        }
    }
}

/// Format an enum variant reference: `TypeName.VARIANT` or `:atom` style per language.
pub(crate) fn format_enum_variant_ref(enum_type: &str, variant: &str, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Python => format!("{enum_type}.{variant}"),
        Language::Node | Language::Wasm => format!("{enum_type}.{variant}"),
        Language::Go => format!("{enum_type}.{variant}"),
        Language::Java => format!("{enum_type}.{variant}"),
        Language::Csharp => format!("{enum_type}.{variant}"),
        Language::Ruby => format!(":{variant}"),
        Language::Php => format!("{enum_type}::{variant}"),
        Language::Elixir => format!(":{variant}"),
        Language::R => format!("\"{variant}\""),
        Language::Rust => format!("{enum_type}::{variant}"),
        Language::Ffi => format!(
            "{}_{}",
            ffi_prefix.to_shouty_snake_case(),
            variant.to_shouty_snake_case()
        ),
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            format!("{enum_type}.{variant}")
        }
    }
}

/// Format the error/exception phrase for a function that can fail.
pub(crate) fn format_error_phrase(error_type: &str, lang: Language) -> String {
    let short = error_type.rsplit("::").next().unwrap_or(error_type);
    match lang {
        Language::Python => {
            let ename = short.to_pascal_case();
            format!("Raises `{ename}`.")
        }
        Language::Go => "Returns `error`.".to_string(),
        Language::Java => {
            let ename = short.to_pascal_case();
            let ename = if ename.ends_with("Exception") {
                ename
            } else {
                format!("{ename}Exception")
            };
            format!("Throws `{ename}`.")
        }
        Language::Node | Language::Wasm => "Throws `Error` with a descriptive message.".to_string(),
        Language::Ruby => {
            let ename = short.to_pascal_case();
            format!("Raises `{ename}`.")
        }
        Language::Csharp => {
            let ename = short.to_pascal_case();
            format!("Throws `{ename}`.")
        }
        Language::Elixir => "Returns `{:error, reason}`".to_string(),
        Language::Php => {
            let ename = short.to_pascal_case();
            format!("Throws `{ename}`.")
        }
        Language::Ffi => "Returns `NULL` on error.".to_string(),
        Language::R => "Stops with error message.".to_string(),
        Language::Rust => {
            let ename = short.to_pascal_case();
            format!("Returns `Err({ename})`.")
        }
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
            let ename = short.to_pascal_case();
            format!("Throws `{ename}`.")
        }
    }
}

/// Like `doc_type` but wraps in the nullable form when `optional` is true.
pub(crate) fn doc_type_with_optional(ty: &TypeRef, lang: Language, optional: bool, ffi_prefix: &str) -> String {
    // If the type is already Optional<T>, don't double-wrap
    if optional && !matches!(ty, TypeRef::Optional(_)) {
        let inner = doc_type(ty, lang, ffi_prefix);
        return match lang {
            Language::Python => format!("{inner} | None"),
            Language::Node | Language::Wasm => format!("{inner} | null"),
            Language::Go => format!("*{inner}"),
            Language::Java => format!("Optional<{}>", java_boxed_type(ty)),
            Language::Csharp => format!("{inner}?"),
            Language::Ruby => format!("{inner}?"),
            Language::Php => format!("?{inner}"),
            Language::Elixir => format!("{inner} | nil"),
            Language::R => format!("{inner} or NULL"),
            Language::Rust => format!("Option<{inner}>"),
            Language::Ffi => format!("{inner}*"),
            Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => {
                format!("{inner}?")
            }
        };
    }
    doc_type(ty, lang, ffi_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_PREFIX, empty_api, make_field};
    use alef_core::config::Language;
    use alef_core::ir::{DefaultValue, PrimitiveType, TypeRef};

    #[test]
    fn test_doc_type_with_optional_true_wraps_correctly() {
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Python, true, TEST_PREFIX),
            "str | None"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Node, true, TEST_PREFIX),
            "string | null"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Go, true, TEST_PREFIX),
            "*string"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Csharp, true, TEST_PREFIX),
            "string?"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ruby, true, TEST_PREFIX),
            "String?"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Php, true, TEST_PREFIX),
            "?string"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Elixir, true, TEST_PREFIX),
            "String.t() | nil"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::R, true, TEST_PREFIX),
            "character or NULL"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Rust, true, TEST_PREFIX),
            "Option<String>"
        );
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Ffi, true, TEST_PREFIX),
            "const char**"
        );
    }

    #[test]
    fn test_doc_type_with_optional_false_is_identity() {
        for lang in [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Java,
            Language::Rust,
        ] {
            assert_eq!(
                doc_type_with_optional(&TypeRef::String, lang, false, TEST_PREFIX),
                crate::doc_type(&TypeRef::String, lang, TEST_PREFIX),
                "optional=false should be identity for {lang:?}"
            );
        }
    }

    #[test]
    fn test_doc_type_with_optional_does_not_double_wrap_already_optional_type() {
        let already_optional = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(
            doc_type_with_optional(&already_optional, Language::Python, true, TEST_PREFIX),
            "str | None"
        );
        assert_eq!(
            doc_type_with_optional(&already_optional, Language::Rust, true, TEST_PREFIX),
            "Option<String>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_i32() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::I32),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "Optional<Integer>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_bool() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::Bool),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "Optional<Boolean>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_boxes_primitive_f64() {
        assert_eq!(
            doc_type_with_optional(
                &TypeRef::Primitive(PrimitiveType::F64),
                Language::Java,
                true,
                TEST_PREFIX
            ),
            "Optional<Double>"
        );
    }

    #[test]
    fn test_doc_type_with_optional_java_non_primitive_not_double_boxed() {
        assert_eq!(
            doc_type_with_optional(&TypeRef::String, Language::Java, true, TEST_PREFIX),
            "Optional<String>"
        );
    }

    #[test]
    fn test_format_default_bool_literal_python_uses_capitalised_form() {
        let api = empty_api();
        let field_true = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(true)),
        );
        let field_false = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(false)),
        );
        assert_eq!(
            format_field_default(&field_true, Language::Python, &api, TEST_PREFIX),
            "`True`"
        );
        assert_eq!(
            format_field_default(&field_false, Language::Python, &api, TEST_PREFIX),
            "`False`"
        );
    }

    #[test]
    fn test_format_default_bool_literal_non_python_uses_lowercase_form() {
        let api = empty_api();
        let field_true = make_field(
            "flag",
            TypeRef::Primitive(PrimitiveType::Bool),
            false,
            Some(DefaultValue::BoolLiteral(true)),
        );
        for lang in [Language::Rust, Language::Java, Language::Go, Language::Node] {
            assert_eq!(
                format_field_default(&field_true, lang, &api, TEST_PREFIX),
                "`true`",
                "bool literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_string_literal_all_languages_produce_quoted_form() {
        let api = empty_api();
        let field = make_field(
            "name",
            TypeRef::String,
            false,
            Some(DefaultValue::StringLiteral("hello".to_string())),
        );
        for lang in [
            Language::Python,
            Language::Rust,
            Language::Java,
            Language::Go,
            Language::Node,
        ] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`\"hello\"`",
                "string literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_int_literal() {
        let api = empty_api();
        let field = make_field(
            "count",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            Some(DefaultValue::IntLiteral(42)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java, Language::Node] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`42`",
                "int literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_int_literal_on_duration_field_shows_ms_suffix() {
        let api = empty_api();
        let field = make_field(
            "timeout",
            TypeRef::Duration,
            false,
            Some(DefaultValue::IntLiteral(5000)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java, Language::Go] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`5000ms`",
                "duration field should show ms suffix for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_float_literal() {
        let api = empty_api();
        let field = make_field(
            "confidence",
            TypeRef::Primitive(PrimitiveType::F32),
            false,
            Some(DefaultValue::FloatLiteral(0.85)),
        );
        for lang in [Language::Python, Language::Rust, Language::Java] {
            assert_eq!(
                format_field_default(&field, lang, &api, TEST_PREFIX),
                "`0.85`",
                "float literal for {lang:?}"
            );
        }
    }

    #[test]
    fn test_format_default_enum_variant_qualified_python_and_rust() {
        let api = empty_api();
        let field = make_field(
            "style",
            TypeRef::Named("HeadingStyle".to_string()),
            false,
            Some(DefaultValue::EnumVariant("HeadingStyle::Atx".to_string())),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`HeadingStyle.ATX`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`HeadingStyle::Atx`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`HeadingStyle.ATX`"
        );
        assert_eq!(
            format_field_default(&field, Language::Ruby, &api, TEST_PREFIX),
            "`:atx`"
        );
        assert_eq!(
            format_field_default(&field, Language::Php, &api, TEST_PREFIX),
            "`HeadingStyle::Atx`"
        );
    }

    #[test]
    fn test_format_default_empty_vec_field() {
        let api = empty_api();
        let field = make_field(
            "items",
            TypeRef::Vec(Box::new(TypeRef::String)),
            false,
            Some(DefaultValue::Empty),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`[]`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`vec![]`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`Collections.emptyList()`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Csharp, &api, TEST_PREFIX),
            "`new List<string>()`"
        );
        assert_eq!(format_field_default(&field, Language::R, &api, TEST_PREFIX), "`list()`");
        assert_eq!(format_field_default(&field, Language::Ruby, &api, TEST_PREFIX), "`[]`");
        assert_eq!(
            format_field_default(&field, Language::Elixir, &api, TEST_PREFIX),
            "`[]`"
        );
        assert_eq!(format_field_default(&field, Language::Ffi, &api, TEST_PREFIX), "`NULL`");
    }

    #[test]
    fn test_format_default_empty_map_field() {
        let api = empty_api();
        let field = make_field(
            "attributes",
            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            false,
            Some(DefaultValue::Empty),
        );
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`{}`"
        );
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`HashMap::new()`"
        );
        assert_eq!(
            format_field_default(&field, Language::Java, &api, TEST_PREFIX),
            "`Collections.emptyMap()`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Elixir, &api, TEST_PREFIX),
            "`%{}`"
        );
        assert_eq!(
            format_field_default(&field, Language::Csharp, &api, TEST_PREFIX),
            "`new Dictionary<string, string>()`"
        );
    }

    #[test]
    fn test_format_default_none_on_optional_field() {
        let api = empty_api();
        let field = make_field("label", TypeRef::String, true, Some(DefaultValue::None));
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`None`"
        );
        assert_eq!(
            format_field_default(&field, Language::Node, &api, TEST_PREFIX),
            "`null`"
        );
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`nil`");
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`None`"
        );
        assert_eq!(format_field_default(&field, Language::Ffi, &api, TEST_PREFIX), "`NULL`");
        assert_eq!(format_field_default(&field, Language::R, &api, TEST_PREFIX), "`NULL`");
    }

    #[test]
    fn test_format_default_none_on_non_optional_field_returns_dash() {
        let api = empty_api();
        let field = make_field(
            "count",
            TypeRef::Primitive(PrimitiveType::U32),
            false,
            Some(DefaultValue::None),
        );
        assert_eq!(format_field_default(&field, Language::Python, &api, TEST_PREFIX), "—");
    }

    #[test]
    fn test_format_default_empty_duration_shows_zero_ms_for_non_rust() {
        let api = empty_api();
        let field = make_field("timeout", TypeRef::Duration, false, Some(DefaultValue::Empty));
        assert_eq!(
            format_field_default(&field, Language::Python, &api, TEST_PREFIX),
            "`0ms`"
        );
        assert_eq!(format_field_default(&field, Language::Java, &api, TEST_PREFIX), "`0ms`");
        assert_eq!(format_field_default(&field, Language::Go, &api, TEST_PREFIX), "`0ms`");
    }

    #[test]
    fn test_format_default_empty_duration_rust_shows_duration_default() {
        let api = empty_api();
        let field = make_field("timeout", TypeRef::Duration, false, Some(DefaultValue::Empty));
        assert_eq!(
            format_field_default(&field, Language::Rust, &api, TEST_PREFIX),
            "`Duration::default()`"
        );
    }
}
