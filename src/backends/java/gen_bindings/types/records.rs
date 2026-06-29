use crate::backends::java::type_map::{java_boxed_type, java_type};
use crate::codegen::shared::binding_fields;
use crate::core::config::{JavaBuilderMode, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{DefaultValue, MethodDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;

use super::builders::{gen_builder_nested_class, should_emit_builder};
use super::shared::{options_field_bridge_trait_name, resolve_field_type};
use crate::backends::java::gen_bindings::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, emit_javadoc, is_serde_default_marker, safe_java_field_name,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_record_type(
    package: &str,
    typ: &TypeDef,
    complex_enums: &AHashSet<String>,
    sealed_unions_with_unwrapped: &AHashSet<String>,
    _lang_rename_all: &str,
    trait_bridges: &[TraitBridgeConfig],
    _main_class: &str,
    builder_mode: JavaBuilderMode,
    enum_defaults: &ahash::AHashMap<String, crate::extract::default_value_for_enum::DefaultEnumVariant>,
    sealed_interface_names: &AHashSet<String>,
    visible_type_names: &std::collections::HashSet<&str>,
) -> String {
    // `fields_joined` holds the comma-separated parameter list used both for the
    // single-line length probe AND for the final single-line emit path — no rebuild.
    // `field_decls` keeps each individual decl so the multi-line emit path can put
    // each on its own line (annotations within a single decl may contain commas,
    // so we cannot split `fields_joined` by ", ").
    let visible_fields: Vec<_> = binding_fields(&typ.fields).collect();
    let mut fields_joined = String::with_capacity(visible_fields.len().saturating_mul(42));
    let mut field_decls: Vec<String> = Vec::with_capacity(visible_fields.len());

    for (i, f) in visible_fields.iter().enumerate() {
        // Complex enums (tagged unions with data) can't be simple Java enums.
        // Use Object for flexible Jackson deserialization.
        let is_complex = matches!(&f.ty, TypeRef::Named(n) if complex_enums.contains(n.as_str()));

        let visitor_trait_name =
            options_field_bridge_trait_name(typ.name.as_str(), f.name.as_str(), &f.ty, trait_bridges);
        let is_visitor_field = visitor_trait_name.is_some();

        // `#[serde(flatten)]` on a `serde_json::Value` field: emit
        // `@JsonAnyGetter Map<String, Object>` so Jackson absorbs unknown
        // sibling fields into the map on read and writes them flat alongside
        // the parent's named fields on write. Mirrors C#'s [JsonExtensionData].
        let is_flattened_json = f.serde_flatten && matches!(&f.ty, TypeRef::Json);

        // Non-optional fields with #[serde(default)] must use boxed types in the record
        // parameter so that null can represent "not set". With @JsonInclude(NON_ABSENT)
        // at the class level, null values are omitted from JSON sent to Rust, letting
        // serde apply its default. The Builder uses the same boxed type, and build()
        // passes null values directly without unboxing (matching the record parameter).
        let has_serde_default = is_serde_default_marker(f.default.as_deref());

        // Resolve field type, replacing unknown types with Json (→ JsonNode in Java)
        let resolved_ty = resolve_field_type(&f.ty, visible_type_names);

        // When IR marks the field optional but the TypeRef is not Optional (extractor
        // stored 'ty: T, optional: true'), still emit boxed T so null is representable.
        // Otherwise primitive types like `long` cannot hold null and auto-unbox NPEs.
        let f_optional_no_wrapper = f.optional && !matches!(resolved_ty, TypeRef::Optional(_));
        let ftype = if is_visitor_field {
            visitor_trait_name.expect("visitor field type is resolved")
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if is_complex {
            "Object".to_string()
        } else if matches!(resolved_ty, TypeRef::Optional(_)) {
            // Java best practice: use @Nullable fields, never Optional in records.
            java_boxed_type(&resolved_ty).to_string()
        } else if f_optional_no_wrapper {
            // Optional IR field whose TypeRef was not Optional — boxed so null is valid.
            java_boxed_type(&resolved_ty).to_string()
        } else if has_serde_default || matches!(resolved_ty, TypeRef::Duration) {
            // Non-optional fields with #[serde(default)] or Duration use boxed types
            // so null can represent "not set" for serde defaults.
            java_boxed_type(&resolved_ty).to_string()
        } else {
            java_type(&resolved_ty).to_string()
        };
        let jname = safe_java_field_name(&f.name);

        // Non-optional List fields: Java initialises them to null when the field is
        // absent from the input JSON. We must NOT serialise that null back to the
        // Rust side — Rust's serde would reject it for a non-optional Vec<T>.
        // @JsonInclude(NON_NULL) at the field level suppresses the null, letting
        // Rust fall back to its serde `default` (empty vec, default value, etc.).
        //
        // When the enclosing record has `@JsonInclude(NON_ABSENT)` (emitted for any
        // serde-aware type), the class-level rule already suppresses null fields,
        // so the field-level annotation is redundant. Keeping it produced lines
        // long enough to bust Checkstyle's 140-char limit after Eclipse spotless
        // reflows record components to a single line.
        let needs_non_null = !f.optional && matches!(&resolved_ty, TypeRef::Vec(_)) && !typ.has_serde;

        // Bytes fields (byte[]), optional or not, must be serialised as a JSON array of
        // integers, not as a base64 string. Jackson's default serialiser for byte[]
        // produces base64, but Rust's serde for Vec<u8> expects [n, n, …].
        // @JsonSerialize(using = ByteArraySerializer.class) overrides the
        // default Jackson behaviour for this field only.
        let needs_bytes_int_serialize = matches!(&resolved_ty, TypeRef::Bytes);

        // Emit `@JsonProperty` in three cases:
        // 1. The field has an explicit `#[serde(rename = "...")]` attribute.
        // 2. The Java camelCase name differs from the snake_case wire name — e.g. `max_tokens`
        //    serialises as `"max_tokens"` on the wire (Rust serde default) but Java converts it
        //    to `maxTokens`. Without `@JsonProperty("max_tokens")`, Jackson serialises using the
        //    Java field name and Rust's serde rejects the camelCase key as unrecognised.
        // 3. A builder is emitted for this type. Jackson's builder-based deserialization requires
        //    @JsonProperty on record fields to correctly map JSON properties to builder setters.
        //
        // The wire name is the explicit serde rename if set, otherwise the original Rust field
        // name (already snake_case per project convention).
        let json_property_name = f.serde_rename.clone().unwrap_or_else(|| f.name.clone());
        let needs_builder = should_emit_builder(typ, builder_mode);
        let has_json_property =
            f.serde_rename.is_some() || jname != json_property_name || (needs_builder && !is_visitor_field);
        // Emit @Nullable for optional fields and for non-optional fields with #[serde(default)]
        // or Duration (which are boxed to allow null = "not set").
        let has_nullable = f.optional || has_serde_default || matches!(resolved_ty, TypeRef::Duration);

        let mut decl = String::new();

        // Fields referencing sealed unions with unwrapped variants need a custom deserializer.
        // When deserializing through a builder, Jackson needs this annotation to use the
        // custom deserializer for the field type. This must come early to be properly
        // recognized by Jackson's polymorphic deserialization.
        // Only apply this annotation if the field type is known (visible).
        let field_type_name = match &resolved_ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n.as_str())
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(type_name) = field_type_name {
            if sealed_unions_with_unwrapped.contains(type_name) {
                decl.push_str("@JsonDeserialize(using = ");
                decl.push_str(type_name);
                decl.push_str("Deserializer.class) ");
            }
        }

        // Visitor field is transient and not serialized to JSON.
        if is_visitor_field {
            decl.push_str("@JsonIgnore ");
        }

        // byte[] fields in input DTOs must round-trip as JSON int arrays so Rust's
        // serde Vec<u8> deserialiser accepts them.
        if needs_bytes_int_serialize {
            decl.push_str("@JsonSerialize(using = ByteArraySerializer.class) ");
        }

        // Java type annotations on a fully-qualified type (e.g. `java.nio.file.Path`)
        // must appear AT the simple-name segment, not before the package prefix:
        //   wrong:   `@Nullable java.nio.file.Path`
        //   right:   `java.nio.file.@Nullable Path`
        // For unqualified types, the leading-position annotation is fine.
        let nullable_at_leading_pos = has_nullable && !ftype.contains('.');
        if nullable_at_leading_pos {
            decl.push_str("@Nullable ");
        }
        if needs_non_null {
            decl.push_str("@JsonInclude(JsonInclude.Include.NON_NULL) ");
        }
        if is_flattened_json {
            // `@JsonAnyGetter` makes Jackson serialize each map entry as a top-level
            // field of the enclosing object. The matching `@JsonAnySetter` on the
            // builder absorbs unknown sibling fields. Combined, they implement the
            // serde flatten semantic for `serde_json::Value` fields.
            decl.push_str("@com.fasterxml.jackson.annotation.JsonAnyGetter ");
        } else if has_json_property && !is_visitor_field {
            decl.push_str("@JsonProperty(\"");
            decl.push_str(&json_property_name);
            decl.push_str("\") ");
        }
        if has_nullable && !nullable_at_leading_pos {
            // Fully-qualified type: insert `@Nullable` at the last package boundary.
            if let Some(idx) = ftype.rfind('.') {
                let (pkg, simple) = ftype.split_at(idx);
                let simple = simple.trim_start_matches('.');
                decl.push_str(pkg);
                decl.push_str(".@Nullable ");
                decl.push_str(simple);
                decl.push(' ');
                decl.push_str(&jname);
            } else {
                decl.push_str("@Nullable ");
                decl.push_str(&ftype);
                decl.push(' ');
                decl.push_str(&jname);
            }
        } else {
            decl.push_str(&ftype);
            decl.push(' ');
            decl.push_str(&jname);
        }

        if i > 0 {
            fields_joined.push_str(", ");
        }
        fields_joined.push_str(&decl);
        field_decls.push(decl);
    }

    // Build the single-line form to check length and scan for imports.
    // Doc strings are intentionally excluded from this check so the threshold
    // stays stable regardless of documentation presence.
    let single_line_len = "public record ".len() + typ.name.len() + 1 + fields_joined.len() + ") { }".len();

    // Build the actual record declaration, splitting across lines if too long.
    let mut record_block = String::new();
    // If no doc exists, provide a default comment describing the type as auto-generated.
    let doc_to_emit = if typ.doc.is_empty() {
        format!("Auto-generated by alef from Rust type {}.", typ.name)
    } else {
        typ.doc.clone()
    };
    emit_javadoc(&mut record_block, &doc_to_emit, "");

    // Check if any fields are binding-excluded (marked with #[cfg_attr(alef, alef(skip))]).
    // When excluded fields are present, add @JsonIgnoreProperties(ignoreUnknown = true)
    // to allow Jackson to deserialize JSON containing those fields without error.
    // Rust may serialize fields excluded from binding (they're still in the core type),
    // but the Java POJO intentionally omits them.
    let has_binding_excluded_fields = typ.fields.iter().any(|f| f.binding_excluded);
    if has_binding_excluded_fields {
        record_block.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    }

    // Suppress absent fields during serialization: null Java values and empty Optionals must
    // not be sent to Rust as `null` JSON. Rust's serde would reject null for non-optional
    // fields, and `serde(skip)` fields cause "unknown field" errors even when null.
    // NON_ABSENT suppresses both `null` references AND `Optional.empty()` values,
    // letting Rust fall back to its `#[serde(default)]` value. This only affects
    // serialization (Java → Rust); deserialization is unaffected.
    // NOTE: The ObjectMapper also has Include.ALWAYS set for compatibility with both
    // options and response serialization, but this class-level annotation takes precedence
    // for types with serde-aware fields, ensuring defaults are omitted as needed.
    let will_emit_builder = should_emit_builder(typ, builder_mode);
    // When a builder is emitted, configure Jackson to use it during deserialization.
    // This ensures that fields with serde defaults (e.g., `enabled = true`) use the
    // builder's defaults instead of Java primitive defaults (false for bool).
    // The builder is emitted as a nested static class `Builder` inside this record,
    // so the reference is `<TypeName>.Builder.class`. Gate on `should_emit_builder`
    // — without the nested Builder, `@JsonDeserialize(builder = ...)` references a
    // non-existent class.
    let builder_type = will_emit_builder.then_some(typ.name.as_str());
    if single_line_len > RECORD_LINE_WRAP_THRESHOLD && visible_fields.len() > 1 {
        let mut multiline_fields = String::new();
        for (i, decl) in field_decls.iter().enumerate() {
            let comma = if i < field_decls.len() - 1 { "," } else { "" };
            // Note: PMD 7.x does not recognize javadoc preceding annotations as belonging
            // to a record component (DanglingJavadoc rule). Record components are self-documenting
            // value types; field-level docs are redundant with the class-level record javadoc.
            // Omitting field docs in multiline mode satisfies PMD and keeps records concise.
            multiline_fields.push_str("    ");
            multiline_fields.push_str(decl);
            multiline_fields.push_str(comma);
            multiline_fields.push('\n');
        }
        record_block.push_str(&crate::backends::java::template_env::render(
            "record_declaration.jinja",
            minijinja::context! {
                has_serde => typ.has_serde,
                builder_type => builder_type,
                multiline => true,
                type_name => &typ.name,
                multiline_fields => multiline_fields,
                fields_joined => "",
            },
        ));
    } else {
        // Reuse fields_joined — no second allocation.
        record_block.push_str(&crate::backends::java::template_env::render(
            "record_declaration.jinja",
            minijinja::context! {
                has_serde => typ.has_serde,
                builder_type => builder_type,
                multiline => false,
                type_name => &typ.name,
                multiline_fields => "",
                fields_joined => &fields_joined,
            },
        ));
    }

    // Add builder() factory method only when the nested Builder class is also
    // emitted (mirrors `should_emit_builder` at the nested-class emission site
    // below). Records that fall below the auto-emit threshold should not
    // expose a factory whose return type doesn't exist.
    if will_emit_builder {
        record_block.push_str(&crate::backends::java::template_env::render(
            "record_builder_factory.jinja",
            minijinja::context! {},
        ));
    }

    // FromJson factory methods are now centralized in JsonUtil.
    // Call JsonUtil.fromJson(json, ClassName.class) instead of ClassName.fromJson(json).

    // Generate a compact constructor that applies Rust-side defaults for non-optional
    // primitive fields whose Java default (0, false, etc.) differs from the Rust default.
    // This ensures that when Jackson deserialises JSON that omits a field, the record
    // gets the Rust default rather than Java's zero value — critical for fields like
    // `batch_size` where 0 is invalid and would panic inside the native call.
    let compact_ctor_lines: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.optional)
        .filter_map(|f| {
            let jname = safe_java_field_name(&f.name);
            let has_serde_default = is_serde_default_marker(f.default.as_deref());
            match &f.typed_default {
                Some(DefaultValue::IntLiteral(n)) if *n != 0 => {
                    // Apply the Rust-side default when the Java primitive is at its zero value.
                    // This handles the case where Jackson deserialises JSON that omits the
                    // field, giving it Java's default of 0, which would be invalid in Rust
                    // (e.g., `batch_size = 0` panics in `slice::chunks`).
                    // Note: we do NOT apply defaults for bool fields — `false` is a valid
                    // explicit value that users may intentionally pass; we can't distinguish
                    // "user passed false" from "JSON omitted the field".
                    // Boxed types (Duration, serde(default) numeric/boolean fields) may arrive
                    // as null when JSON omits the field (Jackson defaults boxed types to null,
                    // not 0), so we null-check before setting the default. Primitive types are
                    // compared to their Java zero value. We do NOT coerce explicit 0 —
                    // that is a user-intentional value and the Rust core will validate it.
                    let is_boxed = matches!(f.ty, TypeRef::Duration) || has_serde_default;
                    // Add "L" suffix only for Long (Duration or 64-bit numeric types with serde(default))
                    let needs_long_suffix = matches!(f.ty, TypeRef::Duration)
                        || (has_serde_default
                            && matches!(
                                f.ty,
                                TypeRef::Primitive(
                                    PrimitiveType::U64
                                        | PrimitiveType::I64
                                        | PrimitiveType::Usize
                                        | PrimitiveType::Isize
                                )
                            ));
                    let suffix = if needs_long_suffix { "L" } else { "" };
                    let cond = if is_boxed {
                        format!("{jname} == null")
                    } else {
                        format!("{jname} == 0")
                    };
                    Some(format!("        if ({cond}) {{ {jname} = {n}{suffix}; }}"))
                }
                Some(DefaultValue::BoolLiteral(true)) if has_serde_default => {
                    // Boxed `@Nullable Boolean` serde(default) fields arrive as null when JSON
                    // omits them; restore the Rust-side `true` default so the accessor reflects it
                    // (matches the boxed-numeric handling above and the Kotlin `= true` default).
                    // Primitive bool is intentionally skipped: its Java zero value `false` is a
                    // valid explicit value indistinguishable from "omitted".
                    Some(format!("        if ({jname} == null) {{ {jname} = true; }}"))
                }
                _ => None,
            }
        })
        .collect();

    if !compact_ctor_lines.is_empty() {
        let mut lines = String::new();
        for line in &compact_ctor_lines {
            lines.push_str(line);
            lines.push('\n');
        }
        record_block.push_str(&crate::backends::java::template_env::render(
            "record_compact_constructor.jinja",
            minijinja::context! {
                type_name => &typ.name,
                lines => lines,
            },
        ));
    }

    // Note: do NOT emit Optional<String>-returning shadow accessors for nullable
    // String fields here. Records auto-generate canonical accessors with the
    // same return type as the component, and you cannot legally override them
    // with a different signature. Callers wanting `Optional` should use
    // `Optional.ofNullable(record.content())` at the call site, or the e2e
    // codegen emits a null-safe pattern.

    // When the type meets builder criteria, inline the Jackson POJO builder as a nested
    // static class instead of emitting a sibling top-level `FooBuilder.java`.
    // Idiomatic Java pattern: `Foo.Builder`, mirroring `ImmutableList.Builder`.
    if will_emit_builder {
        record_block.push('\n');
        // CPD-OFF: generated builder pattern produces identical token sequences across
        // DTO classes that share common fields (e.g. CrawlPageResult / ScrapeResult).
        record_block.push_str("    // CPD-OFF\n");
        let nested = gen_builder_nested_class(
            typ,
            trait_bridges,
            enum_defaults,
            sealed_interface_names,
            visible_type_names,
        );
        record_block.push_str(&nested);
        record_block.push_str("    // CPD-ON\n");
    }

    // Emit impl methods from the IR: static preset factories and wither instance methods.
    // Static methods with no receiver and Self return type become `public static T method(params)`.
    // Instance methods with a ref receiver and Self return type become `public T withX(params)`.
    // The Java reserved word `default` is remapped to `defaultConfig`.
    //
    // NOTE: FFM marshaling for DTO methods is not yet implemented. We skip all Self-returning
    // record methods rather than emitting throwing stubs — stubs break callers that rely on
    // compilation succeeding and mislead users. Once FFM marshaling lands, restore emission.
    let _non_excluded_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.binding_excluded && !m.sanitized)
        .collect();
    // Methods intentionally not emitted here — see NOTE above.

    record_block.push_str("}\n");

    // Scan fields_joined (the joined field declarations) to determine which imports are needed.
    let needs_json_property = fields_joined.contains("@JsonProperty(");
    // @JsonInclude may appear in field annotations OR as a class-level annotation in record_block.
    let needs_json_include = fields_joined.contains("@JsonInclude(") || record_block.contains("@JsonInclude(");
    // @JsonDeserialize may appear at class level (builder) OR at field level (custom deserializers).
    let needs_json_deserialize =
        record_block.contains("@JsonDeserialize(") || fields_joined.contains("@JsonDeserialize(");
    let needs_json_serialize = fields_joined.contains("@JsonSerialize(");
    let needs_json_ignore = fields_joined.contains("@JsonIgnore");
    // @JsonIgnoreProperties may appear at class level when binding-excluded fields exist.
    let needs_json_ignore_properties = record_block.contains("@JsonIgnoreProperties(");
    // @Nullable may appear in record fields OR in builder method signatures.
    let needs_nullable =
        fields_joined.contains("@Nullable") || (will_emit_builder && record_block.contains("@Nullable"));
    // Note: @Transient is not used in record classes — records have no bean-style getters,
    // and field-level @Transient is not valid on record components. Keeping the detection
    // for reference in case of future pattern changes.
    let _needs_transient = fields_joined.contains("@Transient");
    // Optional is needed if fields have Optional<T> in the record's field declarations OR
    // if the nested Builder class uses Optional (for optional fields stored as Optional<T>).
    let needs_optional =
        fields_joined.contains("Optional<") || (will_emit_builder && record_block.contains("Optional<"));
    let mut imports: Vec<&str> = vec![];
    if fields_joined.contains("List<") || record_block.contains("List<") {
        imports.push("java.util.List");
    }
    if fields_joined.contains("Map<") || record_block.contains("Map<") {
        imports.push("java.util.Map");
    }
    if needs_optional {
        imports.push("java.util.Optional");
    }
    // JsonNode is needed when fields reference unknown/skipped types (mapped to Json/Object)
    // JsonNode is needed only when the generated code actually references JsonNode.
    // Some unknown/skipped types are emitted as plain `Object` (java.lang.Object — no
    // import needed) rather than `JsonNode`; checking for the literal type name avoids
    // emitting an unused import that fails maven-checkstyle's UnusedImports rule.
    if fields_joined.contains("JsonNode") || record_block.contains("JsonNode") {
        imports.push("com.fasterxml.jackson.databind.JsonNode");
    }
    // @JsonProperty is needed if the record's fields use it OR the nested builder uses it.
    if needs_json_property || (will_emit_builder && record_block.contains("@JsonProperty(")) {
        imports.push("com.fasterxml.jackson.annotation.JsonProperty");
    }
    if fields_joined.contains("@JsonAlias(") {
        imports.push("com.fasterxml.jackson.annotation.JsonAlias");
    }
    if needs_json_include {
        imports.push("com.fasterxml.jackson.annotation.JsonInclude");
    }
    if needs_json_ignore_properties {
        imports.push("com.fasterxml.jackson.annotation.JsonIgnoreProperties");
    }
    if needs_json_deserialize {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonDeserialize");
    }
    if needs_json_serialize {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonSerialize");
    }
    // @JsonPOJOBuilder is needed only when the nested Builder class is actually
    // emitted inside the record — match `should_emit_builder` (line 362) so we
    // don't import a class we don't reference.
    if should_emit_builder(typ, builder_mode) {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonPOJOBuilder");
    }
    // No `import java.beans.Transient;` is needed: records have no fields to mark
    // `transient` and the `@Transient` annotation is meaningful only on JavaBean
    // getters, not record components. `@JsonIgnore` already covers serialization.
    if needs_json_ignore {
        imports.push("com.fasterxml.jackson.annotation.JsonIgnore");
    }
    if needs_nullable {
        imports.push("org.jspecify.annotations.Nullable");
    }
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&record_block);
    out
}
