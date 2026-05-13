use crate::type_map::{java_boxed_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_class_name;
use alef_core::config::{AdapterConfig, AdapterPattern};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{DefaultValue, EnumDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};

use super::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, emit_javadoc, escape_javadoc_line, format_optional_value, is_tuple_field_name,
    java_apply_rename_all, safe_java_field_name,
};

pub(crate) fn gen_record_type(
    package: &str,
    typ: &TypeDef,
    complex_enums: &AHashSet<String>,
    sealed_unions_with_unwrapped: &AHashSet<String>,
    _lang_rename_all: &str,
    has_visitor_pattern: bool,
    main_class: &str,
) -> String {
    // `fields_joined` holds the comma-separated parameter list used both for the
    // single-line length probe AND for the final single-line emit path — no rebuild.
    // `field_decls` keeps each individual decl so the multi-line emit path can put
    // each on its own line (annotations within a single decl may contain commas,
    // so we cannot split `fields_joined` by ", ").
    let mut fields_joined = String::with_capacity(typ.fields.len().saturating_mul(42));
    let mut field_decls: Vec<String> = Vec::with_capacity(typ.fields.len());

    for (i, f) in typ.fields.iter().enumerate() {
        // Complex enums (tagged unions with data) can't be simple Java enums.
        // Use Object for flexible Jackson deserialization.
        let is_complex = matches!(&f.ty, TypeRef::Named(n) if complex_enums.contains(n.as_str()));

        // Special handling for visitor field in ConversionOptions when visitor pattern is active:
        // Change type from VisitorHandle (opaque) to Visitor (interface), mark as transient.
        let is_visitor_field = has_visitor_pattern && typ.name == "ConversionOptions" && f.name == "visitor";

        // `#[serde(flatten)]` on a `serde_json::Value` field: emit
        // `@JsonAnyGetter Map<String, Object>` so Jackson absorbs unknown
        // sibling fields into the map on read and writes them flat alongside
        // the parent's named fields on write. Mirrors C#'s [JsonExtensionData].
        let is_flattened_json = f.serde_flatten && matches!(&f.ty, TypeRef::Json);

        let ftype = if is_visitor_field {
            "Visitor".to_string()
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if is_complex {
            "Object".to_string()
        } else if f.optional {
            // Java best practice: use @Nullable fields, never Optional in records.
            java_boxed_type(&f.ty).to_string()
        } else {
            java_type(&f.ty).to_string()
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
        let needs_non_null = !f.optional && matches!(&f.ty, TypeRef::Vec(_)) && !typ.has_serde;

        // Non-optional Bytes fields (byte[]) must be serialised as a JSON array of
        // integers, not as a base64 string. Jackson's default serialiser for byte[]
        // produces base64, but Rust's serde for Vec<u8> expects [n, n, …].
        // @JsonSerialize(using = ByteArrayToIntArraySerializer.class) overrides the
        // default Jackson behaviour for this field only.
        let needs_bytes_int_serialize = !f.optional && matches!(&f.ty, TypeRef::Bytes);

        // Emit `@JsonProperty` in two cases:
        // 1. The field has an explicit `#[serde(rename = "...")]` attribute.
        // 2. The Java camelCase name differs from the snake_case wire name — e.g. `max_tokens`
        //    serialises as `"max_tokens"` on the wire (Rust serde default) but Java converts it
        //    to `maxTokens`. Without `@JsonProperty("max_tokens")`, Jackson serialises using the
        //    Java field name and Rust's serde rejects the camelCase key as unrecognised.
        //
        // The wire name is the explicit serde rename if set, otherwise the original Rust field
        // name (already snake_case per project convention).
        let json_property_name = f.serde_rename.clone().unwrap_or_else(|| f.name.clone());
        let has_json_property = f.serde_rename.is_some() || jname != json_property_name;
        let has_nullable = f.optional;

        let mut decl = String::new();

        // Fields referencing sealed unions with unwrapped variants need a custom deserializer.
        // When deserializing through a builder, Jackson needs this annotation to use the
        // custom deserializer for the field type. This must come early to be properly
        // recognized by Jackson's polymorphic deserialization.
        let field_type_name = match &f.ty {
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
            decl.push_str("@JsonSerialize(using = ByteArrayToIntArraySerializer.class) ");
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
    emit_javadoc(&mut record_block, &typ.doc, "");
    // Suppress absent fields during serialization: null Java values and empty Optionals must
    // not be sent to Rust as `null` JSON.  Rust's serde would reject null for non-optional
    // fields, and `serde(skip)` fields (e.g. `cancel_token`) cause "unknown field" errors
    // even when the value is null.  NON_ABSENT suppresses both `null` references AND
    // `Optional.empty()` values, preventing either from appearing in the serialized JSON.
    // Omitting the field lets Rust fall back to its `#[serde(default)]` value.
    // This only affects serialization (Java → Rust). Deserialization (Rust → Java) is
    // unaffected, so result types are safe to annotate with this too.
    if typ.has_serde {
        record_block.push_str("@JsonInclude(JsonInclude.Include.NON_ABSENT)\n");
    }
    // When a builder is available, configure Jackson to use it during deserialization.
    // This ensures that fields with serde defaults (e.g., `enabled = true`) use the
    // builder's defaults instead of Java primitive defaults (false for bool).
    if typ.has_default {
        record_block.push_str("@JsonDeserialize(builder = ");
        record_block.push_str(&typ.name);
        record_block.push_str("Builder.class)\n");
    }
    if single_line_len > RECORD_LINE_WRAP_THRESHOLD && typ.fields.len() > 1 {
        record_block.push_str("public record ");
        record_block.push_str(&typ.name);
        record_block.push_str("(\n");
        for (i, decl) in field_decls.iter().enumerate() {
            let comma = if i < field_decls.len() - 1 { "," } else { "" };
            record_block.push_str("    ");
            record_block.push_str(decl);
            record_block.push_str(comma);
            record_block.push('\n');
        }
        record_block.push_str(") {\n");
    } else {
        // Reuse fields_joined — no second allocation.
        record_block.push_str("public record ");
        record_block.push_str(&typ.name);
        record_block.push('(');
        record_block.push_str(&fields_joined);
        record_block.push_str(") {\n");
    }

    // Add builder() factory method if type has defaults
    if typ.has_default {
        record_block.push_str("    public static ");
        record_block.push_str(&typ.name);
        record_block.push_str("Builder builder() {\n");
        record_block.push_str("        return new ");
        record_block.push_str(&typ.name);
        record_block.push_str("Builder();\n");
        record_block.push_str("    }\n");
    }

    // Emit a static `fromJson(String)` factory for binding consumers (e2e tests
    // and downstream user code). Mirrors the wider STREAM_MAPPER configuration
    // used by the opaque-handle class so SNAKE_CASE field names round-trip
    // correctly with the Rust core's serde representation.
    record_block.push_str("\n    /**\n");
    record_block.push_str("     * Parse a {@code ");
    record_block.push_str(&typ.name);
    record_block.push_str("} from a JSON string.\n");
    record_block.push_str("     *\n");
    record_block.push_str("     * @param json JSON serialisation matching the Rust-side field names (snake_case).\n");
    record_block.push_str("     * @throws ");
    record_block.push_str(main_class);
    record_block.push_str("Exception if the JSON cannot be deserialised.\n");
    record_block.push_str("     */\n");
    record_block.push_str("    public static ");
    record_block.push_str(&typ.name);
    record_block.push_str(" fromJson(String json) throws ");
    record_block.push_str(main_class);
    record_block.push_str("Exception {\n");
    record_block.push_str("        try {\n");
    record_block.push_str("            return new com.fasterxml.jackson.databind.ObjectMapper()\n");
    record_block.push_str("                .registerModule(new com.fasterxml.jackson.datatype.jdk8.Jdk8Module())\n");
    record_block.push_str("                .findAndRegisterModules()\n");
    record_block.push_str(
        "                .setPropertyNamingStrategy(com.fasterxml.jackson.databind.PropertyNamingStrategies.SNAKE_CASE)\n",
    );
    record_block.push_str(
        "                .setSerializationInclusion(com.fasterxml.jackson.annotation.JsonInclude.Include.NON_NULL)\n",
    );
    record_block.push_str(
        "                .configure(com.fasterxml.jackson.databind.MapperFeature.ACCEPT_CASE_INSENSITIVE_ENUMS, true)\n",
    );
    record_block.push_str("                .readValue(json, ");
    record_block.push_str(&typ.name);
    record_block.push_str(".class);\n");
    record_block.push_str("        } catch (Exception e) {\n");
    record_block.push_str("            throw new ");
    record_block.push_str(main_class);
    record_block.push_str("Exception(\"Failed to parse ");
    record_block.push_str(&typ.name);
    record_block.push_str(" from JSON: \" + e.getMessage(), e);\n");
    record_block.push_str("        }\n");
    record_block.push_str("    }\n");

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
            match &f.typed_default {
                Some(DefaultValue::IntLiteral(n)) if *n != 0 => {
                    // Apply the Rust-side default when the Java primitive is at its zero value.
                    // This handles the case where Jackson deserialises JSON that omits the
                    // field, giving it Java's default of 0, which would be invalid in Rust
                    // (e.g., `batch_size = 0` panics in `slice::chunks`).
                    // Note: we do NOT apply defaults for bool fields — `false` is a valid
                    // explicit value that users may intentionally pass; we can't distinguish
                    // "user passed false" from "JSON omitted the field".
                    // Duration fields map to boxed Long in Java; int literals don't auto-box
                    // to Long, so we must use the L suffix to produce a long literal that Java
                    // will auto-box correctly. Boxed types may arrive as null when JSON omits
                    // the field (Jackson defaults boxed numerics to null, not 0), so we
                    // null-check before setting the default. We do NOT coerce explicit 0 —
                    // that is a user-intentional value and the Rust core will validate it.
                    let is_boxed = matches!(f.ty, TypeRef::Duration);
                    let suffix = if is_boxed { "L" } else { "" };
                    let cond = if is_boxed {
                        format!("{jname} == null")
                    } else {
                        format!("{jname} == 0")
                    };
                    Some(format!("        if ({cond}) {jname} = {n}{suffix};"))
                }
                _ => None,
            }
        })
        .collect();

    if !compact_ctor_lines.is_empty() {
        record_block.push_str("    public ");
        record_block.push_str(&typ.name);
        record_block.push_str("{\n");
        for line in &compact_ctor_lines {
            record_block.push_str(line);
            record_block.push('\n');
        }
        record_block.push_str("    }\n");
    }

    // Note: do NOT emit Optional<String>-returning shadow accessors for nullable
    // String fields here. Records auto-generate canonical accessors with the
    // same return type as the component, and you cannot legally override them
    // with a different signature. Callers wanting `Optional` should use
    // `Optional.ofNullable(record.content())` at the call site, or the e2e
    // codegen emits a null-safe pattern.

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
    let needs_nullable = fields_joined.contains("@Nullable");
    // Note: @Transient is not used in record classes — records have no bean-style getters,
    // and field-level @Transient is not valid on record components. Keeping the detection
    // for reference in case of future pattern changes.
    let _needs_transient = fields_joined.contains("@Transient");
    // Optional is needed if fields have Optional<T> in declaration
    let needs_optional = fields_joined.contains("Optional<");
    let mut imports: Vec<&str> = vec![];
    if fields_joined.contains("List<") {
        imports.push("java.util.List");
    }
    if fields_joined.contains("Map<") {
        imports.push("java.util.Map");
    }
    if needs_optional {
        imports.push("java.util.Optional");
    }
    if needs_json_property {
        imports.push("com.fasterxml.jackson.annotation.JsonProperty");
    }
    if fields_joined.contains("@JsonAlias(") {
        imports.push("com.fasterxml.jackson.annotation.JsonAlias");
    }
    if needs_json_include {
        imports.push("com.fasterxml.jackson.annotation.JsonInclude");
    }
    if needs_json_deserialize {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonDeserialize");
    }
    if needs_json_serialize {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonSerialize");
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
    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&record_block);
    out
}

pub(crate) fn gen_enum_class(package: &str, enum_def: &EnumDef, main_class: &str) -> String {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Tagged union: enum has a serde tag AND data variants → generate sealed interface hierarchy
    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_java_tagged_union(package, enum_def);
    }

    // Untagged union with data variants (e.g. EmbeddingInput = String | Vec<String>):
    // emit a transparent JsonNode-wrapper class. Jackson cannot dispatch between
    // alternatives by name (variant identifiers don't appear in the wire JSON), so
    // we hold the raw JsonNode and let serde on the Rust side resolve the variant.
    if enum_def.serde_untagged && has_data_variants {
        return gen_java_untagged_wrapper(package, enum_def, main_class);
    }

    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = [
        "com.fasterxml.jackson.annotation.JsonCreator",
        "com.fasterxml.jackson.annotation.JsonValue",
    ];
    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');

    emit_javadoc(&mut out, &enum_def.doc, "");
    out.push_str("public enum ");
    out.push_str(&enum_def.name);
    out.push_str(" {\n");

    for (i, variant) in enum_def.variants.iter().enumerate() {
        let comma = if i < enum_def.variants.len() - 1 { "," } else { ";" };
        // Use serde_rename if available, otherwise apply rename_all strategy
        let json_name = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| java_apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        if !variant.doc.is_empty() {
            let doc_summary = escape_javadoc_line(variant.doc.lines().next().unwrap_or("").trim());
            // 4 spaces indent + "/** " + " */" = 11 chars overhead; wrap if total > 80
            if doc_summary.len() + 11 > 80 {
                out.push_str("    /**\n");
                out.push_str("     * ");
                out.push_str(&doc_summary);
                out.push('\n');
                out.push_str("     */\n");
            } else {
                out.push_str("    /** ");
                out.push_str(&doc_summary);
                out.push_str(" */\n");
            }
        }
        out.push_str("    ");
        out.push_str(&variant.name);
        out.push_str("(\"");
        out.push_str(&json_name);
        out.push_str("\")");
        out.push_str(comma);
        out.push('\n');
    }

    out.push('\n');
    out.push_str("    /** The string value. */\n");
    out.push_str("    private final String value;\n");
    out.push('\n');
    out.push_str("    ");
    out.push_str(&enum_def.name);
    out.push_str("(final String value) {\n");
    out.push_str("        this.value = value;\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    /** Returns the string value. */\n");
    out.push_str("    @JsonValue\n");
    out.push_str("    public String getValue() {\n");
    out.push_str("        return value;\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    /** Creates an instance from a string value. */\n");
    out.push_str("    @JsonCreator\n");
    out.push_str("    public static ");
    out.push_str(&enum_def.name);
    out.push_str(" fromValue(final String value) {\n");
    out.push_str("        for (");
    out.push_str(&enum_def.name);
    out.push_str(" e : values()) {\n");
    out.push_str("            if (e.value.equalsIgnoreCase(value)) {\n");
    out.push_str("                return e;\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        throw new IllegalArgumentException(\"Unknown value: \" + value);\n");
    out.push_str("    }\n");

    out.push_str("}\n");

    out
}

/// Emit a transparent JsonNode-wrapper for `#[serde(untagged)]` enums.
///
/// Untagged unions like `EmbeddingInput = Single(String) | Multiple(Vec<String>)`
/// have no on-wire discriminator. Jackson's default deserialization tries to match
/// the JSON shape against the Java type; for plain enums it calls `fromValue(...)`
/// which throws on any value that does not match a variant name. The wrapper class
/// holds the JsonNode verbatim, with `@JsonValue` for serialization and
/// `@JsonCreator(mode=DELEGATING)` so Jackson hands the parsed JsonNode straight
/// through. The Rust core (serde) resolves the variant on the way in.
fn gen_java_untagged_wrapper(package: &str, enum_def: &EnumDef, main_class: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let doc = enum_def
        .doc
        .lines()
        .next()
        .map(|line| escape_javadoc_line(line.trim()))
        .unwrap_or_default();
    let exception_class = format!("{main_class}Exception");
    crate::template_env::render(
        "untagged_union_wrapper.jinja",
        minijinja::context! {
            header => header,
            package => package,
            class_name => &enum_def.name,
            doc => doc,
            exception_class => exception_class,
        },
    )
}

pub(crate) fn gen_java_tagged_union(package: &str, enum_def: &EnumDef) -> String {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    // Collect variant names to detect Java type name conflicts.
    // If a variant is named "List", "Map", or "Optional", using those type names
    // inside the sealed interface would refer to the nested record, not java.util.*.
    // We use fully qualified names in that case.
    let variant_names: std::collections::HashSet<&str> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
    let optional_type = if variant_names.contains("Optional") {
        "java.util.Optional"
    } else {
        "Optional"
    };

    // @JsonProperty is only needed for variants with named (non-tuple) fields.
    let needs_json_property = enum_def
        .variants
        .iter()
        .any(|v| v.fields.iter().any(|f| !is_tuple_field_name(&f.name)));

    // Check if any data variants exist (non-unit variants with tuple/newtype fields)
    // to determine if we need the @Nullable import for accessor methods
    let has_data_variants = enum_def
        .variants
        .iter()
        .any(|v| !v.fields.is_empty() && is_tuple_field_name(&v.fields[0].name));

    // Check if any field types need list/map/optional imports (only when not conflicting)
    let needs_list = !variant_names.contains("List")
        && enum_def
            .variants
            .iter()
            .any(|v| v.fields.iter().any(|f| matches!(&f.ty, TypeRef::Vec(_))));
    let needs_map = !variant_names.contains("Map")
        && enum_def
            .variants
            .iter()
            .any(|v| v.fields.iter().any(|f| matches!(&f.ty, TypeRef::Map(_, _))));
    let needs_optional =
        !variant_names.contains("Optional") && enum_def.variants.iter().any(|v| v.fields.iter().any(|f| f.optional));
    // Newtype/tuple variants (field name is a numeric index like "0") are flattened
    // into the parent JSON object. We use a custom deserializer instead of @JsonUnwrapped
    // because Jackson 2.18 doesn't support @JsonUnwrapped on record creator parameters.
    let needs_unwrapped = enum_def
        .variants
        .iter()
        .any(|v| v.fields.len() == 1 && is_tuple_field_name(&v.fields[0].name));

    let mut imports: Vec<&str> = vec![];
    if needs_json_property {
        imports.push("com.fasterxml.jackson.annotation.JsonProperty");
    }
    // When a custom deserializer handles polymorphic dispatch (@JsonDeserialize with a
    // *Deserializer class), @JsonTypeInfo + @JsonSubTypes are redundant and actively
    // harmful: Jackson's AsPropertyTypeDeserializer strips the discriminator field
    // (visible=false) before calling the custom deserializer, so the custom deserializer
    // never sees it and throws "Missing discriminator field". Only emit @JsonTypeInfo /
    // @JsonSubTypes when there is NO custom deserializer (simple polymorphic dispatch).
    if !needs_unwrapped {
        imports.push("com.fasterxml.jackson.annotation.JsonSubTypes");
        imports.push("com.fasterxml.jackson.annotation.JsonTypeInfo");
    }
    if needs_list {
        imports.push("java.util.List");
    }
    if needs_map {
        imports.push("java.util.Map");
    }
    if needs_optional {
        imports.push("java.util.Optional");
    }
    if needs_unwrapped {
        imports.push("com.fasterxml.jackson.databind.deser.std.StdDeserializer");
        imports.push("com.fasterxml.jackson.databind.ser.std.StdSerializer");
        imports.push("com.fasterxml.jackson.core.JsonParser");
        imports.push("com.fasterxml.jackson.core.JsonGenerator");
        imports.push("com.fasterxml.jackson.databind.DeserializationContext");
        imports.push("com.fasterxml.jackson.databind.SerializerProvider");
        imports.push("com.fasterxml.jackson.databind.node.ObjectNode");
        imports.push("com.fasterxml.jackson.databind.annotation.JsonDeserialize");
        imports.push("com.fasterxml.jackson.databind.annotation.JsonSerialize");
    }
    if has_data_variants {
        imports.push("org.jspecify.annotations.Nullable");
    }
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');

    emit_javadoc(&mut out, &enum_def.doc, "");
    // @JsonTypeInfo and @JsonSubTypes annotations — only when no custom deserializer.
    // A custom *Deserializer reads the tag field itself; mixing @JsonTypeInfo (which
    // strips the tag when visible=false) with a custom deserializer causes a NPE/missing-
    // discriminator error because the tag is consumed before the deserializer sees it.
    if !needs_unwrapped {
        out.push_str("@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, property = \"");
        out.push_str(tag_field);
        out.push_str("\", visible = false)\n");
        out.push_str("@JsonSubTypes({\n");
        for (i, variant) in enum_def.variants.iter().enumerate() {
            let discriminator = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| java_apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
            let comma = if i < enum_def.variants.len() - 1 { "," } else { "" };
            out.push_str("    @JsonSubTypes.Type(value = ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(".class, name = \"");
            out.push_str(&discriminator);
            out.push_str("\")");
            out.push_str(comma);
            out.push('\n');
        }
        out.push_str("})\n");
    }
    // Newtype variants with flattened fields cannot directly map to record fields.
    // Allow unknown properties at the interface level so Jackson doesn't fail when
    // encountering flattened inner-type fields.
    out.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    if needs_unwrapped {
        out.push_str("@JsonDeserialize(using = ");
        out.push_str(&enum_def.name);
        out.push_str("Deserializer.class)\n");
        out.push_str("@JsonSerialize(using = ");
        out.push_str(&enum_def.name);
        out.push_str("Serializer.class)\n");
    }
    out.push_str("public sealed interface ");
    out.push_str(&enum_def.name);
    out.push_str(" {\n");

    // Nested records for each variant
    for variant in &enum_def.variants {
        out.push('\n');
        if variant.fields.is_empty() {
            // Unit variant
            if !variant.doc.is_empty() {
                let doc_summary = escape_javadoc_line(variant.doc.lines().next().unwrap_or("").trim());
                out.push_str("    /** ");
                out.push_str(&doc_summary);
                out.push_str(" */\n");
            }
            out.push_str("    record ");
            out.push_str(&variant.name);
            out.push_str("() implements ");
            out.push_str(&enum_def.name);
            out.push_str(" {\n");
            out.push_str("    }\n");
        } else {
            // Build field list using fully qualified names where variant names shadow imports
            let field_parts: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let ftype = if f.optional {
                        let inner = java_boxed_type(&f.ty);
                        let inner_str = inner.as_ref();
                        // Replace "List"/"Map" with fully qualified if conflicting
                        let inner_qualified = if inner_str.starts_with("List<") && variant_names.contains("List") {
                            inner_str.replacen("List<", "java.util.List<", 1)
                        } else if inner_str.starts_with("Map<") && variant_names.contains("Map") {
                            inner_str.replacen("Map<", "java.util.Map<", 1)
                        } else {
                            inner_str.to_string()
                        };
                        format!("{optional_type}<{inner_qualified}>")
                    } else {
                        let t = java_type(&f.ty);
                        let t_str = t.as_ref();
                        if t_str.starts_with("List<") && variant_names.contains("List") {
                            t_str.replacen("List<", "java.util.List<", 1)
                        } else if t_str.starts_with("Map<") && variant_names.contains("Map") {
                            t_str.replacen("Map<", "java.util.Map<", 1)
                        } else {
                            t_str.to_string()
                        }
                    };
                    // Tuple/newtype variants have numeric field names (e.g. "0", "_0").
                    // These are not real JSON keys — serde flattens the inner type's fields
                    // alongside the tag. The custom deserializer handles unwrapping.
                    if is_tuple_field_name(&f.name) {
                        format!("{ftype} value")
                    } else {
                        let json_name = f.name.trim_start_matches('_');
                        let jname = safe_java_field_name(json_name);
                        format!("@JsonProperty(\"{json_name}\") {ftype} {jname}")
                    }
                })
                .collect();

            // Join once; reuse for both the length probe and the single-line emit path.
            let fields_joined: String = field_parts.join(", ");
            let single_len = "    record ".len()
                + variant.name.len()
                + 1
                + fields_joined.len()
                + ") implements ".len()
                + enum_def.name.len()
                + " { }".len();

            if !variant.doc.is_empty() {
                let doc_summary = escape_javadoc_line(variant.doc.lines().next().unwrap_or("").trim());
                out.push_str("    /** ");
                out.push_str(&doc_summary);
                out.push_str(" */\n");
            }
            if single_len > RECORD_LINE_WRAP_THRESHOLD && field_parts.len() > 1 {
                out.push_str("    record ");
                out.push_str(&variant.name);
                out.push_str("(\n");
                for (i, fp) in field_parts.iter().enumerate() {
                    let comma = if i < field_parts.len() - 1 { "," } else { "" };
                    out.push_str("        ");
                    out.push_str(fp);
                    out.push_str(comma);
                    out.push('\n');
                }
                out.push_str("    ) implements ");
                out.push_str(&enum_def.name);
                out.push_str(" {\n");
                out.push_str("    }\n");
            } else {
                out.push_str("    record ");
                out.push_str(&variant.name);
                out.push('(');
                out.push_str(&fields_joined);
                out.push_str(") implements ");
                out.push_str(&enum_def.name);
                out.push_str(" { }\n");
            }
        }
    }

    // Add default accessor methods for each newtype/tuple data variant
    if has_data_variants {
        out.push('\n');
        for variant in &enum_def.variants {
            if variant.fields.is_empty() || !is_tuple_field_name(&variant.fields[0].name) {
                continue;
            }
            let method_name = variant.name.to_lower_camel_case();
            let return_type = java_boxed_type(&variant.fields[0].ty);
            let variant_name = &variant.name;
            out.push_str("    /** Returns the ");
            out.push_str(variant_name);
            out.push_str(" data if this is a ");
            out.push_str(variant_name);
            out.push_str(" variant, otherwise null. */\n");
            out.push_str("    default @Nullable ");
            out.push_str(return_type.as_ref());
            out.push(' ');
            out.push_str(&method_name);
            out.push_str("() {\n");
            out.push_str("        return this instanceof ");
            out.push_str(variant_name);
            out.push_str(" e ? e.value() : null;\n");
            out.push_str("    }\n");
            out.push('\n');
        }
    }

    out.push_str("}\n");

    // Generate custom deserializer + serializer for sealed interfaces with unwrapped
    // variants. The serializer mirrors the deserializer's tag handling: it emits the
    // tag field plus the inner record's fields flattened (e.g. {"role":"user","content":...}).
    if needs_unwrapped {
        out.push('\n');
        gen_sealed_union_deserializer(&mut out, package, enum_def, tag_field);
        out.push('\n');
        gen_sealed_union_serializer(&mut out, package, enum_def, tag_field);
    }

    out
}

