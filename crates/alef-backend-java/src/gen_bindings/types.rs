use crate::type_map::{java_boxed_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_class_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{DefaultValue, EnumDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};

use super::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, emit_javadoc, escape_javadoc_line, format_optional_value, is_tuple_field_name,
    java_apply_rename_all, safe_java_field_name,
};

pub(crate) fn gen_record_type(
    package: &str,
    typ: &TypeDef,
    complex_enums: &AHashSet<String>,
    lang_rename_all: &str,
    has_visitor_pattern: bool,
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
        let ftype = if is_visitor_field {
            "Visitor".to_string()
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
        let needs_non_null = !f.optional && matches!(&f.ty, TypeRef::Vec(_));

        // Non-optional Bytes fields (byte[]) must be serialised as a JSON array of
        // integers, not as a base64 string. Jackson's default serialiser for byte[]
        // produces base64, but Rust's serde for Vec<u8> expects [n, n, …].
        // @JsonSerialize(using = ByteArrayToIntArraySerializer.class) overrides the
        // default Jackson behaviour for this field only.
        let needs_bytes_int_serialize = !f.optional && matches!(&f.ty, TypeRef::Bytes);

        // When the language convention is camelCase but the JSON wire format uses
        // snake_case (the Rust/serde default), add an explicit @JsonProperty annotation
        // so Jackson serialises/deserialises using the correct snake_case key.
        let has_json_property = lang_rename_all == "camelCase" && f.name.contains('_');
        let has_nullable = f.optional;

        let mut decl = String::new();

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
        if has_json_property && !is_visitor_field {
            decl.push_str("@JsonProperty(\"");
            decl.push_str(&f.name);
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
                    // will auto-box correctly.
                    let suffix = if matches!(f.ty, TypeRef::Duration) { "L" } else { "" };
                    Some(format!("        if ({jname} == 0) {jname} = {n}{suffix};"))
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
    let needs_json_deserialize = record_block.contains("@JsonDeserialize(");
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

pub(crate) fn gen_enum_class(package: &str, enum_def: &EnumDef) -> String {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Tagged union: enum has a serde tag AND data variants → generate sealed interface hierarchy
    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_java_tagged_union(package, enum_def);
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
    imports.push("com.fasterxml.jackson.annotation.JsonSubTypes");
    imports.push("com.fasterxml.jackson.annotation.JsonTypeInfo");
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
        imports.push("com.fasterxml.jackson.databind.JsonDeserializer");
        imports.push("com.fasterxml.jackson.databind.deser.std.StdDeserializer");
        imports.push("com.fasterxml.jackson.core.JsonParser");
        imports.push("com.fasterxml.jackson.databind.DeserializationContext");
        imports.push("com.fasterxml.jackson.databind.node.ObjectNode");
        imports.push("com.fasterxml.jackson.databind.annotation.JsonDeserialize");
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
    // @JsonTypeInfo and @JsonSubTypes annotations
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
    // Newtype variants with flattened fields cannot directly map to record fields.
    // Allow unknown properties at the interface level so Jackson doesn't fail when
    // encountering flattened inner-type fields.
    out.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    if needs_unwrapped {
        out.push_str("@JsonDeserialize(using = ");
        out.push_str(&enum_def.name);
        out.push_str("Deserializer.class)\n");
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

    // Generate custom deserializer for sealed interfaces with unwrapped variants
    if needs_unwrapped {
        out.push('\n');
        gen_sealed_union_deserializer(&mut out, package, enum_def, tag_field);
    }

    out
}

pub(crate) fn gen_opaque_handle_class(package: &str, typ: &TypeDef, prefix: &str) -> String {
    let class_name = &typ.name;
    let type_snake = class_name.to_snake_case();
    let header = hash::header(CommentStyle::DoubleSlash);
    let imports = ["java.lang.foreign.MemorySegment"];
    let mut out = crate::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');

    emit_javadoc(&mut out, &typ.doc, "");

    out.push_str("public class ");
    out.push_str(class_name);
    out.push_str(" implements AutoCloseable {\n");
    out.push_str("    private final MemorySegment handle;\n");
    out.push('\n');
    out.push_str("    ");
    out.push_str(class_name);
    out.push_str("(MemorySegment handle) {\n");
    out.push_str("        this.handle = handle;\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    MemorySegment handle() {\n");
    out.push_str("        return this.handle;\n");
    out.push_str("    }\n");
    out.push('\n');
    out.push_str("    @Override\n");
    out.push_str("    public void close() {\n");
    out.push_str("        if (handle != null && !handle.equals(MemorySegment.NULL)) {\n");
    out.push_str("            try {\n");
    out.push_str("                NativeLib.");
    out.push_str(&prefix.to_uppercase());
    out.push('_');
    out.push_str(&type_snake.to_uppercase());
    out.push_str("_FREE.invoke(handle);\n");
    out.push_str("            } catch (Throwable e) {\n");
    out.push_str("                throw new RuntimeException(\"Failed to free ");
    out.push_str(class_name);
    out.push_str(": \" + e.getMessage(), e);\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
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

        // Duration maps to primitive `long` in the public record, but in builder
        // classes we use boxed `Long` so that `null` can represent "not set".
        let field_type = if is_visitor_field {
            "Optional<Visitor>".to_string()
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
        // Builders store the visitor as Optional<Visitor> for null-safe chaining, but
        // expose `withVisitor(Visitor)` to keep the user-facing API ergonomic — callers
        // should not have to write `Optional.of(visitor)` themselves.
        let field_type = if is_visitor_field {
            "Visitor".to_string()
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

fn gen_sealed_union_deserializer(out: &mut String, package: &str, enum_def: &EnumDef, tag_field: &str) {
    // Generate the deserializer class inline in the same file
    // Start indentation at class level (not nested in the interface)
    out.push_str("// Custom deserializer for sealed interface with unwrapped variants\n");
    out.push_str("class ");
    out.push_str(&enum_def.name);
    out.push_str("Deserializer extends StdDeserializer<");
    out.push_str(&enum_def.name);
    out.push_str("> {\n");
    out.push_str("    public ");
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
    out.push_str("        String tagValue = node.get(\"");
    out.push_str(tag_field);
    out.push_str("\").asText(null);\n");
    out.push_str("        if (tagValue == null) {\n");
    out.push_str("            throw new com.fasterxml.jackson.databind.JsonMappingException(\n");
    out.push_str("                parser, \"Missing discriminator field: ");
    out.push_str(tag_field);
    out.push_str("\");\n");
    out.push_str("        }\n\n");

    // Generate a switch/case based on the tag value
    out.push_str("        return switch (tagValue) {\n");
    for variant in &enum_def.variants {
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| {
                let name = &variant.name;
                // Apply the same naming convention as the Rust enum
                enum_def.serde_rename_all.as_deref()
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

