//! Per-language spellings of a public host identifier, and the language-specific initialism
//! policy each one applies.
//!
//! The mechanical transforms these compose live in [`super::case`]; what belongs here is the
//! per-language *choice* — Go uppercasing `URL`, C# preferring `Json`, and so on. ~keep

use super::case::{apply_initialisms, normalize_acronym_to_pascalcase};
use super::host::cased_pascal_type_name;
use crate::core::config::Language;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::HashSet;

/// Convert a Rust snake_case name to the target language convention.
pub fn to_python_name(name: &str) -> String {
    name.to_snake_case()
}

/// Convert a Rust snake_case name to Node.js/TypeScript lowerCamelCase convention.
pub fn to_node_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Public TypeScript type name for a NAPI-RS binding's `.d.ts`, for both a type's own
/// declaration (`export interface Foo`) and every reference to it elsewhere in the file
/// (a field type, a param type, a return type).
///
/// The compiled Rust side wraps `Foo` as a `Js`-prefixed struct (`JsFoo`) and remaps it back to
/// `Foo` at the JS boundary via `#[napi(js_name = "Foo")]`, so the `.d.ts` — which describes the
/// JS boundary, not the Rust struct — must use the identity name everywhere. Both the emitter's
/// declaration site and its reference site (`TypeRef::Named`) call this one function so they
/// cannot independently decide whether to keep the `Js` prefix. ~keep
pub fn node_type_name(name: &str) -> &str {
    name
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
const CSHARP_INITIALISMS: &[&str] = &["GraphQL", "UUID"];

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

/// Convert a Rust PascalCase enum variant name to Go PascalCase convention, with correct
/// acronym-style segmentation.
///
/// This is deliberately a separate function from [`to_go_name`] rather than a shared helper:
/// `to_go_name` is also used for functions, methods and fields whose inputs are snake_case and
/// go through `heck`'s `to_pascal_case` correctly. An enum variant's input is already PascalCase
/// Rust, where `heck::ToPascalCase` re-segments acronym runs incorrectly (`RDFa` → `RdFa`);
/// [`super::case::pascal_to_pascal`] segments it the way [`super::case::pascal_to_snake`] does
/// instead (`RDFa` → `Rdfa`). ~keep
pub fn go_variant_name(name: &str) -> String {
    apply_go_acronyms(&super::case::pascal_to_pascal(name))
}

/// Convert a Rust free-function name to its Go wrapper identifier, disambiguating it from a
/// generated Go type of the same name.
///
/// A Rust crate can expose both a free function (e.g. `model_info`) and a struct (e.g.
/// `ModelInfo`) that map to the same Go PascalCase identifier, which the Go compiler rejects as
/// a redeclaration. Go struct/opaque/enum type names are never disambiguated (types are the
/// canonical identifier host consumers reach for), so on collision the function is renamed by
/// prefixing `Get`. `reserved_type_names` must contain every Go type identifier the backend will
/// emit (already passed through [`go_type_name`]).
pub fn go_free_function_name(func_name: &str, reserved_type_names: &HashSet<String>) -> String {
    let go_name = to_go_name(func_name);
    if reserved_type_names.contains(&go_name) {
        format!("Get{go_name}")
    } else {
        go_name
    }
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
    let first_lower = bytes.iter().position(|b| b.is_ascii_lowercase());
    match first_lower {
        None => pascal.to_lowercase(),
        Some(0) => pascal,
        Some(pos) => {
            let word_end = if pos > 1 { pos - 1 } else { 1 };
            let lower_prefix = pascal[..word_end].to_lowercase();
            format!("{}{}", lower_prefix, &pascal[word_end..])
        }
    }
}

/// Derive the Go package name from the last segment of a Go module path (fallback for when
/// `[go] package_name` is unset; prefer `ResolvedCrateConfig::go_package_name`).
pub fn go_package_name_from_module(module_path: &str) -> String {
    // ~keep `split` always yields at least one item, so a plain `next_back().unwrap_or(...)`
    // never reaches its fallback and an empty module path produced an empty (invalid) Go
    // package name. Filter empty segments so the fallback is reachable.
    let last = module_path.split('/').rfind(|segment| !segment.is_empty());
    match last {
        Some(segment) => segment.replace('-', "").to_lowercase(),
        None => "binding".to_string(),
    }
}

/// Derive the Go-exported error type name for a Rust error type: strips a leading
/// case-insensitive match of the Go package name to avoid revive's stutter lint, e.g.
/// `("SampleError", "sample")` -> `"Error"`. Every caller that names the Go error type
/// (`gen_go_error_struct`, the e2e/docs snippet generator) must go through this, not re-derive
/// the rule, so they can't drift from what the Go backend emits.
pub fn go_error_type_name(error_name: &str, pkg_name: &str) -> String {
    let type_lower = error_name.to_lowercase();
    let pkg_lower = pkg_name.to_lowercase();
    if type_lower.starts_with(&pkg_lower) && type_lower.len() > pkg_lower.len() {
        error_name[pkg_lower.len()..].to_string()
    } else {
        error_name.to_string()
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

/// Convert a Rust PascalCase enum variant name to C# PascalCase convention, with correct
/// acronym-style segmentation.
///
/// This is deliberately a separate function from [`to_csharp_name`] rather than a shared
/// helper: `to_csharp_name` is also used for methods and properties, whose inputs are
/// snake_case and go through `heck`'s `to_pascal_case` correctly. An enum variant's input is
/// already PascalCase Rust, where `heck::ToPascalCase` re-segments acronym runs incorrectly
/// (`RDFa` → `RdFa`); [`super::case::pascal_to_pascal`] segments it the way
/// [`super::case::pascal_to_snake`] does instead (`RDFa` → `Rdfa`). ~keep
pub fn csharp_variant_name(name: &str) -> String {
    apply_initialisms(&super::case::pascal_to_pascal(name), CSHARP_INITIALISMS)
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
///
/// `crate_name` is a crate name (kebab/snake), not an IR type name, so this goes through
/// [`cased_pascal_type_name`] rather than `public_type_name`/`PublicIdentifierKind::Type` --
/// issue #455 made the latter an identity function for `KotlinAndroid`, which would otherwise
/// turn this into a no-op and emit `object sample-parser-rs`. ~keep
pub fn kotlin_android_wrapper_object_name(crate_name: &str) -> String {
    let base = cased_pascal_type_name(Language::KotlinAndroid, crate_name);
    let stem = base.strip_suffix("Rs").unwrap_or(&base);
    stem.to_string()
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
    let normalized = normalize_acronym_to_pascalcase(name);
    apply_initialisms(&normalized, CSHARP_INITIALISMS)
}
