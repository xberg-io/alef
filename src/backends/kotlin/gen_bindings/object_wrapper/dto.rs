use crate::core::ir::{TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::types::{escape_kotlin_string, fits_single_line, kotlin_field_default, kotlin_type_with_string_imports};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name;

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
        out.push_str(&crate::backends::kotlin::template_env::render(
            "data_class_inline.jinja",
            minijinja::context! {
                prefix => prefix,
                fields => field_strings.join(", "),
            },
        ));
    } else {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "data_class_header_only.jinja",
            minijinja::context! {
                prefix => prefix,
            },
        ));
        for (idx, (field, field_str)) in ty.fields.iter().zip(field_strings.iter()).enumerate() {
            emit_cleaned_kdoc(out, &field.doc, "    ");
            // Emit @JsonProperty when the Rust field carries #[serde(rename = "...")]
            // so Jackson maps the wire key to the Kotlin camelCase property name.
            if let Some(rename) = &field.serde_rename {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "json_property_annotation.jinja",
                    minijinja::context! {
                        indent => "    ",
                        value => escape_kotlin_string(rename),
                    },
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
            out.push_str(&crate::backends::kotlin::template_env::render(
                "data_class_field_line.jinja",
                minijinja::context! {
                    indent => "    ",
                    field => field_str,
                },
            ));
        }
        out.push_str(&crate::backends::kotlin::template_env::render(
            "data_class_close.jinja",
            minijinja::context! {
                indent => "",
                suffix => "",
            },
        ));
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
