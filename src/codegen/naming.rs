use crate::core::config::Language;
use heck::{ToKebabCase, ToLowerCamelCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

/// Distinct name surfaces used by generated bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSurface {
    /// Public identifier exposed in the target host language.
    PublicHost,
    /// Wire/JSON field names, tags, and variant values.
    Wire,
    /// Internal Rust identifier emitted by a backend crate.
    InternalRust,
    /// ABI/native symbol such as C FFI or JNI.
    Abi,
}

/// Identifier context within a name surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierContext {
    PublicType,
    PublicMember,
    PublicParameter,
    PublicEnumVariant,
    Wire,
    InternalRust,
    AbiSymbol,
    SwiftSource,
    SwiftRustShim,
    KotlinSource,
    KotlinRustBridge,
    DartType,
    DartValue,
    DartTupleField,
}

/// Public host-language identifier kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicIdentifierKind {
    Function,
    Method,
    Field,
    Type,
    EnumVariant,
    Parameter,
}

/// A generated-name collision within one target scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCollision {
    pub generated: String,
    pub originals: Vec<String>,
}

/// Error raised by centralized naming validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    InvalidIdentifier {
        lang: Language,
        context: IdentifierContext,
        name: String,
    },
    Collision(NameCollision),
}

/// Apply a serde `rename_all` strategy to a Rust identifier.
pub fn apply_serde_rename_all(name: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("lowercase") => name.to_ascii_lowercase(),
        Some("UPPERCASE") => name.to_ascii_uppercase(),
        Some("PascalCase") => name.to_pascal_case(),
        Some("camelCase") => name.to_lower_camel_case(),
        Some("snake_case") => pascal_to_snake(name),
        Some("SCREAMING_SNAKE_CASE") => pascal_to_screaming_snake(name),
        Some("kebab-case") => pascal_to_snake(name).to_kebab_case(),
        Some("SCREAMING-KEBAB-CASE") => pascal_to_snake(name).to_kebab_case().to_ascii_uppercase(),
        Some(_) | None => name.to_string(),
    }
}

/// Resolve a serde wire name, with explicit `serde(rename)` taking precedence.
pub fn serde_wire_name(rust_name: &str, serde_rename: Option<&str>, rename_all: Option<&str>) -> String {
    serde_rename
        .map(str::to_string)
        .unwrap_or_else(|| apply_serde_rename_all(rust_name, rename_all))
}

/// Resolve a wire field name from field metadata.
pub fn wire_field_name(field_name: &str, serde_rename: Option<&str>, rename_all: Option<&str>) -> String {
    serde_wire_name(field_name, serde_rename, rename_all)
}

/// Resolve a wire enum variant value from variant metadata.
pub fn wire_variant_value(variant_name: &str, serde_rename: Option<&str>, rename_all: Option<&str>) -> String {
    serde_wire_name(variant_name, serde_rename, rename_all)
}

/// Resolve a public field/property identifier, applying `rename_fields` before language casing.
pub fn public_field_name(lang: Language, rust_field_name: &str, rename_fields_value: Option<&str>) -> String {
    let base = rename_fields_value.unwrap_or(rust_field_name);
    public_host_identifier(lang, PublicIdentifierKind::Field, base)
}

/// Resolve a public host-language identifier for a Rust name.
pub fn public_host_identifier(lang: Language, kind: PublicIdentifierKind, rust_name: &str) -> String {
    let converted = match kind {
        PublicIdentifierKind::Type => public_type_name(lang, rust_name),
        PublicIdentifierKind::EnumVariant => public_enum_variant_name(lang, rust_name),
        PublicIdentifierKind::Function | PublicIdentifierKind::Method | PublicIdentifierKind::Field => {
            public_member_name(lang, rust_name)
        }
        PublicIdentifierKind::Parameter => public_parameter_name(lang, rust_name),
    };
    escape_identifier_for(lang, &converted, public_identifier_context(kind))
}

/// Resolve an internal Rust identifier and raw-escape Rust keywords.
pub fn internal_rust_identifier(name: &str) -> String {
    crate::core::keywords::rust_raw_ident(name)
}

