//! `object {Crate}` namespace, bridge calls, and Kotlin type/enum/error code emission.
//!
//! Contains:
//! - `emit_function` — emits a JVM wrapper function that delegates to the Java `Bridge` alias
//! - `emit_type_with_imports` — emits a Kotlin `data class` or empty `class` for a type
//! - `emit_enum` — emits a Kotlin `enum class` or `sealed class` for an enum
//! - `emit_error_type_with_imports` — emits a `sealed class` hierarchy for an error type
//! - Kotlin type-string helpers (with import collection)

use crate::core::ir::{EnumDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::helpers::emit_cleaned_kdoc;
use super::shared::{kotlin_field_name, to_lower_camel, to_screaming_snake};
use crate::backends::kotlin::type_map::KotlinMapper;
use crate::codegen::naming::wire_variant_value;
use crate::codegen::type_mapper::TypeMapper;

// ---------------------------------------------------------------------------
// Helper for type name extraction
// ---------------------------------------------------------------------------

/// Get the Kotlin type name for a PrimitiveType.
fn primitive_type_name(pt: &PrimitiveType) -> &'static str {
    use crate::core::ir::PrimitiveType;
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

// ---------------------------------------------------------------------------
// Type/enum/error emitters (re-exported for gen_mpp)
// ---------------------------------------------------------------------------

/// Kotlin zero-value literal for a rendered type string (e.g. `"Short"` → `"0"`,
/// `"Boolean"` → `"false"`, `"String"` → `"\"\""`). Used to seed `open val`
/// defaults on the sealed error class so concrete variants compile without
/// every one declaring an explicit override.
fn kotlin_zero_value(rendered: &str) -> &'static str {
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
/// primary constructor to a single line. Any declaration that fits within this
/// budget is emitted as `data class Foo(val a: T, val b: T)`.
const KTFMT_LINE_WIDTH: usize = 100;

/// Decide whether a data-class declaration should be emitted on a single line.
///
/// ktfmt collapses to a single line when the entire declaration fits within
/// `KTFMT_LINE_WIDTH` characters. We match that heuristic so the emitter output
/// is stable across ktfmt runs without any post-processing step.
///
/// `indent` is the leading indentation (e.g. `""` for top-level, `"    "` for
/// nested). `prefix` is everything up to the opening `(` (e.g. `"data class Foo"`).
/// `field_strings` are the rendered `val name: Type` strings without commas.
/// `suffix` is everything after the closing `)` excluding the trailing newline.
fn fits_single_line(indent: &str, prefix: &str, field_strings: &[String], suffix: &str) -> bool {
    let fields_inline = field_strings.join(", ");
    let total = indent.len() + prefix.len() + 1 + fields_inline.len() + 1 + suffix.len();
    total <= KTFMT_LINE_WIDTH
}

pub(crate) fn emit_type_with_imports(
    ty: &TypeDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    enum_defaults: &std::collections::HashMap<String, String>,
    sealed_class_names: &std::collections::HashSet<String>,
    default_constructible_types: &std::collections::HashSet<String>,
) {
    emit_cleaned_kdoc(out, &ty.doc, "");
    if ty.fields.is_empty() {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "empty_class.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
        return;
    }

    // Pre-compute the per-field JsonSerialize annotation needed when the
    // declared field type references a sealed class.  Jackson dispatches
    // serializers by RUNTIME type, so a `data class Parent(val foo: Sealed)`
    // would look up the serializer for the concrete variant (e.g.
    // `Sealed.Variant`) — which carries `@JsonSerialize(using =
    // JsonSerializer.None::class)` to break the deserializer recursion
    // protection — and emit a default POJO instead of routing through the
    // parent's custom `SealedSerializer`.  `@field:JsonSerialize(\`as\` = ...)`
    // forces Jackson to use the DECLARED static type for serializer lookup,
    // which carries the custom serializer.  For collections we use
    // `contentAs` on the element type.
    //
    // The annotation must be attached to the underlying field (not the
    // constructor parameter) because Kotlin defaults annotations on primary
    // constructor parameters to the parameter use-site, but Jackson reads
    // field-level annotations.  Hence the `@field:` site target.
    let field_sealed_annotations: Vec<Option<String>> = ty
        .fields
        .iter()
        .map(|f| sealed_class_field_annotation(&f.ty, sealed_class_names))
        .collect();

    // Pre-build field strings so we can apply the ktfmt single-line heuristic
    // before committing to an emission style. Field-level KDoc or @JsonProperty
    // annotations force multi-line because they cannot be inlined inside a
    // constructor parameter list.
    let has_field_docs = ty.fields.iter().any(|f| !f.doc.is_empty());
    let has_field_annotations =
        ty.fields.iter().any(|f| f.serde_rename.is_some()) || field_sealed_annotations.iter().any(Option::is_some);
    // Detect `#[serde(flatten)]` fields. In Rust these collect all unknown
    // wire fields into a value (often `serde_json::Value` or `HashMap`); Kotlin
    // has no native equivalent. As a pragmatic mitigation, treat the flatten
    // field as a nullable bag (default null) AND emit
    // `@JsonIgnoreProperties(ignoreUnknown = true)` on the class so Jackson
    // tolerates the unknown sibling keys that Rust would have absorbed.
    // Note: this is lossy — the flatten contents aren't actually captured in
    // the Kotlin struct, but the deserialiser no longer fails outright.
    let has_flatten_field = ty.fields.iter().any(|f| f.serde_flatten);

    let mut field_strings: Vec<String> = Vec::with_capacity(ty.fields.len());
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = kotlin_type_with_string_imports(&field.ty, field.optional, imports);
        let name = kotlin_field_name(&field.name, idx);
        // Append a Kotlin default for fields whose underlying Rust type has a
        // natural empty value. Rust serializers commonly skip Default-valued
        // collections (`#[serde(skip_serializing_if = "...")]`) or skip a
        // field entirely under a feature gate (`#[serde(skip)]`). Without a
        // Kotlin-side default the Jackson Kotlin module fails the entire
        // deserialization with `MissingKotlinParameterException` whenever the
        // wire JSON omits the key — even if the Rust source carries `Default`.
        let (effective_ty_str, default_suffix) = if field.serde_flatten {
            // Force `T?` + default null for flatten fields (see has_flatten_field above).
            let nullable_ty = if ty_str.ends_with('?') {
                ty_str.clone()
            } else {
                format!("{ty_str}?")
            };
            (nullable_ty, " = null".to_string())
        } else {
            let default_suffix = kotlin_field_default(
                &field.ty,
                field.optional,
                field.typed_default.as_ref(),
                enum_defaults,
                default_constructible_types,
            );
            // A `Duration` default is rendered with the `.milliseconds` extension
            // property, which is not in scope without an explicit import.
            if default_suffix.contains(".milliseconds") {
                imports.insert("import kotlin.time.Duration.Companion.milliseconds".to_string());
            }
            (ty_str, default_suffix)
        };
        field_strings.push(format!("val {name}: {effective_ty_str}{default_suffix}"));
    }

    let prefix = format!("data class {}", ty.name);
    let use_single_line = !has_field_docs
        && !has_field_annotations
        && !has_flatten_field
        && fits_single_line("", &prefix, &field_strings, "");

    if has_flatten_field {
        out.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    }

    if use_single_line {
        out.push_str(&format!("{prefix}({})\n", field_strings.join(", ")));
    } else {
        out.push_str(&format!("{prefix}(\n"));
        for (idx, (field, field_str)) in ty.fields.iter().zip(field_strings.iter()).enumerate() {
            emit_cleaned_kdoc(out, &field.doc, "    ");
            // Emit @JsonProperty when the Rust field carries #[serde(rename = "...")]
            // so Jackson maps the wire key to the Kotlin camelCase property name.
            if let Some(rename) = &field.serde_rename {
                out.push_str(&format!(
                    "    @com.fasterxml.jackson.annotation.JsonProperty(\"{}\")\n",
                    escape_kotlin_string(rename)
                ));
            }
            // Emit @field:JsonSerialize(`as` = …) / (contentAs = …) when the
            // field's declared type references a sealed class.  See the
            // `field_sealed_annotations` precomputation above for the
            // rationale.
            if let Some(annotation) = &field_sealed_annotations[idx] {
                out.push_str("    ");
                out.push_str(annotation);
                out.push('\n');
            }
            out.push_str(&format!("    {field_str},\n"));
        }
        out.push_str(")\n");
    }
}

