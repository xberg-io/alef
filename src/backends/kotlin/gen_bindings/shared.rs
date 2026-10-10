//! Shared string-conversion utilities for Kotlin code generation.
//!
//! Used by both the JVM and Native backends as well as the MPP backend.

use crate::codegen::naming::cased_public_type_name;
use crate::core::config::Language;

/// Everything a data-class emitter needs to bridge value-type instance methods
/// through JNI.
///
/// Present only for the JNI targets (Kotlin/Android). When absent, a data class
/// omits its instance methods entirely rather than shipping a body that throws
/// at runtime — the same choice the Java backend makes.
#[derive(Clone, Copy)]
pub struct ValueMethodBridge<'a> {
    /// Simple name of the generated `object` holding the `external fun`s.
    pub bridge_class: &'a str,
    /// Types that can cross the boundary as JSON; see
    /// [`crate::core::jni::value_bridge_serde_type_names`].
    pub serde_type_names: &'a std::collections::HashSet<&'a str>,
}

/// Convert a `snake_case` or `kebab-case` name to `PascalCase`.
///
/// Goes through `cased_public_type_name`, not `public_host_identifier(_,
/// PublicIdentifierKind::Type, _)`: this helper is reused across the Kotlin backend for crate
/// names, function/method names, and IR type-name fragments used in internal JNI symbol
/// synthesis (`nativeFree{TypeName}` and similar) — none of which are the "already PascalCase,
/// do not re-case" input contract issue #455 gave to `public_type_name`. See that function's doc
/// comment in `src/codegen/naming/host.rs`. ~keep
pub fn kotlin_pascal_case(name: &str) -> String {
    cased_public_type_name(Language::Kotlin, name)
}

pub use kotlin_pascal_case as to_pascal_case;

/// Convert a `snake_case` or `kebab-case` name to `lowerCamelCase` and escape Kotlin
/// keywords.
///
/// This is *the* sanitizer for every Kotlin member identifier the backend emits —
/// property names, `fun` names, and parameter names alike. Escaping lives here rather
/// than at the call sites because there are ~60 of them across the JVM, Native, MPP and
/// Android emitters, and a keyword escape applied at only some of them is
/// indistinguishable from no escape at all.
///
/// Use [`to_lower_camel_unescaped`] when the result is a *fragment* of a larger
/// identifier rather than an identifier in its own right — backticks may not appear
/// mid-identifier. ~keep
pub fn to_lower_camel(name: &str) -> String {
    escape_kotlin_ident(&to_lower_camel_unescaped(name))
}