/// Resolve a C-style ABI symbol with an explicit prefix.
pub fn abi_symbol(prefix: &str, name: &str) -> String {
    to_c_name(prefix, name)
}

/// Return a language-safe identifier for a generated name surface.
pub fn escape_identifier(lang: Language, name: &str, surface: NameSurface) -> String {
    let context = match surface {
        NameSurface::PublicHost => IdentifierContext::PublicMember,
        NameSurface::Wire => IdentifierContext::Wire,
        NameSurface::InternalRust => IdentifierContext::InternalRust,
        NameSurface::Abi => IdentifierContext::AbiSymbol,
    };
    escape_identifier_for(lang, name, context)
}

/// Return a language-safe identifier for a specific context.
pub fn escape_identifier_for(lang: Language, name: &str, context: IdentifierContext) -> String {
    match context {
        IdentifierContext::Wire => name.to_string(),
        IdentifierContext::InternalRust => crate::core::keywords::rust_raw_ident(name),
        IdentifierContext::AbiSymbol => sanitize_symbol_component(name),
        IdentifierContext::SwiftSource => crate::core::keywords::swift_case_ident(name),
        IdentifierContext::SwiftRustShim => crate::core::keywords::swift_ident(name),
        IdentifierContext::KotlinSource => backtick_keyword(lang, name),
        IdentifierContext::KotlinRustBridge => crate::core::keywords::kotlin_ident(name),
        IdentifierContext::DartType => dart_type_identifier(name, None),
        IdentifierContext::DartValue => dart_value_identifier(name),
        IdentifierContext::DartTupleField => dart_tuple_field_identifier(name),
        IdentifierContext::PublicType
        | IdentifierContext::PublicMember
        | IdentifierContext::PublicParameter
        | IdentifierContext::PublicEnumVariant => match lang {
            Language::Swift => crate::core::keywords::swift_case_ident(name),
            Language::Zig => crate::core::keywords::zig_ident(name),
            Language::Python => crate::core::keywords::python_ident(name),
            Language::Kotlin | Language::KotlinAndroid => crate::core::keywords::kotlin_ident(name),
            Language::Dart => match context {
                IdentifierContext::PublicType => dart_type_identifier(name, None),
                IdentifierContext::PublicMember
                | IdentifierContext::PublicParameter
                | IdentifierContext::PublicEnumVariant => dart_value_identifier(name),
                _ => unreachable!("matched public identifier contexts only"),
            },
            Language::Gleam => crate::core::keywords::gleam_ident(name),
            _ if is_reserved_keyword(lang, name) => format!("{name}_"),
            _ => name.to_string(),
        },
    }
}

/// Validate that a generated identifier is syntactically usable for a language.
pub fn is_valid_identifier(lang: Language, name: &str, surface: NameSurface) -> bool {
    if matches!(surface, NameSurface::Wire) {
        return !name.is_empty();
    }
    match lang {
        Language::Rust => crate::core::keywords::is_valid_rust_ident_chars(name.trim_start_matches("r#")),
        Language::Swift => {
            let unescaped = name.strip_prefix('`').and_then(|s| s.strip_suffix('`')).unwrap_or(name);
            is_ascii_identifier(unescaped)
        }
        Language::Zig => is_ascii_identifier(name) && !name.starts_with(|ch: char| ch.is_ascii_digit()),
        Language::Csharp => {
            let unescaped = name.strip_prefix('@').unwrap_or(name);
            is_ascii_identifier(unescaped)
        }
        _ => is_ascii_identifier(name),
    }
}

/// Validate a generated identifier for a specific context.
pub fn validate_identifier(lang: Language, name: &str, context: IdentifierContext) -> Result<(), NameError> {
    if is_valid_identifier_for(lang, name, context) {
        Ok(())
    } else {
        Err(NameError::InvalidIdentifier {
            lang,
            context,
            name: name.to_string(),
        })
    }
}

