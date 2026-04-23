use crate::type_map::{java_boxed_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_class_name;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{EnumDef, PrimitiveType, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

use super::helpers::{
    emit_javadoc, escape_javadoc_line, format_optional_value, is_tuple_field_name, java_apply_rename_all,
    safe_java_field_name,
};

const RECORD_LINE_WRAP_THRESHOLD: usize = 100;

pub(crate) fn gen_record_type(
    package: &str,
    typ: &TypeDef,
    complex_enums: &AHashSet<String>,
    lang_rename_all: &str,
) -> String {
    // Generate the record body first, then scan for needed imports.
    // For each field, if the language uses camelCase but the JSON key is snake_case
    // (the Rust default), annotate with @JsonProperty so Jackson maps correctly.
    // Also collect per-field doc strings for Javadoc emission.
    let (field_list, field_docs): (Vec<String>, Vec<String>) = typ
        .fields
        .iter()
        .map(|f| {
            // Complex enums (tagged unions with data) can't be simple Java enums.
            // Use Object for flexible Jackson deserialization.
            let is_complex = matches!(&f.ty, TypeRef::Named(n) if complex_enums.contains(n.as_str()));
            let ftype = if is_complex {
                "Object".to_string()
            } else if f.optional {
                format!("Optional<{}>", java_boxed_type(&f.ty))
            } else {
                java_type(&f.ty).to_string()
            };
            let jname = safe_java_field_name(&f.name);
            // When the language convention is camelCase but the JSON wire format uses
            // snake_case (the Rust/serde default), add an explicit @JsonProperty annotation
            // so Jackson serialises/deserialises using the correct snake_case key.
            let decl = if lang_rename_all == "camelCase" && f.name.contains('_') {
                format!("@JsonProperty(\"{}\") {} {}", f.name, ftype, jname)
            } else {
                format!("{} {}", ftype, jname)
            };
            (decl, f.doc.clone())
        })
        .unzip();

    // Build the single-line form to check length and scan for imports.
    // Doc strings are intentionally excluded from this check so the threshold
    // stays stable regardless of documentation presence.
    let single_line = format!("public record {}({}) {{ }}", typ.name, field_list.join(", "));

    // Build the actual record declaration, splitting across lines if too long.
    let mut record_block = String::new();
    emit_javadoc(&mut record_block, &typ.doc, "");
    if single_line.len() > RECORD_LINE_WRAP_THRESHOLD && field_list.len() > 1 {
        writeln!(record_block, "public record {}(", typ.name).ok();
        for (i, (field, doc)) in field_list.iter().zip(field_docs.iter()).enumerate() {
            let comma = if i < field_list.len() - 1 { "," } else { "" };
            if !doc.is_empty() {
                // Inline single-line doc for record components in multi-line form.
                let doc_summary = escape_javadoc_line(doc.lines().next().unwrap_or("").trim());
                writeln!(record_block, "    /** {doc_summary} */").ok();
            }
            writeln!(record_block, "    {}{}", field, comma).ok();
        }
        writeln!(record_block, ") {{").ok();
    } else {
        writeln!(record_block, "public record {}({}) {{", typ.name, field_list.join(", ")).ok();
    }

    // Add builder() factory method if type has defaults
    if typ.has_default {
        writeln!(record_block, "    public static {}Builder builder() {{", typ.name).ok();
        writeln!(record_block, "        return new {}Builder();", typ.name).ok();
        writeln!(record_block, "    }}").ok();
    }

    writeln!(record_block, "}}").ok();

    // Scan the single-line form to determine which imports are needed
    let needs_json_property = field_list.iter().any(|f| f.contains("@JsonProperty("));
    let mut out = String::with_capacity(record_block.len() + 512);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    if single_line.contains("List<") {
        writeln!(out, "import java.util.List;").ok();
    }
    if single_line.contains("Map<") {
        writeln!(out, "import java.util.Map;").ok();
    }
    if single_line.contains("Optional<") {
        writeln!(out, "import java.util.Optional;").ok();
    }
    if needs_json_property {
        writeln!(out, "import com.fasterxml.jackson.annotation.JsonProperty;").ok();
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

            let single = format!(
                "    record {}({}) implements {} {{ }}",
                variant.name,
                field_parts.join(", "),
                enum_def.name
            );

            if !variant.doc.is_empty() {
                let doc_summary = escape_javadoc_line(variant.doc.lines().next().unwrap_or("").trim());
                writeln!(out, "    /** {doc_summary} */").ok();
            }
            if single.len() > RECORD_LINE_WRAP_THRESHOLD && field_parts.len() > 1 {
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
                    variant.name,
                    field_parts.join(", "),
                    enum_def.name
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
                        PrimitiveType::Bool => "false".to_string(),
                        PrimitiveType::F32 | PrimitiveType::F64 => "0.0".to_string(),
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
        writeln!(body, "            {}{}", field_name, comma).ok();
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

    writeln!(out).ok();
    out.push_str(&body);

    out
}