/// Return the `@field:JsonSerialize(...)` annotation source needed for a
/// field whose declared type references a sealed class, or `None` if the
/// type does not reference a sealed class.
///
/// Recognised shapes (Optional layers are unwrapped first):
/// - `Named(sealed)` → `@field:JsonSerialize(\`as\` = sealed::class)`
/// - `Vec<Named(sealed)>` → `@field:JsonSerialize(contentAs = sealed::class)`
/// - `Map<_, Named(sealed)>` → `@field:JsonSerialize(contentAs = sealed::class)`
///
/// Other shapes (nested generics, sealed inside `Map` key, …) are ignored —
/// they don't appear in the current codebase, and `contentAs` cannot express
/// them anyway.
fn sealed_class_field_annotation(
    ty: &TypeRef,
    sealed_class_names: &std::collections::HashSet<String>,
) -> Option<String> {
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    match base {
        TypeRef::Named(name) if sealed_class_names.contains(name) => Some(format!(
            "@field:com.fasterxml.jackson.databind.annotation.JsonSerialize(`as` = {name}::class)"
        )),
        TypeRef::Vec(inner) => {
            let inner_base = match inner.as_ref() {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            };
            if let TypeRef::Named(name) = inner_base {
                if sealed_class_names.contains(name) {
                    return Some(format!(
                        "@field:com.fasterxml.jackson.databind.annotation.JsonSerialize(contentAs = {name}::class)"
                    ));
                }
            }
            None
        }
        TypeRef::Map(_, value) => {
            let value_base = match value.as_ref() {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            };
            if let TypeRef::Named(name) = value_base {
                if sealed_class_names.contains(name) {
                    return Some(format!(
                        "@field:com.fasterxml.jackson.databind.annotation.JsonSerialize(contentAs = {name}::class)"
                    ));
                }
            }
            None
        }
        _ => None,
    }
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String, package: &str) {
    emit_cleaned_kdoc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "enum_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            // Emit per-variant KDoc above the enum constant. Indent matches
            // the template's 4-space lead.
            emit_cleaned_kdoc(out, &en.variants[idx].doc, "    ");
            // When the Rust serde discriminator differs from the Kotlin
            // `SCREAMING_SNAKE_CASE` constant, emit a `@JsonProperty` so
            // Jackson maps the wire value to the right constant on
            // deserialize and back on serialize. This is the typical case
            // when the Rust source uses `#[serde(rename_all = "snake_case")]`
            // or per-variant `#[serde(rename = "...")]`.
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let comma = if idx + 1 == names.len() { ";" } else { "," };

            if discriminator != *name {
                // Format: annotation + variant, optionally on a single line if it fits
                let annotation = format!(
                    "@com.fasterxml.jackson.annotation.JsonProperty(\"{}\")",
                    escape_kotlin_string(&discriminator)
                );
                let variant_line = format!("{}{}", name, comma);
                let total_length = 4 + annotation.len() + 1 + variant_line.len(); // 4 indent, space sep

                if total_length <= KTFMT_LINE_WIDTH {
                    // Fit on single line: "    @annotation VariantName,"
                    out.push_str(&format!("    {} {}\n", annotation, variant_line));
                } else {
                    // Multi-line: annotation on one line, variant on the next
                    out.push_str(&format!("    {}\n", annotation));
                    out.push_str(&format!("    {}\n", variant_line));
                }
            } else {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "enum_variant.jinja",
                    minijinja::context! {
                        name => name,
                        comma => comma,
                    },
                ));
            }
        }

        // Emit @JsonValue method for serialization
        // ktfmt wants "when" on a new line for expression-bodied functions, even if it would fit
        out.push_str("\n    @com.fasterxml.jackson.annotation.JsonValue\n");
        out.push_str("    fun toWire(): String =\n");
        out.push_str("        when (this) {\n");
        for (idx, name) in names.iter().enumerate() {
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            out.push_str(&format!(
                "            {} -> \"{}\"\n",
                name,
                escape_kotlin_string(&discriminator)
            ));
        }
        out.push_str("        }\n");

        // Emit @JsonCreator companion object method for deserialization
        out.push_str("\n    companion object {\n");
        out.push_str("        @com.fasterxml.jackson.annotation.JsonCreator\n");
        out.push_str("        @JvmStatic\n");
        out.push_str("        fun fromWire(value: String): ");
        out.push_str(&en.name);
        out.push_str(" =\n");
        out.push_str("            when (value) {\n");
        for (idx, name) in names.iter().enumerate() {
            let discriminator = wire_variant_value(
                &en.variants[idx].name,
                en.variants[idx].serde_rename.as_deref(),
                en.serde_rename_all.as_deref(),
            );
            let discriminator_lower = discriminator.to_lowercase();
            if discriminator != discriminator_lower {
                // Accept both the serde-renamed wire form (e.g. "Angle") and its lowercase
                // variant (e.g. "angle"). Some core enums implement Serialize/Deserialize
                // manually via a token normaliser (see UrlEscapeStyle), so the wire form on
                // the JSON boundary may be lowercase even when alef's IR sees the raw
                // PascalCase variant name. Matching both keeps the binding robust against
                // either convention without forcing the core to add #[serde(rename_all)].
                // Emit each match value on its own line per ktfmt's multi-value arm formatting
                out.push_str(&format!(
                    "                \"{}\",\n",
                    escape_kotlin_string(&discriminator)
                ));
                out.push_str(&format!(
                    "                \"{}\" -> {}\n",
                    escape_kotlin_string(&discriminator_lower),
                    name
                ));
            } else {
                out.push_str(&format!(
                    "                \"{}\" -> {}\n",
                    escape_kotlin_string(&discriminator),
                    name
                ));
            }
        }
        out.push_str("                else -> throw IllegalArgumentException(\"Unknown ");
        out.push_str(&en.name);
        out.push_str(" value: $value\")\n");
        out.push_str("            }\n");
        out.push_str("    }\n");

        out.push_str("}\n");
    } else {
        // Sealed classes with data variants need a Jackson custom deserializer so that
        // Jackson (used by e2e tests via ObjectMapper) can reconstruct the correct
        // subtype.  Unit-only sealed classes use a simple `when` dispatch and do not
        // need deserialization support.
        let needs_deserializer = en.serde_tag.is_some() || en.serde_untagged;
        if needs_deserializer {
            out.push_str("@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = ");
            out.push_str(&en.name);
            out.push_str("Deserializer::class)\n");
        }
        // Sealed classes need custom serializers so that round-trip
        // (Kotlin → JSON → Rust) works correctly.
        // - Tagged: the tag field must be injected into the JSON output.
        // - Untagged: newtype variants must serialize as their inner value,
        //   not as a data-class wrapper object.
        let needs_serializer = en.serde_tag.is_some() || en.serde_untagged;
        if needs_serializer {
            out.push_str("@com.fasterxml.jackson.databind.annotation.JsonSerialize(using = ");
            out.push_str(&en.name);
            out.push_str("Serializer::class)\n");
        }
        out.push_str(&crate::backends::kotlin::template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));

        // Collect all variant names so we can detect name-shadowing in field types.
        // Inside a sealed class body, a nested data class `Foo` shadows any outer
        // `Foo` with the same simple name.  When a field type has the same name as a
        // sibling variant we must fully-qualify the field type with the package path
        // to avoid the compiler resolving the type to the variant itself (Bug E).
        let variant_names: std::collections::HashSet<&str> = en.variants.iter().map(|v| v.name.as_str()).collect();

        for variant in &en.variants {
            // Sealed-class variants render their rustdoc above the nested
            // object/data class declaration.
            emit_cleaned_kdoc(out, &variant.doc, "    ");
            if variant.fields.is_empty() {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "sealed_object_variant.jinja",
                    minijinja::context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                // Newtype/tuple variants (a single tuple-named field wrapping an
                // inner type, e.g. `data class User(val message: UserMessage)`)
                // do NOT need the inherited annotation reset:
                //   - The parent serializer routes via `value.<inner>` (e.g.
                //     `mapper.valueToTree(value.message)`), so the type Jackson
                //     resolves the serializer for is the INNER non-sealed class
                //     — no recursion is possible.
                //   - The parent deserializer routes via
                //     `ctx.readTreeAsValue<Inner>(payload, Inner::class.java)`,
                //     reading into the inner non-sealed class — no recursion.
                //
                // Emitting `@JsonSerialize(using = None::class)` on newtype
                // variants is in fact HARMFUL: when Jackson encounters a value
                // of runtime type `Sealed.Variant`, the variant-level reset
                // annotation defeats the parent's custom serializer entirely,
                // so the value is emitted as a default POJO `{"<field>":...}`
                // instead of the discriminator-flattened form
                // (`{"role":"user",...}` for tagged sealed classes, or just the
                // inner value for untagged ones).
                //
                // Named-field struct variants (variants carrying their own
                // named fields directly) DO need the reset: the parent
                // (de)serializer routes via `value as Sealed.Variant` or
                // `readTreeAsValue<Variant>(...)`, both of which target the
                // variant subtype — inheriting the parent's custom annotation
                // would loop back into the parent (de)serializer.
                let is_newtype_variant = variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name);
                let emit_reset = !is_newtype_variant;
                if needs_deserializer && emit_reset {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n");
                }
                if needs_serializer && emit_reset {
                    out.push_str("    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n");
                }

                // Pre-build field strings for the ktfmt single-line heuristic.
                // Annotations force multi-line because they cannot be inlined.
                let has_annotations = (needs_deserializer || needs_serializer) && emit_reset;
                let mut variant_field_strings: Vec<String> = Vec::with_capacity(variant.fields.len());
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = kotlin_type_disambiguated(&f.ty, f.optional, &variant_names, package);
                    let field_type_name = match &f.ty {
                        TypeRef::Named(name) => Some(name.as_str()),
                        TypeRef::String => Some("String"),
                        TypeRef::Primitive(p) => Some(primitive_type_name(p)),
                        _ => None,
                    };
                    let name = super::shared::kotlin_field_name_with_type(
                        &f.name,
                        idx,
                        field_type_name,
                        &variant.name,
                        variant.fields.len(),
                    );
                    variant_field_strings.push(format!("val {name}: {ty_str}"));
                }

                let variant_prefix = format!("data class {}", variant.name);
                let variant_suffix = format!(" : {}()", en.name);
                let use_single_line = !has_annotations
                    && fits_single_line("    ", &variant_prefix, &variant_field_strings, &variant_suffix);

                if use_single_line {
                    out.push_str(&format!(
                        "    {variant_prefix}({fields}){variant_suffix}\n",
                        fields = variant_field_strings.join(", ")
                    ));
                } else {
                    out.push_str(&format!("    {variant_prefix}(\n"));
                    for field_str in &variant_field_strings {
                        out.push_str(&format!("        {field_str},\n"));
                    }
                    out.push_str(&format!("    ){variant_suffix}\n"));
                }
            }
        }
        out.push_str("}\n");

        // Emit the custom Jackson deserializer immediately after the sealed class.
        if needs_deserializer {
            if let Some(tag_field) = &en.serde_tag {
                emit_kotlin_tagged_deserializer(out, en, tag_field);
            } else if en.serde_untagged {
                emit_kotlin_untagged_deserializer(out, en);
            }
        }
        // Emit the custom Jackson serializer for tagged/untagged sealed classes
        // so that round-trip (Kotlin → JSON → Rust) works correctly.
        if let Some(tag_field) = &en.serde_tag {
            emit_kotlin_tagged_serializer(out, en, tag_field);
        } else if en.serde_untagged {
            emit_kotlin_untagged_serializer(out, en);
        }
    }
}