/// Returns whether a generated identifier is syntactically usable for a specific context.
pub fn is_valid_identifier_for(lang: Language, name: &str, context: IdentifierContext) -> bool {
    match context {
        IdentifierContext::Wire => !name.is_empty(),
        IdentifierContext::InternalRust => {
            crate::core::keywords::is_valid_rust_ident_chars(name.trim_start_matches("r#"))
        }
        IdentifierContext::AbiSymbol => is_ascii_identifier(name),
        IdentifierContext::SwiftSource => {
            let unescaped = name.strip_prefix('`').and_then(|s| s.strip_suffix('`')).unwrap_or(name);
            is_ascii_identifier(unescaped)
        }
        IdentifierContext::DartTupleField => name.starts_with("field") && is_ascii_identifier(name),
        _ => is_valid_identifier(lang, name, NameSurface::PublicHost),
    }
}

/// Resolve a Dart type identifier, preserving core type names by adding context.
pub fn dart_type_identifier(name: &str, parent: Option<&str>) -> String {
    if is_dart_core_type(name) || is_reserved_keyword(Language::Dart, name) {
        match parent {
            Some(parent) if !parent.is_empty() => format!("{parent}{name}"),
            _ => format!("{name}Node"),
        }
    } else {
        name.to_string()
    }
}

/// Resolve a Dart value/member identifier.
pub fn dart_value_identifier(name: &str) -> String {
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return format!("field{name}");
    }
    crate::core::keywords::dart_ident(name)
}

/// Resolve a Dart tuple field identifier.
pub fn dart_tuple_field_identifier(name: &str) -> String {
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("field{name}")
    } else {
        dart_value_identifier(name)
    }
}

/// Resolve an ABI symbol from already-separated path components.
pub fn abi_symbol_from_components<I, S>(components: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parts = components
        .into_iter()
        .enumerate()
        .map(|(idx, component)| {
            let sanitized = sanitize_symbol_component(component.as_ref());
            if idx == 0 {
                sanitized
            } else {
                sanitized.trim_start_matches('_').to_string()
            }
        })
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let symbol = parts.join("_");
    if symbol.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        parts.insert(0, "_".to_string());
        parts.join("_")
    } else {
        symbol
    }
}

/// Return all generated-name collisions in a target scope.
pub fn detect_name_collisions<I, O, G>(items: I, generate: G) -> Vec<NameCollision>
where
    I: IntoIterator<Item = O>,
    O: AsRef<str>,
    G: Fn(&str) -> String,
{
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for item in items {
        let original = item.as_ref();
        grouped
            .entry(generate(original))
            .or_default()
            .push(original.to_string());
    }

    grouped
        .into_iter()
        .filter_map(|(generated, originals)| {
            let unique: HashSet<_> = originals.iter().collect();
            (unique.len() > 1).then_some(NameCollision { generated, originals })
        })
        .collect()
}

fn public_member_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Python | Language::Ruby | Language::Elixir | Language::Ffi | Language::R | Language::Rust => {
            name.to_snake_case()
        }
        Language::Go => to_go_name(name),
        Language::Csharp => to_csharp_name(name),
        Language::Node
        | Language::Php
        | Language::Wasm
        | Language::Java
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart => name.to_lower_camel_case(),
        Language::Gleam | Language::Zig | Language::C | Language::Jni => name.to_snake_case(),
    }
}

fn public_parameter_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Go => go_param_name(name),
        _ => public_member_name(lang, name),
    }
}

fn public_type_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Go => go_type_name(&name.to_pascal_case()),
        Language::Csharp => csharp_type_name(&name.to_pascal_case()),
        Language::Python
        | Language::Node
        | Language::Ruby
        | Language::Php
        | Language::Elixir
        | Language::Wasm
        | Language::Java
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig
        | Language::Ffi
        | Language::R
        | Language::Rust
        | Language::C
        | Language::Jni => name.to_pascal_case(),
    }
}

