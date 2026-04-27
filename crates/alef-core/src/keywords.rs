//! Reserved keyword lists and field-name escaping for all supported language backends.
//!
//! Each language backend may encounter Rust field names that are reserved keywords
//! in the target language. This module provides a central registry of those keywords
//! and a function to compute the safe name to use in the generated binding.
//!
//! # Escape strategy
//!
//! When a field name is reserved in the target language it is escaped by appending
//! a trailing underscore (e.g. `class` → `class_`).  The original name is preserved
//! in language-level attribute annotations so the user-visible API still exposes the
//! original name (e.g. `#[pyo3(get, name = "class")]`, `#[serde(rename = "class")]`).

/// Python reserved keywords and soft-keywords that cannot be used as identifiers.
///
/// Includes the `type` soft-keyword (Python 3.12+) and the built-in constants
/// `None`, `True`, `False` which are also reserved in identifier position.
pub const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "type", "while", "with", "yield",
];

/// Java reserved keywords (including all contextual/reserved identifiers).
pub const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
];

/// C# reserved keywords.
pub const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// PHP reserved keywords.
pub const PHP_KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "die",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enddeclare",
    "endfor",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "eval",
    "exit",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "instanceof",
    "insteadof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

/// Ruby reserved keywords.
pub const RUBY_KEYWORDS: &[&str] = &[
    "__ENCODING__",
    "__FILE__",
    "__LINE__",
    "BEGIN",
    "END",
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
];

/// Elixir reserved keywords (including sigil names and special atoms).
pub const ELIXIR_KEYWORDS: &[&str] = &[
    "after", "and", "catch", "do", "else", "end", "false", "fn", "in", "nil", "not", "or", "rescue", "true", "when",
];

/// Go reserved keywords.
pub const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// JavaScript / TypeScript reserved keywords (union of both).
pub const JS_KEYWORDS: &[&str] = &[
    "abstract",
    "arguments",
    "await",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "function",
    "goto",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "int",
    "interface",
    "let",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "volatile",
    "while",
    "with",
    "yield",
];

/// R reserved keywords.
pub const R_KEYWORDS: &[&str] = &[
    "FALSE", "Inf", "NA", "NaN", "NULL", "TRUE", "break", "else", "for", "function", "if", "in", "next", "repeat",
    "return", "while",
];

/// Kotlin reserved keywords (hard + soft + modifier keywords that conflict with identifiers).
pub const KOTLIN_KEYWORDS: &[&str] = &[
    "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is", "null",
    "object", "package", "return", "super", "this", "throw", "true", "try", "typealias", "typeof", "val", "var",
    "when", "while",
    // Soft keywords commonly mistaken as identifiers
    "by", "init", "constructor", "field", "value", "where",
];

/// Swift reserved keywords (declarations + statements + expressions/types + patterns).
pub const SWIFT_KEYWORDS: &[&str] = &[
    "associatedtype", "class", "deinit", "enum", "extension", "fileprivate", "func", "import", "init", "inout",
    "internal", "let", "open", "operator", "private", "protocol", "public", "rethrows", "static", "struct",
    "subscript", "typealias", "var", "break", "case", "continue", "default", "defer", "do", "else", "fallthrough",
    "for", "guard", "if", "in", "repeat", "return", "switch", "where", "while", "as", "Any", "catch", "false", "is",
    "nil", "super", "self", "Self", "throw", "throws", "true", "try", "_",
];

/// Dart reserved + built-in identifiers that cannot be used as plain identifiers.
pub const DART_KEYWORDS: &[&str] = &[
    "abstract", "as", "assert", "async", "await", "break", "case", "catch", "class", "const", "continue", "covariant",
    "default", "deferred", "do", "dynamic", "else", "enum", "export", "extends", "extension", "external", "factory",
    "false", "final", "finally", "for", "Function", "get", "hide", "if", "implements", "import", "in", "interface",
    "is", "late", "library", "mixin", "new", "null", "of", "on", "operator", "part", "required", "rethrow", "return",
    "set", "show", "static", "super", "switch", "sync", "this", "throw", "true", "try", "typedef", "var", "void",
    "when", "while", "with", "yield",
];

/// Gleam reserved keywords.
pub const GLEAM_KEYWORDS: &[&str] = &[
    "as", "assert", "auto", "case", "const", "delegate", "derive", "echo", "else", "fn", "if", "implement", "import",
    "let", "macro", "opaque", "panic", "pub", "test", "todo", "type", "use",
];

/// Zig reserved keywords.
pub const ZIG_KEYWORDS: &[&str] = &[
    "addrspace", "align", "allowzero", "and", "anyframe", "anytype", "asm", "async", "await", "break", "callconv",
    "catch", "comptime", "const", "continue", "defer", "else", "enum", "errdefer", "error", "export", "extern", "fn",
    "for", "if", "inline", "linksection", "noalias", "noinline", "nosuspend", "or", "orelse", "packed", "pub",
    "resume", "return", "struct", "suspend", "switch", "test", "threadlocal", "try", "union", "unreachable", "usingnamespace",
    "var", "volatile", "while",
];

