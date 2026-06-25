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
            // Serializable types that are used as nested fields in other types benefit from
            // having a Builder so Jackson can properly deserialize them with correct defaults.
            // Examples: PreprocessingOptions (used in ParseOptions), metadata structures.
            // Even if has_default is false (due to manual impl Default not being detected),
            // we should still emit a Builder for has_serde types to ensure proper deserialization.
            if typ.has_serde {
                // Serializable types always get a builder in Auto mode for proper nested deserialization
                return true;
            }

            // First, only emit if the type has defaults (canonical condition for builder emission).
            if !typ.has_default {
                return false;
            }

            let visible_fields: Vec<_> = binding_fields(&typ.fields).collect();
            let field_count = visible_fields.len();

            // A `#[serde(flatten)]` field on a `serde_json::Value` type requires
            // `@JsonAnySetter` to absorb unknown sibling keys at deserialize-time.
            // That annotation can only live on a builder setter method — it cannot
            // appear on a record component.  Force builder emission for any type
            // that carries such a field, regardless of the Auto field-count thresholds.
            if visible_fields
                .iter()
                .any(|f| f.serde_flatten && matches!(&f.ty, TypeRef::Json))
            {
                return true;
            }

            // Auto: emit if field count >= 8, OR (has complex field AND count >= 5).
            if field_count >= BUILDER_AUTO_THRESHOLD {
                return true;
            }

            // Check for complex fields when count is 5-7.
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

    // Annotation tells Jackson to use this builder when deserializing the record.
    // Builder defaults (e.g., enabled=true) are applied during deserialization.
    // Explicitly specify buildMethodName="build" to ensure Jackson calls the build() method.
    body.push_str("    /** Jackson builder for ");
    body.push_str(&typ.name);
    body.push_str(" deserialization. */\n");
    body.push_str("    @com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    body.push_str("    @JsonPOJOBuilder(withPrefix = \"with\", buildMethodName = \"build\")\n");
    body.push_str("    public static final class Builder {\n");
    body.push('\n');

    // Generate field declarations with defaults (8-space indent — nested inside record)
    for field in binding_fields(&typ.fields) {
        let field_name = safe_java_field_name(&field.name);

        // Skip unnamed tuple fields (name is "_0", "_1", "0", "1", etc.) — Java requires named fields
        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        let visitor_trait_name =
            options_field_bridge_trait_name(typ.name.as_str(), field.name.as_str(), &field.ty, trait_bridges);
        let is_visitor_field = visitor_trait_name.is_some();

        // `#[serde(flatten)]` on a `serde_json::Value` field — store as
        // `java.util.HashMap<String, Object>` so the builder's matching
        // `@JsonAnySetter` method can accumulate sibling fields. The record's
        // accessor returns the same `java.util.Map<String, Object>` view.
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);

        // Duration maps to primitive `long` in the public record, but in builder
        // classes we use boxed `Long` so that `null` can represent "not set".
        // Similarly, non-optional fields with #[serde(default)] use boxed types so that
        // `null` can represent "not set" in the builder, allowing Rust's serde defaults to apply.
        let has_serde_default = is_serde_default_marker(field.default.as_deref());

        // Resolve field type, replacing unknown types with Json (→ JsonNode in Java)
        let resolved_field_ty = resolve_field_type(&field.ty, visible_type_names);

        // For optional IR fields whose TypeRef does NOT carry the Optional wrapper
        // (e.g. extractor recorded `ty: String, optional: true`), the record uses the
        // `@Nullable T` convention rather than `Optional<T>`. The Builder field must
        // match: boxed T with null default, NOT `String foo = Optional.empty();` which
        // is a type/value mismatch (uncompilable).
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
            // Optional IR field whose type was already unwrapped: emit boxed T so null is valid.
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
            // The visitor field is wrapped in Optional<Visitor> regardless of the IR's
            // optionality, so its default has to be Optional.empty() to match the type.
            "Optional.empty()".to_string()
        } else if is_flattened_json {
            // Flatten field: live `HashMap` accumulator that the @JsonAnySetter
            // builder method (emitted later) writes into.
            "new java.util.HashMap<>()".to_string()
        } else if field_is_optional_in_binding {
            // Optional IR field stored as boxed @Nullable T: default to null (matches field_type).
            "null".to_string()
        } else if field.optional {
            // For fields where the TypeRef itself wraps Optional, default Optional.empty() / Optional.of(value).
            // The serde-default markers (bare `/* serde(default) */` or the named
            // `serde(default = "path")` form) are signal values set by the extractor
            // when a field carries #[serde(default)] — they must NOT be emitted as a
            // Java expression. Treat them as "no real default, use Optional.empty()".
            if let Some(default) = &field.default
                && !is_serde_default_marker(Some(default))
            {
                // If there's an explicit default, wrap it in Optional.of()
                format_optional_value(&field.ty, default)
            } else {
                // If no default, use Optional.empty()
                "Optional.empty()".to_string()
            }
        } else {
            // For non-Optional fields, use regular defaults.
            // Same serde-default-marker filter as above — fall through to the
            // type-driven match arm so Vec emits `List.of()`, Map emits `Map.of()`, etc.
            if let Some(default) = &field.default
                && !is_serde_default_marker(Some(default))
            {
                default.clone()
            } else if is_serde_default_marker(field.default.as_deref()) {
                // Field has #[serde(default)]: special handling per type.
                if matches!(&field.ty, TypeRef::Named(_)) {
                    // Non-optional enum field with #[serde(default)].
                    // The Rust side will deserialize a missing field using Rust's Default trait,
                    // which means Jackson must also initialize the Builder field to a valid enum.
                    // Consult the enum_defaults map to find the correct default variant.
                    // For sealed interfaces (TypeDef-based enums), emit `new EnumName.Variant()` only
                    // if the variant has zero fields (is_zero_field=true). Variants with fields
                    // cannot be instantiated without arguments, so default to null.
                    // For traditional enums (EnumDef), emit `EnumName.Variant` (static reference).
                    match &field.ty {
                        TypeRef::Named(name) => {
                            enum_defaults
                                .get(name.as_str())
                                .map(|variant_meta| {
                                    let variant_name = &variant_meta.variant_name;
                                    // Check if this is a sealed interface (TypeDef-based enum in Java)
                                    if sealed_interface_names.contains(name.as_str()) {
                                        // Sealed interface: instantiate with `new` only if variant has zero fields.
                                        // Sealed interface record variants with fields cannot be instantiated
                                        // without arguments, so default to null and rely on Jackson's
                                        // @JsonInclude(NON_ABSENT) to omit the field, letting Rust's serde
                                        // apply its default_* function.
                                        if variant_meta.is_zero_field {
                                            format!("new {name}.{variant_name}()")
                                        } else {
                                            // Variant has fields: cannot instantiate without args
                                            "null".to_string()
                                        }
                                    } else {
                                        // Traditional enum: static reference
                                        format!("{name}.{variant_name}")
                                    }
                                })
                                .unwrap_or_else(|| {
                                    // For unknown enums or enums with no variants, default to null
                                    // and hope Jackson sets it (shouldn't happen with valid input).
                                    "null".to_string()
                                })
                        }
                        _ => "null".to_string(),
                    }
                } else {
                    // Non-optional, non-enum field with #[serde(default)].
                    // Use null as the builder default. With @JsonInclude(NON_ABSENT) at the class
                    // level, null fields are omitted from the JSON sent to Rust's serde, which then
                    // applies the Rust default (e.g., (1, 3) for a tuple, empty vec for Vec, etc.).
                    // This prevents round-trip mismatches where Jackson initializes the field to
                    // List.of() for a Vec, but Rust expects a tuple or other collection type.
                    "null".to_string()
                }
            } else {
                match &field.ty {
                    TypeRef::Path => {
                        // Path is an interface (java.nio.file.Path) with no public constructor.
                        // Default to null — Jackson's builder will only set it if present in JSON.
                        "null".to_string()
                    }
                    TypeRef::String | TypeRef::Char => {
                        // Use typed_default (from Rust's impl Default) if available.
                        // This ensures char fields (e.g. strong_em_symbol: '*') default
                        // to a valid single-character string rather than "" which serde
                        // cannot deserialize as char.
                        match &field.typed_default {
                            Some(DefaultValue::StringLiteral(s)) => {
                                // Escape Java string literal: backslash, quote, and the
                                // common control chars so newlines/tabs become valid
                                // Java escapes rather than embedded raw characters
                                // (which fail Java's single-line string lexer).
                                let escaped = s
                                    .replace('\\', "\\\\")
                                    .replace('"', "\\\"")
                                    .replace('\n', "\\n")
                                    .replace('\r', "\\r")
                                    .replace('\t', "\\t");
                                format!("\"{escaped}\"")
                            }
                            _ => "\"\"".to_string(),
                        }
                    }
                    TypeRef::Json => "null".to_string(),
                    TypeRef::Bytes => "new byte[0]".to_string(),
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::Bool => {
                            // Use typed_default from the extracted impl Default block.
                            // This correctly handles any type where a field defaults to true
                            // (e.g. ProcessConfig.structure, ParseOptions.autolinks).
                            match &field.typed_default {
                                Some(DefaultValue::BoolLiteral(b)) => b.to_string(),
                                _ => "false".to_string(),
                            }
                        }
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

        // Emit `@JsonProperty(<wire-name>)` only when the Java field name differs from
        // the wire name or serde explicitly renamed the field.
        let wire_name: Option<String> = if is_flattened_json {
            // Flatten fields have no single wire name — the matching
            // `@JsonAnySetter` setter intercepts every unknown sibling field.
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

        // Add @Nullable for fields that store a plain boxed value with null as the "not set"
        // sentinel. Optional<T>-backed fields keep Optional.empty() as their sentinel instead.
        let needs_nullable_annotation = !is_visitor_field
            && (field_is_optional_in_binding
                || matches!(resolved_field_ty, TypeRef::Duration)
                || (has_serde_default && !matches!(resolved_field_ty, TypeRef::Optional(_))));

        body.push_str("        ");
        // For fully-qualified types (e.g., java.nio.file.Path), @Nullable must appear
        // at the simple-name segment per Java spec:
        //   wrong:   @Nullable java.nio.file.Path
        //   right:   java.nio.file.@Nullable Path
        // For unqualified types, the leading-position annotation is fine.
        let nullable_at_leading_pos = needs_nullable_annotation && !field_type.contains('.');
        if nullable_at_leading_pos {
            body.push_str("@Nullable ");
        }
        body.push_str("private ");
        if needs_nullable_annotation && !nullable_at_leading_pos {
            // Fully-qualified type: insert @Nullable at the last package boundary.
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

        // Emit field initializer only when it's not Java's default.
        // Java defaults: null for references, 0 for numeric, false for boolean, etc.
        // Suppress redundant initializers to fix PMD RedundantFieldInitializer rule.
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

    // Generate withXxx() methods (8-space indent for method body, 12 for the body statements)
    for field in binding_fields(&typ.fields) {
        // Skip unnamed tuple fields (name is "_0", "_1", "0", "1", etc.) — Java requires named fields
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

        // Resolve field type, replacing unknown types with Json (→ JsonNode in Java)
        let resolved_field_ty = resolve_field_type(&field.ty, visible_type_names);

        // Builders store the visitor as Optional<Visitor> for null-safe chaining, but
        // expose `withVisitor(Visitor)` to keep the user-facing API ergonomic — callers
        // should not have to write `Optional.of(visitor)` themselves.
        let field_type = if is_visitor_field {
            visitor_trait_name.expect("visitor field type is resolved")
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if matches!(resolved_field_ty, TypeRef::Optional(_)) {
            // Use @Nullable annotation in the setter signature, not Optional<T>.
            // This matches Java best practices and the record field annotation pattern.
            java_boxed_type(&resolved_field_ty).to_string()
        } else if has_serde_default || matches!(resolved_field_ty, TypeRef::Duration) {
            // Non-optional fields with #[serde(default)] or Duration must box the parameter type
            // so that null can represent "not set" when Jackson deserializes.
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
            // The regular `with<Field>(Map)` setter must not bind to a wire
            // field of the same name (e.g. an actual `content` array field
            // would be miscast as a `Map`). `@JsonIgnore` prevents Jackson
            // from picking it up; the matching `@JsonAnySetter` below
            // intercepts every flattened sibling field instead.
            body.push_str("        @com.fasterxml.jackson.annotation.JsonIgnore\n");
        } else {
            // Jackson's BuilderBasedDeserializer requires @JsonProperty on every
            // setter method to map JSON fields to setters. Without it, Jackson will
            // not call the setter, leaving the builder field at its default value.
            // Always emit the wire name (which may be identical to the field name
            // if there's no serde rename) so Jackson can match it deterministically.
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
        // Java requires type-use annotations on a qualified name to appear at the
        // simple-name segment, not before the package prefix:
        //   wrong:   `@Nullable java.nio.file.Path`
        //   right:   `java.nio.file.@Nullable Path`
        // Match the record-field declaration logic above (see `nullable_at_leading_pos`).
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
        // Match the Builder field's actual type: if it is stored as Optional<T>, wrap;
        // if it is stored as plain @Nullable T (field_is_optional_in_binding), assign directly.
        let field_stored_as_optional = is_visitor_field
            || (field.optional && matches!(resolve_field_type(&field.ty, visible_type_names), TypeRef::Optional(_)));
        if field_stored_as_optional {
            // Builder stores optional fields as Optional<T> (see field declaration above);
            // the setter accepts a plain @Nullable T for ergonomics, so wrap here.
            body.push_str("            this.");
            body.push_str(&field_name);
            body.push_str(" = Optional.ofNullable(value);\n");
        } else {
            // For non-optional fields with #[serde(default)] or Duration, we also accept
            // @Nullable to support Jackson's null injection when fields are absent.
            body.push_str("            this.");
            body.push_str(&field_name);
            body.push_str(" = value;\n");
        }
        body.push_str("            return this;\n");
        body.push_str("        }\n");
        body.push('\n');

        // Flatten field: emit `@JsonAnySetter` so Jackson absorbs unknown
        // sibling fields into the map during deserialization. Without this,
        // any field not declared on the builder triggers
        // `Unrecognized field "<name>" not marked as ignorable`.
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

    // Generate build() method
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
        // Match the Builder field's actual type: call .orElse(null) only when the
        // backing field is stored as Optional<T>; for plain @Nullable T storage
        // (field_is_optional_in_binding) the field IS already nullable T.
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

#[cfg(test)]
mod tests {
    use crate::backends::java::gen_bindings::helpers::is_serde_default_marker;

    #[test]
    fn is_serde_default_marker_bare_marker() {
        assert!(is_serde_default_marker(Some("/* serde(default) */")));
    }

    #[test]
    fn is_serde_default_marker_function_path() {
        assert!(is_serde_default_marker(Some("serde(default = \"default_true\")")));
    }

    #[test]
    fn is_serde_default_marker_none() {
        assert!(!is_serde_default_marker(None));
    }

    #[test]
    fn is_serde_default_marker_explicit_literal_not_serde() {
        assert!(!is_serde_default_marker(Some("true")));
        assert!(!is_serde_default_marker(Some("0")));
        assert!(!is_serde_default_marker(Some("null")));
    }
}
