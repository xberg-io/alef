/// Dart core library class names that cannot be shadowed by generated type names.
///
/// These are not keywords (they are not in DART_RESERVED) but they are defined in
/// `dart:core` and would shadow the built-in generic types if used as class names,
/// causing "Expected 0 type arguments" errors for any code that uses `List<T>`, etc.
#[allow(dead_code)]
const DART_CORE_TYPES: &[&str] = &[
    "bool",
    "double",
    "Duration",
    "Error",
    "Exception",
    "Future",
    "int",
    "Invocation",
    "Iterable",
    "Iterator",
    "List",
    "Map",
    "MapEntry",
    "Null",
    "num",
    "Object",
    "Pattern",
    "RegExp",
    "RuneIterator",
    "Runes",
    "Set",
    "Sink",
    "StackTrace",
    "Stream",
    "String",
    "StringBuffer",
    "Symbol",
    "Type",
    "Uri",
];

/// Dart reserved words and built-in identifiers that cannot be used as identifiers.
///
/// Includes all reserved words, built-in identifiers, and async-reserved words.
/// Source: <https://dart.dev/language/keywords>
const DART_RESERVED: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "async",
    "await",
    "base",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "covariant",
    "default",
    "deferred",
    "do",
    "dynamic",
    "else",
    "enum",
    "export",
    "extends",
    "extension",
    "external",
    "factory",
    "false",
    "final",
    "finally",
    "for",
    "Function",
    "get",
    "hide",
    "if",
    "implements",
    "import",
    "in",
    "interface",
    "is",
    "late",
    "library",
    "mixin",
    "new",
    "null",
    "on",
    "operator",
    "part",
    "required",
    "rethrow",
    "return",
    "sealed",
    "set",
    "show",
    "static",
    "super",
    "switch",
    "sync",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "var",
    "void",
    "when",
    "while",
    "with",
    "yield",
];

/// Make a generated class name safe for use as a Dart type declaration.
///
/// Dart core library classes (like `List`, `Map`, `Set`, `String`, etc.) cannot be
/// shadowed by generated classes: doing so breaks `List<T>` generics in the same file.
///
/// When `name` conflicts with a Dart core type, the parent enum or struct name is
/// prepended (e.g. `NodeContent` + `List` → `NodeContentList`). If `parent` is empty
/// or None, a trailing `Node` suffix is appended instead.
#[allow(dead_code)]
pub(crate) fn dart_safe_type_name(name: &str, parent: Option<&str>) -> String {
    if DART_CORE_TYPES.contains(&name) || DART_RESERVED.contains(&name) {
        match parent {
            Some(p) if !p.is_empty() => format!("{p}{name}"),
            _ => format!("{name}Node"),
        }
    } else {
        name.to_string()
    }
}

/// Escape a Dart identifier to avoid conflicts with reserved keywords or
/// invalid names such as numeric tuple-variant field indices.
///
/// Rules applied in order:
/// 1. Names whose first character is an ASCII digit (e.g. `"0"`) get `field`
///    prepended: `"0"` → `"field0"`.
/// 2. Names that exactly match a Dart reserved word get a trailing `_`
///    appended: `"default"` → `"default_"`.
/// 3. All other names are returned unchanged.
pub(crate) fn dart_safe_ident(name: &str) -> String {
    // Numeric tuple-field index: "0", "1", … → "field0", "field1", …
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return format!("field{name}");
    }
    if DART_RESERVED.contains(&name) {
        return format!("{name}_");
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::dart_safe_ident;

    #[test]
    fn reserved_keyword_default_gets_trailing_underscore() {
        assert_eq!(dart_safe_ident("default"), "default_");
    }

    #[test]
    fn reserved_keyword_final_gets_trailing_underscore() {
        assert_eq!(dart_safe_ident("final"), "final_");
    }

    #[test]
    fn reserved_keyword_class_gets_trailing_underscore() {
        assert_eq!(dart_safe_ident("class"), "class_");
    }

    #[test]
    fn reserved_keyword_return_gets_trailing_underscore() {
        assert_eq!(dart_safe_ident("return"), "return_");
    }

    #[test]
    fn reserved_keyword_required_gets_trailing_underscore() {
        assert_eq!(dart_safe_ident("required"), "required_");
    }

    #[test]
    fn numeric_ident_zero_gets_field_prefix() {
        assert_eq!(dart_safe_ident("0"), "field0");
    }

    #[test]
    fn numeric_ident_one_gets_field_prefix() {
        assert_eq!(dart_safe_ident("1"), "field1");
    }

    #[test]
    fn normal_ident_passes_through_unchanged() {
        assert_eq!(dart_safe_ident("radius"), "radius");
        assert_eq!(dart_safe_ident("xCoord"), "xCoord");
        assert_eq!(dart_safe_ident("field0"), "field0");
    }
}