pub(crate) fn gen_opaque_handle_class(
    package: &str,
    typ: &TypeDef,
    prefix: &str,
    adapters: &[AdapterConfig],
    main_class: &str,
) -> String {
    let class_name = &typ.name;
    let type_snake = class_name.to_snake_case();
    let header = hash::header(CommentStyle::DoubleSlash);

    // Detect streaming adapters owned by this opaque type. When present we need
    // additional imports (Iterator, NoSuchElementException, ObjectMapper).
    let streaming_adapters: Vec<&AdapterConfig> = adapters
        .iter()
        .filter(|a| {
            matches!(a.pattern, AdapterPattern::Streaming)
                && a.owner_type.as_deref() == Some(class_name.as_str())
                && a.item_type.is_some()
                && a.params.first().is_some_and(|p| !p.ty.is_empty())
        })
        .collect();
    let has_streaming = !streaming_adapters.is_empty();

    // Instance methods on this opaque handle (skip static and any method whose name
    // collides with a streaming adapter — those are emitted by the streaming codegen).
    let streaming_method_names: AHashSet<String> = streaming_adapters.iter().map(|a| a.name.to_snake_case()).collect();
    let instance_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.is_static)
        .filter(|m| !streaming_method_names.contains(&m.name.to_snake_case()))
        .collect();
    let has_instance_methods = !instance_methods.is_empty();
    let needs_helpers = has_streaming || has_instance_methods;

    // Check instance methods for List, Map, Optional return types
    let mut has_list_return = false;
    let mut has_optional_return = false;
    let mut has_map_return = false;
    for method in &instance_methods {
        let return_type_str = java_type(&method.return_type).to_string();
        if return_type_str.contains("List<") {
            has_list_return = true;
        }
        if return_type_str.contains("Optional<") {
            has_optional_return = true;
        }
        if return_type_str.contains("Map<") {
            has_map_return = true;
        }
    }

    // Build the class body first so we can compute imports from actual usage —
    // Checkstyle's UnusedImports rule fails if we declare an import that
    // never appears in the file body (e.g. when every instance method body
    // is a `TODO unsupported return shape` stub).
    let mut body = String::new();

    emit_javadoc(&mut body, &typ.doc, "");

    body.push_str("public class ");
    body.push_str(class_name);
    body.push_str(" implements AutoCloseable {\n");
    body.push_str("    private final MemorySegment handle;\n");
    body.push('\n');
    body.push_str("    ");
    body.push_str(class_name);
    body.push_str("(MemorySegment handle) {\n");
    body.push_str("        this.handle = handle;\n");
    body.push_str("    }\n");
    body.push('\n');
    body.push_str("    MemorySegment handle() {\n");
    body.push_str("        return this.handle;\n");
    body.push_str("    }\n");
    body.push('\n');

    // Emit streaming iterator methods (e.g. chatStream(req) -> Iterator<ChatCompletionChunk>).
    for adapter in &streaming_adapters {
        gen_streaming_method(&mut body, adapter, prefix, &type_snake, main_class);
    }

    // Emit non-streaming instance methods (chat, embed, moderate, …).
    for method in &instance_methods {
        gen_instance_method(&mut body, method, prefix, &type_snake, main_class);
    }

    body.push_str("    @Override\n");
    body.push_str("    public void close() {\n");
    body.push_str("        if (handle != null && !handle.equals(MemorySegment.NULL)) {\n");
    body.push_str("            try {\n");
    body.push_str("                NativeLib.");
    body.push_str(&prefix.to_uppercase());
    body.push('_');
    body.push_str(&type_snake.to_uppercase());
    body.push_str("_FREE.invoke(handle);\n");
    body.push_str("            } catch (Throwable e) {\n");
    body.push_str("                throw new RuntimeException(\"Failed to free ");
    body.push_str(class_name);
    body.push_str(": \" + e.getMessage(), e);\n");
    body.push_str("            }\n");
    body.push_str("        }\n");
    body.push_str("    }\n");

    if needs_helpers {
        gen_streaming_helpers(&mut body, prefix, main_class);
    }

    body.push_str("}\n");

    let mut imports: Vec<&str> = vec!["java.lang.foreign.MemorySegment"];
    if needs_helpers {
        // `Arena.ofConfined()` is referenced by every helper / instance-method
        // template even when the method body is a stub, so the import is
        // always live in that branch.
        imports.push("java.lang.foreign.Arena");
        // `ValueLayout` only appears when an instance method or streaming
        // helper actually marshals memory; stub methods (`unsupported return
        // shape`) never reference it. Scan the rendered body to avoid an
        // unused-import Checkstyle violation.
        if body.contains("ValueLayout") {
            imports.push("java.lang.foreign.ValueLayout");
        }
        // Same reasoning for ObjectMapper — STREAM_MAPPER references it, but
        // not all paths reach STREAM_MAPPER.
        if body.contains("ObjectMapper") {
            imports.push("com.fasterxml.jackson.databind.ObjectMapper");
        }
    }
    if has_streaming {
        imports.push("java.util.Iterator");
        imports.push("java.util.NoSuchElementException");
    }
    if has_list_return {
        imports.push("java.util.List");
    }
    if has_optional_return {
        imports.push("java.util.Optional");
    }
    if has_map_return {
        imports.push("java.util.Map");
    }

    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&body);
    out
}

