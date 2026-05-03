use crate::type_map::{java_boxed_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_class_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{DefaultValue, EnumDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

use super::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, emit_javadoc, escape_javadoc_line, format_optional_value, is_tuple_field_name,
    java_apply_rename_all, safe_java_field_name,
};

pub(crate) fn gen_record_type(
    package: &str,
    typ: &TypeDef,
    complex_enums: &AHashSet<String>,
    lang_rename_all: &str,
) -> String {
    // Single pass: build per-field (decl, doc) pairs and the comma-joined declaration
    // string simultaneously.  This avoids the map+unzip double-allocation and the
    // second .join(", ") that previously rebuilt the same content for emission.
    //
    // `fields_joined` holds the comma-separated parameter list used both for the
    // single-line length probe AND for the final single-line emit path — no rebuild.
    struct FieldEntry {
        decl: String,
        doc: String,
    }

    let mut field_entries: Vec<FieldEntry> = Vec::with_capacity(typ.fields.len());
    // Pre-size: average field decl ≈ 40 chars + 2 for ", " separator.
    let mut fields_joined = String::with_capacity(typ.fields.len().saturating_mul(42));

    for (i, f) in typ.fields.iter().enumerate() {
        // Complex enums (tagged unions with data) can't be simple Java enums.
        // Use Object for flexible Jackson deserialization.
        let is_complex = matches!(&f.ty, TypeRef::Named(n) if complex_enums.contains(n.as_str()));
        let ftype = if is_complex {
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

        // When the language convention is camelCase but the JSON wire format uses
        // snake_case (the Rust/serde default), add an explicit @JsonProperty annotation
        // so Jackson serialises/deserialises using the correct snake_case key.
        let has_json_property = lang_rename_all == "camelCase" && f.name.contains('_');
        let has_nullable = f.optional;

        let mut decl = String::new();
        if has_nullable {
            decl.push_str("@Nullable ");
        }
        if needs_non_null {
            decl.push_str("@JsonInclude(JsonInclude.Include.NON_NULL) ");
        }
        if has_json_property {
            decl.push_str(&format!("@JsonProperty(\"{}\") ", f.name));
        }
        decl.push_str(&format!("{} {}", ftype, jname));

        if i > 0 {
            fields_joined.push_str(", ");
        }
        fields_joined.push_str(&decl);

        field_entries.push(FieldEntry {
            decl,
            doc: f.doc.clone(),
        });
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
        writeln!(record_block, "@JsonInclude(JsonInclude.Include.NON_ABSENT)").ok();
    }
    // When a builder is available, configure Jackson to use it during deserialization.
    // This ensures that fields with serde defaults (e.g., `enabled = true`) use the
    // builder's defaults instead of Java primitive defaults (false for bool).
    if typ.has_default {
        writeln!(record_block, "@com.fasterxml.jackson.databind.annotation.JsonDeserialize(builder = {}Builder.class)", typ.name).ok();
    }
    if single_line_len > RECORD_LINE_WRAP_THRESHOLD && field_entries.len() > 1 {
        writeln!(record_block, "public record {}(", typ.name).ok();
        for (i, entry) in field_entries.iter().enumerate() {
            let comma = if i < field_entries.len() - 1 { "," } else { "" };
            if !entry.doc.is_empty() {
                // Inline single-line doc for record components in multi-line form.
                let doc_summary = escape_javadoc_line(entry.doc.lines().next().unwrap_or("").trim());
                writeln!(record_block, "    /** {doc_summary} */").ok();
            }
            writeln!(record_block, "    {}{}", entry.decl, comma).ok();
        }
        writeln!(record_block, ") {{").ok();
    } else {
        // Reuse fields_joined — no second allocation.
        writeln!(record_block, "public record {}({}) {{", typ.name, fields_joined).ok();
    }

    // Add builder() factory method if type has defaults
    if typ.has_default {
        writeln!(record_block, "    public static {}Builder builder() {{", typ.name).ok();
        writeln!(record_block, "        return new {}Builder();", typ.name).ok();
        writeln!(record_block, "    }}").ok();
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
                    Some(format!("        if ({jname} == 0) {jname} = {n};"))
                }
                _ => None,
            }
        })
        .collect();

    if !compact_ctor_lines.is_empty() {
        writeln!(record_block, "    public {}{{", typ.name).ok();
        for line in &compact_ctor_lines {
            writeln!(record_block, "{line}").ok();
        }
        writeln!(record_block, "    }}").ok();
    }

    // Note: do NOT emit Optional<String>-returning shadow accessors for nullable
    // String fields here. Records auto-generate canonical accessors with the
    // same return type as the component, and you cannot legally override them
    // with a different signature. Callers wanting `Optional` should use
    // `Optional.ofNullable(record.content())` at the call site, or the e2e
    // codegen emits a null-safe pattern.

    writeln!(record_block, "}}").ok();

    // Scan fields_joined (the joined field declarations) to determine which imports are needed.
    let needs_json_property = fields_joined.contains("@JsonProperty(");
    // @JsonInclude may appear in field annotations OR as a class-level annotation in record_block.
    let needs_json_include = fields_joined.contains("@JsonInclude(") || record_block.contains("@JsonInclude(");
    let needs_json_deserialize = record_block.contains("@com.fasterxml.jackson.databind.annotation.JsonDeserialize(");
    let needs_nullable = fields_joined.contains("@Nullable");
    // Optional is needed if fields have Optional<T> in declaration
    let needs_optional = fields_joined.contains("Optional<");
    let mut out = String::with_capacity(record_block.len() + 512);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    if fields_joined.contains("List<") {
        writeln!(out, "import java.util.List;").ok();
    }
    if fields_joined.contains("Map<") {
        writeln!(out, "import java.util.Map;").ok();
    }
    if needs_optional {
        writeln!(out, "import java.util.Optional;").ok();
    }
    if needs_json_property {
        writeln!(out, "import com.fasterxml.jackson.annotation.JsonProperty;").ok();
    }
    if needs_json_include {
        writeln!(out, "import com.fasterxml.jackson.annotation.JsonInclude;").ok();
    }
    if needs_json_deserialize {
        writeln!(out, "import com.fasterxml.jackson.databind.annotation.JsonDeserialize;").ok();
    }
    if needs_nullable {
        writeln!(out, "import org.jetbrains.annotations.Nullable;").ok();
    }
    writeln!(out).ok();
    write!(out, "{}", record_block).ok();

    out
}