fn public_enum_variant_name(lang: Language, name: &str) -> String {
    match lang {
        Language::Python | Language::Ffi | Language::C | Language::Rust => pascal_to_screaming_snake(name),
        Language::Ruby | Language::Elixir | Language::R | Language::Gleam | Language::Zig => pascal_to_snake(name),
        Language::Go => go_type_name(&name.to_pascal_case()),
        Language::Csharp => csharp_type_name(&name.to_pascal_case()),
        Language::Node
        | Language::Php
        | Language::Wasm
        | Language::Java
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Jni => name.to_pascal_case(),
    }
}

fn public_identifier_context(kind: PublicIdentifierKind) -> IdentifierContext {
    match kind {
        PublicIdentifierKind::Function | PublicIdentifierKind::Method | PublicIdentifierKind::Field => {
            IdentifierContext::PublicMember
        }
        PublicIdentifierKind::Type => IdentifierContext::PublicType,
        PublicIdentifierKind::EnumVariant => IdentifierContext::PublicEnumVariant,
        PublicIdentifierKind::Parameter => IdentifierContext::PublicParameter,
    }
}

fn is_ascii_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().expect("non-empty string has a first char");
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn sanitize_symbol_component(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len() + 1);
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            sanitized.push(ch.to_ascii_lowercase());
        } else {
            sanitized.push('_');
        }
    }
    while sanitized.contains("__") {
        sanitized = sanitized.replace("__", "_");
    }
    let sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("_{sanitized}")
    } else {
        sanitized
    }
}

