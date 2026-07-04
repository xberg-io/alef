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

/// Python `str` instance methods that an enum member name can shadow.
///
/// `StrEnum` inherits from `str`, so variant names that match `str` instance methods
/// (e.g., `Title` → `title`) shadow the inherited method and trigger mypy `assignment`
/// errors at the class body. This constant lists all such methods that must be escaped
/// with a trailing underscore when used as enum member names (e.g., `title_`).
pub const PYTHON_STR_METHODS: &[&str] = &[
    "capitalize",
    "casefold",
    "center",
    "count",
    "encode",
    "endswith",
    "expandtabs",
    "find",
    "format",
    "format_map",
    "index",
    "isalnum",
    "isalpha",
    "isascii",
    "isdecimal",
    "isdigit",
    "isidentifier",
    "islower",
    "isnumeric",
    "isprintable",
    "isspace",
    "istitle",
    "isupper",
    "join",
    "ljust",
    "lower",
    "lstrip",
    "maketrans",
    "partition",
    "removeprefix",
    "removesuffix",
    "replace",
    "rfind",
    "rindex",
    "rjust",
    "rpartition",
    "rsplit",
    "rstrip",
    "split",
    "splitlines",
    "startswith",
    "strip",
    "swapcase",
    "title",
    "translate",
    "upper",
    "zfill",
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
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "null",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "when",
    "while",
    // Soft keywords commonly mistaken as identifiers
    "by",
    "init",
    "constructor",
    "field",
    "value",
    "where",
];

/// Swift reserved keywords (declarations + statements + expressions/types + patterns).
pub const SWIFT_KEYWORDS: &[&str] = &[
    "associatedtype",
    "class",
    "deinit",
    "enum",
    "extension",
    "fileprivate",
    "func",
    "import",
    "init",
    "inout",
    "internal",
    "let",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "rethrows",
    "static",
    "struct",
    "subscript",
    "typealias",
    "var",
    "break",
    "case",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "fallthrough",
    "for",
    "guard",
    "if",
    "in",
    "repeat",
    "return",
    "switch",
    "where",
    "while",
    "as",
    "Any",
    "catch",
    "false",
    "is",
    "nil",
    "super",
    "self",
    "Self",
    "throw",
    "throws",
    "true",
    "try",
    "_",
];

/// Dart reserved + built-in identifiers that cannot be used as plain identifiers.
pub const DART_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "async",
    "await",
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
    "of",
    "on",
    "operator",
    "part",
    "required",
    "rethrow",
    "return",
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

/// Gleam reserved keywords.
pub const GLEAM_KEYWORDS: &[&str] = &[
    "as",
    "assert",
    "auto",
    "case",
    "const",
    "delegate",
    "derive",
    "echo",
    "else",
    "fn",
    "if",
    "implement",
    "import",
    "let",
    "macro",
    "opaque",
    "panic",
    "pub",
    "test",
    "todo",
    "type",
    "use",
];

/// Zig reserved keywords.
pub const ZIG_KEYWORDS: &[&str] = &[
    "addrspace",
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "async",
    "await",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "linksection",
    "noalias",
    "noinline",
    "nosuspend",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
    "while",
];

/// Rust reserved keywords (strict, reserved, and weak keywords from all editions).
///
/// This list covers every identifier that cannot be used as a bare identifier in Rust
/// source code.  When a serde-renamed field name (e.g. `"type"`) is used as a Rust
/// function parameter or struct-literal field, it must be written as a raw identifier
/// (`r#type`) to avoid a compile error.
pub const RUST_KEYWORDS: &[&str] = &[
    // Strict keywords
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", // Edition-2018+ keywords
    "async", "await", "dyn",
    // Reserved keywords (may not be valid today but are reserved for future use)
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield",
    "try",
];