pub(crate) fn gen_enum_class(package: &str, enum_def: &EnumDef) -> String {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Tagged union: enum has a serde tag AND data variants → generate sealed interface hierarchy
    if enum_def.serde_tag.is_some() && has_data_variants {
        return gen_java_tagged_union(package, enum_def);
    }

    let mut out = String::with_capacity(1024);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    writeln!(out, "import com.fasterxml.jackson.annotation.JsonCreator;").ok();
    writeln!(out, "import com.fasterxml.jackson.annotation.JsonValue;").ok();
    writeln!(out).ok();

    emit_javadoc(&mut out, &enum_def.doc, "");
    writeln!(out, "public enum {} {{", enum_def.name).ok();

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
                writeln!(out, "    /**").ok();
                writeln!(out, "     * {doc_summary}").ok();
                writeln!(out, "     */").ok();
            } else {
                writeln!(out, "    /** {doc_summary} */").ok();
            }
        }
        writeln!(out, "    {}(\"{}\"){}", variant.name, json_name, comma).ok();
    }

    writeln!(out).ok();
    writeln!(out, "    /** The string value. */").ok();
    writeln!(out, "    private final String value;").ok();
    writeln!(out).ok();
    writeln!(out, "    {}(final String value) {{", enum_def.name).ok();
    writeln!(out, "        this.value = value;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Returns the string value. */").ok();
    writeln!(out, "    @JsonValue").ok();
    writeln!(out, "    public String getValue() {{").ok();
    writeln!(out, "        return value;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Creates an instance from a string value. */").ok();
    writeln!(out, "    @JsonCreator").ok();
    writeln!(
        out,
        "    public static {} fromValue(final String value) {{",
        enum_def.name
    )
    .ok();
    writeln!(out, "        for ({} e : values()) {{", enum_def.name).ok();
    writeln!(out, "            if (e.value.equalsIgnoreCase(value)) {{").ok();
    writeln!(out, "                return e;").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }}").ok();
    writeln!(
        out,
        "        throw new IllegalArgumentException(\"Unknown value: \" + value);"
    )
    .ok();
    writeln!(out, "    }}").ok();

    writeln!(out, "}}").ok();

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

    let mut out = String::with_capacity(2048);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    if needs_json_property {
        writeln!(out, "import com.fasterxml.jackson.annotation.JsonProperty;").ok();
    }
    writeln!(out, "import com.fasterxml.jackson.annotation.JsonSubTypes;").ok();
    writeln!(out, "import com.fasterxml.jackson.annotation.JsonTypeInfo;").ok();

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
    // into the parent JSON object using @JsonUnwrapped.
    let needs_unwrapped = enum_def
        .variants
        .iter()
        .any(|v| v.fields.len() == 1 && is_tuple_field_name(&v.fields[0].name));
    if needs_list {
        writeln!(out, "import java.util.List;").ok();
    }
    if needs_map {
        writeln!(out, "import java.util.Map;").ok();
    }
    if needs_optional {
        writeln!(out, "import java.util.Optional;").ok();
    }
    if needs_unwrapped {
        writeln!(out, "import com.fasterxml.jackson.annotation.JsonUnwrapped;").ok();
    }
    writeln!(out).ok();

    emit_javadoc(&mut out, &enum_def.doc, "");
    // @JsonTypeInfo and @JsonSubTypes annotations
    writeln!(
        out,
        "@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, property = \"{tag_field}\", visible = false)"
    )
    .ok();
    writeln!(out, "@JsonSubTypes({{").ok();
    for (i, variant) in enum_def.variants.iter().enumerate() {
        let discriminator = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| java_apply_rename_all(&variant.name, enum_def.serde_rename_all.as_deref()));
        let comma = if i < enum_def.variants.len() - 1 { "," } else { "" };
        writeln!(
            out,
            "    @JsonSubTypes.Type(value = {}.{}.class, name = \"{}\"){}",
            enum_def.name, variant.name, discriminator, comma
        )
        .ok();
    }
    writeln!(out, "}})").ok();
    writeln!(out, "public sealed interface {} {{", enum_def.name).ok();

    // Nested records for each variant
    for variant in &enum_def.variants {
        writeln!(out).ok();
        if variant.fields.is_empty() {
            // Unit variant
            if !variant.doc.is_empty() {
                let doc_summary = escape_javadoc_line(variant.doc.lines().next().unwrap_or("").trim());
                writeln!(out, "    /** {doc_summary} */").ok();
            }
            writeln!(out, "    record {}() implements {} {{", variant.name, enum_def.name).ok();
            writeln!(out, "    }}").ok();
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
                    // alongside the tag. Use @JsonUnwrapped so Jackson does the same.
                    if is_tuple_field_name(&f.name) {
                        format!("@JsonUnwrapped {ftype} value")
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
                writeln!(out, "    /** {doc_summary} */").ok();
            }
            if single_len > RECORD_LINE_WRAP_THRESHOLD && field_parts.len() > 1 {
                writeln!(out, "    record {}(", variant.name).ok();
                for (i, fp) in field_parts.iter().enumerate() {
                    let comma = if i < field_parts.len() - 1 { "," } else { "" };
                    writeln!(out, "        {}{}", fp, comma).ok();
                }
                writeln!(out, "    ) implements {} {{", enum_def.name).ok();
                writeln!(out, "    }}").ok();
            } else {
                writeln!(
                    out,
                    "    record {}({}) implements {} {{ }}",
                    variant.name, fields_joined, enum_def.name
                )
                .ok();
            }
        }
    }

    writeln!(out).ok();
    writeln!(out, "}}").ok();
    out
}