/// `lowerCamelCase` conversion without keyword escaping.
///
/// Only correct where the result is concatenated into a longer identifier (a generated
/// test-function name, for example); a bare emitted identifier must use
/// [`to_lower_camel`]. ~keep
pub fn to_lower_camel_unescaped(name: &str) -> String {
    let pascal = kotlin_pascal_case(name);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Convert a `PascalCase` name to `SCREAMING_SNAKE_CASE`, with correct acronym-style
/// segmentation (e.g. `RDFa` -> `RDFA`, `HTMLParser` -> `HTML_PARSER`, not a `_` before every
/// uppercase letter).
pub fn to_screaming_snake(name: &str) -> String {
    crate::codegen::naming::pascal_to_screaming_snake(name)
}

/// Kotlin reserved keywords that must be backtick-escaped when used as
/// identifiers. Hard keywords cannot appear bare in any position; emitting
/// e.g. `val object: String` is a parse error. Wrapping in backticks
/// (`val \`object\`: String`) keeps the wire name intact while satisfying
/// the Kotlin grammar.
///
/// Deliberately *not* Kotlin's soft/modifier keywords (`get`, `set`, `value`, `field`,
/// `by`, `where`, `init`, `constructor`, ...). Those are listed in the grammar's own
/// `simpleIdentifier` production, so Kotlin accepts them bare as property, function and
/// parameter names; escaping them would churn every consumer that has a field named
/// `value` for no grammatical gain. `crate::docs::formatting::reserved_words` keeps a
/// wider, deliberately conservative superset for the docs identifier gate — the two
/// lists answer different questions and are expected to differ. ~keep
const KOTLIN_HARD_KEYWORDS: &[&str] = &[
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
];

/// Backtick-escape `name` when it collides with a Kotlin hard keyword, leaving the
/// spelling otherwise untouched.
///
/// Backticks rather than a trailing underscore: they are pure syntax, so the JVM member
/// name, the Jackson property name derived from it, and the JNI symbol built from the
/// same Rust name all still read `object`. A rename to `object_` would silently move the
/// JSON key for every field that lacks an explicit `#[serde(rename)]`, since
/// `emit_data_class` only emits `@JsonProperty` when the Rust field carries one. ~keep
pub fn escape_kotlin_ident(name: &str) -> String {
    if KOTLIN_HARD_KEYWORDS.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

/// Field-name resolution for Kotlin record-style data class params. IR
/// positional fields use names like `_0`, `_1` which lowerCamelCase to `0`/`1`
/// — invalid Kotlin identifiers. Map them to `field0`, `field1`, ...
/// Names that collide with Kotlin hard keywords are backtick-escaped so they
/// remain wire-compatible without breaking the grammar.
pub fn kotlin_field_name(raw: &str, idx: usize) -> String {
    let stripped = raw.trim_start_matches('_');
    if stripped.is_empty() || stripped.chars().all(|c| c.is_ascii_digit()) {
        return format!("field{idx}");
    }
    to_lower_camel(raw)
}

/// Derive a payload-informed field name for sealed-class tuple variants.
///
/// For tuple variants with a single payload, this function derives smarter names:
/// - If the field name is positional (like `_0`), infer from the type:
///   - Named type `Pdf Metadata` with variant name `Pdf` → strip prefix "Pdf" → `metadata`
///   - Primitive type (String, Int, etc.) → use generic `value`
/// - If the field name is a struct field name (like `reason`), use it directly.
/// - For multiple tuple fields, use generic names: `value0`, `value1`, etc.
///
/// # Arguments
///
/// * `field_name` - Raw field name from IR (`_0`, `_1`, or named like `reason`)
/// * `field_idx` - Position in the variant's field list
/// * `field_type_name` - The simple type name (e.g., `PdfMetadata` from `TypeRef::Named`)
/// * `variant_name` - The variant name (e.g., `Pdf`)
/// * `total_fields` - Total number of fields in the variant
pub fn kotlin_field_name_with_type(
    field_name: &str,
    field_idx: usize,
    field_type_name: Option<&str>,
    variant_name: &str,
    total_fields: usize,
) -> String {
    let stripped = field_name.trim_start_matches('_');

    if !stripped.is_empty() && !stripped.chars().all(|c| c.is_ascii_digit()) {
        return to_lower_camel(field_name);
    }

    if total_fields == 1
        && let Some(type_name) = field_type_name
    {
        if let Some(remainder) = type_name.strip_prefix(variant_name) {
            let derived = to_lower_camel(remainder);
            if !derived.is_empty() {
                return derived;
            }
        }

        if is_primitive_or_stdlib_type(type_name) {
            return "value".to_string();
        }
    }

    if total_fields > 1 {
        return format!("value{}", field_idx);
    }

    "value".to_string()
}

/// Check if a type name is a primitive or stdlib type (String, Int, Long, etc.).
fn is_primitive_or_stdlib_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "Byte"
            | "Short"
            | "Int"
            | "Long"
            | "Float"
            | "Double"
            | "Boolean"
            | "Unit"
            | "Char"
            | "Any"
            | "Nothing"
    )
}