/// Emit a non-streaming instance method on an opaque-handle owner.
fn gen_instance_method(out: &mut String, method: &MethodDef, prefix: &str, owner_snake: &str, main_class: &str) {
    let method_name = method.name.to_lower_camel_case();
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let method_upper = method.name.to_snake_case().to_uppercase();
    let exception_class = format!("{main_class}Exception");
    let ffi_handle = format!("NativeLib.{prefix_upper}_{owner_upper}_{method_upper}");

    let params_sig: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ptype = if p.optional {
                java_boxed_type(&p.ty).to_string()
            } else {
                java_type(&p.ty).to_string()
            };
            format!("final {} {}", ptype, p.name.to_lower_camel_case())
        })
        .collect();

    let is_bytes_result = method.error_type.is_some()
        && (matches!(method.return_type, TypeRef::Bytes)
            || matches!(&method.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes)));

    let (is_optional_return, dispatch_return) = match &method.return_type {
        TypeRef::Optional(inner) => (true, (**inner).clone()),
        other => (false, other.clone()),
    };

    let return_type_java = if is_bytes_result {
        if is_optional_return {
            "java.util.Optional<byte[]>"
        } else {
            "byte[]"
        }
        .to_string()
    } else {
        java_type(&method.return_type).to_string()
    };

    out.push_str("    public ");
    out.push_str(&return_type_java);
    out.push(' ');
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push_str(") throws ");
    out.push_str(&exception_class);
    out.push_str(" {\n");

    for p in &method.params {
        if !p.optional && param_needs_null_check(&p.ty) {
            let pname = p.name.to_lower_camel_case();
            out.push_str(&crate::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => pname },
            ));
        }
    }

    out.push_str("        try (var arena = Arena.ofConfined()) {\n");

    let mut named_ptr_frees: Vec<(String, String)> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();

    for p in &method.params {
        let pname = p.name.to_lower_camel_case();
        let cname = format!("c{}", to_class_name(&p.name));
        match &p.ty {
            TypeRef::String | TypeRef::Char | TypeRef::Json => {
                out.push_str(&crate::template_env::render(
                    "stream_method_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Path => {
                // Path → C string requires `.toString()` because Java's SegmentAllocator.allocateFrom
                // accepts String, not java.nio.file.Path. Reuse marshal_path.jinja which already
                // emits the conversion.
                out.push_str(&crate::template_env::render(
                    "marshal_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char | TypeRef::Json) => {
                out.push_str(&crate::template_env::render(
                    "stream_method_optional_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
                // Optional Path also needs `.toString()` — reuse marshal_optional_path
                // (declared below in the marshal module). Inline the conversion to avoid
                // adding another template.
                out.push_str(&format!(
                    "            var {cname} = {pname} != null ? arena.allocateFrom({pname}.toString()) : MemorySegment.NULL;\n"
                ));
                call_args.push(cname);
            }
            TypeRef::Named(type_name) => {
                let req_snake = type_name.to_snake_case();
                let req_upper = req_snake.to_uppercase();
                let from_json = format!("NativeLib.{prefix_upper}_{req_upper}_FROM_JSON");
                let req_free = format!("NativeLib.{prefix_upper}_{req_upper}_FREE");
                if p.optional {
                    // Optional Named param (e.g. `query: Option<&BatchListQuery>` in Rust
                    // surfaces as `TypeRef::Named` + `optional: true` in the IR after the
                    // FFI extraction strips the `Option`). Pass MemorySegment.NULL when
                    // the Java arg is null instead of serializing `null` and feeding it
                    // to <Type>_from_json which then errors with "invalid type: null,
                    // expected struct <Type>".
                    out.push_str(&crate::template_env::render(
                        "stream_method_optional_named_param.jinja",
                        minijinja::context! {
                            c_name => cname,
                            param_name => pname,
                            from_json => from_json,
                            exception_class => exception_class,
                            method_name => method_name,
                        },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "stream_method_named_param.jinja",
                        minijinja::context! {
                            c_name => cname,
                            param_name => pname,
                            from_json => from_json,
                            exception_class => exception_class,
                            method_name => method_name,
                        },
                    ));
                }
                named_ptr_frees.push((cname.clone(), req_free));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                let type_name = match inner.as_ref() {
                    TypeRef::Named(n) => n,
                    _ => unreachable!(),
                };
                let req_snake = type_name.to_snake_case();
                let req_upper = req_snake.to_uppercase();
                let from_json = format!("NativeLib.{prefix_upper}_{req_upper}_FROM_JSON");
                let req_free = format!("NativeLib.{prefix_upper}_{req_upper}_FREE");
                out.push_str(&crate::template_env::render(
                    "stream_method_optional_named_param.jinja",
                    minijinja::context! {
                        c_name => cname,
                        param_name => pname,
                        from_json => from_json,
                        exception_class => exception_class,
                        method_name => method_name,
                    },
                ));
                named_ptr_frees.push((cname.clone(), req_free));
                call_args.push(cname);
            }
            TypeRef::Primitive(_) | TypeRef::Duration => {
                call_args.push(pname);
            }
            _ => {
                out.push_str(&crate::template_env::render(
                    "stream_method_unsupported_param.jinja",
                    minijinja::context! {
                        param_name => pname,
                        exception_class => exception_class,
                        method_name => method_name,
                    },
                ));
                return;
            }
        }
    }

    let render_named_frees = |indent: &str| -> String {
        let mut frees = String::new();
        for (cname, free_handle) in &named_ptr_frees {
            frees.push_str(&crate::template_env::render(
                "stream_method_free_named_ptr.jinja",
                minijinja::context! {
                    indent => indent,
                    c_name => cname,
                    free_handle => free_handle,
                },
            ));
        }
        frees
    };

    let mut call_args_full = vec!["this.handle".to_string()];
    call_args_full.extend(call_args);
    let args_joined = call_args_full.join(", ");

    if is_bytes_result {
        let free_bytes = format!("NativeLib.{prefix_upper}_FREE_BYTES");
        let empty_return = if is_optional_return {
            "return java.util.Optional.empty();"
        } else {
            "return null;"
        };
        let success_return = if is_optional_return {
            "java.util.Optional.of(result)"
        } else {
            "result"
        };
        out.push_str(&crate::template_env::render(
            "stream_method_bytes_result.jinja",
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                empty_return => empty_return,
                free_bytes => free_bytes,
                success_return => success_return,
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Named(_)) {
        let return_type_name = match &dispatch_return {
            TypeRef::Named(n) => n.clone(),
            _ => unreachable!(),
        };
        let ret_snake = return_type_name.to_snake_case();
        let ret_upper = ret_snake.to_uppercase();
        let ret_free = format!("NativeLib.{prefix_upper}_{ret_upper}_FREE");
        let ret_to_json = format!("NativeLib.{prefix_upper}_{ret_upper}_TO_JSON");

        out.push_str(&crate::template_env::render(
            "stream_method_named_result.jinja",
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                to_json => ret_to_json,
                exception_class => exception_class,
                method_name => method_name,
                prefix_upper => prefix_upper,
                return_type_name => return_type_name,
                ret_free => ret_free,
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Unit) {
        out.push_str(&crate::template_env::render(
            "stream_method_unit_result.jinja",
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
            },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "stream_method_unsupported_return.jinja",
            minijinja::context! {
                named_frees => render_named_frees("            "),
                method_name => method_name,
                exception_class => exception_class,
            },
        ));
    }

    out.push_str(&crate::template_env::render(
        "stream_method_catch.jinja",
        minijinja::context! {
            exception_class => exception_class,
            method_name => method_name,
        },
    ));
}

/// True when the given `TypeRef` is a reference type whose Java representation may
/// be null (so we should `Objects.requireNonNull` it for non-optional params).
fn param_needs_null_check(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Named(_)
            | TypeRef::Bytes
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _)
    )
}

/// Emit a streaming iterator method body for an opaque-handle owner.
///
/// Generates `public Iterator<Item> <camelName>(Request request)` that calls the
/// FFI iterator-handle trio (`_start`, `_next`, `_free`), deserializing each chunk
/// pointer via `<item>_to_json` + `<item>_free` and rethrowing FFI errors as
/// `<MainClass>Exception`.
fn gen_streaming_method(out: &mut String, adapter: &AdapterConfig, prefix: &str, owner_snake: &str, main_class: &str) {
    let method_name = adapter.name.to_lower_camel_case();
    let item_type = adapter.item_type.as_deref().unwrap_or("Object");
    let request_type_full = adapter.params[0].ty.as_str();
    // Strip any leading module path (e.g. `liter_llm::ChatCompletionRequest` → `ChatCompletionRequest`).
    let request_type = request_type_full.rsplit("::").next().unwrap_or(request_type_full);
    let request_snake = request_type.to_snake_case();
    let prefix_upper = prefix.to_uppercase();
    let owner_upper = owner_snake.to_uppercase();
    let adapter_upper = adapter.name.to_snake_case().to_uppercase();
    let request_upper = request_snake.to_uppercase();
    let item_snake = item_type.to_snake_case();
    let item_upper = item_snake.to_uppercase();
    let exception_class = format!("{main_class}Exception");

    let request_param = adapter.params[0].name.to_lower_camel_case();
    let request_param = if request_param.is_empty() {
        "request".to_string()
    } else {
        request_param
    };

    let start_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_START");
    let next_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_NEXT");
    let free_handle = format!("{prefix_upper}_{owner_upper}_{adapter_upper}_FREE");
    let req_from_json = format!("{prefix_upper}_{request_upper}_FROM_JSON");
    let req_free = format!("{prefix_upper}_{request_upper}_FREE");
    let item_to_json = format!("{prefix_upper}_{item_upper}_TO_JSON");
    let item_free = format!("{prefix_upper}_{item_upper}_FREE");

    out.push_str(&crate::template_env::render(
        "streaming_iterator_method.jinja",
        minijinja::context! {
            item_type => item_type,
            method_name => method_name,
            request_type => request_type,
            request_param => request_param,
            exception_class => exception_class,
            req_from_json => req_from_json,
            start_handle => start_handle,
            req_free => req_free,
            next_handle => next_handle,
            prefix_upper => prefix_upper,
            item_to_json => item_to_json,
            item_free => item_free,
            free_handle => free_handle,
        },
    ));
}

/// Emit shared helpers (`STREAM_MAPPER`, `checkLastFfiError`, optionally `readBytesResult`)
/// used by the streaming iterator method bodies above.
fn gen_streaming_helpers(out: &mut String, prefix: &str, main_class: &str) {
    let prefix_upper = prefix.to_uppercase();
    let exception_class = format!("{main_class}Exception");
    let needs_read_bytes_result = out.contains("readBytesResult(");
    let free_bytes = format!("NativeLib.{prefix_upper}_FREE_BYTES");

    out.push_str(&crate::template_env::render(
        "streaming_helpers.jinja",
        minijinja::context! {
            exception_class => exception_class,
            prefix_upper => prefix_upper,
            needs_read_bytes_result => needs_read_bytes_result,
            free_bytes => free_bytes,
        },
    ));
}

// ---------------------------------------------------------------------------
// Record types (Java records)
// ---------------------------------------------------------------------------

/// Emit a Javadoc comment block into `out` at the given indentation level.
///
/// `indent` is the leading whitespace prepended to each line (e.g. `""` for
/// top-level declarations, `"    "` for class members).  Does nothing when
/// `doc` is empty.
pub(crate) fn gen_builder_class(package: &str, typ: &TypeDef, has_visitor_pattern: bool) -> String {
    let mut body = String::with_capacity(2048);

    emit_javadoc(&mut body, &typ.doc, "");
    // Annotation tells Jackson to use this builder when deserializing the record.
    // Builder defaults (e.g., enabled=true) are applied during deserialization.
    body.push_str("@JsonPOJOBuilder(withPrefix = \"with\")\n");
    body.push_str("public class ");
    body.push_str(&typ.name);
    body.push_str("Builder {\n");
    body.push('\n');

    // Generate field declarations with defaults
    for field in &typ.fields {
        let field_name = safe_java_field_name(&field.name);

        // Skip unnamed tuple fields (name is "_0", "_1", "0", "1", etc.) — Java requires named fields
        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        // Match the record's special-cased ConversionOptions.visitor field (VisitorHandle in
        // the IR but exposed as the user-facing `Visitor` interface) so the builder's
        // build() call passes a Visitor — not a VisitorHandle — to the record constructor.
        let is_visitor_field = has_visitor_pattern && typ.name == "ConversionOptions" && field.name == "visitor";

        // `#[serde(flatten)]` on a `serde_json::Value` field — store as
        // `java.util.HashMap<String, Object>` so the builder's matching
        // `@JsonAnySetter` method can accumulate sibling fields. The record's
        // accessor returns the same `java.util.Map<String, Object>` view.
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);

        // Duration maps to primitive `long` in the public record, but in builder
        // classes we use boxed `Long` so that `null` can represent "not set".
        let field_type = if is_visitor_field {
            "Optional<Visitor>".to_string()
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if field.optional {
            format!("Optional<{}>", java_boxed_type(&field.ty))
        } else if matches!(field.ty, TypeRef::Duration) {
            java_boxed_type(&field.ty).to_string()
        } else {
            java_type(&field.ty).to_string()
        };

        let default_value = if is_visitor_field {
            // The visitor field is wrapped in Optional<Visitor> regardless of the IR's
            // optionality, so its default has to be Optional.empty() to match the type.
            "Optional.empty()".to_string()
        } else if is_flattened_json {
            // Flatten field: live `HashMap` accumulator that the @JsonAnySetter
            // builder method (emitted later) writes into.
            "new java.util.HashMap<>()".to_string()
        } else if field.optional {
            // For Optional fields, always use Optional.empty() or Optional.of(value)
            if let Some(default) = &field.default {
                // If there's an explicit default, wrap it in Optional.of()
                format_optional_value(&field.ty, default)
            } else {
                // If no default, use Optional.empty()
                "Optional.empty()".to_string()
            }
        } else {
            // For non-Optional fields, use regular defaults
            if let Some(default) = &field.default {
                default.clone()
            } else {
                match &field.ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
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
                            // (e.g. ProcessConfig.structure, ConversionOptions.autolinks).
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

        // Emit `@JsonProperty(<wire-name>)` so Jackson's BuilderBasedDeserializer matches
        // the wire key to this builder field. Always emit it — relying on Jackson's
        // SNAKE_CASE naming strategy alone breaks for fields whose camelCase form starts
        // with a single lowercase letter followed by an uppercase (e.g. `xRobotsTag`
        // → `xrobots_tag`, not `x_robots_tag`). The wire name is the field's
        // `#[serde(rename = "...")]` override if set, otherwise the Rust field name
        // (already snake_case per project convention).
        let wire_name: Option<String> = if is_flattened_json {
            // Flatten fields have no single wire name — the matching
            // `@JsonAnySetter` setter intercepts every unknown sibling field.
            None
        } else {
            Some(field.serde_rename.clone().unwrap_or_else(|| field.name.clone()))
        };
        if let Some(wire) = wire_name {
            body.push_str("    @JsonProperty(\"");
            body.push_str(&wire);
            body.push_str("\")\n");
        }
        body.push_str("    private ");
        body.push_str(&field_type);
        body.push(' ');
        body.push_str(&field_name);
        body.push_str(" = ");
        body.push_str(&default_value);
        body.push_str(";\n");
    }

    body.push('\n');

    // Generate withXxx() methods
    for field in &typ.fields {
        // Skip unnamed tuple fields (name is "_0", "_1", "0", "1", etc.) — Java requires named fields
        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        let field_name = safe_java_field_name(&field.name);
        let field_name_pascal = to_class_name(&field.name);
        let is_visitor_field = has_visitor_pattern && typ.name == "ConversionOptions" && field.name == "visitor";
        let is_flattened_json = field.serde_flatten && matches!(&field.ty, TypeRef::Json);
        // Builders store the visitor as Optional<Visitor> for null-safe chaining, but
        // expose `withVisitor(Visitor)` to keep the user-facing API ergonomic — callers
        // should not have to write `Optional.of(visitor)` themselves.
        let field_type = if is_visitor_field {
            "Visitor".to_string()
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if field.optional {
            format!("Optional<{}>", java_boxed_type(&field.ty))
        } else if matches!(field.ty, TypeRef::Duration) {
            java_boxed_type(&field.ty).to_string()
        } else {
            java_type(&field.ty).to_string()
        };

        body.push_str("    /** Sets the ");
        body.push_str(&field_name);
        body.push_str(" field. */\n");
        let setter_wire_name: Option<String> = if is_flattened_json {
            None
        } else {
            Some(field.serde_rename.clone().unwrap_or_else(|| field.name.clone()))
        };
        if is_flattened_json {
            // The regular `with<Field>(Map)` setter must not bind to a wire
            // field of the same name (e.g. an actual `content` array field
            // would be miscast as a `Map`). `@JsonIgnore` prevents Jackson
            // from picking it up; the matching `@JsonAnySetter` below
            // intercepts every flattened sibling field instead.
            body.push_str("    @com.fasterxml.jackson.annotation.JsonIgnore\n");
        } else if let Some(wire) = &setter_wire_name {
            // Jackson's BuilderBasedDeserializer reads property names from the `with*`
            // setter methods, not the private fields — always emit @JsonProperty so the
            // wire key maps deterministically regardless of Jackson's naming strategy
            // quirks (e.g. `withXRobotsTag` → `xrobots_tag` instead of `x_robots_tag`).
            body.push_str("    @JsonProperty(\"");
            body.push_str(wire);
            body.push_str("\")\n");
        }
        body.push_str("    public ");
        body.push_str(&typ.name);
        body.push_str("Builder with");
        body.push_str(&field_name_pascal);
        body.push_str("(final ");
        body.push_str(&field_type);
        body.push_str(" value) {\n");
        if is_visitor_field {
            body.push_str("        this.");
            body.push_str(&field_name);
            body.push_str(" = Optional.ofNullable(value);\n");
        } else {
            body.push_str("        this.");
            body.push_str(&field_name);
            body.push_str(" = value;\n");
        }
        body.push_str("        return this;\n");
        body.push_str("    }\n");
        body.push('\n');

        // Flatten field: emit `@JsonAnySetter` so Jackson absorbs unknown
        // sibling fields into the map during deserialization. Without this,
        // any field not declared on the builder triggers
        // `Unrecognized field "<name>" not marked as ignorable`.
        if is_flattened_json {
            body.push_str("    /** Absorbs unknown sibling fields (serde flatten). */\n");
            body.push_str("    @com.fasterxml.jackson.annotation.JsonAnySetter\n");
            body.push_str("    public ");
            body.push_str(&typ.name);
            body.push_str("Builder ");
            body.push_str(&field_name);
            body.push_str("Entry(final String key, final Object value) {\n");
            body.push_str("        this.");
            body.push_str(&field_name);
            body.push_str(".put(key, value);\n");
            body.push_str("        return this;\n");
            body.push_str("    }\n");
            body.push('\n');
        }
    }

    // Generate build() method
    body.push_str("    /** Builds the ");
    body.push_str(&typ.name);
    body.push_str(" instance. */\n");
    body.push_str("    public ");
    body.push_str(&typ.name);
    body.push_str(" build() {\n");
    body.push_str("        return new ");
    body.push_str(&typ.name);
    body.push_str("(\n");
    let non_tuple_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| {
            !(f.name.starts_with('_') && f.name[1..].chars().all(|c| c.is_ascii_digit())
                || f.name.chars().next().is_none_or(|c| c.is_ascii_digit()))
        })
        .collect();
    for (i, field) in non_tuple_fields.iter().enumerate() {
        let field_name = safe_java_field_name(&field.name);
        let comma = if i < non_tuple_fields.len() - 1 { "," } else { "" };
        let is_visitor_field = has_visitor_pattern && typ.name == "ConversionOptions" && field.name == "visitor";
        if field.optional || is_visitor_field {
            body.push_str("            ");
            body.push_str(&field_name);
            body.push_str(".orElse(null)");
            body.push_str(comma);
            body.push('\n');
        } else {
            body.push_str("            ");
            body.push_str(&field_name);
            body.push_str(comma);
            body.push('\n');
        }
    }
    body.push_str("        );\n");
    body.push_str("    }\n");

    body.push_str("}\n");

    // Assemble with conditional imports based on what's actually used in the body
    let mut imports: Vec<&str> = vec![];
    if body.contains("List<") {
        imports.push("java.util.List");
    }
    if body.contains("Map<") {
        imports.push("java.util.Map");
    }
    if body.contains("Optional<") {
        imports.push("java.util.Optional");
    }
    // Builder classes with @JsonPOJOBuilder annotation need Jackson imports
    if body.contains("@JsonPOJOBuilder") {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonPOJOBuilder");
    }
    if body.contains("@JsonProperty(") {
        imports.push("com.fasterxml.jackson.annotation.JsonProperty");
    }
    if body.contains("@JsonAlias(") {
        imports.push("com.fasterxml.jackson.annotation.JsonAlias");
    }
    let header = hash::header(CommentStyle::DoubleSlash);
    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&body);
    out
}

/// Generate a custom deserializer for sealed interfaces with tuple/newtype variants.
///
/// This deserializer handles the case where Jackson encounters a @JsonTypeInfo with
/// a discriminator tag but one or more variants have flattened/unwrapped fields.
/// Jackson 2.18 doesn't support @JsonUnwrapped on record creator parameters,
/// so we manually deserialize the JSON object, extract the tag, and reconstruct
/// the variant record.
/// Generate a `ByteArrayToIntArraySerializer` class for a given Java package.
///
/// Jackson serialises `byte[]` as base64 by default, but Rust's serde for `Vec<u8>`
/// expects a JSON array of integers `[72, 101, 108, …]`. This class overrides that
/// behaviour so that `BatchBytesItem.content` and any other `byte[]` field annotated
/// with `@JsonSerialize(using = ByteArrayToIntArraySerializer.class)` serialises
/// correctly at the FFI boundary.
pub(crate) fn gen_byte_array_serializer(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = [
        "com.fasterxml.jackson.core.JsonGenerator",
        "com.fasterxml.jackson.databind.SerializerProvider",
        "com.fasterxml.jackson.databind.ser.std.StdSerializer",
    ];
    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str("/**\n");
    out.push_str(" * Serialises {@code byte[]} as a JSON array of integers.\n");
    out.push_str(" *\n");
    out.push_str(" * <p>Jackson's default serialiser encodes {@code byte[]} as a base64 string, but\n");
    out.push_str(" * Rust's {@code serde} for {@code Vec<u8>} expects {@code [72, 101, 108, ...]}.\n");
    out.push_str(" * Annotate any {@code byte[]} field sent to the FFI layer with\n");
    out.push_str(" * {@code @JsonSerialize(using = ByteArrayToIntArraySerializer.class)}.\n");
    out.push_str(" */\n");
    out.push_str("public class ByteArrayToIntArraySerializer extends StdSerializer<byte[]> {\n");
    out.push_str("    /** Default constructor required by Jackson. */\n");
    out.push_str("    public ByteArrayToIntArraySerializer() {\n");
    out.push_str("        super(byte[].class);\n");
    out.push_str("    }\n\n");
    out.push_str("    @Override\n");
    out.push_str("    public void serialize(final byte[] value, final JsonGenerator gen,\n");
    out.push_str("            final SerializerProvider provider) throws java.io.IOException {\n");
    out.push_str("        gen.writeStartArray();\n");
    out.push_str("        for (byte b : value) {\n");
    out.push_str("            gen.writeNumber(b & 0xFF);\n");
    out.push_str("        }\n");
    out.push_str("        gen.writeEndArray();\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

fn gen_sealed_union_deserializer(out: &mut String, _package: &str, enum_def: &EnumDef, tag_field: &str) {
    // Generate the deserializer class inline in the same file
    // Start indentation at class level (not nested in the interface)
    out.push_str("// Custom deserializer for sealed interface with unwrapped variants\n");
    out.push_str("class ");
    out.push_str(&enum_def.name);
    out.push_str("Deserializer extends StdDeserializer<");
    out.push_str(&enum_def.name);
    out.push_str("> {\n");
    out.push_str("    ");
    out.push_str(&enum_def.name);
    out.push_str("Deserializer() {\n");
    out.push_str("        super(");
    out.push_str(&enum_def.name);
    out.push_str(".class);\n");
    out.push_str("    }\n\n");

    out.push_str("    @Override\n");
    out.push_str("    public ");
    out.push_str(&enum_def.name);
    out.push_str(" deserialize(JsonParser parser, DeserializationContext ctx)\n");
    out.push_str("            throws java.io.IOException {\n");
    out.push_str("        ObjectNode node = parser.getCodec().readTree(parser);\n");
    out.push_str("        com.fasterxml.jackson.databind.JsonNode tagNode = node.get(\"");
    out.push_str(tag_field);
    out.push_str("\");\n");
    out.push_str("        if (tagNode == null || tagNode.isNull()) {\n");
    out.push_str("            throw new com.fasterxml.jackson.databind.JsonMappingException(\n");
    out.push_str("                parser, \"Missing discriminator field: ");
    out.push_str(tag_field);
    out.push_str("\");\n");
    out.push_str("        }\n");
    out.push_str("        String tagValue = tagNode.asText();\n");
    // Remove the discriminator field before deserialising the inner type so that
    // the target builder (e.g. TextMetadataBuilder) does not encounter an
    // unrecognised property and throw UnrecognizedPropertyException.
    out.push_str("        node.remove(\"");
    out.push_str(tag_field);
    out.push_str("\");\n\n");

    // Generate a switch/case based on the tag value
    out.push_str("        return switch (tagValue) {\n");
    for variant in &enum_def.variants {
        let discriminator = variant.serde_rename.clone().unwrap_or_else(|| {
            let name = &variant.name;
            // Apply the same naming convention as the Rust enum
            enum_def
                .serde_rename_all
                .as_deref()
                .map(|strategy| java_apply_rename_all(name, Some(strategy)))
                .unwrap_or_else(|| java_apply_rename_all(name, None))
        });

        out.push_str("            case \"");
        out.push_str(&discriminator);
        out.push_str("\" -> ");

        if variant.fields.is_empty() {
            // Unit variant
            out.push_str("new ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("();\n");
        } else if variant.fields.len() == 1 && is_tuple_field_name(&variant.fields[0].name) {
            // Newtype/tuple variant - deserialize the inner type from the whole object
            let field = &variant.fields[0];
            let inner_type = java_type(&field.ty);
            out.push_str("new ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str("(ctx.readTreeAsValue(node, ");
            out.push_str(inner_type.as_ref());
            out.push_str(".class));\n");
        } else {
            // Named field variant - deserialize using Jackson's normal deserialization
            out.push_str("ctx.readTreeAsValue(node, ");
            out.push_str(&enum_def.name);
            out.push('.');
            out.push_str(&variant.name);
            out.push_str(".class);\n");
        }
    }
    out.push_str("            default -> throw new com.fasterxml.jackson.databind.JsonMappingException(\n");
    out.push_str("                parser, \"Unknown ");
    out.push_str(&enum_def.name);
    out.push_str(" discriminator: \" + tagValue);\n");
    out.push_str("        };\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// Emit the companion serializer that mirrors `gen_sealed_union_deserializer`.
///
/// For an internally-tagged enum like `#[serde(tag = "role")] enum Message { User(UserMessage), ... }`,
/// the deserializer reads the `role` field, strips it, and dispatches to the matching variant.
/// The serializer must do the inverse: emit a flat object containing the tag field plus the
/// inner record's fields. Without this, Jackson's default serialization wraps the inner value
/// (e.g. `{"value": {...UserMessage...}}`) and Rust's serde rejects the missing tag.
fn gen_sealed_union_serializer(out: &mut String, _package: &str, enum_def: &EnumDef, tag_field: &str) {
    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .map(|v| {
            let discriminator = v.serde_rename.clone().unwrap_or_else(|| {
                let name = &v.name;
                enum_def
                    .serde_rename_all
                    .as_deref()
                    .map(|strategy| java_apply_rename_all(name, Some(strategy)))
                    .unwrap_or_else(|| java_apply_rename_all(name, None))
            });
            let is_unit = v.fields.is_empty();
            let is_tuple = !is_unit && v.fields.len() == 1 && is_tuple_field_name(&v.fields[0].name);
            minijinja::context! {
                name => &v.name,
                discriminator => discriminator,
                is_unit => is_unit,
                is_tuple => is_tuple,
            }
        })
        .collect();
    out.push_str(&crate::template_env::render(
        "sealed_union_serializer.jinja",
        minijinja::context! {
            class_name => &enum_def.name,
            tag_field => tag_field,
            variants => variants,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashSet;
    use alef_core::ir::{CoreWrapper, DefaultValue, FieldDef, PrimitiveType, TypeRef};

    fn make_config_type_with_duration_default() -> TypeDef {
        TypeDef {
            name: "CrawlConfig".to_string(),
            rust_path: "kreuzberg::CrawlConfig".to_string(),
            original_rust_path: "kreuzberg::CrawlConfig".to_string(),
            fields: vec![FieldDef {
                name: "request_timeout".to_string(),
                ty: TypeRef::Duration,
                optional: false,
                default: Some("30000".to_string()),
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: Some(DefaultValue::IntLiteral(30000)),
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
        }
    }

    fn make_request_type_with_multiword_fields() -> TypeDef {
        TypeDef {
            name: "ChatCompletionRequest".to_string(),
            rust_path: "liter_llm::ChatCompletionRequest".to_string(),
            original_rust_path: "liter_llm::ChatCompletionRequest".to_string(),
            fields: vec![
                FieldDef {
                    name: "model".to_string(),
                    ty: TypeRef::String,
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
                },
                FieldDef {
                    name: "max_tokens".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
                    optional: true,
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
                },
                FieldDef {
                    name: "top_p".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                    optional: true,
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
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
        }
    }

    /// Single-word field names like `model` should NOT get `@JsonProperty`
    /// — camelCase equals snake_case, so no annotation is needed.
    #[test]
    fn single_word_field_has_no_json_property() {
        let typ = make_request_type_with_multiword_fields();
        let out = gen_record_type(
            "dev.kreuzberg",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            false,
            "LiterLlmRs",
        );
        // `model` is single-word: camelCase == snake_case, so no annotation needed.
        assert!(
            !out.contains("@JsonProperty(\"model\")"),
            "single-word field must not get @JsonProperty"
        );
    }

    /// Multi-word snake_case fields like `max_tokens` → `maxTokens` MUST get
    /// `@JsonProperty("max_tokens")` so Jackson sends the snake_case wire name
    /// that Rust's serde expects.
    #[test]
    fn multiword_snake_case_field_gets_json_property_annotation() {
        let typ = make_request_type_with_multiword_fields();
        let out = gen_record_type(
            "dev.kreuzberg",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            false,
            "LiterLlmRs",
        );
        assert!(
            out.contains("@JsonProperty(\"max_tokens\")"),
            "multi-word field max_tokens must have @JsonProperty(\"max_tokens\") annotation; got:\n{out}"
        );
        assert!(
            out.contains("@JsonProperty(\"top_p\")"),
            "multi-word field top_p must have @JsonProperty(\"top_p\") annotation; got:\n{out}"
        );
        // The import must also be present.
        assert!(
            out.contains("import com.fasterxml.jackson.annotation.JsonProperty;"),
            "JsonProperty import must be present when @JsonProperty annotations are emitted"
        );
    }

    #[test]
    fn boxed_duration_compact_ctor_only_null_checks_not_zero() {
        let typ = make_config_type_with_duration_default();
        let out = gen_record_type(
            "dev.kreuzberg",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            false,
            "Kreuzcrawl",
        );
        assert!(
            out.contains("requestTimeout == null"),
            "expected null-check in compact ctor"
        );
        assert!(
            !out.contains("requestTimeout == 0"),
            "must not coerce explicit 0 — that is a user-intentional value"
        );
    }
}
