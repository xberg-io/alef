use crate::backends::kotlin::gen_bindings::shared::to_screaming_snake;
use crate::backends::kotlin::type_map::KotlinMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};
use std::collections::BTreeSet;

/// Get the Kotlin type name for a PrimitiveType.
pub(super) fn primitive_type_name(pt: &PrimitiveType) -> &'static str {
    match pt {
        PrimitiveType::Bool => "Boolean",
        PrimitiveType::I8 => "Byte",
        PrimitiveType::I16 => "Short",
        PrimitiveType::I32 => "Int",
        PrimitiveType::I64 => "Long",
        PrimitiveType::U8 => "Byte",
        PrimitiveType::U16 => "Short",
        PrimitiveType::U32 => "Int",
        PrimitiveType::U64 => "Long",
        PrimitiveType::F32 => "Float",
        PrimitiveType::F64 => "Double",
        PrimitiveType::Usize | PrimitiveType::Isize => "Long",
    }
}

/// Kotlin zero-value literal for a rendered type string.
pub(super) fn kotlin_zero_value(rendered: &str) -> &'static str {
    match rendered.trim_end_matches('?') {
        "Boolean" => "false",
        "Byte" | "Short" | "Int" => "0",
        "Long" => "0L",
        "Float" => "0.0f",
        "Double" => "0.0",
        "String" => "\"\"",
        _ => "null",
    }
}

/// Maximum line length ktfmt uses when deciding whether to collapse a data-class
/// primary constructor to a single line.
pub(super) const KTFMT_LINE_WIDTH: usize = 100;

/// Decide whether a data-class declaration should be emitted on a single line.
pub(super) fn fits_single_line(indent: &str, prefix: &str, field_strings: &[String], suffix: &str) -> bool {
    let fields_inline = field_strings.join(", ");
    let total = indent.len() + prefix.len() + 1 + fields_inline.len() + 1 + suffix.len();
    total <= KTFMT_LINE_WIDTH
}

pub(crate) fn kotlin_type_with_string_imports(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<String>) -> String {
    let inner = render_type_ref_with_string_imports(ty, imports);
    if optional { format!("{inner}?") } else { inner }
}

fn render_type_ref_with_string_imports(ty: &TypeRef, imports: &mut BTreeSet<String>) -> String {
    let mapper = KotlinMapper;
    match ty {
        TypeRef::Path => {
            imports.insert("import java.nio.file.Path".to_string());
            mapper.map_type(ty)
        }
        TypeRef::Duration => {
            imports.insert("import kotlin.time.Duration".to_string());
            mapper.map_type(ty)
        }
        TypeRef::Optional(inner) => format!("{}?", render_type_ref_with_string_imports(inner, imports)),
        TypeRef::Vec(inner) => {
            format!("List<{}>", render_type_ref_with_string_imports(inner, imports))
        }
        TypeRef::Map(k, v) => {
            format!(
                "Map<{}, {}>",
                render_type_ref_with_string_imports(k, imports),
                render_type_ref_with_string_imports(v, imports)
            )
        }
        _ => mapper.map_type(ty),
    }
}

/// Return the Kotlin-side default suffix for a data-class constructor field.
///
/// Emits the field's typed default whenever the extractor was able to resolve
/// one (`#[derive(Default)]` plus explicit `Default` impls), so each generated
/// `data class` constructor parameter behaves like the Rust source. Falls back
/// to type-driven defaults (` = null` for `Optional`, ` = emptyList()` for
/// `Vec`, ` = emptyMap()` for `Map`) when the IR has no typed default — most
/// commonly for fields gated under a feature flag the binding crate does not
/// enable, where the wire JSON omits the key entirely.
///
/// This matters because the Jackson Kotlin module insists on supplying a
/// value for every non-nullable constructor parameter when deserializing.
/// Rust serializers commonly skip empty collections (`skip_serializing_if`),
/// optional fields with default values, and feature-gated fields. Without a
/// Kotlin-side default the deserialization fails with
/// `MissingKotlinParameterException`.
pub(super) fn kotlin_field_default(
    ty: &TypeRef,
    optional: bool,
    typed_default: Option<&crate::core::ir::DefaultValue>,
    enum_defaults: &std::collections::HashMap<String, String>,
    default_constructible_types: &std::collections::HashSet<String>,
) -> String {
    if let Some(default) = typed_default {
        // For optional fields with DefaultValue::Empty, the natural Kotlin default
        // is null regardless of the inner type — Rust's Option<T>::default() is
        // None, and we shouldn't synthesise type-specific zero-values like "" or 0.
        if optional && matches!(default, crate::core::ir::DefaultValue::Empty) {
            return " = null".to_string();
        }
        if let Some(literal) = render_kotlin_default(ty, default, enum_defaults, default_constructible_types) {
            return format!(" = {literal}");
        }
    }
    if optional {
        return " = null".to_string();
    }
    match ty {
        TypeRef::Optional(_) => " = null".to_string(),
        TypeRef::Vec(_) => " = emptyList()".to_string(),
        TypeRef::Map(_, _) => " = emptyMap()".to_string(),
        _ => String::new(),
    }
}

