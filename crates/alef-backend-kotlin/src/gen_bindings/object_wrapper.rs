//! `object {Crate}` namespace, bridge calls, and Kotlin type/enum/error code emission.
//!
//! Contains:
//! - `emit_function` — emits a JVM wrapper function that delegates to the Java `Bridge` alias
//! - `emit_type_with_imports` — emits a Kotlin `data class` or empty `class` for a type
//! - `emit_enum` — emits a Kotlin `enum class` or `sealed class` for an enum
//! - `emit_error_type_with_imports` — emits a `sealed class` hierarchy for an error type
//! - Kotlin type-string helpers (with import collection)

use alef_core::ir::{EnumDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::helpers::emit_cleaned_kdoc;
use super::shared::{kotlin_field_name, to_lower_camel, to_screaming_snake};
use crate::type_map::KotlinMapper;
use alef_codegen::type_mapper::TypeMapper;

// ---------------------------------------------------------------------------
// Type/enum/error emitters (re-exported for gen_mpp)
// ---------------------------------------------------------------------------

pub(crate) fn emit_type_with_imports(ty: &TypeDef, out: &mut String, imports: &mut BTreeSet<String>) {
    emit_cleaned_kdoc(out, &ty.doc, "");
    if ty.fields.is_empty() {
        out.push_str(&crate::template_env::render(
            "empty_class.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
        return;
    }
    out.push_str(&crate::template_env::render(
        "data_class_header.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));
    for (idx, field) in ty.fields.iter().enumerate() {
        let ty_str = kotlin_type_with_string_imports(&field.ty, field.optional, imports);
        let name = kotlin_field_name(&field.name, idx);
        let comma = if idx + 1 == ty.fields.len() { "" } else { "," };
        out.push_str(&crate::template_env::render(
            "class_field.jinja",
            minijinja::context! {
                name => &name,
                type => &ty_str,
                comma => comma,
            },
        ));
    }
    out.push_str(")\n");
}

pub(crate) fn emit_enum(en: &EnumDef, out: &mut String) {
    emit_cleaned_kdoc(out, &en.doc, "");
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        out.push_str(&crate::template_env::render(
            "enum_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        let names: Vec<String> = en.variants.iter().map(|v| to_screaming_snake(&v.name)).collect();
        for (idx, name) in names.iter().enumerate() {
            let comma = if idx + 1 == names.len() { ";" } else { "," };
            out.push_str(&crate::template_env::render(
                "enum_variant.jinja",
                minijinja::context! {
                    name => name,
                    comma => comma,
                },
            ));
        }
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
        out.push_str(&crate::template_env::render(
            "sealed_class_header.jinja",
            minijinja::context! {
                name => &en.name,
            },
        ));
        for variant in &en.variants {
            if variant.fields.is_empty() {
                out.push_str(&crate::template_env::render(
                    "sealed_object_variant.jinja",
                    minijinja::context! {
                        name => &variant.name,
                        parent_name => &en.name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "variant_data_class_header.jinja",
                    minijinja::context! {
                        name => &variant.name,
                    },
                ));
                for (idx, f) in variant.fields.iter().enumerate() {
                    let ty_str = kotlin_type(&f.ty, f.optional, &mut BTreeSet::new());
                    let name = kotlin_field_name(&f.name, idx);
                    let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                    out.push_str(&crate::template_env::render(
                        "variant_class_field.jinja",
                        minijinja::context! {
                            name => &name,
                            type => &ty_str,
                            comma => comma,
                        },
                    ));
                }
                out.push_str(&crate::template_env::render(
                    "variant_close.jinja",
                    minijinja::context! {
                        parent_name => &en.name,
                    },
                ));
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
    }
}

/// Derive the JSON discriminator value for a variant, applying `rename_all` or
/// defaulting to the variant name in lowercase (serde's default for enums).
fn variant_discriminator(variant: &alef_core::ir::EnumVariant, rename_all: Option<&str>) -> String {
    if let Some(r) = &variant.serde_rename {
        return r.clone();
    }
    match rename_all {
        Some("snake_case") => heck::ToSnakeCase::to_snake_case(variant.name.as_str()),
        Some("camelCase") => heck::ToLowerCamelCase::to_lower_camel_case(variant.name.as_str()),
        Some("PascalCase") => heck::ToPascalCase::to_pascal_case(variant.name.as_str()),
        Some("SCREAMING_SNAKE_CASE") => heck::ToSnakeCase::to_snake_case(variant.name.as_str()).to_uppercase(),
        Some("kebab-case") => heck::ToKebabCase::to_kebab_case(variant.name.as_str()),
        Some("SCREAMING-KEBAB-CASE") => heck::ToKebabCase::to_kebab_case(variant.name.as_str()).to_uppercase(),
        Some("lowercase") => variant.name.to_lowercase(),
        Some("UPPERCASE") => variant.name.to_uppercase(),
        // serde default for enums: use the variant name as-is (PascalCase).
        // Most enums in this codebase use explicit serde_rename per variant, so
        // this fallback is rarely hit.
        _ => variant.name.clone(),
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
            use alef_core::ir::PrimitiveType;
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
    out.push_str("    override fun deserialize(\n");
    out.push_str("        parser: com.fasterxml.jackson.core.JsonParser,\n");
    out.push_str("        ctx: com.fasterxml.jackson.databind.DeserializationContext,\n");
    out.push_str("    ): ");
    out.push_str(name);
    out.push_str(" {\n");
    out.push_str("        val node = parser.codec.readTree<com.fasterxml.jackson.databind.node.ObjectNode>(parser)\n");
    out.push_str("        return when (node.get(\"");
    out.push_str(tag_field);
    out.push_str("\")?.asText()) {\n");

    for variant in &en.variants {
        let discriminator = variant_discriminator(variant, en.serde_rename_all.as_deref());
        out.push_str("            \"");
        out.push_str(&discriminator);
        out.push_str("\" -> ");

        if variant.fields.is_empty() {
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push('\n');
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant: `readTreeAsValue` already returns the correct
            // variant subtype — return it directly without wrapping in the variant
            // constructor again.
            let inner_class = kotlin_class_name_for_type(&variant.fields[0].ty);
            out.push_str("ctx.readTreeAsValue(node, ");
            out.push_str(&inner_class);
            out.push_str("::class.java)\n");
        } else {
            // Named-field variant: `readTreeAsValue` returns the correct data class
            // subtype — return it directly.
            out.push_str("ctx.readTreeAsValue(node, ");
            out.push_str(name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("::class.java)\n");
        }
    }

    out.push_str("            else -> throw com.fasterxml.jackson.databind.exc.InvalidFormatException(\n");
    out.push_str("                parser, \"Unknown ");
    out.push_str(name);
    out.push_str(" tag\", node.get(\"");
    out.push_str(tag_field);
    out.push_str("\")?.asText(), ");
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
                TypeRef::String => (
                    "node.isTextual",
                    format!("{name}.{}(node.asText())", variant.name),
                ),
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
                        format!("{name}.{}(ctx.readTreeAsValue(node, {class_name}::class.java))", variant.name),
                    )
                }
                TypeRef::Named(n) => (
                    "node.isObject",
                    format!("{name}.{}(ctx.readTreeAsValue(node, {n}::class.java))", variant.name),
                ),
                _ => {
                    let class_name = kotlin_class_name_for_type(ty);
                    (
                        "node.isObject",
                        format!("{name}.{}(ctx.readTreeAsValue(node, {class_name}::class.java))", variant.name),
                    )
                }
            }
        } else {
            // Struct variant with named fields — JSON must be an object.
            let struct_class = format!("{name}.{}", variant.name);
            (
                "node.isObject",
                format!(
                    "{name}.{}(ctx.readTreeAsValue(node, {struct_class}::class.java))",
                    variant.name
                ),
            )
        };

        out.push_str("        if (");
        out.push_str(condition);
        out.push_str(") return ");
        out.push_str(&inner_expr);
        out.push('\n');
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

pub(crate) fn emit_error_type_with_imports(
    error: &alef_core::ir::ErrorDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
) {
    emit_cleaned_kdoc(out, &error.doc, "");
    out.push_str(&crate::template_env::render(
        "error_sealed_class_header.jinja",
        minijinja::context! {
            name => &error.name,
        },
    ));
    for variant in &error.variants {
        if variant.is_unit {
            out.push_str(&crate::template_env::render(
                "error_object_variant.jinja",
                minijinja::context! {
                    name => &variant.name,
                    parent_name => &error.name,
                    message => variant.message_template.as_deref().unwrap_or(&variant.name),
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "variant_data_class_header.jinja",
                minijinja::context! {
                    name => &variant.name,
                },
            ));
            for (idx, f) in variant.fields.iter().enumerate() {
                let ty_str = kotlin_type_with_string_imports(&f.ty, f.optional, imports);
                let name = kotlin_field_name(&f.name, idx);
                // `message` on Throwable subclasses must be `override` because
                // `kotlin.Throwable` declares `open val message: String?`.
                let modifier = if name == "message" { "override " } else { "" };
                let comma = if idx + 1 == variant.fields.len() { "" } else { "," };
                out.push_str(&crate::template_env::render(
                    "error_field.jinja",
                    minijinja::context! {
                        modifier => modifier,
                        name => &name,
                        type => &ty_str,
                        comma => comma,
                    },
                ));
            }
            let message_template = variant.message_template.as_deref().unwrap_or(&variant.name);
            out.push_str(&crate::template_env::render(
                "error_variant_close.jinja",
                minijinja::context! {
                    parent_name => &error.name,
                    message => message_template,
                },
            ));
        }
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
    // companion (e.g. `LiterLlm.createClient(...)` returns Java `DefaultClient`
    // but the Kotlin facade must hand back the coroutine-friendly Kotlin
    // `DefaultClient` wrapper).
    let returns_client_type = match &f.return_type {
        TypeRef::Named(n) => client_type_names.contains(n.as_str()),
        _ => false,
    };

    out.push_str(&crate::template_env::render(
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
        out.push_str(&crate::template_env::render(
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

// ---------------------------------------------------------------------------
// Type-string rendering (&'static str imports variant — used internally for enums)
// ---------------------------------------------------------------------------

fn kotlin_type(ty: &TypeRef, optional: bool, imports: &mut BTreeSet<&'static str>) -> String {
    let inner = render_type_ref_with_imports(ty, imports);
    if optional { format!("{inner}?") } else { inner }
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
    use alef_core::ir::{CoreWrapper, EnumVariant, FieldDef};

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
        }
    }

    fn make_variant(name: &str, serde_rename: Option<&str>, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            doc: String::new(),
            is_default: false,
            serde_rename: serde_rename.map(str::to_string),
            is_tuple: false,
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
        emit_enum(&en, &mut out);
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
        // Fix: the variant value is returned directly from readTreeAsValue — the
        // variant constructor must NOT be emitted around readTreeAsValue because
        // readTreeAsValue already returns the correct variant subtype.
        assert!(
            out.contains("\"system\" -> ctx.readTreeAsValue(node, SystemMessage::class.java)"),
            "tagged deserializer must return readTreeAsValue result directly (no variant-constructor wrap); got:\n{out}",
        );
        assert!(
            !out.contains("Message.System(ctx.readTreeAsValue"),
            "tagged deserializer must NOT double-wrap via variant constructor; got:\n{out}",
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
        emit_enum(&en, &mut out);
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
            "OcrDocument",
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
                        make_field(
                            "media_type",
                            TypeRef::Named("MediaType".to_string()),
                        ),
                    ],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out);

        // Must return readTreeAsValue directly — no `OcrDocument.Base64(...)` wrap.
        assert!(
            out.contains("\"base64\" -> ctx.readTreeAsValue(node, OcrDocument.Base64::class.java)"),
            "tagged deserializer must return readTreeAsValue directly for named-field variant; got:\n{out}",
        );
        assert!(
            !out.contains("OcrDocument.Base64(ctx.readTreeAsValue"),
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
        emit_enum(&en, &mut out);

        assert!(
            out.contains("\"text\" -> ctx.readTreeAsValue(node, TextContent::class.java)"),
            "tagged deserializer must return readTreeAsValue directly for newtype variant; got:\n{out}",
        );
        assert!(
            !out.contains("ContentPart.Text(ctx.readTreeAsValue"),
            "tagged deserializer must NOT wrap readTreeAsValue result in variant constructor; got:\n{out}",
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
                    vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::Named("ContentPart".to_string()))))],
                ),
            ],
        );
        let mut out = String::new();
        emit_enum(&en, &mut out);

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
        emit_enum(&en, &mut out);
        assert!(
            !out.contains("@JsonDeserialize") && !out.contains("Deserializer"),
            "unit-only enum must not emit a deserializer; got:\n{out}",
        );
        assert!(
            out.contains("enum class FinishReason"),
            "must emit enum class; got:\n{out}"
        );
    }
}