fn backtick_keyword(lang: Language, name: &str) -> String {
    if is_reserved_keyword(lang, name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

fn is_dart_core_type(name: &str) -> bool {
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
    DART_CORE_TYPES.contains(&name)
}

fn is_reserved_keyword(lang: Language, name: &str) -> bool {
    match lang {
        Language::Python => crate::core::keywords::PYTHON_KEYWORDS.contains(&name),
        Language::Node | Language::Wasm => crate::core::keywords::JS_KEYWORDS.contains(&name),
        Language::Ruby => crate::core::keywords::RUBY_KEYWORDS.contains(&name),
        Language::Php => crate::core::keywords::PHP_KEYWORDS.contains(&name),
        Language::Elixir => crate::core::keywords::ELIXIR_KEYWORDS.contains(&name),
        Language::Go => crate::core::keywords::GO_KEYWORDS.contains(&name),
        Language::Java | Language::Jni => crate::core::keywords::JAVA_KEYWORDS.contains(&name),
        Language::Csharp => crate::core::keywords::CSHARP_KEYWORDS.contains(&name),
        Language::R => crate::core::keywords::R_KEYWORDS.contains(&name),
        Language::Kotlin | Language::KotlinAndroid => crate::core::keywords::KOTLIN_KEYWORDS.contains(&name),
        Language::Swift => crate::core::keywords::SWIFT_KEYWORDS.contains(&name),
        Language::Dart => crate::core::keywords::DART_KEYWORDS.contains(&name),
        Language::Gleam => crate::core::keywords::GLEAM_KEYWORDS.contains(&name),
        Language::Zig => crate::core::keywords::ZIG_KEYWORDS.contains(&name),
        Language::Rust => crate::core::keywords::RUST_KEYWORDS.contains(&name),
        Language::Ffi | Language::C => false,
    }
}

/// Convert a Rust snake_case name to the target language convention.
pub fn to_python_name(name: &str) -> String {
    name.to_snake_case()
}

/// Convert a Rust snake_case name to Node.js/TypeScript lowerCamelCase convention.
pub fn to_node_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Convert a Rust snake_case name to Ruby snake_case convention.
pub fn to_ruby_name(name: &str) -> String {
    name.to_snake_case()
}

/// Convert a Rust snake_case name to PHP lowerCamelCase convention.
pub fn to_php_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Convert a Rust snake_case name to Elixir snake_case convention.
pub fn to_elixir_name(name: &str) -> String {
    name.to_snake_case()
}

/// Well-known initialisms that must be fully uppercased per Go naming conventions.
/// See: https://go.dev/wiki/CodeReviewComments#initialisms
const INITIALISMS: &[&str] = &[
    "API", "ASCII", "CPU", "CSS", "DNS", "EOF", "FTP", "GID", "GraphQL", "GUI", "HTML", "HTTP", "HTTPS", "ID", "IMAP",
    "IP", "JSON", "LHS", "MFA", "POP", "QPS", "RAM", "RHS", "RPC", "SLA", "SMTP", "SQL", "SSH", "SSL", "TCP", "TLS",
    "TTL", "UDP", "UI", "UID", "UUID", "URI", "URL", "UTF8", "VM", "XML", "XMPP", "XSRF", "XSS",
];

/// Initialisms preserved in C# PascalCase. Microsoft's framework design guidelines
/// recommend `Json`/`Http`/`Url` rather than `JSON`/`HTTP`/`URL` (3+ letter
/// initialisms use PascalCase, 2-letter ones use all-caps). This list intentionally
/// excludes generic acronyms so they round-trip cleanly through heck's PascalCase
/// (matching alef's hardcoded helper names like `{Type}ToJson`/`{Type}FromJson`),
/// while still preserving product names like `GraphQL` that heck would mangle.
// `Id` deliberately omitted: Microsoft's modern framework design guidelines
// (and the de-facto convention in EF Core, ASP.NET Core, Azure SDKs) treat
// `Id` as a word — `EntityId`, not `EntityID`. Keeping `ID` here also
// diverges from the e2e codegen, which calls `to_upper_camel_case` directly
// and emits `.Id` accessors; reconciling both sides to `Id` matches the
// existing test expectations.
const CSHARP_INITIALISMS: &[&str] = &["GraphQL", "UUID"];

/// Apply initialism uppercasing to a PascalCase name using the provided list.
///
/// Scans word boundaries in the PascalCase string and replaces any run of
/// characters that matches a known initialism (case-insensitively) with the
/// canonical form from the list. For example `ImageUrl` becomes `ImageURL`,
/// `UserId` becomes `UserID`, and `GraphQlRouteConfig` becomes `GraphQLRouteConfig`.
fn apply_initialisms(name: &str, list: &[&str]) -> String {
    if name.is_empty() {
        return name.to_string();
    }

    // Split the PascalCase string into words at uppercase letter boundaries.
    // Each "word" is a contiguous sequence starting with an uppercase letter.
    let mut words: Vec<&str> = Vec::new();
    let mut word_start = 0;
    let bytes = name.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            words.push(&name[word_start..i]);
            word_start = i;
        }
    }
    words.push(&name[word_start..]);

    // For each word, check if it matches a known initialism (case-insensitive).
    let mut result = String::with_capacity(name.len());
    let mut i = 0;
    while i < words.len() {
        // Try to match the longest possible span of consecutive words to a known initialism
        // (longest-match first). This handles multi-segment initialisms like "GraphQL" which
        // heck splits into "Graph" + "Ql".
        let mut matched = false;
        for span in (1..=(words.len() - i)).rev() {
            let candidate: String = words[i..i + span].concat();
            let candidate_upper = candidate.to_ascii_uppercase();
            if let Some(&canonical) = list.iter().find(|&&s| s.to_ascii_uppercase() == candidate_upper) {
                result.push_str(canonical);
                i += span;
                matched = true;
                break;
            }
        }
        if !matched {
            result.push_str(words[i]);
            i += 1;
        }
    }
    result
}

/// Apply Go initialism uppercasing to a PascalCase name.
///
/// Scans word boundaries in the PascalCase string and replaces any run of
/// characters that matches a known initialism (case-insensitively) with the
/// all-caps form. For example `ImageUrl` becomes `ImageURL` and `UserId`
/// becomes `UserID`.
fn apply_go_acronyms(name: &str) -> String {
    apply_initialisms(name, INITIALISMS)
}

/// Convert a Rust snake_case name to Go PascalCase convention with acronym uppercasing.
pub fn to_go_name(name: &str) -> String {
    apply_go_acronyms(&name.to_pascal_case())
}