/// True when a field's name is a tuple-field index (e.g. `"0"`, `"_0"`).
fn is_tuple_field_name(name: &str) -> bool {
    let stripped = name.trim_start_matches('_');
    !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit())
}

/// Return the simple Kotlin class name that Jackson can deserialise a TypeRef into
/// using `readTreeAsValue(node, <name>::class.java)`.
/// For user-defined Named types it is the short class name (same package, no import needed).
fn kotlin_class_name_for_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean".to_string(),
                PrimitiveType::U8 | PrimitiveType::I8 => "Byte".to_string(),
                PrimitiveType::U16 | PrimitiveType::I16 => "Short".to_string(),
                PrimitiveType::U32 | PrimitiveType::I32 => "Int".to_string(),
                PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                    "Long".to_string()
                }
                PrimitiveType::F32 => "Float".to_string(),
                PrimitiveType::F64 => "Double".to_string(),
            }
        }
        TypeRef::Named(n) => n.clone(),
        TypeRef::Vec(_) => "List".to_string(),
        TypeRef::Map(_, _) => "Map".to_string(),
        _ => "Any".to_string(),
    }
}

/// Emit a Jackson `StdSerializer` for an internally-tagged (`#[serde(tag = ...)]`)
/// sealed class.  The serializer adds the tag field back into the JSON object so
/// that round-tripping Kotlin → JSON → Rust works correctly.
///
/// Strategy:
/// - For **newtype/tuple variants** (single `_0` field holding an inner type):
///   serialize `value.field0` as a JSON object tree, then inject the tag field.
/// - For **named-field struct variants**: serialize the variant data class as a
///   tree (Jackson sees it as a plain data class), then inject the tag field.
/// - **Unit variants**: write `{"<tag>": "<discriminator>"}` directly.
fn emit_kotlin_tagged_serializer(out: &mut String, en: &EnumDef, tag_field: &str) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Serializer : com.fasterxml.jackson.databind.ser.std.StdSerializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    // Suppress detekt LongMethod: the number of branches scales with the number of
    // variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun serialize(\n");
    out.push_str("        value: ");
    out.push_str(name);
    out.push_str(",\n");
    out.push_str("        gen: com.fasterxml.jackson.core.JsonGenerator,\n");
    out.push_str("        provider: com.fasterxml.jackson.databind.SerializerProvider,\n");
    out.push_str("    ) {\n");
    // Use the codec as ObjectMapper so we can call valueToTree; fall back to a
    // fresh ObjectMapper if the codec is not one (shouldn't happen in practice).
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str("        val mapper = (gen.codec as? com.fasterxml.jackson.databind.ObjectMapper) ?: com.fasterxml.jackson.databind.ObjectMapper().findAndRegisterModules()\n");
    out.push_str("        val node: com.fasterxml.jackson.databind.node.ObjectNode = when (value) {\n");

    for variant in &en.variants {
        let discriminator = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("            is ");
        out.push_str(name);
        out.push('.');
        out.push_str(&variant.name);
        out.push_str(" -> {\n");

        if variant.fields.is_empty() {
            // Unit variant: emit just the tag.
            out.push_str("                val n = mapper.createObjectNode()\n");
            out.push_str("                n.put(\"");
            out.push_str(tag_field);
            out.push_str("\", \"");
            out.push_str(&discriminator);
            out.push_str("\")\n");
            out.push_str("                n\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: serialize the inner value as a tree then
            // inject the tag field so the output matches the tagged serde format.
            let field = &variant.fields[0];
            let field_name = super::shared::kotlin_field_name_with_type(
                &field.name,
                0,
                match &field.ty {
                    TypeRef::Named(n) => Some(n.as_str()),
                    TypeRef::String => Some("String"),
                    TypeRef::Primitive(p) => Some(primitive_type_name(p)),
                    _ => None,
                },
                &variant.name,
                1,
            );
            out.push_str("                @Suppress(\"UNCHECKED_CAST\")\n");
            out.push_str(
                "                val n = mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value.",
            );
            out.push_str(&field_name);
            out.push_str(") as com.fasterxml.jackson.databind.node.ObjectNode\n");
            out.push_str("                n.put(\"");
            out.push_str(tag_field);
            out.push_str("\", \"");
            out.push_str(&discriminator);
            out.push_str("\")\n");
            out.push_str("                n\n");
        } else {
            // Named-field struct variant: the data class carries the payload
            // fields directly.  Cast `value` to the concrete variant type before
            // calling valueToTree so Jackson resolves the serializer against the
            // variant class (which has @JsonSerialize reset to the default POJO
            // serializer), not against the parent sealed class (which would
            // re-trigger InputDocumentSerializer and cause infinite recursion).
            out.push_str("                @Suppress(\"UNCHECKED_CAST\")\n");
            out.push_str(
                "                val n = mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value as ",
            );
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(") as com.fasterxml.jackson.databind.node.ObjectNode\n");
            out.push_str("                n.put(\"");
            out.push_str(tag_field);
            out.push_str("\", \"");
            out.push_str(&discriminator);
            out.push_str("\")\n");
            out.push_str("                n\n");
        }

        out.push_str("            }\n");
    }

    out.push_str("        }\n");
    out.push_str("        mapper.writeTree(gen, node)\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a Jackson `StdDeserializer` for an internally-tagged (`#[serde(tag = ...)]`)
/// sealed class.  The deserializer reads the tag field from the JSON object and
/// dispatches to the correct variant by calling `ctx.readTreeAsValue`.
fn emit_kotlin_tagged_deserializer(out: &mut String, en: &EnumDef, tag_field: &str) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Deserializer : com.fasterxml.jackson.databind.deser.std.StdDeserializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    // Suppress detekt LongMethod: the number of when-branches scales with the number
    // of variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun deserialize(\n");
    out.push_str("        parser: com.fasterxml.jackson.core.JsonParser,\n");
    out.push_str("        ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
    out.push_str("    ): ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("        val node = parser.codec.readTree<com.fasterxml.jackson.databind.node.ObjectNode>(parser)\n");
    // Bug D fix: strip the tag field from the payload before passing it to
    // readTreeAsValue.  Inner types (e.g. SystemMessage, ContentPart.Text) do
    // not declare a `role`/`type` field, so Jackson rejects the extra key with
    // UnrecognizedPropertyException unless it is removed first.
    // Note: `deepCopy()` on `ObjectNode` is not generic in Kotlin's view of
    // the Jackson API (the Java signature `<T extends JsonNode> T deepCopy()`
    // is not callable with explicit type arguments in Kotlin 2.x), so we cast
    // the result explicitly rather than using `deepCopy<ObjectNode>()`.
    out.push_str("        val tag = node.get(\"");
    out.push_str(tag_field);
    out.push_str("\")?.asText()\n");
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str(
        "        val payload = (node.deepCopy() as com.fasterxml.jackson.databind.node.ObjectNode).apply { remove(\"",
    );
    out.push_str(tag_field);
    out.push_str("\") }\n");
    out.push_str("        return when (tag) {\n");

    for variant in &en.variants {
        let discriminator = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            en.serde_rename_all.as_deref(),
        );
        out.push_str("            \"");
        out.push_str(&discriminator);
        out.push_str("\" -> ");

        if variant.fields.is_empty() {
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push('\n');
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: the `_0` IR field holds an inner named type
            // (e.g. `SystemMessage`).  Deserialize the tag-stripped payload as
            // that inner type and wrap it in the variant constructor.
            let inner_class = kotlin_class_name_for_type(&variant.fields[0].ty);
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("(ctx.readTreeAsValue<");
            out.push_str(&inner_class);
            out.push_str(">(payload, ");
            out.push_str(&inner_class);
            out.push_str("::class.java))\n");
        } else {
            // Named-field struct variant: the variant data class fields are the
            // same as the JSON object fields (minus the tag).  `readTreeAsValue`
            // constructs the correct variant subtype directly from the stripped
            // payload — no constructor wrap needed.  Explicit Kotlin type
            // parameter avoids `Any!` inference on the Java generic return type.
            out.push_str("ctx.readTreeAsValue<");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(">(payload, ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("::class.java)\n");
        }
    }

    out.push_str("            else -> throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("                parser, \"Unknown ");
    out.push_str(name);
    out.push_str(" tag\", tag, ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("            )\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a Jackson `StdDeserializer` for an untagged (`#[serde(untagged)]`) sealed
/// class.  The deserializer inspects the JSON node kind and tries variants in order.
fn emit_kotlin_untagged_deserializer(out: &mut String, en: &EnumDef) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Deserializer : com.fasterxml.jackson.databind.deser.std.StdDeserializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    // Suppress detekt LongMethod: the number of if-branches scales with the number
    // of variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun deserialize(\n");
    out.push_str("        parser: com.fasterxml.jackson.core.JsonParser,\n");
    out.push_str("        ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
    out.push_str("    ): ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("        val node = parser.codec.readTree<com.fasterxml.jackson.databind.JsonNode>(parser)\n");

    for variant in &en.variants {
        if variant.fields.is_empty() {
            // Unit variant in an untagged enum — skip shape-based dispatch; cannot match.
            continue;
        }

        // Determine what JSON shape this variant expects based on its first field.
        let (condition, inner_expr) = if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Tuple/newtype variant — the JSON IS the inner value.
            let ty = &variant.fields[0].ty;
            match ty {
                TypeRef::String => ("node.isTextual", format!("{name}.{}(node.asText())", variant.name)),
                TypeRef::Vec(elem_ty) => {
                    // Use JavaType to carry the generic element type so Jackson can
                    // construct a properly-typed List<T> rather than a raw List<*>.
                    let elem_class = kotlin_class_name_for_type(elem_ty);
                    let expr = format!(
                        "run {{\n                val javaType = ctx.typeFactory.constructCollectionType(List::class.java, {elem_class}::class.java)\n                @Suppress(\"UNCHECKED_CAST\")\n                {name}.{}(ctx.readTreeAsValue<List<{elem_class}>>(node, javaType) as List<{elem_class}>)\n            }}",
                        variant.name,
                    );
                    ("node.isArray", expr)
                }
                TypeRef::Primitive(_) => {
                    let class_name = kotlin_class_name_for_type(ty);
                    (
                        "node.isNumber",
                        format!(
                            "{name}.{}(ctx.readTreeAsValue(node, {class_name}::class.java))",
                            variant.name
                        ),
                    )
                }
                TypeRef::Named(n) => {
                    // Named types can be either enums (stringify via @JsonValue + @JsonCreator)
                    // or structs (objectify). Without enum type information in the backend,
                    // we conservatively check node.isObject for struct variants and fall through
                    // to a catch-all deserialization that handles both cases at the end.
                    // For now, we'll check for both textual and object nodes to support both.
                    (
                        "true", // Try all Named types; let deserialization determine success
                        format!(
                            "try {{ {name}.{}(ctx.readTreeAsValue(node, {n}::class.java)) }} catch (_: com.fasterxml.jackson.databind.exc.MismatchedInputException) {{ null as? {name} }} catch (_: com.fasterxml.jackson.databind.exc.UnrecognizedPropertyException) {{ null as? {name} }}",
                            variant.name
                        ),
                    )
                }
                _ => {
                    let class_name = kotlin_class_name_for_type(ty);
                    (
                        "node.isObject",
                        format!(
                            "{name}.{}(ctx.readTreeAsValue(node, {class_name}::class.java))",
                            variant.name
                        ),
                    )
                }
            }
        } else {
            // Struct variant with named fields — JSON must be an object.
            // `readTreeAsValue` returns the correct data class subtype directly;
            // no variant-constructor wrapping needed.
            let struct_class = format!("{name}.{}", variant.name);
            (
                "node.isObject",
                format!("ctx.readTreeAsValue<{struct_class}>(node, {struct_class}::class.java)"),
            )
        };

        out.push_str("        if (");
        out.push_str(condition);
        out.push_str(") ");
        if condition == "true" && inner_expr.contains("try {") {
            // For try-catch branches, only return if result is not null
            out.push_str("{\n");
            out.push_str("            val result = ");
            out.push_str(&inner_expr);
            out.push('\n');
            out.push_str("            if (result != null) return result\n");
            out.push_str("        }\n");
        } else {
            out.push_str("return ");
            out.push_str(&inner_expr);
            out.push('\n');
        }
    }

    out.push_str("        throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("            parser, \"Cannot deserialize ");
    out.push_str(name);
    out.push_str(": no matching variant for JSON shape\", null, ");
    out.push_str(name);
    out.push_str("::class.java,\n");
    out.push_str("        )\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit a Jackson `StdSerializer` for an untagged (`#[serde(untagged)]`) sealed
/// class.  Each variant serializes as its inner value (for newtype variants) or
/// as a plain JSON object (for struct variants).
///
/// Without this serializer, Jackson would emit `{"field0": "..."}` for a newtype
/// variant like `UserContent.Text(field0: String)`, but Rust expects just `"..."`.
fn emit_kotlin_untagged_serializer(out: &mut String, en: &EnumDef) {
    let name = &en.name;
    out.push('\n');
    out.push_str("private class ");
    out.push_str(name);
    out.push_str("Serializer : com.fasterxml.jackson.databind.ser.std.StdSerializer<");
    out.push_str(name);
    out.push_str(">(");
    out.push_str(name);
    out.push_str("::class.java) {\n");
    // Suppress detekt LongMethod: the number of branches scales with the number of
    // variants; for enums with many variants the function body will exceed detekt's
    // 60-line default threshold.  The generated code is correct and intentionally long.
    out.push_str("    @Suppress(\"LongMethod\")\n");
    out.push_str("    override fun serialize(\n");
    out.push_str("        value: ");
    out.push_str(name);
    out.push_str(",\n");
    out.push_str("        gen: com.fasterxml.jackson.core.JsonGenerator,\n");
    out.push_str("        provider: com.fasterxml.jackson.databind.SerializerProvider,\n");
    out.push_str("    ) {\n");
    out.push_str("        @Suppress(\"UNCHECKED_CAST\")\n");
    out.push_str("        val mapper = (gen.codec as? com.fasterxml.jackson.databind.ObjectMapper) ?: com.fasterxml.jackson.databind.ObjectMapper().findAndRegisterModules()\n");
    out.push_str("        when (value) {\n");

    for variant in &en.variants {
        if variant.fields.is_empty() {
            // Unit variant in an untagged enum: emit null (safest fallback).
            out.push_str("            is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> gen.writeNull()\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: serialize the inner value directly
            // (not wrapped in an object), matching serde's untagged behaviour.
            // Use the same payload-derived field name that the data-class declaration
            // uses (via kotlin_field_name_with_type), so `value.<field>` resolves.
            let field = &variant.fields[0];
            let field_name = super::shared::kotlin_field_name_with_type(
                &field.name,
                0,
                match &field.ty {
                    TypeRef::Named(n) => Some(n.as_str()),
                    TypeRef::String => Some("String"),
                    TypeRef::Primitive(p) => Some(primitive_type_name(p)),
                    _ => None,
                },
                &variant.name,
                1,
            );
            // When the inner type is Vec<SealedClass>, mapper.writeValue dispatches to
            // each element's runtime-subtype serializer (which has @JsonSerialize reset
            // to None), losing the sealed-class "type" discriminator. Use
            // provider.findValueSerializer on the declared element type instead so the
            // sealed-class serializer (which writes "type") is always called.
            if let TypeRef::Vec(inner) = &field.ty {
                if let TypeRef::Named(elem_type) = inner.as_ref() {
                    out.push_str("            is ");
                    out.push_str(name);
                    out.push('.');
                    out.push_str(&variant.name);
                    out.push_str(" -> {\n");
                    out.push_str("                gen.writeStartArray()\n");
                    out.push_str(&format!(
                        "                val elemSerializer = provider.findValueSerializer({}::class.java)\n",
                        elem_type
                    ));
                    out.push_str(&format!("                for (elem in value.{field_name}) {{\n"));
                    out.push_str("                    elemSerializer.serialize(elem, gen, provider)\n");
                    out.push_str("                }\n");
                    out.push_str("                gen.writeEndArray()\n");
                    out.push_str("            }\n");
                } else {
                    out.push_str("            is ");
                    out.push_str(name);
                    out.push('.');
                    out.push_str(&variant.name);
                    out.push_str(" -> mapper.writeValue(gen, value.");
                    out.push_str(&field_name);
                    out.push_str(")\n");
                }
            } else {
                out.push_str("            is ");
                out.push_str(name);
                out.push('.');
                out.push_str(&variant.name);
                out.push_str(" -> mapper.writeValue(gen, value.");
                out.push_str(&field_name);
                out.push_str(")\n");
            }
        } else {
            // Named-field struct variant: cast to the concrete variant type before
            // serializing so Jackson resolves the serializer against the variant
            // class (which has @JsonSerialize reset to the default POJO serializer),
            // not against the parent sealed class (which would recurse infinitely).
            out.push_str("            is ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(" -> mapper.writeValue(gen, value as ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(")\n");
        }
    }

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Interpolate `{N}` placeholder tokens in an error message template with
/// Kotlin string-template syntax.
///
/// Rust error message templates use `{0}`, `{1}`, … to reference the Nth
/// positional field. Kotlin error variant fields are named `field0`, `field1`,
/// … by the `kotlin_field_name` helper. This function replaces each `{N}` with
/// `$fieldN` when the next character is NOT an identifier-continuation
/// character (letter, digit, underscore) — matching ktlint's
/// `standard:string-template` rule which flags `${field0}` as redundant in
/// that position. When the next character would make the interpolation
/// ambiguous (e.g. `{0}suffix` where `field0suffix` is also a valid Kotlin
/// identifier) `${fieldN}` is emitted instead.
fn interpolate_error_message_template(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(open) = remaining.find('{') {
        let after_open = &remaining[open + 1..];
        if let Some(close) = after_open.find('}') {
            let token = &after_open[..close];
            if token.chars().all(|c| c.is_ascii_digit()) && !token.is_empty() {
                out.push_str(&remaining[..open]);
                let after_close = &after_open[close + 1..];
                let next_is_ident_cont = after_close
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                if next_is_ident_cont {
                    out.push_str("${field");
                    out.push_str(token);
                    out.push('}');
                } else {
                    out.push_str("$field");
                    out.push_str(token);
                }
                remaining = &remaining[open + 1 + close + 1..];
                continue;
            }
        }
        out.push_str(&remaining[..open + 1]);
        remaining = &remaining[open + 1..];
    }
    out.push_str(remaining);
    out
}

pub(crate) fn emit_error_type_with_imports(
    error: &crate::core::ir::ErrorDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    emit_cleaned_kdoc(out, &error.doc, "");
    out.push_str(&crate::backends::kotlin::template_env::render(
        "error_sealed_class_header.jinja",
        minijinja::context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            let raw_msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            // Unit variants have no fields so {N} tokens would be invalid in
            // practice, but run through the interpolator for consistency and
            // to avoid emitting literal placeholder tokens if they appear.
            let message = interpolate_error_message_template(raw_msg);
            out.push_str(&crate::backends::kotlin::template_env::render(
                "error_object_variant.jinja",
                minijinja::context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => message,
                },
            ));
        } else {
            let raw_msg = variant.message_template.as_deref().unwrap_or(&variant.name);
            let message = interpolate_error_message_template(raw_msg);

            // Pre-build field strings for the ktfmt single-line heuristic.
            // Each entry includes the optional `override` modifier for `message` fields.
            let mut err_field_strings: Vec<String> = Vec::with_capacity(variant.fields.len());
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = kotlin_type_with_string_imports(&f.ty, f.optional, imports);
                let name = kotlin_field_name(&f.name, idx);
                let modifier = if name == "message" { "override " } else { "" };
                err_field_strings.push(format!("{modifier}val {name}: {ty_str}"));
            }

            let err_prefix = format!("data class {}", variant.name);
            let err_suffix = format!(" : {}(\"{message}\")", error.name);
            let use_single_line = fits_single_line("    ", &err_prefix, &err_field_strings, &err_suffix);

            if use_single_line {
                out.push_str(&format!(
                    "    {err_prefix}({fields}){err_suffix}\n",
                    fields = err_field_strings.join(", ")
                ));
            } else {
                out.push_str(&format!("    {err_prefix}(\n"));
                for field_str in &err_field_strings {
                    out.push_str(&format!("        {field_str},\n"));
                }
                out.push_str(&format!("    ){err_suffix}\n"));
            }
        }
    }
    // Emit `open val` property declarations with sensible defaults for each
    // whitelisted introspection method (status_code, is_transient, error_type).
    // The JNI bridge throws a flat `<App>BridgeException` rather than
    // constructing sealed-class variants, so requiring every variant to
    // override these abstract properties would break compilation. Each variant
    // can still opt into a concrete override when domain code constructs the
    // error directly, while the defaults keep the sealed class self-contained.
    for method in error.methods.iter().filter(|m| !m.sanitized) {
        let prop_name = to_lower_camel(&method.name);
        let ty_str = kotlin_type_with_string_imports(&method.return_type, false, imports);
        let default = kotlin_zero_value(&ty_str);
        out.push_str(&format!("    open val {prop_name}: {ty_str} = {default}\n"));
    }
    out.push_str("}\n");
}

// ---------------------------------------------------------------------------
// Function emitter
// ---------------------------------------------------------------------------

/// Emit a JVM wrapper function body (delegates to Bridge) inside an `object` block.
///
/// `client_type_names` lists struct types that have a hand-written Kotlin
/// wrapper class (see [`super::emit_jvm_client_class`]). Functions returning
/// those types must wrap the raw Java result in the Kotlin class so the public
/// type matches the wrapper's signature.
pub(crate) fn emit_function(
    f: &FunctionDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    _java_package: &str,
    client_type_names: &std::collections::HashSet<&str>,
) {
    emit_cleaned_kdoc(out, &f.doc, "    ");
    let params: Vec<String> = f.params.iter().map(|p| format_param_with_imports(p, imports)).collect();
    let return_ty = kotlin_type_with_string_imports(&f.return_type, false, imports);
    let async_kw = if f.is_async { "suspend " } else { "" };
    let func_name_camel = to_lower_camel(&f.name);
    let call_args = f
        .params
        .iter()
        .map(|p| to_lower_camel(&p.name))
        .collect::<Vec<_>>()
        .join(", ");

    // Detect a client-type return so we can wrap the Java result in its Kotlin
    // companion (e.g. `SampleLlm.createClient(...)` returns Java `DefaultClient`
    // but the Kotlin facade must hand back the coroutine-friendly Kotlin
    // `DefaultClient` wrapper).
    let returns_client_type = match &f.return_type {
        TypeRef::Named(n) => client_type_names.contains(n.as_str()),
        _ => false,
    };

    out.push_str(&crate::backends::kotlin::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            async_kw => async_kw,
            name => func_name_camel,
            params => params.join(", "),
            return_type => return_ty,
        },
    ));
    out.push('\n');

    // The Java facade returns `Optional<T>` for Rust `Option<T>` returns; the
    // Kotlin facade exposes the friendlier `T?`. Unwrap with `.orElse(null)` so
    // the types line up.
    let optional_suffix = if matches!(f.return_type, TypeRef::Optional(_)) && !returns_client_type {
        ".orElse(null)"
    } else {
        ""
    };

    if f.is_async {
        // The Java facade lowers async Rust functions to blocking calls (it
        // awaits the future internally and returns the resolved value, not a
        // CompletionStage). Wrap the call in `withContext(Dispatchers.IO)` so
        // the suspend function yields the calling thread while the JNI call
        // blocks under it.
        if returns_client_type {
            let wrapper = return_ty.trim_end_matches('?');
            out.push_str(&format!(
                "        return withContext(Dispatchers.IO) {{ {wrapper}(Bridge.{func_name_camel}({call_args})) }}\n"
            ));
        } else {
            out.push_str(&format!(
                "        return withContext(Dispatchers.IO) {{\n            Bridge.{func_name_camel}({call_args}){optional_suffix}\n        }}\n"
            ));
        }
    } else if matches!(f.return_type, TypeRef::Unit) {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "bridge_call_unit.jinja",
            minijinja::context! {
                name => func_name_camel,
                args => call_args,
            },
        ));
        out.push('\n');
    } else if returns_client_type {
        let wrapper = return_ty.trim_end_matches('?');
        out.push_str(&format!(
            "        return {wrapper}(Bridge.{func_name_camel}({call_args}))\n"
        ));
    } else {
        out.push_str(&format!(
            "        return Bridge.{func_name_camel}({call_args}){optional_suffix}\n"
        ));
    }
    out.push_str("    }\n");
}

// ---------------------------------------------------------------------------
// Parameter formatting
// ---------------------------------------------------------------------------

pub(crate) fn format_param_with_imports(p: &ParamDef, imports: &mut BTreeSet<String>) -> String {
    let ty_str = kotlin_type_with_string_imports(&p.ty, p.optional, imports);
    // Optional params get a `= null` default so callers can drop them via
    // named-argument syntax (e.g. `createClient(apiKey = "x", baseUrl = "y")`)
    // without having to spell out every nullable downstream argument.
    let default = if p.optional { " = null" } else { "" };
    format!("{}: {}{}", to_lower_camel(&p.name), ty_str, default)
}

// ---------------------------------------------------------------------------
// Type-string rendering (String-keyed imports variant — used by JVM + MPP)
// ---------------------------------------------------------------------------

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
fn kotlin_field_default(
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

fn escape_kotlin_string(s: &str) -> String {
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
fn kotlin_type_disambiguated(
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{CoreWrapper, EnumVariant, FieldDef};

    fn make_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    fn make_variant(name: &str, serde_rename: Option<&str>, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            doc: String::new(),
            is_default: false,
            serde_rename: serde_rename.map(str::to_string),
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
        }
    }

    fn make_enum(
        name: &str,
        serde_tag: Option<&str>,
        serde_untagged: bool,
        serde_rename_all: Option<&str>,
        variants: Vec<EnumVariant>,
    ) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("crate::{name}"),
            original_rust_path: format!("crate::{name}"),
            variants,
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: serde_tag.map(str::to_string),
            serde_untagged,
            serde_rename_all: serde_rename_all.map(str::to_string),
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        }
    }

    /// Regression: sealed classes with `#[serde(tag = ...)]` must emit
    /// `@JsonDeserialize` annotation and a companion deserializer that reads the
    /// tag field and dispatches per variant.
    #[test]
    fn emit_enum_tagged_sealed_class_emits_json_deserialize_annotation() {
        let en = make_enum(
            "Message",
            Some("role"),
            false,
            None,
            vec![
                make_variant(
                    "System",
                    Some("system"),
                    vec![make_field("_0", TypeRef::Named("SystemMessage".to_string()))],
                ),
                make_variant(
                    "User",
                    Some("user"),
                    vec![make_field("_0", TypeRef::Named("UserMessage".to_string()))],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");
        assert!(
            out.contains(
                "@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = MessageDeserializer::class)"
            ),
            "missing @JsonDeserialize annotation on tagged sealed class; got:\n{out}",
        );
        assert!(
            out.contains("private class MessageDeserializer"),
            "missing MessageDeserializer class; got:\n{out}",
        );
        assert!(
            out.contains("node.get(\"role\")"),
            "deserializer must read the 'role' tag field; got:\n{out}",
        );
        assert!(
            out.contains("\"system\" ->"),
            "deserializer must dispatch on variant 'system'; got:\n{out}",
        );
        // Bug D regression: the tag field must be stripped from the payload before
        // passing it to readTreeAsValue.  Inner types (e.g. SystemMessage) do not
        // declare a `role` field, so Jackson rejects the extra key without this fix.
        assert!(
            out.contains("val tag = node.get(\"role\")?.asText()"),
            "tagged deserializer must extract tag into separate variable; got:\n{out}",
        );
        assert!(
            out.contains("val payload = (node.deepCopy() as com.fasterxml.jackson.databind.node.ObjectNode).apply { remove(\"role\") }"),
            "tagged deserializer must strip tag field from payload via cast-safe deepCopy; got:\n{out}",
        );
        // Newtype variant: must wrap the inner type in the variant constructor.
        // readTreeAsValue<InnerType> returns the inner type; the variant constructor
        // wraps it to produce the sealed-class value.  Uses `payload` (tag-stripped).
        assert!(
            out.contains("Message.System(ctx.readTreeAsValue<SystemMessage>(payload, SystemMessage::class.java))"),
            "tagged deserializer must wrap readTreeAsValue<InnerType>(payload) in variant constructor for newtype; got:\n{out}",
        );
    }

    /// Regression: sealed classes with `#[serde(untagged)]` must emit
    /// `@JsonDeserialize` annotation and a companion deserializer that tries
    /// variants by JSON shape.
    #[test]
    fn emit_enum_untagged_sealed_class_emits_json_deserialize_annotation() {
        let en = make_enum(
            "EmbeddingInput",
            None,
            true,
            None,
            vec![
                make_variant("Single", None, vec![make_field("_0", TypeRef::String)]),
                make_variant(
                    "Multiple",
                    None,
                    vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");
        assert!(
            out.contains(
                "@com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = EmbeddingInputDeserializer::class)"
            ),
            "missing @JsonDeserialize annotation on untagged sealed class; got:\n{out}",
        );
        assert!(
            out.contains("private class EmbeddingInputDeserializer"),
            "missing EmbeddingInputDeserializer class; got:\n{out}",
        );
        assert!(
            out.contains("node.isTextual"),
            "untagged deserializer must check isTextual for String variant; got:\n{out}",
        );
        assert!(
            out.contains("node.isArray"),
            "untagged deserializer must check isArray for List variant; got:\n{out}",
        );
        // Fix: List<T> variants must use JavaType (constructCollectionType) rather
        // than raw List::class.java so Jackson knows the element type.
        assert!(
            out.contains("ctx.typeFactory.constructCollectionType(List::class.java, String::class.java)"),
            "untagged deserializer must use constructCollectionType for List<String> variant; got:\n{out}",
        );
        assert!(
            !out.contains("ctx.readTreeAsValue(node, List::class.java)"),
            "untagged deserializer must NOT use raw List::class.java; got:\n{out}",
        );
    }

    /// Regression (Bug A): tagged sealed class with a multi-field named-field variant
    /// (e.g. `Base64 { data: String, mediaType: String }`) must return
    /// `ctx.readTreeAsValue(node, <Variant>::class.java)` directly — not wrap the
    /// result in a second variant constructor call.
    #[test]
    fn tagged_deserializer_named_field_variant_no_double_wrap() {
        let en = make_enum(
            "InputDocument",
            Some("type"),
            false,
            Some("snake_case"),
            vec![
                make_variant("Url", Some("url"), vec![make_field("url", TypeRef::String)]),
                make_variant(
                    "Base64",
                    Some("base64"),
                    vec![
                        make_field("data", TypeRef::String),
                        make_field("media_type", TypeRef::Named("MediaType".to_string())),
                    ],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // Must return readTreeAsValue directly on payload (tag-stripped) — no `InputDocument.Base64(...)` wrap.
        // The explicit Kotlin type parameter avoids `Any!` inference.
        assert!(
            out.contains(
                "\"base64\" -> ctx.readTreeAsValue<InputDocument.Base64>(payload, InputDocument.Base64::class.java)"
            ),
            "tagged deserializer must return readTreeAsValue<T>(payload) directly for named-field variant; got:\n{out}",
        );
        assert!(
            !out.contains("InputDocument.Base64(ctx.readTreeAsValue"),
            "tagged deserializer must NOT wrap readTreeAsValue result in variant constructor; got:\n{out}",
        );
    }

    /// Regression (Bug A): tagged sealed class with a single-field newtype variant
    /// (e.g. `Text(field0: String)` for `ContentPart`) must also return
    /// `ctx.readTreeAsValue` directly without a variant constructor wrap.
    #[test]
    fn tagged_deserializer_newtype_variant_no_double_wrap() {
        let en = make_enum(
            "ContentPart",
            Some("type"),
            false,
            Some("snake_case"),
            vec![make_variant(
                "Text",
                Some("text"),
                vec![make_field("_0", TypeRef::Named("TextContent".to_string()))],
            )],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // Newtype variant: must wrap the inner-type result in the variant constructor.
        // The variant class (ContentPart.Text) is different from the inner type (TextContent).
        // Uses `payload` (tag-stripped node) — Bug D fix.
        assert!(
            out.contains("ContentPart.Text(ctx.readTreeAsValue<TextContent>(payload, TextContent::class.java))"),
            "tagged deserializer must wrap readTreeAsValue<InnerType>(payload) in variant constructor for newtype variant; got:\n{out}",
        );
    }

    /// Regression (Bug C): untagged sealed class with a `List<T>` variant where T
    /// is a complex/named type must emit `constructCollectionType` with the correct
    /// element class, not raw `List::class.java`.
    #[test]
    fn untagged_deserializer_list_of_named_type_uses_java_type() {
        let en = make_enum(
            "UserContent",
            None,
            true,
            None,
            vec![
                make_variant("Text", None, vec![make_field("_0", TypeRef::String)]),
                make_variant(
                    "Parts",
                    None,
                    vec![make_field(
                        "_0",
                        TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
                    )],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        assert!(
            out.contains("ctx.typeFactory.constructCollectionType(List::class.java, ContentPart::class.java)"),
            "untagged deserializer must use constructCollectionType for List<ContentPart>; got:\n{out}",
        );
        assert!(
            !out.contains("ctx.readTreeAsValue(node, List::class.java)"),
            "untagged deserializer must NOT use raw List::class.java for List<ContentPart>; got:\n{out}",
        );
    }

    /// Unit-only enums (Kotlin `enum class`) must NOT get a `@JsonDeserialize`
    /// annotation — they serialise to/from string values and Jackson handles
    /// them natively.
    #[test]
    fn emit_enum_unit_only_does_not_emit_json_deserialize() {
        let en = make_enum(
            "FinishReason",
            None,
            false,
            None,
            vec![make_variant("Stop", None, vec![]), make_variant("Length", None, vec![])],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");
        assert!(
            !out.contains("@JsonDeserialize") && !out.contains("Deserializer"),
            "unit-only enum must not emit a deserializer; got:\n{out}",
        );
        assert!(
            out.contains("enum class FinishReason"),
            "must emit enum class; got:\n{out}"
        );
    }

    /// Regression (Bug D): tagged sealed-class deserializer must strip the tag field
    /// from the JSON payload before passing it to readTreeAsValue.
    ///
    /// Without this fix Jackson raises `UnrecognizedPropertyException` because the
    /// inner type (e.g. `SystemMessage`) does not declare a `role` field.
    #[test]
    fn tagged_deserializer_strips_tag_field_from_payload() {
        let en = make_enum(
            "Message",
            Some("role"),
            false,
            None,
            vec![make_variant(
                "System",
                Some("system"),
                vec![make_field("_0", TypeRef::Named("SystemMessage".to_string()))],
            )],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // The tag value must be read into a local variable first.
        assert!(
            out.contains("val tag = node.get(\"role\")?.asText()"),
            "deserializer must extract tag into a local variable; got:\n{out}",
        );
        // A tag-stripped payload must be created via deepCopy (cast) + remove.
        assert!(
            out.contains(
                "val payload = (node.deepCopy() as com.fasterxml.jackson.databind.node.ObjectNode).apply { remove(\"role\") }"
            ),
            "deserializer must create tag-stripped payload via cast-safe deepCopy; got:\n{out}",
        );
        // The when-expression dispatches on `tag`, not `node.get(...)`.
        assert!(
            out.contains("return when (tag)"),
            "deserializer must dispatch on extracted tag variable; got:\n{out}",
        );
        // readTreeAsValue must receive `payload`, not the original `node`.
        assert!(
            out.contains("readTreeAsValue<SystemMessage>(payload, SystemMessage::class.java)"),
            "deserializer must pass tag-stripped payload to readTreeAsValue; got:\n{out}",
        );
        assert!(
            !out.contains("readTreeAsValue<SystemMessage>(node, SystemMessage::class.java)"),
            "deserializer must NOT pass un-stripped node to readTreeAsValue; got:\n{out}",
        );
    }

    /// Regression (Bug E): when a sealed-class variant field type has the same
    /// simple name as a sibling variant, the field type must be fully-qualified
    /// with the package path to prevent the Kotlin compiler from resolving the
    /// type to the nested variant class (which causes self-recursion /
    /// StackOverflowError in Jackson).
    ///
    /// TODO(alef-generic-cleanup): replace dev.sample_core/dev.sample_crate samplellm Android examples
    /// with neutral fixture package names.
    /// Example: `ContentPart::ImageUrl { image_url: ImageUrl }` — inside
    /// `ContentPart`, `ImageUrl` refers to the nested `data class ImageUrl` unless
    /// the field type is explicitly qualified as `dev.sample_core.samplellm.android.ImageUrl`.
    #[test]
    fn sealed_class_variant_field_type_qualified_when_name_clashes_with_sibling_variant() {
        // Mirrors the real ContentPart::ImageUrl { image_url: ImageUrl } case.
        let en = make_enum(
            "ContentPart",
            Some("type"),
            false,
            None,
            vec![
                make_variant("Text", Some("text"), vec![make_field("text", TypeRef::String)]),
                make_variant(
                    "ImageUrl",
                    Some("image_url"),
                    // Field type name `ImageUrl` matches variant name `ImageUrl` — clash!
                    vec![make_field("image_url", TypeRef::Named("ImageUrl".to_string()))],
                ),
            ],
        );
        let mut out = String::new();
        // Provide a non-empty package so disambiguation can emit the FQN.
        emit_enum(&en, &mut out, "dev.sample_crate.samplellm.android");

        // The variant data class must qualify the field type to avoid self-reference.
        assert!(
            out.contains("val imageUrl: dev.sample_crate.samplellm.android.ImageUrl"),
            "variant field type must be package-qualified when it clashes with a sibling variant name; got:\n{out}",
        );
        // The variant data class header itself is unqualified.
        assert!(
            out.contains("data class ImageUrl("),
            "variant class declaration must still use simple name; got:\n{out}",
        );
    }

    /// Non-clashing variant field types must NOT be package-qualified (verbosity
    /// guard — only disambiguate when the field type name matches a sibling variant).
    #[test]
    fn sealed_class_variant_field_type_unqualified_when_no_clash() {
        let en = make_enum(
            "ContentPart",
            Some("type"),
            false,
            None,
            vec![make_variant(
                "Document",
                Some("document"),
                // `DocumentContent` does not match any variant name — no qualification needed.
                vec![make_field("document", TypeRef::Named("DocumentContent".to_string()))],
            )],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "dev.sample_crate.samplellm.android");

        // Must use the simple name.
        assert!(
            out.contains("val document: DocumentContent"),
            "non-clashing field type must remain unqualified; got:\n{out}",
        );
        // Must NOT spuriously qualify.
        assert!(
            !out.contains("dev.sample_crate.samplellm.android.DocumentContent"),
            "non-clashing field type must not be package-qualified; got:\n{out}",
        );
    }

    /// Regression (Bug G): sealed class variant data classes must each carry a bare
    /// `@JsonDeserialize` annotation to prevent Jackson annotation inheritance from
    /// the parent sealed class.
    ///
    /// When the sealed class has `@JsonDeserialize(using = FooDeserializer::class)`,
    /// every nested variant class inherits that annotation.  This causes infinite
    /// recursion (or an "Unknown tag" error on the stripped payload) when
    /// `ctx.readTreeAsValue(payload, Foo.Variant::class.java)` is called inside
    /// `FooDeserializer` — Jackson re-invokes `FooDeserializer` for the variant
    /// class, which then fails because the variant's payload has no tag field.
    ///
    /// Emitting bare `@JsonDeserialize` (defaulting to `using = JsonDeserializer.None`)
    /// on each variant data class overrides the inherited annotation with the default
    /// POJO deserializer, breaking the recursion cycle.
    #[test]
    fn sealed_class_variant_data_classes_get_json_deserialize_reset_annotation() {
        let en = make_enum(
            "InputDocument",
            Some("type"),
            false,
            Some("snake_case"),
            vec![
                make_variant("Url", Some("document_url"), vec![make_field("url", TypeRef::String)]),
                make_variant(
                    "Base64",
                    Some("base64"),
                    vec![
                        make_field("data", TypeRef::String),
                        make_field("mediaType", TypeRef::String),
                    ],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // Named-field struct variants must carry @JsonDeserialize(using = None) and
        // @JsonSerialize(using = None) to reset the inherited custom (de)serializers.
        // The parent tagged deserializer calls readTreeAsValue(payload, Variant::class.java)
        // which would loop back into the parent deserializer without this reset.
        assert!(
            out.contains("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n    data class Url("),
            "Url variant must have @JsonDeserialize(using=None) and @JsonSerialize(using=None) reset annotations; got:\n{out}",
        );
        assert!(
            out.contains("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize(using = com.fasterxml.jackson.databind.JsonDeserializer.None::class)\n    @com.fasterxml.jackson.databind.annotation.JsonSerialize(using = com.fasterxml.jackson.databind.JsonSerializer.None::class)\n    data class Base64("),
            "Base64 variant must have @JsonDeserialize(using=None) and @JsonSerialize(using=None) reset annotations; got:\n{out}",
        );
    }

    /// Regression (Bug G — untagged): newtype variants of untagged sealed classes do NOT
    /// need reset annotations because the parent serializer dispatches via the inner value
    /// type (no recursion). However, the serializer for Vec<SealedClass> variants must use
    /// provider.findValueSerializer so the sealed-class serializer (which adds "type") is
    /// invoked per element, not the variant's reset-to-None subtype serializer.
    #[test]
    fn untagged_sealed_class_vec_variant_serializer_uses_declared_type_serializer() {
        let en = make_enum(
            "UserContent",
            None,
            true,
            None,
            vec![
                make_variant("Text", None, vec![make_field("_0", TypeRef::String)]),
                make_variant(
                    "Parts",
                    None,
                    vec![make_field(
                        "_0",
                        TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))),
                    )],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // Newtype variants are NOT given reset annotations (no recursion risk).
        assert!(
            !out.contains("    @com.fasterxml.jackson.databind.annotation.JsonDeserialize\n    @com.fasterxml.jackson.databind.annotation.JsonSerialize\n    data class Text("),
            "Text newtype variant must NOT have reset annotations; got:\n{out}",
        );
        // Parts serializer must use provider.findValueSerializer for element dispatch.
        assert!(
            out.contains("provider.findValueSerializer(ContentPart::class.java)"),
            "Parts serializer must use provider.findValueSerializer(ContentPart::class.java); got:\n{out}",
        );
        // Text serializer uses direct mapper.writeValue (String is not a sealed class).
        assert!(
            out.contains("is UserContent.Text -> mapper.writeValue(gen, value.value)"),
            "Text serializer must use mapper.writeValue; got:\n{out}",
        );
    }

    /// Regression: untagged sealed-class serializer must use the payload-derived field
    /// name (e.g. `value`) rather than the literal `field0`.  Without this fix the
    /// generated `when`-branch emits `value.field0` which is an unresolved reference
    /// because the data-class declaration uses the name derived by
    /// `kotlin_field_name_with_type` (e.g. `Single(val value: String)`).
    #[test]
    fn untagged_serializer_tuple_variant_uses_payload_derived_field_name() {
        // EmbeddingInput pattern: single-field tuple variants whose field type is a
        // primitive (String, List<String>) → field name must be `value`, not `field0`.
        let en = make_enum(
            "EmbeddingInput",
            None,
            true,
            None,
            vec![
                make_variant("Single", None, vec![make_field("_0", TypeRef::String)]),
                make_variant(
                    "Multiple",
                    None,
                    vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::String)))],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // Serializer when-branches must reference `value.value`, not `value.field0`.
        assert!(
            out.contains("-> mapper.writeValue(gen, value.value)"),
            "untagged serializer must use payload-derived field name `value`; got:\n{out}",
        );
        assert!(
            !out.contains("value.field0"),
            "untagged serializer must NOT use hardcoded `field0`; got:\n{out}",
        );
    }

    /// Regression (Bug G — serializer cast): named-field struct variants in a tagged
    /// sealed class serializer must cast `value` to the concrete variant type before
    /// calling `mapper.valueToTree`.  Without the cast, Jackson would resolve the
    /// serializer against the parent sealed class type, re-triggering the custom
    /// serializer and causing infinite recursion.
    #[test]
    fn tagged_serializer_named_field_variant_casts_to_concrete_type() {
        let en = make_enum(
            "InputDocument",
            Some("type"),
            false,
            Some("snake_case"),
            vec![make_variant(
                "Url",
                Some("document_url"),
                vec![make_field("url", TypeRef::String)],
            )],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out, "");

        // The serializer must cast `value` to `InputDocument.Url` before calling
        // valueToTree so Jackson uses the variant class's serializer (reset to
        // default POJO), not the parent sealed class's custom serializer.
        assert!(
            out.contains("mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value as InputDocument.Url) as com.fasterxml.jackson.databind.node.ObjectNode"),
            "tagged serializer must cast value to concrete variant type; got:\n{out}",
        );
        // Must NOT call valueToTree on `value` without a cast.
        assert!(
            !out.contains("mapper.valueToTree<com.fasterxml.jackson.databind.node.ObjectNode>(value) as"),
            "tagged serializer must NOT call valueToTree on un-cast parent-type value; got:\n{out}",
        );
    }
}
