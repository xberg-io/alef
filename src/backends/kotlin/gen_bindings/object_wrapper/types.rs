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
        TypeRef::Path => mapper.map_type(ty),
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
pub(crate) fn kotlin_field_default(
    ty: &TypeRef,
    optional: bool,
    typed_default: Option<&crate::core::ir::DefaultValue>,
    enum_defaults: &std::collections::HashMap<String, String>,
    default_constructible_types: &std::collections::HashSet<String>,
) -> String {
    if let Some(default) = typed_default {
        if optional && matches!(default, crate::core::ir::DefaultValue::Empty) {
            return " = null".to_string();
        }
        if let Some(literal) = render_kotlin_default(ty, default, enum_defaults, default_constructible_types) {
            return format!(" = {literal}");
        }
        // A `#[serde(default = "path")]` field (`FunctionCall`/`PublicFunctionCall`), a manual
        // `impl Default` body alef could not constant-fold (`Unresolved`), or a resolved
        // tuple/struct-variant enum default this renderer has no per-argument expression for
        // (`TupleVariant`/`StructVariant`) all state that a default exists and that this
        // renderer cannot spell its value. Falling through to the type-driven fallbacks below
        // would answer that with `null`/`emptyList()`/`emptyMap()`, which is a *claim* about the
        // Rust value — and a wrong one whenever the real value is a populated collection or a
        // `Some(..)`. Kotlin has no way to reach the Rust value from a data-class parameter
        // default, so the only honest rendering is no default at all: the parameter stays
        // required, which costs ergonomics and never disagrees with the source crate. Same rule
        // the non-finite `FloatLiteral` case follows in `render_kotlin_default`. ~keep
        if matches!(
            default,
            crate::core::ir::DefaultValue::FunctionCall(_)
                | crate::core::ir::DefaultValue::PublicFunctionCall(_)
                | crate::core::ir::DefaultValue::Unresolved(_)
                | crate::core::ir::DefaultValue::TupleVariant(..)
                | crate::core::ir::DefaultValue::StructVariant(..)
        ) {
            return String::new();
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

/// Names of types whose emitted Kotlin data class can be constructed with no arguments.
///
/// A Rust `Default` implementation is necessary but not sufficient. `emit_type_with_imports`
/// gives a constructor parameter a Kotlin default only where [`kotlin_field_default`] can render
/// one, so a `Default`-deriving type can still emit a bare `val count: Int`. Treating such a type
/// as default-constructible makes [`render_kotlin_default`] emit `Name()`, which does not compile.
///
/// Computed as a greatest fixpoint: seed with every `Default`-bearing type, then repeatedly drop
/// any whose emitted constructor still has a bare parameter. Iteration is required because
/// [`kotlin_field_default`] consults the set while it is being built, so dropping one type can
/// invalidate another that defaulted a field to `Dropped()`. The set only ever shrinks, so this
/// terminates.
pub(crate) fn default_constructible_type_names(
    types: &[crate::core::ir::TypeDef],
    enum_defaults: &std::collections::HashMap<String, String>,
) -> std::collections::HashSet<String> {
    let mut constructible: std::collections::HashSet<String> = types
        .iter()
        .filter(|ty| !ty.is_trait && !ty.is_opaque && ty.has_default)
        .map(|ty| ty.name.clone())
        .collect();

    loop {
        let dropped: Vec<String> = types
            .iter()
            .filter(|ty| constructible.contains(&ty.name))
            .filter(|ty| !every_field_has_a_kotlin_default(ty, enum_defaults, &constructible))
            .map(|ty| ty.name.clone())
            .collect();
        if dropped.is_empty() {
            return constructible;
        }
        for name in dropped {
            constructible.remove(&name);
        }
    }
}

/// Whether every constructor parameter the data class emits carries a Kotlin default.
///
/// Mirrors the field walk in `emit_type_with_imports`: `binding_excluded` fields are dropped
/// entirely, and `#[serde(flatten)]` fields are always emitted nullable with `= null`.
fn every_field_has_a_kotlin_default(
    ty: &crate::core::ir::TypeDef,
    enum_defaults: &std::collections::HashMap<String, String>,
    default_constructible_types: &std::collections::HashSet<String>,
) -> bool {
    ty.fields.iter().filter(|field| !field.binding_excluded).all(|field| {
        field.serde_flatten
            || !kotlin_field_default(
                &field.ty,
                field.optional,
                field.typed_default.as_ref(),
                enum_defaults,
                default_constructible_types,
            )
            .is_empty()
    })
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
            if matches!(ty, TypeRef::Duration) {
                Some(format!("{n}.milliseconds"))
            } else if matches!(ty, TypeRef::Primitive(p) if matches!(p,
                PrimitiveType::I64 | PrimitiveType::U64
                | PrimitiveType::Usize | PrimitiveType::Isize))
            {
                Some(format!("{n}L"))
            } else {
                Some(n.to_string())
            }
        }
        // `{f}` on a whole-valued f64 prints `1`, and `val ratio: Double = 1` is an *integer*
        // literal Kotlin refuses to widen; NaN and the infinities print as `NaN`/`inf`, which
        // name nothing. Both are handled once in `float_literal_digits`. ~keep
        DefaultValue::FloatLiteral(f) => {
            use crate::core::ir::PrimitiveType;
            let digits = crate::codegen::shared::float_literal_digits(*f)?;
            if matches!(ty, TypeRef::Primitive(PrimitiveType::F32)) {
                Some(format!("{digits}f"))
            } else {
                Some(digits)
            }
        }
        DefaultValue::StringLiteral(s) => Some(format!("\"{}\"", escape_kotlin_string(s))),
        // Every element must render or the whole collection falls back to the type-based
        // default, the same all-or-nothing rule the extractor applies when lowering. ~keep
        DefaultValue::ListLiteral(items) => {
            let element_ty = match ty {
                TypeRef::Vec(inner) => inner.as_ref(),
                other => other,
            };
            items
                .iter()
                .map(|item| render_kotlin_default(element_ty, item, enum_defaults, default_constructible_types))
                .collect::<Option<Vec<String>>>()
                .map(|values| format!("listOf({})", values.join(", ")))
        }
        DefaultValue::EnumVariant(variant) => match ty {
            TypeRef::Named(name) => {
                if enum_defaults.contains_key(name.as_str()) {
                    Some(format!("{name}.{}", to_screaming_snake(variant)))
                } else {
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
            // `#[derive(Default)]` picks a `#[default]` variant; bubble it up
            // to apply its own defaults via `#[serde(default)]` on the wire).
            TypeRef::Named(name) => {
                if let Some(variant) = enum_defaults.get(name.as_str()) {
                    // Enum with a declared `#[default]` variant.
                    let value = variant.as_str();
                    if value.is_empty() {
                        // Sentinel for "enum without a `#[default]` variant".
                        None
                    } else {
                        Some(format!("{name}.{}", to_screaming_snake(value)))
                    }
                } else if default_constructible_types.contains(name.as_str()) {
                    Some(format!("{name}()"))
                } else {
                    None
                }
            }
            _ => None,
        },
        DefaultValue::None => Some("null".to_string()),
        // Alef read the `Default` impl but could not constant-fold its body: the value is
        // genuinely unknown, not the type's zero. Rendering nothing here — rather than reusing
        // the `Empty` arm above — is what lets `kotlin_field_default` tell the two apart and
        // leave the parameter required instead of guessing `emptyList()`/`0`/`Name()`. ~keep
        DefaultValue::Unresolved(_) => None,
        // Resolved (alef read the value), but there is no Kotlin expression for "construct enum
        // variant X with these field values" the way `EnumVariant` above has one. Same "no
        // default" answer as `Unresolved`, for the same reason: guessing would risk a value the
        // real default does not hold. ~keep
        DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..) => None,
        DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => None,
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
        _ => render_type_ref_with_imports(ty, &mut BTreeSet::new()),
    }
}

fn render_type_ref_with_imports(ty: &TypeRef, imports: &mut BTreeSet<&'static str>) -> String {
    let mapper = KotlinMapper;
    match ty {
        TypeRef::Path => mapper.map_type(ty),
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