/// Apply Go acronym uppercasing to a name that is already in PascalCase (e.g. an IR type name).
///
/// IR type names come directly from Rust PascalCase (e.g. `ImageUrl`, `JsonSchemaFormat`).
/// This function uppercases known acronym segments so they conform to Go naming conventions
/// (e.g. `ImageUrl` → `ImageURL`, `JsonSchemaFormat` → `JSONSchemaFormat`).
pub fn go_type_name(name: &str) -> String {
    apply_go_acronyms(name)
}

/// Convert a Rust snake_case parameter/variable name to Go lowerCamelCase with acronym uppercasing.
///
/// Go naming conventions require that acronyms in identifiers be fully uppercased.
/// `to_lower_camel_case` alone converts `base_url` → `baseUrl`, but Go wants `baseURL`.
/// This function converts via PascalCase (which applies acronym uppercasing) then lowercases
/// the first "word" (the initial run of uppercase letters treated as a unit) while preserving
/// the case of subsequent words/acronyms:
/// - `base_url`  → `BaseURL`  → `baseURL`
/// - `api_key`   → `APIKey`   → `apiKey`
/// - `user_id`   → `UserID`   → `userID`
/// - `json`      → `JSON`     → `json`
///
/// A parameter literally named `result` is renamed to `resultArg`. The Go return-marshalling
/// templates (`var_decl_slice`, `var_decl_type`, `result_json_unmarshal`, …) declare a hard-coded
/// local named `result` to hold the unmarshalled return value, so a parameter of the same name
/// would collide (`result redeclared`). `resultArg` is the only reserved rename needed because it is
/// the sole identifier the generated function bodies hard-code as a local.
pub fn go_param_name(name: &str) -> String {
    if name == "result" {
        return "resultArg".to_string();
    }
    let pascal = apply_go_acronyms(&name.to_pascal_case());
    if pascal.is_empty() {
        return pascal;
    }
    let bytes = pascal.as_bytes();
    // Find the boundary of the first "word":
    // - If the string begins with a multi-char uppercase run followed by a lowercase letter,
    //   the run minus its last char is an acronym prefix (e.g. "APIKey": run="API", next='K')
    //   → lowercase "AP" and keep "IKey" → "apIKey" ... but Go actually wants "apiKey".
    //   The real rule: lowercase the whole leading uppercase run regardless, because the
    //   acronym-prefix IS the first word.
    // - If the string begins with a single uppercase char (e.g. "BaseURL"), lowercase just it.
    //
    // Concretely: find how many leading bytes are uppercase. If that whole run is followed by
    // end-of-string, lowercase everything. If followed by more chars, lowercase the entire run.
    // For "APIKey": upper_len=3, next='K'(uppercase) but that starts the second word.
    // Actually: scan for the first lowercase char to find where the first word ends.
    let first_lower = bytes.iter().position(|b| b.is_ascii_lowercase());
    match first_lower {
        None => {
            // Entire string is uppercase (single acronym like "JSON", "URL") — all lowercase.
            pascal.to_lowercase()
        }
        Some(0) => {
            // Starts with lowercase (already correct)
            pascal
        }
        Some(pos) => {
            // pos is the index of the first lowercase char.
            // The first "word" ends just before pos-1 (the char at pos-1 is the first char of
            // the next PascalCase word that isnds with a lowercase continuation).
            // For "BaseURL": pos=1 ('a'), so uppercase run = ['B'], lowercase just index 0.
            // For "APIKey":  pos=4 ('e' in "Key"), uppercase run = "APIK", next lower = 'e',
            //   so word boundary is at pos-1=3 ('K' is start of "Key").
            //   → lowercase "API" (indices 0..2), keep "Key" → "apiKey" ✓
            // For "UserID":  pos=1 ('s'), uppercase run starts at 'U', lowercase just 'U' → "userID"... wait
            //   "UserID": 'U'(upper),'s'(lower) → pos=1, word="U", lower "U" → "u"+"serID" = "userID" ✓
            let word_end = if pos > 1 { pos - 1 } else { 1 };
            let lower_prefix = pascal[..word_end].to_lowercase();
            format!("{}{}", lower_prefix, &pascal[word_end..])
        }
    }
}