/// Return the escaped field name for use in the generated binding of the given language,
/// or `None` if the name is not reserved and no escaping is needed.
///
/// The escape strategy appends `_` to the name (e.g. `class` → `class_`).
/// Call sites should use the returned value as the Rust field name in the binding struct
/// and add language-appropriate attribute annotations to preserve the original name in
/// the user-facing API.
pub fn python_safe_name(name: &str) -> Option<String> {
    if PYTHON_KEYWORDS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Like `python_safe_name` but always returns a `String`, using the original when no
/// escaping is needed. Convenience wrapper for call sites that always need a `String`.
pub fn python_ident(name: &str) -> String {
    python_safe_name(name).unwrap_or_else(|| name.to_string())
}

/// Returns `Some(escaped_name)` if `name` is a Kotlin reserved keyword, else `None`.
pub fn kotlin_safe_name(name: &str) -> Option<String> {
    if KOTLIN_KEYWORDS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Kotlin identifier.
pub fn kotlin_ident(name: &str) -> String {
    kotlin_safe_name(name).unwrap_or_else(|| name.to_string())
}

/// Returns `Some(escaped_name)` if `name` is a Swift reserved keyword, else `None`.
pub fn swift_safe_name(name: &str) -> Option<String> {
    if SWIFT_KEYWORDS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Swift identifier.
pub fn swift_ident(name: &str) -> String {
    swift_safe_name(name).unwrap_or_else(|| name.to_string())
}

/// Returns `Some(escaped_name)` if `name` is a Dart reserved keyword, else `None`.
pub fn dart_safe_name(name: &str) -> Option<String> {
    if DART_KEYWORDS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Dart identifier.
pub fn dart_ident(name: &str) -> String {
    dart_safe_name(name).unwrap_or_else(|| name.to_string())
}

/// Returns `Some(escaped_name)` if `name` is a Gleam reserved keyword, else `None`.
pub fn gleam_safe_name(name: &str) -> Option<String> {
    if GLEAM_KEYWORDS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Gleam identifier.
pub fn gleam_ident(name: &str) -> String {
    gleam_safe_name(name).unwrap_or_else(|| name.to_string())
}

/// Returns `Some(escaped_name)` if `name` is a Zig reserved keyword, else `None`.
pub fn zig_safe_name(name: &str) -> Option<String> {
    if ZIG_KEYWORDS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Zig identifier.
pub fn zig_ident(name: &str) -> String {
    zig_safe_name(name).unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_class_is_reserved() {
        assert_eq!(python_safe_name("class"), Some("class_".to_string()));
    }

    #[test]
    fn python_ordinary_name_is_none() {
        assert_eq!(python_safe_name("layout_class"), None);
    }

    #[test]
    fn python_ident_reserved() {
        assert_eq!(python_ident("class"), "class_");
    }

    #[test]
    fn python_ident_ordinary() {
        assert_eq!(python_ident("layout_class"), "layout_class");
    }

    #[test]
    fn kotlin_class_is_reserved() {
        assert_eq!(kotlin_safe_name("class"), Some("class_".to_string()));
        assert_eq!(kotlin_safe_name("fun"), Some("fun_".to_string()));
        assert_eq!(kotlin_safe_name("ordinary"), None);
        assert_eq!(kotlin_ident("typealias"), "typealias_");
    }

    #[test]
    fn swift_init_is_reserved() {
        assert_eq!(swift_safe_name("init"), Some("init_".to_string()));
        assert_eq!(swift_safe_name("Self"), Some("Self_".to_string()));
        assert_eq!(swift_safe_name("normal"), None);
        assert_eq!(swift_ident("protocol"), "protocol_");
    }

    #[test]
    fn dart_async_is_reserved() {
        assert_eq!(dart_safe_name("async"), Some("async_".to_string()));
        assert_eq!(dart_safe_name("late"), Some("late_".to_string()));
        assert_eq!(dart_safe_name("normal"), None);
        assert_eq!(dart_ident("required"), "required_");
    }

    #[test]
    fn gleam_pub_is_reserved() {
        assert_eq!(gleam_safe_name("pub"), Some("pub_".to_string()));
        assert_eq!(gleam_safe_name("opaque"), Some("opaque_".to_string()));
        assert_eq!(gleam_safe_name("normal"), None);
        assert_eq!(gleam_ident("type"), "type_");
    }

    #[test]
    fn zig_comptime_is_reserved() {
        assert_eq!(zig_safe_name("comptime"), Some("comptime_".to_string()));
        assert_eq!(zig_safe_name("errdefer"), Some("errdefer_".to_string()));
        assert_eq!(zig_safe_name("normal"), None);
        assert_eq!(zig_ident("usingnamespace"), "usingnamespace_");
    }

    #[test]
    fn python_keywords_covers_common_cases() {
        for kw in &[
            "def", "return", "yield", "pass", "import", "from", "type", "None", "True", "False",
        ] {
            assert!(
                python_safe_name(kw).is_some(),
                "expected {kw:?} to be a Python reserved keyword"
            );
        }
    }
}