/// The `.registerModule(...)` chain link that teaches a Jackson mapper the wire shape of
/// `kotlin.time.Duration`: a plain millisecond integer, the same shape the Rust side's
/// `duration_ms` serde adapters read and write, and the same shape the generated e2e suites'
/// test-side mapper already used (`e2e::codegen::kotlin::test_file`).
///
/// ~keep Without this, Jackson serializes a `kotlin.time.Duration` (an inline class over a
/// `Long`) as its raw bit pattern — nanoseconds shifted left by the unit bit — so a
/// `100.milliseconds` `extra_wait` crossed the JNI boundary as `200000000`, which Rust read as
/// ~2.3 days of milliseconds and a consumer's kotlin_android e2e suite hung until the CI job
/// timeout. Every mapper that marshals a generated DTO to or from the native side must carry
/// this link; `indent` is the column the chain's `.registerModule(` lines sit at.
pub fn duration_millis_jackson_module(indent: usize) -> String {
    let pad = " ".repeat(indent);
    let lines = [
        ".registerModule(",
        "    com.fasterxml.jackson.databind.module.SimpleModule()",
        "        .addSerializer(",
        "            kotlin.time.Duration::class.java,",
        "            object : com.fasterxml.jackson.databind.JsonSerializer<kotlin.time.Duration>() {",
        "                override fun serialize(",
        "                    value: kotlin.time.Duration,",
        "                    gen: com.fasterxml.jackson.core.JsonGenerator,",
        "                    serializers: com.fasterxml.jackson.databind.SerializerProvider,",
        "                ) {",
        "                    gen.writeNumber(value.inWholeMilliseconds)",
        "                }",
        "            },",
        "        )",
        "        .addDeserializer(",
        "            kotlin.time.Duration::class.java,",
        "            object : com.fasterxml.jackson.databind.JsonDeserializer<kotlin.time.Duration>() {",
        "                override fun deserialize(",
        "                    p: com.fasterxml.jackson.core.JsonParser,",
        "                    ctxt: com.fasterxml.jackson.databind.DeserializationContext,",
        "                ): kotlin.time.Duration = with(kotlin.time.Duration) { p.longValue.milliseconds }",
        "            },",
        "        ),",
        ")",
    ];
    lines
        .iter()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Assemble a complete `.kt` file from package, imports, and body with file-level suppression.
///
/// Emits the generated file header, file-level @file:Suppress annotation to silence
/// ktlint/detekt rules that generated code inherently violates, package declaration,
/// imports, and body content.
pub fn assemble_kt_file(package: &str, imports: &std::collections::BTreeSet<String>, body: &str) -> String {
    let mut content = String::new();
    content.push_str(crate::core::hash::SELF_MARKING_HEADER_LINE);
    content.push('\n');
    content.push_str(
        "@file:Suppress(\n    \
         \"ktlint:standard:trailing-comma-on-call-site\",\n    \
         \"ktlint:standard:trailing-comma-on-declaration-site\",\n    \
         \"ktlint:standard:spacing-between-declarations-with-comments\",\n    \
         \"ktlint:standard:spacing-between-declarations-with-annotations\",\n    \
         \"ktlint:standard:when-entry-bracing\",\n    \
         \"ktlint:standard:blank-line-between-when-conditions\",\n    \
         \"ktlint:standard:blank-line-before-declaration\",\n    \
         \"ktlint:standard:chain-method-continuation\",\n    \
         \"ktlint:standard:annotation\",\n    \
         \"ktlint:standard:max-line-length\",\n    \
         \"ktlint:standard:no-semi\",\n    \
         \"ktlint:standard:statement-wrapping\",\n    \
         \"MaxLineLength\",\n    \
         \"TooManyFunctions\",\n    \
         \"FunctionParameterNaming\",\n    \
         \"LongParameterList\",\n    \
         \"CyclomaticComplexMethod\",\n    \
         \"LongMethod\",\n    \
         \"MagicNumber\",\n    \
         \"ReturnCount\",\n    \
         \"NestedBlockDepth\",\n    \
         \"UnusedParameter\",\n\
         )\n\n",
    );
    content.push_str(&crate::backends::kotlin::template_env::render(
        "package_declaration.jinja",
        crate::alef_context! {
            package => package,
        },
    ));
    content.push('\n');
    for import in imports {
        content.push_str(import);
        content.push('\n');
    }
    if !imports.is_empty() {
        content.push('\n');
    }
    content.push_str(body);
    content
}
