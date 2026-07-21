use crate::core::ir::{TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::types::{escape_kotlin_string, fits_single_line, kotlin_field_default, kotlin_type_with_string_imports};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::kotlin_field_name;
use heck::ToLowerCamelCase;

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

    // Enumerate before filtering so `original_idx` stays stable for field naming, then drop
    // `binding_excluded` fields entirely — matching every other backend. Keeping them (as the
    // legacy nullable `= null` branch did) leaks force-controlled / internal knobs into the public
    // DTO; a `[crates.exclude].fields` entry must remove the field, not just null its type.
    let visible_fields: Vec<(usize, &crate::core::ir::FieldDef)> = ty
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.binding_excluded)
        .collect();

    let field_sealed_annotations: Vec<Option<String>> = visible_fields
        .iter()
        .map(|(_, f)| sealed_class_field_annotation(&f.ty, sealed_class_names))
        .collect();

    let has_field_docs = visible_fields.iter().any(|(_, f)| !f.doc.is_empty());
    let has_field_annotations = visible_fields.iter().any(|(_, f)| f.serde_rename.is_some())
        || field_sealed_annotations.iter().any(Option::is_some);
    // Detect `#[serde(flatten)]` fields. In Rust these collect all unknown
    let has_flatten_field = visible_fields.iter().any(|(_, f)| f.serde_flatten);

    let mut field_strings: Vec<String> = Vec::with_capacity(visible_fields.len());
    for (original_idx, field) in visible_fields.iter() {
        let ty_str = kotlin_type_with_string_imports(&field.ty, field.optional, imports);
        let name = kotlin_field_name(&field.name, *original_idx);
        // collections (`#[serde(skip_serializing_if = "...")]`) or skip a
        // field entirely under a feature gate (`#[serde(skip)]`). Without a
        let (effective_ty_str, default_suffix) = if field.serde_flatten {
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
            if default_suffix.contains(".milliseconds") {
                imports.insert("import kotlin.time.Duration.Companion.milliseconds".to_string());
            }
            (ty_str, default_suffix)
        };
        field_strings.push(format!("val {name}: {effective_ty_str}{default_suffix}"));
    }

    use crate::codegen::shared::partition_methods;
    let (instance_methods, _) = partition_methods(&ty.methods);
    let instance_methods: Vec<_> = instance_methods.into_iter().filter(|m| !m.sanitized).collect();
    let has_instance_methods = !instance_methods.is_empty();

    let prefix = format!("data class {}", ty.name);
    let use_single_line = !has_field_docs
        && !has_field_annotations
        && !has_flatten_field
        && !has_instance_methods
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
        for (idx, ((_, field), field_str)) in visible_fields.iter().zip(field_strings.iter()).enumerate() {
            emit_cleaned_kdoc(out, &field.doc, "    ");
            // Emit @JsonProperty when the Rust field carries #[serde(rename = "...")]
            if let Some(rename) = &field.serde_rename {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "json_property_annotation.jinja",
                    minijinja::context! {
                        indent => "    ",
                        value => escape_kotlin_string(rename),
                    },
                ));
            }
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
                suffix => if has_instance_methods { " {" } else { "" },
            },
        ));
    }

    for method in &instance_methods {
        let method_name = heck::AsLowerCamelCase(method.name.as_str()).to_string();
        let return_type_str = kotlin_type_with_string_imports(&method.return_type, false, imports);

        let params_sig: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let ptype = kotlin_type_with_string_imports(&p.ty, p.optional, imports);
                let pname = p.name.to_lower_camel_case();
                format!("{pname}: {ptype}")
            })
            .collect();

        out.push_str("\n    fun ");
        out.push_str(&method_name);
        out.push('(');
        out.push_str(&params_sig.join(", "));
        out.push_str("): ");
        out.push_str(&return_type_str);
        out.push_str(" {\n");
        out.push_str("        throw UnsupportedOperationException(\n");
        out.push_str("            \"");
        out.push_str(&method_name);
        out.push_str(" is not yet bridged via JNI; reconstruct via Builder.\"\n");
        out.push_str("        )\n");
        out.push_str("    }\n");
    }

    if has_instance_methods {
        out.push_str("}\n");
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