/// Convert a Rust snake_case name to Java lowerCamelCase convention.
pub fn to_java_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Convert a Rust snake_case name to C# PascalCase convention with initialism uppercasing.
///
/// Converts snake_case to PascalCase via `heck` and then restores C#-preserved initialisms.
/// The C# list is intentionally narrow (Microsoft's framework design guidelines prefer
/// `Json`/`Http`/`Url` over `JSON`/`HTTP`/`URL`), so only product names like `GraphQL`
/// and short 2-letter abbreviations get all-caps. This keeps method names like
/// `to_json` → `ToJson` in lockstep with alef's hardcoded `{Type}ToJson` /
/// `{Type}FromJson` helper declarations.
pub fn to_csharp_name(name: &str) -> String {
    apply_initialisms(&name.to_pascal_case(), CSHARP_INITIALISMS)
}

/// Derive the C# wrapper class name emitted by [`crate::backends::csharp::CsharpBackend`].
///
/// Converts the crate name to PascalCase, strips the Rust binding crate suffix "-rs",
/// and appends the idiomatic C# "Converter" suffix. For example:
/// - `sample-parser-rs` -> `SampleParser` -> `SampleParserConverter`
/// - `document_tools` -> `DocumentTools` -> `DocumentToolsConverter`
///
/// The README generator uses this helper so the generated C# usage example references
/// the same class name that the bindings actually emit.
pub fn csharp_wrapper_class_name(crate_name: &str, _namespace: &str) -> String {
    let base = to_csharp_name(crate_name);
    // Strip Rust-binding "Rs" suffix (from "-rs" crate suffix) and append idiomatic Converter suffix.
    let stem = base.strip_suffix("Rs").unwrap_or(&base);
    format!("{stem}Converter")
}

/// Derive the Kotlin Android wrapper object name emitted by the `KotlinAndroidBackend`.
///
/// Converts the crate name to PascalCase and strips the Rust binding crate
/// suffix "-rs".  The bare PascalCase name keeps the call site idiomatic
/// (`SampleParser.extractFile(...)` rather than `SampleParserConverter.extractFile(...)`)
/// and matches the bridge object emitted at `<Crate>Bridge` by
/// `crate::core::jni::bridge_class_name`.  For example:
/// - `sample-parser-rs` -> `SampleParser`
/// - `document_tools` -> `DocumentTools`
pub fn kotlin_android_wrapper_object_name(crate_name: &str) -> String {
    let base = public_type_name(Language::KotlinAndroid, crate_name);
    let stem = base.strip_suffix("Rs").unwrap_or(&base);
    stem.to_string()
}

/// Normalize 3+ letter acronyms at the start of a name to PascalCase.
///
/// C# convention: 3+ letter acronyms use PascalCase (Uri, Xml, Json) not all-caps (URI, XML, JSON).
/// This function detects names like "URI", "XML", "JSON" and converts them to "Uri", "Xml", "Json".
/// Leaves already-correct names like "Uri" unchanged, and preserves non-acronym names.
///
/// Examples:
/// - `URI`  → `Uri`  (acronym → PascalCase)
/// - `Uri`  → `Uri`  (already correct)
/// - `XML`  → `Xml`
/// - `Xml`  → `Xml`
/// - `JSON` → `Json`
/// - `Json` → `Json`
/// - `HttpStatus` → `HttpStatus` (not an acronym)
fn normalize_acronym_to_pascalcase(name: &str) -> String {
    if name.is_empty() {
        return name.to_string();
    }

    // Check if the name is all uppercase and 3+ letters (an acronym like "URI", "XML", "JSON")
    if name.len() >= 3 && name.chars().all(|c| c.is_ascii_uppercase()) {
        // Convert "URI" → "Uri", "XML" → "Xml", "JSON" → "Json"
        let mut result = String::with_capacity(name.len());
        result.push(name.chars().next().unwrap().to_ascii_uppercase());
        result.extend(name.chars().skip(1).map(|c| c.to_ascii_lowercase()));
        return result;
    }

    // Not an all-caps acronym — return as-is
    name.to_string()
}