/// Render a `DefaultValue` as a Kotlin expression. Returns `None` when no
/// rendering is possible (e.g. `Empty` on a scalar type — no Kotlin literal
/// for "default of T" beyond what `kotlin_field_default` can synthesise).
fn render_kotlin_default(
    ty: &TypeRef,
    default: &crate::core::ir::DefaultValue,
    enum_defaults: &std::collections::HashMap<String, String>,
    default_constructible_types: &std::collections::HashSet<String>,
) -> Option<String> {
    use crate::core::ir::DefaultValue;
    match default {
        DefaultValue::BoolLiteral(b) => Some(b.to_string()),
        DefaultValue::IntLiteral(n) => {
            use crate::core::ir::PrimitiveType;
            // Duration fields represent milliseconds in the IR and must be
            // wrapped with the Kotlin `.milliseconds` extension to match
            // the declared type `kotlin.time.Duration`.
            if matches!(ty, TypeRef::Duration) {
                Some(format!("{n}.milliseconds"))
            }
            // Long suffix when the Kotlin field type is Long; Byte/Short would
            // need a cast but the IR rarely produces those for top-level fields.
            else if matches!(ty, TypeRef::Primitive(p) if matches!(p,
                PrimitiveType::I64 | PrimitiveType::U64
                | PrimitiveType::Usize | PrimitiveType::Isize))
            {
                Some(format!("{n}L"))
            } else {
                Some(n.to_string())
            }
        }
        DefaultValue::FloatLiteral(f) => {
            use crate::core::ir::PrimitiveType;
            if matches!(ty, TypeRef::Primitive(PrimitiveType::F32)) {
                Some(format!("{f}f"))
            } else {
                Some(f.to_string())
            }
        }
        DefaultValue::StringLiteral(s) => {
            // The Kotlin type for `TypeRef::Char` resolves to `String` in
            // `KotlinMapper` (mirroring the JVM/Panama convention of
            // representing a `char` as a one-character `String`), so emit a
            // double-quoted Kotlin string literal regardless of the IR's
            // distinction between `Char` and `String`.
            Some(format!("\"{}\"", escape_kotlin_string(s)))
        }
        DefaultValue::EnumVariant(variant) => match ty {
            TypeRef::Named(name) => {
                if enum_defaults.contains_key(name.as_str()) {
                    // True enum — variant names are SCREAMING_SNAKE_CASE
                    Some(format!("{name}.{}", to_screaming_snake(variant)))
                } else {
                    // Sealed class from a tagged/untagged Rust enum — variant
                    // names are PascalCase as declared in Rust
                    Some(format!("{name}.{}", variant))
                }
            }
            _ => None,
        },
        DefaultValue::Empty => match ty {
            TypeRef::Vec(_) => Some("emptyList()".to_string()),
            TypeRef::Map(_, _) => Some("emptyMap()".to_string()),
            TypeRef::Optional(_) => Some("null".to_string()),
            TypeRef::String => Some("\"\"".to_string()),
            TypeRef::Primitive(p) => {
                use crate::core::ir::PrimitiveType;
                match p {
                    PrimitiveType::Bool => Some("false".to_string()),
                    PrimitiveType::F32 => Some("0.0f".to_string()),
                    PrimitiveType::F64 => Some("0.0".to_string()),
                    PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                        Some("0L".to_string())
                    }
                    _ => Some("0".to_string()),
                }
            }
            // For Named enum types, the source Rust enum's
            // `#[derive(Default)]` picks a `#[default]` variant; bubble it up
            // via the supplied lookup so e.g. `heading_style: HeadingStyle`
            // emits ` = HeadingStyle.ATX`.
            //
            // For Named non-enum types (i.e. data class structs): don't try to
            // synthesize a constructor call because we don't know if all fields
            // have defaults. Instead, fall through to the field-level logic in
            // kotlin_field_default which will use `null` for optional fields or
            // omit the default for required ones (allowing Jackson's Kotlin module
            // to apply its own defaults via `#[serde(default)]` on the wire).
            TypeRef::Named(name) => {
                if let Some(variant) = enum_defaults.get(name.as_str()) {
                    // Enum with a declared `#[default]` variant.
                    let value = variant.as_str();
                    if value.is_empty() {
                        // Sentinel for "enum without a `#[default]` variant".
                        // No Kotlin literal can synthesise the value; fall
                        // through to the type-based default path so optional
                        // fields become `null` and required ones get no
                        // default.
                        None
                    } else {
                        Some(format!("{name}.{}", to_screaming_snake(value)))
                    }
                } else if default_constructible_types.contains(name.as_str()) {
                    // Non-enum data class whose Rust source has `Default` impl
                    // (has_default = true) and all of whose Kotlin fields also
                    // get constructor defaults. `Name()` invokes the no-arg
                    // synthesized constructor — equivalent to the Rust
                    // `Default::default()` semantics that the IR captures via
                    // `DefaultValue::Empty` here. Without this, Jackson's
                    // Kotlin module raises MissingKotlinParameterException
                    // when the wire JSON omits a non-nullable struct field
                    // (common for partial-update payloads in test fixtures).
                    Some(format!("{name}()"))
                } else {
                    // Non-enum Named types we can't safely default-construct:
                    // sealed classes from tagged/untagged Rust enums (protected
                    // constructor) or data classes whose fields don't all have
                    // defaults. Fall through to the field-level logic: `null`
                    // for optional fields, no default for required ones.
                    None
                }
            }
            _ => None,
        },
        DefaultValue::None => Some("null".to_string()),
    }
}