pub(crate) fn gen_opaque_handle_class(package: &str, typ: &TypeDef, prefix: &str) -> String {
    let mut out = String::with_capacity(1024);
    let class_name = &typ.name;
    let type_snake = class_name.to_snake_case();

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    writeln!(out, "import java.lang.foreign.MemorySegment;").ok();
    writeln!(out).ok();

    if !typ.doc.is_empty() {
        writeln!(out, "/**").ok();
        for line in typ.doc.lines() {
            writeln!(out, " * {}", line).ok();
        }
        writeln!(out, " */").ok();
    }

    writeln!(out, "public class {} implements AutoCloseable {{", class_name).ok();
    writeln!(out, "    private final MemorySegment handle;").ok();
    writeln!(out).ok();
    writeln!(out, "    {}(MemorySegment handle) {{", class_name).ok();
    writeln!(out, "        this.handle = handle;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    MemorySegment handle() {{").ok();
    writeln!(out, "        return this.handle;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    @Override").ok();
    writeln!(out, "    public void close() {{").ok();
    writeln!(
        out,
        "        if (handle != null && !handle.equals(MemorySegment.NULL)) {{"
    )
    .ok();
    writeln!(out, "            try {{").ok();
    writeln!(
        out,
        "                NativeLib.{}_{}_FREE.invoke(handle);",
        prefix.to_uppercase(),
        type_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "            }} catch (Throwable e) {{").ok();
    writeln!(
        out,
        "                throw new RuntimeException(\"Failed to free {}: \" + e.getMessage(), e);",
        class_name
    )
    .ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

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
pub(crate) fn gen_builder_class(package: &str, typ: &TypeDef) -> String {
    let mut body = String::with_capacity(2048);

    emit_javadoc(&mut body, &typ.doc, "");
    // Annotation tells Jackson to use this builder when deserializing the record.
    // Builder defaults (e.g., enabled=true) are applied during deserialization.
    writeln!(body, "@com.fasterxml.jackson.databind.annotation.JsonPOJOBuilder(withPrefix = \"with\")").ok();
    writeln!(body, "public class {}Builder {{", typ.name).ok();
    writeln!(body).ok();

    // Generate field declarations with defaults
    for field in &typ.fields {
        let field_name = safe_java_field_name(&field.name);

        // Skip unnamed tuple fields (name is "_0", "_1", "0", "1", etc.) — Java requires named fields
        if field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit())
            || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            continue;
        }

        // Duration maps to primitive `long` in the public record, but in builder
        // classes we use boxed `Long` so that `null` can represent "not set".
        let field_type = if field.optional {
            format!("Optional<{}>", java_boxed_type(&field.ty))
        } else if matches!(field.ty, TypeRef::Duration) {
            java_boxed_type(&field.ty).to_string()
        } else {
            java_type(&field.ty).to_string()
        };

        let default_value = if field.optional {
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
                    TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
                    TypeRef::Json => "null".to_string(),
                    TypeRef::Bytes => "new byte[0]".to_string(),
                    TypeRef::Primitive(p) => match p {
                        PrimitiveType::Bool => {
                            // Special handling for boolean fields with serde defaults
                            // Check if this is a known type with non-standard defaults
                            let should_be_true = (typ.name == "PreprocessingOptions"
                                && matches!(field.name.as_str(), "enabled" | "remove_navigation" | "remove_forms"))
                                || (typ.name == "ConversionOptions"
                                    && matches!(field.name.as_str(), "autolinks" | "default_title" | "br_in_tables"
                                        | "wrap" | "extract_metadata" | "escape_asterisks" | "escape_underscores"
                                        | "escape_misc" | "escape_ascii" | "include_document_structure" | "extract_images"
                                        | "capture_svg" | "infer_dimensions" | "debug" | "skip_images"));

                            if should_be_true {
                                "true".to_string()
                            } else {
                                "false".to_string()
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

        writeln!(body, "    private {} {} = {};", field_type, field_name, default_value).ok();
    }

    writeln!(body).ok();

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
        let field_type = if field.optional {
            format!("Optional<{}>", java_boxed_type(&field.ty))
        } else if matches!(field.ty, TypeRef::Duration) {
            java_boxed_type(&field.ty).to_string()
        } else {
            java_type(&field.ty).to_string()
        };

        writeln!(body, "    /** Sets the {} field. */", field_name).ok();
        writeln!(
            body,
            "    public {}Builder with{}(final {} value) {{",
            typ.name, field_name_pascal, field_type
        )
        .ok();
        writeln!(body, "        this.{} = value;", field_name).ok();
        writeln!(body, "        return this;").ok();
        writeln!(body, "    }}").ok();
        writeln!(body).ok();
    }

    // Generate build() method
    writeln!(body, "    /** Builds the {} instance. */", typ.name).ok();
    writeln!(body, "    public {} build() {{", typ.name).ok();
    writeln!(body, "        return new {}(", typ.name).ok();
    let non_tuple_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| {
            // Include named fields (skip unnamed tuple fields)
            !(f.name.starts_with('_') && f.name[1..].chars().all(|c| c.is_ascii_digit())
                || f.name.chars().next().is_none_or(|c| c.is_ascii_digit()))
        })
        .collect();
    for (i, field) in non_tuple_fields.iter().enumerate() {
        let field_name = safe_java_field_name(&field.name);
        let comma = if i < non_tuple_fields.len() - 1 { "," } else { "" };
        // For optional fields, extract the value from Optional using orElse(null)
        if field.optional {
            writeln!(body, "            {}.orElse(null){}", field_name, comma).ok();
        } else {
            writeln!(body, "            {}{}", field_name, comma).ok();
        }
    }
    writeln!(body, "        );").ok();
    writeln!(body, "    }}").ok();

    writeln!(body, "}}").ok();

    // Now assemble with conditional imports based on what's actually used in the body
    let mut out = String::with_capacity(body.len() + 512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();

    if body.contains("List<") {
        writeln!(out, "import java.util.List;").ok();
    }
    if body.contains("Map<") {
        writeln!(out, "import java.util.Map;").ok();
    }
    if body.contains("Optional<") {
        writeln!(out, "import java.util.Optional;").ok();
    }
    // Builder classes with @JsonPOJOBuilder annotation need Jackson imports
    if body.contains("@com.fasterxml.jackson.databind.annotation.JsonPOJOBuilder") {
        writeln!(out, "import com.fasterxml.jackson.databind.annotation.JsonPOJOBuilder;").ok();
    }

    writeln!(out).ok();
    out.push_str(&body);

    out
}
