use crate::backends::java::type_map::{java_boxed_type, java_type};
use crate::codegen::naming::to_class_name;
use crate::codegen::shared::binding_fields;
use crate::core::config::{JavaBuilderMode, TraitBridgeConfig};
use crate::core::ir::{DefaultValue, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;

use super::shared::{is_options_field_bridge, options_field_bridge_trait_name, resolve_field_type};
use crate::backends::java::gen_bindings::helpers::{
    format_optional_value, is_serde_default_marker, safe_java_field_name,
};

pub(super) const BUILDER_AUTO_THRESHOLD: usize = 8;

/// Check if a field type is complex (nested object, collection of complex types, etc.).
fn is_complex_field_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Json
    )
}

/// Decide whether to emit a builder for this type based on its field count and configuration.
pub(super) fn should_emit_builder(typ: &TypeDef, builder_mode: JavaBuilderMode) -> bool {
    match builder_mode {
        JavaBuilderMode::Always => true,
        JavaBuilderMode::Never => false,
        JavaBuilderMode::Auto => {
            if typ.has_serde {
                return true;
            }

            if !typ.has_default {
                return false;
            }

            let visible_fields: Vec<_> = binding_fields(&typ.fields).collect();
            let field_count = visible_fields.len();

            // A `#[serde(flatten)]` field on a `serde_json::Value` type requires
            if visible_fields
                .iter()
                .any(|f| f.serde_flatten && matches!(&f.ty, TypeRef::Json))
            {
                return true;
            }

            if field_count >= BUILDER_AUTO_THRESHOLD {
                return true;
            }

            if field_count >= 5 {
                return visible_fields.iter().any(|f| is_complex_field_type(&f.ty));
            }

            false
        }
    }
}