pub(super) fn escape_kotlin_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Type-string rendering (&'static str imports variant — used internally for enums)
// ---------------------------------------------------------------------------

/// Like the basic kotlin_type helper but fully-qualifies `Named` type references whose
/// simple name clashes with a sibling variant name in the enclosing sealed
/// class.  This prevents the Kotlin compiler from resolving the type to the
/// nested variant class instead of the outer same-named top-level class (Bug E).
pub(super) fn kotlin_type_disambiguated(
    ty: &TypeRef,
    optional: bool,
    variant_names: &std::collections::HashSet<&str>,
    package: &str,
) -> String {
    let inner = render_type_ref_disambiguated(ty, variant_names, package);
    if optional { format!("{inner}?") } else { inner }
}

fn render_type_ref_disambiguated(
    ty: &TypeRef,
    variant_names: &std::collections::HashSet<&str>,
    package: &str,
) -> String {
    // Built-in Kotlin collection types share simple names (`List`, `Map`, `Set`)
    // with potential sealed-class variants. Inside the sealed body those simple
    // names resolve to the nested variant data class, shadowing the stdlib type.
    // When a generic collection's simple name clashes with a sibling variant the
    // renderer must fully-qualify the stdlib path.
    let list_name = if variant_names.contains("List") {
        "kotlin.collections.List"
    } else {
        "List"
    };
    let map_name = if variant_names.contains("Map") {
        "kotlin.collections.Map"
    } else {
        "Map"
    };
    match ty {
        TypeRef::Named(n) if !package.is_empty() && variant_names.contains(n.as_str()) => {
            format!("{package}.{n}")
        }
        TypeRef::Optional(inner) => {
            format!("{}?", render_type_ref_disambiguated(inner, variant_names, package))
        }
        TypeRef::Vec(inner) => {
            format!(
                "{list_name}<{}>",
                render_type_ref_disambiguated(inner, variant_names, package),
            )
        }
        TypeRef::Map(k, v) => {
            format!(
                "{map_name}<{}, {}>",
                render_type_ref_disambiguated(k, variant_names, package),
                render_type_ref_disambiguated(v, variant_names, package),
            )
        }
        _ => {
            // No clash or non-Named type — fall back to the standard renderer.
            render_type_ref_with_imports(ty, &mut BTreeSet::new())
        }
    }
}

fn render_type_ref_with_imports(ty: &TypeRef, imports: &mut BTreeSet<&'static str>) -> String {
    let mapper = KotlinMapper;
    match ty {
        TypeRef::Path => {
            imports.insert("import java.nio.file.Path");
            mapper.map_type(ty)
        }
        TypeRef::Duration => {
            imports.insert("import kotlin.time.Duration");
            mapper.map_type(ty)
        }
        TypeRef::Optional(inner) => format!("{}?", render_type_ref_with_imports(inner, imports)),
        TypeRef::Vec(inner) => {
            format!("List<{}>", render_type_ref_with_imports(inner, imports))
        }
        TypeRef::Map(k, v) => {
            format!(
                "Map<{}, {}>",
                render_type_ref_with_imports(k, imports),
                render_type_ref_with_imports(v, imports)
            )
        }
        _ => mapper.map_type(ty),
    }
}

// ---------------------------------------------------------------------------
