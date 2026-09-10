use crate::backends::java::type_map::{java_boxed_type, java_type};
use crate::codegen::java_literal::escape_java_string_literal;
use crate::codegen::naming::field_uses_duration_map_wire;
use crate::codegen::shared::binding_fields;
use crate::core::config::{JavaBuilderMode, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{DefaultValue, MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;

use super::builders::{gen_builder_nested_class, should_emit_builder};
use super::shared::{options_field_bridge_trait_name, resolve_field_type};
use crate::backends::java::gen_bindings::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, boxes_to_carry_literal_default, emit_javadoc, is_serde_default_marker,
    java_literal_default, safe_java_field_name, serde_default_collection_literal,
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

        // A `Vec`/`Map` field with `#[serde(default, skip_serializing_if = "...")]` gets a
        // non-null eager default in the builder (`serde_default_collection_literal`), so the
        // record component must not be `@Nullable` for it either — see the `has_nullable`
        // computation below, and the compact-constructor restore further down that is what makes
        // dropping `@Nullable` here correct rather than merely convenient. ~keep
        let serde_default_collection_default = serde_default_collection_literal(
            &f.ty,
            has_serde_default,
            f.serde_skip_serializing_if,
            f.typed_default.as_ref(),
        );

        // alef could not read the real value out of `impl Default`. `java_literal_default`
        // correctly refuses to restore a guessed literal for this in the compact constructor
        // below, but that guarantee only holds if the component is boxed (nullable) too —
        // otherwise the unboxed primitive's own implicit zero fabricates the exact value the
        // restore logic was written to avoid. ~keep
        // `TupleVariant`/`StructVariant` are resolved values (alef read them), not `Unresolved`,
        // but this backend has no per-argument Java expression for "construct enum variant X
        // with these field values" either — grouping them here keeps a resolved-but-unrenderable
        // default from falling through to the unboxed primitive's own implicit zero. ~keep
        let is_unresolved_default = matches!(
            &f.typed_default,
            Some(DefaultValue::Unresolved(_) | DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..))
        );

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
        } else if has_serde_default
            || matches!(resolved_ty, TypeRef::Duration)
            || is_unresolved_default
            || boxes_to_carry_literal_default(&f.ty, f.typed_default.as_ref())
        {
            // Non-optional fields with #[serde(default)], Duration, an unresolved default, or a
            // literal default use boxed types so the compact constructor can tell "unset" from
            // a real value
            java_boxed_type(&resolved_ty).to_string()
        } else {
            java_type(&resolved_ty).to_string()
        };
        let jname = safe_java_field_name(&f.name);

        let needs_non_null = !f.optional && matches!(&resolved_ty, TypeRef::Vec(_)) && !typ.has_serde;

        let needs_bytes_int_serialize = matches!(&resolved_ty, TypeRef::Bytes);

        // `std::time::Duration`'s serde derive produces `{"secs":<u64>,"nanos":<u32>}`, not the
        // bare millisecond integer this field's Java type (`Long`) would otherwise serialize to
        // — the FFI layer deserializes struct JSON straight into the real core type, so a plain
        // integer fails with `invalid type: integer ..., expected struct Duration`. A field
        // carrying `#[serde(with = "...")]` (the `duration_ms` convention) already writes that
        // bare integer, so it must not get these converters — see
        // `crate::codegen::naming::field_uses_duration_map_wire`. ~keep
        let needs_duration_serde = field_uses_duration_map_wire(f);

        // 1. The field has an explicit `#[serde(rename = "...")]` attribute.
        let json_property_name = f.serde_rename.clone().unwrap_or_else(|| f.name.clone());
        let needs_builder = should_emit_builder(typ, builder_mode);
        let has_json_property =
            f.serde_rename.is_some() || jname != json_property_name || (needs_builder && !is_visitor_field);
        // Emit @Nullable for optional fields and for non-optional fields with #[serde(default)] —
        // except a `Vec`/`Map` field that qualifies for `serde_default_collection_default`: the
        // compact constructor restores `null` to that same literal, so the component can never
        // actually observe `null` and `@Nullable` would be a lie.
        let has_nullable = f.optional
            || (has_serde_default && serde_default_collection_default.is_none())
            || matches!(resolved_ty, TypeRef::Duration)
            || is_unresolved_default
            || boxes_to_carry_literal_default(&f.ty, f.typed_default.as_ref());

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
        if let Some(type_name) = field_type_name
            && sealed_unions_with_unwrapped.contains(type_name)
        {
            decl.push_str("@JsonDeserialize(using = ");
            decl.push_str(type_name);
            decl.push_str("Deserializer.class) ");
        }

        if is_visitor_field {
            decl.push_str("@JsonIgnore ");
        }

        if needs_bytes_int_serialize {
            decl.push_str("@JsonSerialize(using = ByteArraySerializer.class) ");
        }

        if needs_duration_serde {
            decl.push_str("@JsonSerialize(using = DurationMillisSerializer.class) ");
            decl.push_str("@JsonDeserialize(using = DurationMillisDeserializer.class) ");
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
            decl.push_str(&escape_java_string_literal(&json_property_name));
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

    // Only binding-visible fields are record components, so only they can be assigned here — a
    // line naming a `#[alef(skip)]`ped field would not compile. ~keep
    let compact_ctor_lines: Vec<String> = binding_fields(&typ.fields)
        .filter(|f| !f.optional)
        .filter_map(|f| {
            // `java_literal_default` returning `Some` is also what boxed the component, so the
            // sentinel this tests against is guaranteed to be `null`. A field that instead
            // qualifies for `serde_default_collection_literal` is not boxed for that reason — a
            // `List`/`Map` component is already a reference type — but the `has_nullable`
            // computation above deliberately left it non-`@Nullable` on the strength of this exact
            // restore line existing, so it must be checked here too. ~keep
            let literal = java_literal_default(&f.ty, f.typed_default.as_ref()).or_else(|| {
                serde_default_collection_literal(
                    &f.ty,
                    is_serde_default_marker(f.default.as_deref()),
                    f.serde_skip_serializing_if,
                    f.typed_default.as_ref(),
                )
            })?;
            let jname = safe_java_field_name(&f.name);
            Some(format!("        if ({jname} == null) {{ {jname} = {literal}; }}"))
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