/// Emit a Javadoc comment block into `out` at the given indentation level.
///
/// `indent` is the leading whitespace prepended to each line (e.g. `""` for
/// top-level declarations, `"    "` for class members).  Does nothing when
/// `doc` is empty.
/// Generate the Jackson POJO builder as a nested static class body, indented with 4 spaces.
///
/// The returned string is meant to be inlined inside the owning record class body — it does NOT
/// include a file header or import block.  All imports required by the builder body (e.g.
/// `@JsonPOJOBuilder`, `@JsonProperty`, `Optional`) must be added by the caller
/// (`gen_record_type`) to the combined file's import block.
pub(super) fn gen_builder_nested_class(
    typ: &TypeDef,
    trait_bridges: &[TraitBridgeConfig],
    enum_defaults: &ahash::AHashMap<String, crate::extract::default_value_for_enum::DefaultEnumVariant>,
    sealed_interface_names: &AHashSet<String>,
    visible_type_names: &std::collections::HashSet<&str>,
) -> String {
    let mut body = String::with_capacity(2048);

    body.push_str("    /** Jackson builder for ");
    body.push_str(&typ.name);
    body.push_str(" deserialization. */\n");
    body.push_str("    @com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    body.push_str("    @JsonPOJOBuilder(withPrefix = \"with\", buildMethodName = \"build\")\n");
    body.push_str("    public static final class Builder {\n");
    body.push('\n');

    for field in binding_fields(&typ.fields) {
        let field_name = safe_java_field_name(&field.name);

        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        let visitor_trait_name =
            options_field_bridge_trait_name(typ.name.as_str(), field.name.as_str(), &field.ty, trait_bridges);
        let is_visitor_field = visitor_trait_name.is_some();

        // `#[serde(flatten)]` on a `serde_json::Value` field — store as
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);

        // Similarly, non-optional fields with #[serde(default)] use boxed types so that
        let has_serde_default = is_serde_default_marker(field.default.as_deref());

        let resolved_field_ty = resolve_field_type(&field.ty, visible_type_names);

        let field_is_optional_in_binding = field.optional && !matches!(resolved_field_ty, TypeRef::Optional(_));
        let field_type = if is_visitor_field {
            format!(
                "Optional<{}>",
                visitor_trait_name.expect("visitor field type is resolved")
            )
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if matches!(resolved_field_ty, TypeRef::Optional(_)) {
            format!("Optional<{}>", java_boxed_type(&resolved_field_ty))
        } else if field_is_optional_in_binding {
            java_boxed_type(&resolved_field_ty).to_string()
        } else if matches!(resolved_field_ty, TypeRef::Duration) {
            java_boxed_type(&resolved_field_ty).to_string()
        } else if has_serde_default {
            // Non-optional fields with #[serde(default)] use boxed types so null can represent "not set"
            java_boxed_type(&resolved_field_ty).to_string()
        } else {
            java_type(&resolved_field_ty).to_string()
        };

        let default_value = if is_visitor_field {
            "Optional.empty()".to_string()
        } else if is_flattened_json {
            "new java.util.HashMap<>()".to_string()
        } else if field_is_optional_in_binding {
            "null".to_string()
        } else if field.optional {
            // when a field carries #[serde(default)] — they must NOT be emitted as a
            if let Some(default) = &field.default
                && !is_serde_default_marker(Some(default))
            {
                format_optional_value(&field.ty, default)
            } else {
                "Optional.empty()".to_string()
            }
        } else {
            if let Some(default) = &field.default
                && !is_serde_default_marker(Some(default))
            {
                default.clone()
            } else if is_serde_default_marker(field.default.as_deref()) {
                // Field has #[serde(default)]: special handling per type.
                if matches!(&field.ty, TypeRef::Named(_)) {
                    // Non-optional enum field with #[serde(default)].
                    match &field.ty {
                        TypeRef::Named(name) => enum_defaults
                            .get(name.as_str())
                            .map(|variant_meta| {
                                let variant_name = &variant_meta.variant_name;
                                if sealed_interface_names.contains(name.as_str()) {
                                    if variant_meta.is_zero_field {
                                        format!("new {name}.{variant_name}()")
                                    } else {
                                        "null".to_string()
                                    }
                                } else {
                                    format!("{name}.{variant_name}")
                                }
                            })
                            .unwrap_or_else(|| "null".to_string()),
                        _ => "null".to_string(),
                    }
                } else {
                    // Non-optional, non-enum field with #[serde(default)].
                    "null".to_string()
                }
            } else {
                match &field.ty {
                    TypeRef::Path => "null".to_string(),
                    TypeRef::String | TypeRef::Char => match &field.typed_default {
                        Some(DefaultValue::StringLiteral(s)) => {
                            let escaped = s
                                .replace('\\', "\\\\")
                                .replace('"', "\\\"")
                                .replace('\n', "\\n")
                                .replace('\r', "\\r")
                                .replace('\t', "\\t");
                            format!("\"{escaped}\"")
                        }
                        _ => "\"\"".to_string(),
                    },
                    TypeRef::Json => "null".to_string(),
                    TypeRef::Bytes => "new byte[0]".to_string(),
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::Bool => match &field.typed_default {
                            Some(DefaultValue::BoolLiteral(b)) => b.to_string(),
                            _ => "false".to_string(),
                        },
                        PrimitiveType::F32 => "0.0f".to_string(),
                        PrimitiveType::F64 => "0.0".to_string(),
                        _ => "0".to_string(),
                    },
                    TypeRef::Vec(_) => "List.of()".to_string(),
                    TypeRef::Map(_, _) => "Map.of()".to_string(),
                    TypeRef::Optional(_) => "Optional.empty()".to_string(),
                    TypeRef::Duration => "null".to_string(),
                    _ => "null".to_string(),
                }
            }
        };

        let wire_name: Option<String> = if is_flattened_json {
            None
        } else {
            let wire = field.serde_rename.clone().unwrap_or_else(|| field.name.clone());
            if field.serde_rename.is_some() || field_name != wire {
                Some(wire)
            } else {
                None
            }
        };
        if let Some(wire) = wire_name {
            body.push_str("        @JsonProperty(\"");
            body.push_str(&wire);
            body.push_str("\")\n");
        }

        let needs_nullable_annotation = !is_visitor_field
            && (field_is_optional_in_binding
                || matches!(resolved_field_ty, TypeRef::Duration)
                || (has_serde_default && !matches!(resolved_field_ty, TypeRef::Optional(_))));

        body.push_str("        ");
        let nullable_at_leading_pos = needs_nullable_annotation && !field_type.contains('.');
        if nullable_at_leading_pos {
            body.push_str("@Nullable ");
        }
        body.push_str("private ");
        if needs_nullable_annotation && !nullable_at_leading_pos {
            if let Some(idx) = field_type.rfind('.') {
                let (pkg, simple) = field_type.split_at(idx);
                let simple = simple.trim_start_matches('.');
                body.push_str(pkg);
                body.push_str(".@Nullable ");
                body.push_str(simple);
                body.push(' ');
                body.push_str(&field_name);
            } else {
                body.push_str("@Nullable ");
                body.push_str(&field_type);
                body.push(' ');
                body.push_str(&field_name);
            }
        } else {
            body.push_str(&field_type);
            body.push(' ');
            body.push_str(&field_name);
        }

        let is_redundant_default = default_value == "null"
            || default_value == "false"
            || default_value == "0"
            || default_value == "0L"
            || default_value == "0.0"
            || default_value == "0.0f"
            || default_value == "\"\"";
        if !is_redundant_default {
            body.push_str(" = ");
            body.push_str(&default_value);
        }
        body.push_str(";\n");
    }

    body.push('\n');

    for field in binding_fields(&typ.fields) {
        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        let field_name = safe_java_field_name(&field.name);
        let field_name_pascal = to_class_name(&field.name);
        let visitor_trait_name =
            options_field_bridge_trait_name(typ.name.as_str(), field.name.as_str(), &field.ty, trait_bridges);
        let is_visitor_field = visitor_trait_name.is_some();
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);
        let has_serde_default = is_serde_default_marker(field.default.as_deref());

        let resolved_field_ty = resolve_field_type(&field.ty, visible_type_names);

        let field_type = if is_visitor_field {
            visitor_trait_name.expect("visitor field type is resolved")
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if matches!(resolved_field_ty, TypeRef::Optional(_)) {
            java_boxed_type(&resolved_field_ty).to_string()
        } else if has_serde_default || matches!(resolved_field_ty, TypeRef::Duration) {
            // Non-optional fields with #[serde(default)] or Duration must box the parameter type
            java_boxed_type(&resolved_field_ty).to_string()
        } else {
            java_type(&resolved_field_ty).to_string()
        };

        body.push_str("        /** Sets the ");
        body.push_str(&field_name);
        body.push_str(" field. */\n");
        let setter_wire_name: Option<String> = if is_flattened_json {
            None
        } else {
            let wire = field.serde_rename.clone().unwrap_or_else(|| field.name.clone());
            if field.serde_rename.is_some() || field_name != wire {
                Some(wire)
            } else {
                None
            }
        };
        if is_flattened_json {
            body.push_str("        @com.fasterxml.jackson.annotation.JsonIgnore\n");
        } else {
            let wire = if let Some(w) = &setter_wire_name {
                w.clone()
            } else {
                field.serde_rename.clone().unwrap_or_else(|| field.name.clone())
            };
            body.push_str("        @JsonProperty(\"");
            body.push_str(&wire);
            body.push_str("\")\n");
        }
        body.push_str("        public Builder with");
        body.push_str(&field_name_pascal);
        body.push_str("(final ");
        let needs_nullable_on_param =
            (field.optional || has_serde_default || matches!(field.ty, TypeRef::Duration)) && !is_visitor_field;
        if needs_nullable_on_param {
            if let Some(idx) = field_type.rfind('.') {
                let (pkg, simple) = field_type.split_at(idx);
                let simple = simple.trim_start_matches('.');
                body.push_str(pkg);
                body.push_str(".@Nullable ");
                body.push_str(simple);
            } else {
                body.push_str("@Nullable ");
                body.push_str(&field_type);
            }
        } else {
            body.push_str(&field_type);
        }
        body.push_str(" value) {\n");
        let field_stored_as_optional = is_visitor_field
            || (field.optional && matches!(resolve_field_type(&field.ty, visible_type_names), TypeRef::Optional(_)));
        if field_stored_as_optional {
            body.push_str("            this.");
            body.push_str(&field_name);
            body.push_str(" = Optional.ofNullable(value);\n");
        } else {
            // For non-optional fields with #[serde(default)] or Duration, we also accept
            body.push_str("            this.");
            body.push_str(&field_name);
            body.push_str(" = value;\n");
        }
        body.push_str("            return this;\n");
        body.push_str("        }\n");
        body.push('\n');

        if is_flattened_json {
            body.push_str("        /** Absorbs unknown sibling fields (serde flatten). */\n");
            body.push_str("        @com.fasterxml.jackson.annotation.JsonAnySetter\n");
            body.push_str("        public Builder ");
            body.push_str(&field_name);
            body.push_str("Entry(final String key, final Object value) {\n");
            body.push_str("            this.");
            body.push_str(&field_name);
            body.push_str(".put(key, value);\n");
            body.push_str("            return this;\n");
            body.push_str("        }\n");
            body.push('\n');
        }
    }

    body.push_str("        /** Constructs a ");
    body.push_str(&typ.name);
    body.push_str(" instance from the builder's current state. */\n");
    body.push_str("        public ");
    body.push_str(&typ.name);
    body.push_str(" build() {\n");
    body.push_str("            return new ");
    body.push_str(&typ.name);
    body.push_str("(\n");
    let non_tuple_fields: Vec<_> = binding_fields(&typ.fields)
        .filter(|f| {
            !(f.name.starts_with('_') && f.name[1..].chars().all(|c| c.is_ascii_digit())
                || f.name.chars().next().is_none_or(|c| c.is_ascii_digit()))
        })
        .collect();
    for (i, field) in non_tuple_fields.iter().enumerate() {
        let field_name = safe_java_field_name(&field.name);
        let comma = if i < non_tuple_fields.len() - 1 { "," } else { "" };
        let is_visitor_field =
            is_options_field_bridge(typ.name.as_str(), field.name.as_str(), &field.ty, trait_bridges);
        let field_stored_as_optional = is_visitor_field
            || (field.optional && matches!(resolve_field_type(&field.ty, visible_type_names), TypeRef::Optional(_)));
        if field_stored_as_optional {
            body.push_str("                ");
            body.push_str(&field_name);
            body.push_str(".orElse(null)");
            body.push_str(comma);
            body.push('\n');
        } else {
            body.push_str("                ");
            body.push_str(&field_name);
            body.push_str(comma);
            body.push('\n');
        }
    }
    body.push_str("            );\n");
    body.push_str("        }\n");

    body.push_str("    }\n");

    body
}
