//! Renders the syntax a caller of a generated binding actually types to name one enum variant
//! (e.g. the literal a field's documented "Default" cell shows). Split out of `formatting.rs`
//! (file-modularization rule) rather than growing that already-grandfathered file.

use crate::core::config::Language;
use crate::core::keywords::swift_enum_case_ident;
use crate::docs::naming::{enum_variant_name, type_name};
use heck::ToLowerCamelCase;

/// Render the syntax a caller of the generated binding actually types to name one enum
/// variant -- e.g. the literal a field's documented "Default" cell shows. `enum_type_raw` and
/// `variant_raw` are the UNTRANSFORMED Rust names (as extracted from source); each arm below
/// asks the real per-language naming authority for the identifier the compiled binding
/// declares, rather than guessing a plausible-looking spelling, so this can never advertise
/// syntax the generated code does not accept.
///
/// Verified against what each backend's own generator emits (task #558 audit):
/// - Go concatenates the type and variant with NO separator and Go acronym-uppercases both
///   (`backends/go/gen_bindings/types/enums.rs::go_enum_constant_for_wire_value`,
///   `go_type_name` + `to_go_name` from `codegen::naming`), e.g. `ModePreferContent`, not
///   `Mode.PreferContent`.
/// - Java declares the enum constant as the RAW, untransformed variant identifier -- no
///   shouty-snake-case (`backends/java/gen_bindings/types/enums.rs::gen_enum_class`,
///   `simple_enum_class.jinja` pushes `variant.name` verbatim).
/// - Swift's `case` is lowerCamelCase (`backends/swift/gen_bindings/enums.rs`:
///   `swift_enum_case_ident(&variant.name.to_lower_camel_case())`, via the same
///   `core::keywords::swift_enum_case_ident` this function calls).
/// - Dart's member is lowerCamelCase (`backends/dart/gen_bindings/types.rs`:
///   `dart_safe_ident(&variant.name.to_lower_camel_case())`, a thin pass-through to
///   `codegen::naming::dart_value_identifier`, which this function calls directly).
/// - PHP's class constant uppercases the whole variant name with NO underscores inserted
///   (`backends/php/gen_bindings/types/enums.rs::enum_constant_entries`:
///   `variant.name.to_uppercase()`), e.g. `PREFERCONTENT`, not `PREFER_CONTENT`. That
///   function also escapes a handful of PHP-reserved constant names (`CLASS`, `DEFAULT`,
///   ...); this doc renderer does not reproduce that narrow collision case.
///
/// Every other language keeps the docs layer's own generic per-language transform
/// (`enum_variant_name`), which was independently confirmed correct for those languages. ~keep
pub(crate) fn format_enum_variant_ref(
    enum_type_raw: &str,
    variant_raw: &str,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    match lang {
        Language::Go => format!(
            "{}{}",
            crate::codegen::naming::go_type_name(enum_type_raw),
            crate::codegen::naming::to_go_name(variant_raw)
        ),
        // ~keep Java's constant is cased by the same authority the generator uses
        // (`backends::java::gen_bindings::types::enums::gen_enum_class`), NOT rendered raw. This
        // line used to interpolate `variant_raw`, which was correct only for as long as the
        // generator itself pushed the Rust variant name verbatim; once that was fixed to
        // SCREAMING_SNAKE these docs would have documented a constant that does not exist.
        Language::Java => format!(
            "{}.{}",
            type_name(enum_type_raw, lang, ffi_prefix),
            enum_variant_name(variant_raw, lang, ffi_prefix)
        ),
        Language::Swift => format!(
            "{}.{}",
            type_name(enum_type_raw, lang, ffi_prefix),
            swift_enum_case_ident(&variant_raw.to_lower_camel_case())
        ),
        Language::Dart => format!(
            "{}.{}",
            type_name(enum_type_raw, lang, ffi_prefix),
            crate::codegen::naming::dart_value_identifier(&variant_raw.to_lower_camel_case())
        ),
        Language::Php => format!(
            "{}::{}",
            type_name(enum_type_raw, lang, ffi_prefix),
            variant_raw.to_uppercase()
        ),
        Language::Ruby | Language::Elixir => format!(":{}", enum_variant_name(variant_raw, lang, ffi_prefix)),
        Language::R => format!("\"{}\"", enum_variant_name(variant_raw, lang, ffi_prefix)),
        Language::Rust => format!(
            "{}::{}",
            type_name(enum_type_raw, lang, ffi_prefix),
            enum_variant_name(variant_raw, lang, ffi_prefix)
        ),
        Language::Ffi | Language::C | Language::Jni => enum_variant_name(variant_raw, lang, ffi_prefix),
        Language::Python
        | Language::Node
        | Language::Wasm
        | Language::Csharp
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Gleam
        | Language::Zig => format!(
            "{}.{}",
            type_name(enum_type_raw, lang, ffi_prefix),
            enum_variant_name(variant_raw, lang, ffi_prefix)
        ),
    }
}
