use alef_core::config::Language;
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};

pub(crate) fn lang_display_name(lang: Language) -> &'static str {
    match lang {
        Language::Python => "Python",
        Language::Node => "TypeScript",
        Language::Ruby => "Ruby",
        Language::Php => "PHP",
        Language::Elixir => "Elixir",
        Language::Go => "Go",
        Language::Java => "Java",
        Language::Csharp => "C#",
        Language::Ffi => "C",
        Language::Wasm => "WebAssembly",
        Language::R => "R",
        Language::Rust => "Rust",
        Language::Kotlin => "Kotlin",
        Language::Swift => "Swift",
        Language::Dart => "Dart",
        Language::Gleam => "Gleam",
        Language::Zig => "Zig",
    }
}

/// Get the slug used in file names (e.g. `typescript` for `Node`).
pub(crate) fn lang_slug(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "c",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
    }
}

/// Get the code fence language identifier.
pub(crate) fn lang_code_fence(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node | Language::Wasm => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi => "c",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
    }
}

/// Convert a Rust type name to the idiomatic name for the target language.
pub(crate) fn type_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    // Strip module path prefix if present
    let short = name.rsplit("::").next().unwrap_or(name);
    match lang {
        Language::Python
        | Language::Node
        | Language::Wasm
        | Language::Ruby
        | Language::Go
        | Language::Java
        | Language::Csharp
        | Language::Php
        | Language::Elixir
        | Language::R
        | Language::Rust
        | Language::Kotlin
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => short.to_pascal_case(),
        Language::Ffi => {
            // C: prefix with configured FFI prefix (PascalCase) and PascalCase type name
            format!("{}{}", ffi_prefix, short.to_pascal_case())
        }
    }
}

/// Convert a Rust function name to the idiomatic name for the target language.
pub(crate) fn func_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let base = match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::R | Language::Rust => name.to_snake_case(),
        Language::Node | Language::Wasm | Language::Java | Language::Php => to_camel_case(name),
        Language::Csharp | Language::Go => name.to_pascal_case(),
        Language::Ffi => format!("{}_{}", ffi_prefix.to_snake_case(), name.to_snake_case()),
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => to_camel_case(name),
    };
    // Handle reserved keywords
    match (lang, base.as_str()) {
        (Language::Java, "default") => "defaultOptions".to_string(),
        (Language::Csharp, "Default") => "CreateDefault".to_string(),
        _ => base,
    }
}

/// Convert a Rust field name to the idiomatic name for the target language.
pub(crate) fn field_name(name: &str, lang: Language) -> String {
    match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::R | Language::Ffi | Language::Rust => {
            name.to_snake_case()
        }
        // Go and C# exported fields/properties are PascalCase
        Language::Go | Language::Csharp => name.to_pascal_case(),
        Language::Node | Language::Wasm | Language::Java | Language::Php => to_camel_case(name),
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => to_camel_case(name),
    }
}

/// Convert a Rust enum variant name to the idiomatic name for the target language.
pub(crate) fn enum_variant_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    // Special-case acronym variants that don't split cleanly
    if name == "RDFa" {
        return match lang {
            Language::Python | Language::Java => "RDFA".to_string(),
            Language::Ruby | Language::Elixir => "rdfa".to_string(),
            Language::R => "rdfa".to_string(),
            Language::Ffi => format!("{}_{}", ffi_prefix.to_shouty_snake_case(), "RDFA"),
            _ => "RDFa".to_string(),
        };
    }
    match lang {
        Language::Python => {
            // Python: UPPER_SNAKE_CASE
            name.to_shouty_snake_case()
        }
        Language::Java => {
            // Java: UPPER_SNAKE_CASE
            name.to_shouty_snake_case()
        }
        Language::Ruby | Language::Elixir => {
            // Ruby/Elixir: :snake_atom style
            name.to_snake_case()
        }
        Language::Go
        | Language::Node
        | Language::Wasm
        | Language::Csharp
        | Language::Php
        | Language::Kotlin
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => name.to_pascal_case(),
        Language::R => name.to_snake_case(),
        // Rust: PascalCase enum variants
        Language::Rust => name.to_pascal_case(),
        Language::Ffi => format!("{}_{}", ffi_prefix.to_shouty_snake_case(), name.to_shouty_snake_case()),
    }
}

/// Convert snake_case or PascalCase to camelCase.
pub(crate) fn to_camel_case(s: &str) -> String {
    let pascal = s.to_upper_camel_case();
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TEST_PREFIX;
    use alef_core::config::Language;

    #[test]
    fn test_enum_variant_name_python() {
        assert_eq!(enum_variant_name("Atx", Language::Python, TEST_PREFIX), "ATX");
        assert_eq!(
            enum_variant_name("SnakeCase", Language::Python, TEST_PREFIX),
            "SNAKE_CASE"
        );
    }

    #[test]
    fn test_enum_variant_name_java() {
        assert_eq!(enum_variant_name("Atx", Language::Java, TEST_PREFIX), "ATX");
    }

    #[test]
    fn test_enum_variant_name_ffi() {
        assert_eq!(enum_variant_name("Atx", Language::Ffi, TEST_PREFIX), "HTM_ATX");
    }

    #[test]
    fn test_type_name_ffi_uses_prefix() {
        assert_eq!(
            type_name("ConversionOptions", Language::Ffi, "Kreuzberg"),
            "KreuzbergConversionOptions"
        );
        assert_eq!(
            type_name("ConversionResult", Language::Ffi, "Kreuzberg"),
            "KreuzbergConversionResult"
        );
    }

    #[test]
    fn test_func_name_ffi_uses_prefix() {
        assert_eq!(func_name("convert", Language::Ffi, "Kreuzberg"), "kreuzberg_convert");
    }

    #[test]
    fn test_enum_variant_name_ffi_uses_prefix() {
        assert_eq!(enum_variant_name("Atx", Language::Ffi, "Kreuzberg"), "KREUZBERG_ATX");
    }

    #[test]
    fn test_field_name_go_pascal_case() {
        assert_eq!(field_name("heading_style", Language::Go), "HeadingStyle");
        assert_eq!(field_name("list_indent_type", Language::Go), "ListIndentType");
    }

    #[test]
    fn test_func_name_conventions() {
        assert_eq!(func_name("convert", Language::Python, TEST_PREFIX), "convert");
        assert_eq!(func_name("convert_html", Language::Node, TEST_PREFIX), "convertHtml");
        assert_eq!(func_name("convert_html", Language::Go, TEST_PREFIX), "ConvertHtml");
        assert_eq!(func_name("convert", Language::Ffi, TEST_PREFIX), "htm_convert");
    }

    #[test]
    fn test_type_name_ffi_prefix() {
        assert_eq!(
            type_name("ConversionOptions", Language::Ffi, TEST_PREFIX),
            "HtmConversionOptions"
        );
        assert_eq!(
            type_name("ConversionResult", Language::Ffi, TEST_PREFIX),
            "HtmConversionResult"
        );
    }
}

// ---------------------------------------------------------------------------
// Default value formatting
// ---------------------------------------------------------------------------