/// Apply C# initialism handling to a name that is already in PascalCase (e.g. an IR type name).
///
/// IR type names come directly from Rust PascalCase (e.g. `GraphQLRouteConfig`, `HttpStatus`).
/// When such names have been processed by `heck::ToPascalCase` they may lose initialism
/// capitalisation for the names we explicitly preserve (e.g. `GraphQLRouteConfig` →
/// `GraphQlRouteConfig`). This function restores them.
///
/// Examples:
/// - `GraphQlRouteConfig`   → `GraphQLRouteConfig`
/// - `GraphQLRouteConfig`   → `GraphQLRouteConfig`  (idempotent)
/// - `HttpStatus`           → `HttpStatus`          (left alone — `Http` not in `CSHARP_INITIALISMS`)
pub fn csharp_type_name(name: &str) -> String {
    // First normalize 3+ letter acronyms to PascalCase (URI → Uri, XML → Xml, JSON → Json)
    let normalized = normalize_acronym_to_pascalcase(name);
    // Then apply the preserved initialism rules (GraphQL, ID, UUID)
    apply_initialisms(&normalized, CSHARP_INITIALISMS)
}

/// Convert a Rust name to a C-style prefixed snake_case identifier (e.g. `prefix_name`).
pub fn to_c_name(prefix: &str, name: &str) -> String {
    format!("{}_{}", prefix, name.to_snake_case())
}

/// Convert a Rust type name to class name convention for target language.
pub fn to_class_name(name: &str) -> String {
    name.to_pascal_case()
}

/// Convert to SCREAMING_SNAKE for constants.
pub fn to_constant_name(name: &str) -> String {
    name.to_shouty_snake_case()
}

/// Convert a PascalCase or mixed-case name to snake_case with correct acronym handling.
///
/// Use this instead of `heck::ToSnakeCase` when the input is a PascalCase Rust type or
/// enum variant name — `heck` inserts an underscore before every uppercase letter, which
/// incorrectly splits acronym-style names like `Rdfa` into `rd_fa`.
///
/// Rules:
/// - A run of consecutive uppercase letters is treated as a single acronym word.
/// - If the run is followed by a lowercase letter, the last uppercase char begins the
///   next word (e.g. `XMLHttp` → `xml_http`).
/// - A single uppercase letter followed by lowercase is a normal word start.
///
/// Examples:
/// - `MyType`         → `my_type`
/// - `Rdfa`           → `rdfa`
/// - `HTMLParser`     → `html_parser`
/// - `XMLHttpRequest` → `xml_http_request`
/// - `IOError`        → `io_error`
/// - `URLPath`        → `url_path`
/// - `JSONLD`         → `jsonld`
pub fn pascal_to_snake(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(n + 4);
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        if ch.is_ascii_uppercase() {
            let run_start = i;
            while i < n && chars[i].is_ascii_uppercase() {
                i += 1;
            }
            let run_end = i;
            let run_len = run_end - run_start;
            if run_len == 1 {
                if !out.is_empty() {
                    out.push('_');
                }
                out.extend(chars[run_start].to_lowercase());
            } else {
                let split = if i < n && chars[i].is_ascii_lowercase() {
                    run_len - 1
                } else {
                    run_len
                };
                if !out.is_empty() {
                    out.push('_');
                }
                for &c in chars.iter().skip(run_start).take(split) {
                    out.extend(c.to_lowercase());
                }
                if split < run_len {
                    out.push('_');
                    out.extend(chars[run_start + split].to_lowercase());
                }
            }
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// Convert a PascalCase name to SCREAMING_SNAKE_CASE with correct acronym handling.
///
/// Examples:
/// - `MyType`     → `MY_TYPE`
/// - `Rdfa`       → `RDFA`
/// - `HTMLParser` → `HTML_PARSER`
pub fn pascal_to_screaming_snake(name: &str) -> String {
    pascal_to_snake(name).to_ascii_uppercase()
}

#[cfg(test)]
mod tests;