/// Escape a name that is a Rust keyword by prepending the raw-identifier prefix (`r#`).
///
/// Returns `Some("r#<name>")` when `name` is a Rust keyword, `None` otherwise.
/// Use this when emitting a Rust identifier (function parameter, struct-literal field,
/// local variable) whose text comes from an external source (e.g. a serde rename) and
/// may coincide with a reserved word.
///
/// Note: PyO3 strips the `r#` prefix when deriving the Python-facing name, so a parameter
/// declared as `r#type` is still exposed to Python as `type`.
pub fn rust_raw_ident_safe(name: &str) -> Option<String> {
    if RUST_KEYWORDS.contains(&name) {
        Some(format!("r#{name}"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Rust identifier, escaping reserved keywords with
/// the raw-identifier prefix (`r#`).
pub fn rust_raw_ident(name: &str) -> String {
    rust_raw_ident_safe(name).unwrap_or_else(|| name.to_string())
}

/// Returns `true` if `name` is a syntactically valid Rust identifier (ignoring whether
/// it is a reserved keyword — use `rust_raw_ident` to handle keywords separately).
///
/// Valid identifiers start with a letter or `_` and contain only alphanumeric characters
/// and `_`.  Names like `"self-harm"` or `"self-harm/intent"` (serde renames containing
/// hyphens or slashes) are NOT valid identifiers and should fall back to the Rust field
/// name instead of being used directly as parameter names.
pub fn is_valid_rust_ident_chars(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().expect("non-empty string has a first char");
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

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

/// Returns `Some(escaped_name)` if `name` is either a Python reserved keyword
/// OR a `str` instance method name that would shadow in a `StrEnum` context.
///
/// Use this for `StrEnum` variant names to prevent mypy `assignment` errors.
/// Escaping appends a trailing underscore (e.g., `title` → `title_`).
pub fn python_str_enum_safe_name(name: &str) -> Option<String> {
    if PYTHON_KEYWORDS.contains(&name) || PYTHON_STR_METHODS.contains(&name) {
        Some(format!("{name}_"))
    } else {
        None
    }
}

/// Like `python_str_enum_safe_name` but always returns a `String`, using the original
/// when no escaping is needed. Convenience wrapper for `StrEnum` variant names.
pub fn python_str_enum_ident(name: &str) -> String {
    python_str_enum_safe_name(name).unwrap_or_else(|| name.to_string())
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

/// Returns `Some(backtick_escaped_name)` if `name` is a Swift reserved keyword,
/// else `None`.
///
/// Use this for identifiers that appear in *emitted Swift source code* — enum
/// cases, struct field names, function parameter labels — where the idiomatic
/// escape for a keyword collision is `` `keyword` `` (backticks) rather than a
/// trailing underscore. For identifiers on the Rust side of the swift-bridge
/// boundary use [`swift_safe_name`] / [`swift_ident`] instead.
pub fn swift_case_safe_name(name: &str) -> Option<String> {
    if SWIFT_KEYWORDS.contains(&name) {
        Some(format!("`{name}`"))
    } else {
        None
    }
}

/// Convenience: always returns a usable Swift identifier for emitted Swift
/// code, wrapping reserved keywords in backticks (`` `default` ``).
///
/// This is the Swift-idiomatic escape for keyword-collision identifiers in
/// Swift source — distinct from [`swift_ident`], which appends a trailing
/// underscore for use on the Rust side of the bridge.
pub fn swift_case_ident(name: &str) -> String {
    swift_case_safe_name(name).unwrap_or_else(|| name.to_string())
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
///
/// Sanitizes the input so that it is a valid Zig identifier:
///   1. Non-`[A-Za-z0-9_]` characters are replaced with `_` (so serde renames like
///      `og:image` or `Content-Type` become `og_image` / `Content_Type`).
///   2. A leading digit is prefixed with `_`.
///   3. The result is then checked against Zig's reserved-word list and escaped
///      with a trailing `_` if necessary.
pub fn zig_ident(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len() + 1);
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        sanitized.insert(0, '_');
    }
    zig_safe_name(&sanitized).unwrap_or(sanitized)
}

/// Python builtin names that are usable in type positions. A class member
/// (field or method) with one of these names shadows the builtin for every
/// annotation in that class body — `mypy --strict` rejects the annotation with
/// `Variable "..." is not valid as a type`. Annotations in such a class must
/// qualify the builtin as `builtins.<name>`.
pub const PYTHON_BUILTIN_TYPE_NAMES: &[&str] = &[
    "bool",
    "bytearray",
    "bytes",
    "complex",
    "dict",
    "float",
    "frozenset",
    "int",
    "list",
    "memoryview",
    "object",
    "set",
    "str",
    "tuple",
    "type",
];

/// Qualify builtin type names shadowed by `member_names` inside one Python
/// class body (or type-hint string), rewriting type-position occurrences to
/// `builtins.<name>`. Returns `None` when nothing needed rewriting.
///
/// Occurrences are left alone when they are:
/// - inside string literals (including triple-quoted docstrings) or comments,
/// - attribute accesses (preceded by `.`),
/// - the declaration name itself — a field (`bytes: ...`), parameter
///   (`(bytes: ...`), or method (`def bytes(`) — recognized as an identifier
///   followed by `:` whose preceding context is a declaration position
///   (line start, `(`, `,`, or `*`). Return-type positions (`-> bytes:`) are
///   still qualified.
pub fn qualify_shadowed_python_builtins(block: &str, member_names: &std::collections::HashSet<&str>) -> Option<String> {
    let shadowed: std::collections::HashSet<&str> = PYTHON_BUILTIN_TYPE_NAMES
        .iter()
        .copied()
        .filter(|b| member_names.contains(b))
        .collect();
    if shadowed.is_empty() {
        return None;
    }

    const TRIPLE_DOUBLE: &str = "\"\"\"";
    const TRIPLE_SINGLE: &str = "'''";

    let mut out = String::with_capacity(block.len() + 64);
    let mut changed = false;
    // Cross-line string state: Some(delim) while inside a (possibly triple) quote.
    let mut string_delim: Option<&'static str> = None;

    for line in block.split_inclusive('\n') {
        let line_bytes = line.as_bytes();
        let mut i = 0;
        while i < line_bytes.len() {
            // Inside a string literal: scan for its closing delimiter.
            if let Some(delim) = string_delim {
                if line[i..].starts_with(delim) {
                    out.push_str(delim);
                    i += delim.len();
                    string_delim = None;
                } else {
                    out.push(line_bytes[i] as char);
                    i += 1;
                }
                continue;
            }
            let ch = line_bytes[i] as char;
            // Comment: copy the rest of the line verbatim.
            if ch == '#' {
                out.push_str(&line[i..]);
                break;
            }
            // String start (triple quotes before singles).
            if line[i..].starts_with(TRIPLE_DOUBLE) || line[i..].starts_with(TRIPLE_SINGLE) {
                let delim = if line_bytes[i] == b'"' {
                    TRIPLE_DOUBLE
                } else {
                    TRIPLE_SINGLE
                };
                out.push_str(delim);
                i += 3;
                string_delim = Some(delim);
                continue;
            }
            if ch == '"' || ch == '\'' {
                let delim = if ch == '"' { "\"" } else { "'" };
                out.push(ch);
                i += 1;
                string_delim = Some(delim);
                continue;
            }
            // Identifier token.
            if ch.is_ascii_alphabetic() || ch == '_' {
                let start = i;
                while i < line_bytes.len() && ((line_bytes[i] as char).is_ascii_alphanumeric() || line_bytes[i] == b'_')
                {
                    i += 1;
                }
                let ident = &line[start..i];
                let is_attribute = line[..start].ends_with('.');
                if !is_attribute && shadowed.contains(ident) && !is_declaration_name(line, start, i) {
                    out.push_str("builtins.");
                    out.push_str(ident);
                    changed = true;
                } else {
                    out.push_str(ident);
                }
                continue;
            }
            out.push(ch);
            i += 1;
        }
        // Single-quoted strings do not continue across lines in generated code.
        if let Some(delim) = string_delim {
            if delim.len() == 1 {
                string_delim = None;
            }
        }
    }

    changed.then_some(out)
}

/// True when the identifier at `line[start..end]` is a declaration NAME rather
/// than a type usage: followed by `:` and preceded by a declaration position
/// (line start, `(`, `,`, or `*`), or preceded by `def `.
fn is_declaration_name(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].trim_end();
    if before.ends_with("def") {
        return true;
    }
    let followed_by_colon = line[end..].trim_start().starts_with(':');
    if !followed_by_colon {
        return false;
    }
    before.is_empty() || before.ends_with('(') || before.ends_with(',') || before.ends_with('*')
}

#[cfg(test)]
mod tests;
