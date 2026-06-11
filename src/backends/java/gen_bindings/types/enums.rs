use crate::backends::java::type_map::{java_boxed_type, java_type};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{EnumDef, TypeRef};
use heck::ToLowerCamelCase;

use super::serializers::{gen_sealed_union_deserializer, gen_sealed_union_serializer};
use crate::backends::java::gen_bindings::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, emit_javadoc, escape_javadoc_line, is_tuple_field_name, java_apply_rename_all,
    safe_java_field_name,
};

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
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');

    let mut enum_javadocs = String::new();
    emit_javadoc(&mut enum_javadocs, &enum_def.doc, "");
    let mut variants_block = String::new();
    for (i, variant) in enum_def.variants.iter().enumerate() {
        let comma = if i < enum_def.variants.len() - 1 { "," } else { ";" };
        // Use serde_rename if available, otherwise apply rename_all strategy.
        // When the Rust enum has no explicit #[serde(rename_all)], Serde uses the variant
        // name unchanged (PascalCase), but Rust may have custom deserialization via a parse()
        // function that expects lowercase. To match Rust's deserialization expectations, always
        // apply lowercase normalization when rename_all is not explicitly set.
        let json_name = variant
            .serde_rename
            .clone()
            .unwrap_or_else(|| match enum_def.serde_rename_all.as_deref() {
                Some(rename_all) => java_apply_rename_all(&variant.name, Some(rename_all)),
                None => variant.name.to_lowercase(),
            });
        emit_javadoc(&mut variants_block, &variant.doc, "    ");
        variants_block.push_str("    ");
        variants_block.push_str(&variant.name);
        variants_block.push_str("(\"");
        variants_block.push_str(&json_name);
        variants_block.push_str("\")");
        variants_block.push_str(comma);
        variants_block.push('\n');
    }
    variants_block.push('\n');

    // Collect excluded variant names to document in comments or emit validation logic
    let excluded_variant_json_names: Vec<String> = enum_def
        .excluded_variants
        .iter()
        .map(|v| {
            v.serde_rename
                .clone()
                .unwrap_or_else(|| match enum_def.serde_rename_all.as_deref() {
                    Some(rename_all) => java_apply_rename_all(&v.name, Some(rename_all)),
                    None => v.name.to_lowercase(),
                })
        })
        .collect();

    out.push_str(&crate::backends::java::template_env::render(
        "simple_enum_class.jinja",
        minijinja::context! {
            javadocs => enum_javadocs,
            enum_name => &enum_def.name,
            variants_block => variants_block,
            has_excluded_variants => !excluded_variant_json_names.is_empty(),
            excluded_variant_names => excluded_variant_json_names,
        },
    ));

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
    crate::backends::java::template_env::render(
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
    let mut out = crate::backends::java::template_env::render(
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
        // A single tuple field of type `()` (Rust unit) carries no data — emit a
        // payload-less record so Java doesn't try to declare a `void value` field,
        // which fails compilation with "void type not allowed here".
        let is_unit_tuple = variant.fields.len() == 1
            && is_tuple_field_name(&variant.fields[0].name)
            && matches!(&variant.fields[0].ty, TypeRef::Unit);
        if variant.fields.is_empty() || is_unit_tuple {
            // Unit variant
            emit_javadoc(&mut out, &variant.doc, "    ");
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
                        // Replace "List"/"Map" with fully qualified if conflicting. Use
                        // `replace` (all occurrences) so nested `List<List<T>>` also resolves
                        // the inner `List` to `java.util.List`, not the shadowing variant.
                        let mut inner_qualified = inner_str.to_string();
                        if variant_names.contains("List") {
                            inner_qualified = inner_qualified.replace("List<", "java.util.List<");
                        }
                        if variant_names.contains("Map") {
                            inner_qualified = inner_qualified.replace("Map<", "java.util.Map<");
                        }
                        format!("{optional_type}<{inner_qualified}>")
                    } else {
                        let t = java_type(&f.ty);
                        let mut t_str = t.into_owned();
                        if variant_names.contains("List") {
                            t_str = t_str.replace("List<", "java.util.List<");
                        }
                        if variant_names.contains("Map") {
                            t_str = t_str.replace("Map<", "java.util.Map<");
                        }
                        t_str
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

            emit_javadoc(&mut out, &variant.doc, "    ");
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
            // Skip accessors for unit-tuple variants — there's no value to return.
            if matches!(&variant.fields[0].ty, TypeRef::Unit) {
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
