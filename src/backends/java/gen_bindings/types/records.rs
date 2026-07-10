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
    let visible_fields: Vec<_> = binding_fields(&typ.fields).collect();
    let mut fields_joined = String::with_capacity(visible_fields.len().saturating_mul(42));
    let mut field_decls: Vec<String> = Vec::with_capacity(visible_fields.len());

    for (i, f) in visible_fields.iter().enumerate() {
        let is_complex = matches!(&f.ty, TypeRef::Named(n) if complex_enums.contains(n.as_str()));

        let visitor_trait_name =
            options_field_bridge_trait_name(typ.name.as_str(), f.name.as_str(), &f.ty, trait_bridges);
        let is_visitor_field = visitor_trait_name.is_some();

        // `#[serde(flatten)]` on a `serde_json::Value` field: emit
        let is_flattened_json = f.serde_flatten && matches!(&f.ty, TypeRef::Json);

        // Non-optional fields with #[serde(default)] must use boxed types in the record
        let has_serde_default = is_serde_default_marker(f.default.as_deref());

        let resolved_ty = resolve_field_type(&f.ty, visible_type_names);

        let f_optional_no_wrapper = f.optional && !matches!(resolved_ty, TypeRef::Optional(_));
        let ftype = if is_visitor_field {
            visitor_trait_name.expect("visitor field type is resolved")
        } else if is_flattened_json {
            "Map<String, Object>".to_string()
        } else if is_complex {
            "Object".to_string()
        } else if matches!(resolved_ty, TypeRef::Optional(_)) {
            java_boxed_type(&resolved_ty).to_string()
        } else if f_optional_no_wrapper {
            java_boxed_type(&resolved_ty).to_string()
        } else if has_serde_default || matches!(resolved_ty, TypeRef::Duration) {
            // Non-optional fields with #[serde(default)] or Duration use boxed types
            java_boxed_type(&resolved_ty).to_string()
        } else {
            java_type(&resolved_ty).to_string()
        };
        let jname = safe_java_field_name(&f.name);

        let needs_non_null = !f.optional && matches!(&resolved_ty, TypeRef::Vec(_)) && !typ.has_serde;

        let needs_bytes_int_serialize = matches!(&resolved_ty, TypeRef::Bytes);

        // 1. The field has an explicit `#[serde(rename = "...")]` attribute.
        let json_property_name = f.serde_rename.clone().unwrap_or_else(|| f.name.clone());
        let needs_builder = should_emit_builder(typ, builder_mode);
        let has_json_property =
            f.serde_rename.is_some() || jname != json_property_name || (needs_builder && !is_visitor_field);
        // Emit @Nullable for optional fields and for non-optional fields with #[serde(default)]
        let has_nullable = f.optional || has_serde_default || matches!(resolved_ty, TypeRef::Duration);

        let mut decl = String::new();

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

        if is_visitor_field {
            decl.push_str("@JsonIgnore ");
        }

        if needs_bytes_int_serialize {
            decl.push_str("@JsonSerialize(using = ByteArraySerializer.class) ");
        }

        let nullable_at_leading_pos = has_nullable && !ftype.contains('.');
        if nullable_at_leading_pos {
            decl.push_str("@Nullable ");
        }
        if needs_non_null {
            decl.push_str("@JsonInclude(JsonInclude.Include.NON_NULL) ");
        }
        if is_flattened_json {
            decl.push_str("@com.fasterxml.jackson.annotation.JsonAnyGetter ");
        } else if has_json_property && !is_visitor_field {
            decl.push_str("@JsonProperty(\"");
            decl.push_str(&json_property_name);
            decl.push_str("\") ");
        }
        if has_nullable && !nullable_at_leading_pos {
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

    let single_line_len = "public record ".len() + typ.name.len() + 1 + fields_joined.len() + ") { }".len();

    let mut record_block = String::new();
    let doc_to_emit = if typ.doc.is_empty() {
        format!("Auto-generated by alef from Rust type {}.", typ.name)
    } else {
        typ.doc.clone()
    };
    emit_javadoc(&mut record_block, &doc_to_emit, "");

    // Check if any fields are binding-excluded (marked with #[cfg_attr(alef, alef(skip))]).
    let has_binding_excluded_fields = typ.fields.iter().any(|f| f.binding_excluded);
    if has_binding_excluded_fields {
        record_block.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    }

    // letting Rust fall back to its `#[serde(default)]` value. This only affects
    // NOTE: The ObjectMapper also has Include.ALWAYS set for compatibility with both
    let will_emit_builder = should_emit_builder(typ, builder_mode);
    let builder_type = will_emit_builder.then_some(typ.name.as_str());
    if single_line_len > RECORD_LINE_WRAP_THRESHOLD && visible_fields.len() > 1 {
        let mut multiline_fields = String::new();
        for (i, decl) in field_decls.iter().enumerate() {
            let comma = if i < field_decls.len() - 1 { "," } else { "" };
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

    if will_emit_builder {
        record_block.push_str(&crate::backends::java::template_env::render(
            "record_builder_factory.jinja",
            minijinja::context! {},
        ));
    }

    let compact_ctor_lines: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.optional)
        .filter_map(|f| {
            let jname = safe_java_field_name(&f.name);
            let has_serde_default = is_serde_default_marker(f.default.as_deref());
            match &f.typed_default {
                Some(DefaultValue::IntLiteral(n)) if *n != 0 => {
                    let is_boxed = matches!(f.ty, TypeRef::Duration) || has_serde_default;
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

    if will_emit_builder {
        record_block.push('\n');
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

    // NOTE: FFM marshaling for DTO methods is not yet implemented. We skip all Self-returning
    let _non_excluded_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.binding_excluded && !m.sanitized)
        .collect();
    // Methods intentionally not emitted here — see NOTE above.

    record_block.push_str("}\n");

    let needs_json_property = fields_joined.contains("@JsonProperty(");
    let needs_json_include = fields_joined.contains("@JsonInclude(") || record_block.contains("@JsonInclude(");
    let needs_json_deserialize =
        record_block.contains("@JsonDeserialize(") || fields_joined.contains("@JsonDeserialize(");
    let needs_json_serialize = fields_joined.contains("@JsonSerialize(");
    let needs_json_ignore = fields_joined.contains("@JsonIgnore");
    let needs_json_ignore_properties = record_block.contains("@JsonIgnoreProperties(");
    let needs_nullable =
        fields_joined.contains("@Nullable") || (will_emit_builder && record_block.contains("@Nullable"));
    let _needs_transient = fields_joined.contains("@Transient");
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
    if fields_joined.contains("JsonNode") || record_block.contains("JsonNode") {
        imports.push("com.fasterxml.jackson.databind.JsonNode");
    }
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
    if should_emit_builder(typ, builder_mode) {
        imports.push("com.fasterxml.jackson.databind.annotation.JsonPOJOBuilder");
    }
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
