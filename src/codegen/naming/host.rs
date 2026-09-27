//! The public host-language identifier surface: the names a consumer of a generated binding
//! actually types.
//!
//! [`public_host_identifier`] is the single entry point for every codegen backend, and the
//! per-kind helpers below stay private to the `naming` module so no backend can reach past its
//! escaping step. [`public_casing`] is the one exception: it is `pub(crate)`, not private,
//! because `crate::docs::naming` needs this module's casing decision without its escaping
//! decision (see that function's doc comment for why) -- it is still unreachable from outside
//! this crate, and still not a route any codegen backend should take instead of
//! [`public_host_identifier`]. ~keep

use super::case::{pascal_to_pascal, pascal_to_screaming_snake, pascal_to_snake};
use super::identifiers::escape_identifier_for;
use super::languages::{
    csharp_type_name, csharp_variant_name, go_param_name, go_type_name, go_variant_name, to_csharp_name, to_go_name,
};
use super::surfaces::{IdentifierContext, PublicIdentifierKind};
use crate::core::config::Language;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};

/// Resolve a public field/property identifier, applying `rename_fields` before language casing.
pub fn public_field_name(lang: Language, rust_field_name: &str, rename_fields_value: Option<&str>) -> String {
    let base = rename_fields_value.unwrap_or(rust_field_name);
    public_host_identifier(lang, PublicIdentifierKind::Field, base)
}
/// Resolve a public host-language identifier for a Rust name.
pub fn public_host_identifier(lang: Language, kind: PublicIdentifierKind, rust_name: &str) -> String {
    let converted = public_casing(lang, kind, rust_name);
    escape_identifier_for(lang, &converted, public_identifier_context(kind))
}

/// Resolve the per-language *casing* of a public host identifier, without the escaping step
/// [`public_host_identifier`] applies on top.
///
/// This is the single per-language idiom decision every emitter in this crate must share --
/// which case convention a member, type, enum variant or parameter takes, and which
/// initialisms are spelled out in each. It is the authority [`crate::docs::naming`] consumes
/// for exactly the same reason every codegen backend does: the alternative is two
/// independently-hand-maintained copies of this table silently disagreeing about whether Go
/// wants `BaseUrl` or `BaseURL`, or whether a Java enum constant is `Inline` or `INLINE`.
///
/// Exposed casing-only (not through [`public_host_identifier`]) because a caller that must
/// apply its own position- or language-specific override *before* any keyword escaping --
/// `crate::docs::naming::func_name`'s Java `new` -> `create` rename runs on the bare cased
/// name, not on an already `new_`-escaped one -- needs the undecorated result. Every other
/// caller should prefer [`public_host_identifier`], which is the one that also makes the
/// result a legal identifier. ~keep
pub(crate) fn public_casing(lang: Language, kind: PublicIdentifierKind, rust_name: &str) -> String {
    match kind {
        PublicIdentifierKind::Type => public_type_name(lang, rust_name),
        PublicIdentifierKind::EnumVariant => public_enum_variant_name(lang, rust_name),
        PublicIdentifierKind::Function | PublicIdentifierKind::Method | PublicIdentifierKind::Field => {
            public_member_name(lang, rust_name)
        }
        PublicIdentifierKind::Parameter => public_parameter_name(lang, rust_name),
    }
}

/// Qualify a type name with a dotted package/namespace, leaving an already-qualified name alone.
///
/// JVM-family and .NET targets accept a type spelled either bare (`SampleClient`) or fully
/// qualified (`dev.sample.bindings.SampleClient`), and one configured value — an
/// `[e2e.call.overrides.<lang>] class`, an `options_type`, a bridge class — reaches several
/// emitters. Prefixing unconditionally turns the qualified spelling into
/// `dev.sample.bindings.dev.sample.bindings.SampleClient`, which does not resolve; never
/// prefixing leaves a bare name unresolvable from a child package. The "does it already carry a
/// package" decision has to be made in exactly one place, or two emitters reading the same config
/// value disagree and the generated file carries both spellings. ~keep
pub fn qualified_type_path(package: &str, type_name: &str) -> String {
    if package.is_empty() || type_name.contains('.') {
        return type_name.to_string();
    }
    format!("{package}.{type_name}")
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

pub(super) fn public_type_name(lang: Language, name: &str) -> String {
    match lang {
        // ~keep Go and C# type names are already PascalCase Rust IR names at every real call
        // site (`go_type_name(&typ.name)` / `csharp_type_name(&typ.name)` in the backends
        // themselves) -- an extra `heck::to_pascal_case()` pre-step re-segments an irregular
        // acronym run (`RDFaChunk` -> `RdFaChunk`) that `go_type_name`/`csharp_type_name` would
        // otherwise leave alone. See issue #449: KotlinAndroid is the one caller that legitimately
        // passes a non-PascalCase (crate) name through this function, which is why the pre-step
        // stays for every other arm below.
        Language::Go => go_type_name(name),
        Language::Csharp => csharp_type_name(name),
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
        // ~keep Java enum constants are idiomatically SCREAMING_SNAKE (`INLINE`), the same
        // convention Kotlin's own generator already applies to its enum entries -- this used to
        // sit in the `to_pascal_case` arm below, which is why every simple Java enum this crate
        // generates currently declares `Inline("inline")` instead of `INLINE("inline")`. See
        // `crate::backends::java::gen_bindings::types::enums::gen_enum_class`, which now routes
        // its constant declarations through this same authority.
        Language::Python | Language::Ffi | Language::C | Language::Java => pascal_to_screaming_snake(name),
        Language::Ruby | Language::Elixir | Language::R | Language::Zig => pascal_to_snake(name),
        Language::Go => go_variant_name(name),
        Language::Csharp => csharp_variant_name(name),
        // ~keep Gleam custom-type constructors are PascalCase (`Circle`, `Square`), unlike
        // Gleam's snake_case fields/functions in `public_member_name` below -- this used to sit
        // in the snake_case arm above, which would have emitted a lowercase constructor Gleam's
        // own parser rejects.
        // ~keep Rust's own enum variants are PascalCase (`HeadingStyle::Atx`). This arm used to
        // include Rust, which was invisible for as long as `docs::naming` kept a second, correct
        // table of its own; unifying the two onto this authority surfaced it as
        // `HeadingStyle::ATX` in generated docs. A Rust variant is never shouty-snake.
        Language::Rust
        | Language::Node
        | Language::Php
        | Language::Wasm
        | Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Jni => pascal_to_pascal(name),
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
