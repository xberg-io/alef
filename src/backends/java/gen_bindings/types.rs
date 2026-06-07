use crate::backends::java::type_map::{java_boxed_type, java_return_type, java_type};
use crate::codegen::naming::to_class_name;
use crate::codegen::shared::binding_fields;
use crate::core::config::{AdapterConfig, AdapterPattern, BridgeBinding, JavaBuilderMode, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{DefaultValue, EnumDef, MethodDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToSnakeCase};

use super::helpers::{
    RECORD_LINE_WRAP_THRESHOLD, emit_javadoc, escape_javadoc_line, format_optional_value, is_tuple_field_name,
    java_apply_rename_all, safe_java_field_name, safe_java_method_name,
};
use super::marshal::{is_ffi_string_return, java_ffi_return_cast, java_ffi_return_expr};

/// Resolve a TypeRef to its Java type, replacing unknown/excluded Named types with JsonNode.
///
/// When a field references a type that was excluded from code generation (e.g. `#[alef(skip)]`),
/// we use `JsonNode` to preserve the object structure without requiring a Java type definition.
fn resolve_field_type(ty: &TypeRef, visible_types: &std::collections::HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if !visible_types.contains(name.as_str()) => {
            // Unknown type: replace with Json to produce JsonNode
            TypeRef::Json
        }
        TypeRef::Optional(inner) => {
            // Recursively resolve optional's inner type
            TypeRef::Optional(Box::new(resolve_field_type(inner, visible_types)))
        }
        TypeRef::Vec(inner) => {
            // Recursively resolve vec's inner type
            TypeRef::Vec(Box::new(resolve_field_type(inner, visible_types)))
        }
        TypeRef::Map(k, v) => {
            // Recursively resolve map's key and value types
            TypeRef::Map(
                Box::new(resolve_field_type(k, visible_types)),
                Box::new(resolve_field_type(v, visible_types)),
            )
        }
        _ => ty.clone(),
    }
}

fn is_options_field_bridge(
    type_name: &str,
    field_name: &str,
    field_ty: &TypeRef,
    trait_bridges: &[TraitBridgeConfig],
) -> bool {
    trait_bridges.iter().any(|bridge| {
        let alias_matches = bridge
            .type_alias
            .as_deref()
            .is_none_or(|alias| matches!(field_ty, TypeRef::Named(name) if name == alias));

        bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(type_name)
            && bridge.resolved_options_field() == Some(field_name)
            && alias_matches
    })
}

fn options_field_bridge_trait_name(
    type_name: &str,
    field_name: &str,
    field_ty: &TypeRef,
    trait_bridges: &[TraitBridgeConfig],
) -> Option<String> {
    trait_bridges.iter().find_map(|bridge| {
        let alias_matches = bridge
            .type_alias
            .as_deref()
            .is_none_or(|alias| matches!(field_ty, TypeRef::Named(name) if name == alias));

        if bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(type_name)
            && bridge.resolved_options_field() == Some(field_name)
            && alias_matches
        {
            Some(bridge.trait_name.clone())
        } else {
            None
        }
    })
}

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
        let has_serde_default = f.default == Some("/* serde(default) */".to_string());

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

        // Non-optional Bytes fields (byte[]) must be serialised as a JSON array of
        // integers, not as a base64 string. Jackson's default serialiser for byte[]
        // produces base64, but Rust's serde for Vec<u8> expects [n, n, …].
        // @JsonSerialize(using = ByteArrayToIntArraySerializer.class) overrides the
        // default Jackson behaviour for this field only.
        let needs_bytes_int_serialize = !f.optional && matches!(&resolved_ty, TypeRef::Bytes);

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
            let has_serde_default = f.default == Some("/* serde(default) */".to_string());
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
                    Some(format!("        if ({cond}) {jname} = {n}{suffix};"))
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
    let non_excluded_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.binding_excluded && !m.sanitized)
        .collect();
    for method in &non_excluded_methods {
        // Only emit Self-returning methods here; other signatures need more complex bridging.
        let returns_self = matches!(&method.return_type, TypeRef::Named(n) if n == &typ.name);
        if !returns_self {
            continue;
        }
        let is_static_method = method.receiver.is_none();
        let java_method_name = safe_java_method_name(&method.name);
        // Build parameter list.
        let params_str = method
            .params
            .iter()
            .map(|p| {
                let ptype = if p.optional {
                    java_boxed_type(&p.ty).to_string()
                } else {
                    java_type(&p.ty).to_string()
                };
                let pname = safe_java_field_name(&p.name);
                format!("{ptype} {pname}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Javadoc from the IR method doc.
        emit_javadoc(&mut record_block, &method.doc, "    ");
        if is_static_method {
            // Body: delegate via the Jackson ObjectMapper round-trip so the native static
            // method on the core type is invoked.  Use fromJson(toJson(core_result)) to stay
            // consistent with the rest of the Java binding's serde-based FFI pattern.
            // We cannot call the Rust static method directly from Java (no JNI symbol for
            // DTO methods); instead we rely on the fact that all preset factory methods are
            // expressible as combinations of the Builder and known field values.
            // Emit `throw new UnsupportedOperationException` as a honest placeholder rather
            // than silently omitting the method.
            record_block.push_str(&crate::backends::java::template_env::render(
                "record_unsupported_method.jinja",
                minijinja::context! {
                    is_static => true,
                    type_name => &typ.name,
                    method_name => java_method_name,
                    params_str => params_str,
                    guidance => "use the Builder instead",
                },
            ));
        } else {
            // Instance wither: reconstruct via builder with updated field.
            record_block.push_str(&crate::backends::java::template_env::render(
                "record_unsupported_method.jinja",
                minijinja::context! {
                    is_static => false,
                    type_name => &typ.name,
                    method_name => java_method_name,
                    params_str => params_str,
                    guidance => "reconstruct via Builder",
                },
            ));
        }
    }

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
        if variant.fields.is_empty() {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_opaque_handle_class(
    package: &str,
    typ: &TypeDef,
    prefix: &str,
    adapters: &[AdapterConfig],
    main_class: &str,
    enum_names: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
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
                && !a.skip_languages.iter().any(|l| l == "java")
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
    // Static factory methods: receiver is None (no &self). These are constructors /
    // preset factories (e.g. `Parser::default()`, `LanguageRegistry::default()`,
    // `DownloadManager::new(version)`) that return a new instance of the type.
    // The FFI backend never exports `_default` / `_to_json` / `_from_json` for opaque
    // types — those C functions only exist for non-opaque, serde-derivable, non-Update
    // value types. `gen_native_lib` already skips emitting the matching MethodHandle
    // constants; skip the static-factory wrappers here too so we don't reference
    // missing `NativeLib.<PREFIX>_<TYPE>_DEFAULT` constants from `defaultInstance()`.
    let static_factory_methods: Vec<&MethodDef> = typ
        .methods
        .iter()
        .filter(|m| m.receiver.is_none())
        .filter(|m| !matches!(m.name.as_str(), "default" | "to_json" | "from_json"))
        .collect();
    let has_instance_methods = !instance_methods.is_empty();
    let has_static_factories = !static_factory_methods.is_empty();
    let needs_helpers = has_streaming || has_instance_methods;

    // Build the class body first so we can compute imports from actual usage —
    // Checkstyle's UnusedImports rule fails if we declare an import that
    // never appears in the file body (e.g. when every instance method body
    // is a `Unsupported return shape` stub).
    let mut body = String::new();

    emit_javadoc(&mut body, &typ.doc, "");
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_header.jinja",
        minijinja::context! { class_name => class_name },
    ));

    // Emit streaming iterator methods (e.g. chatStream(req) -> Iterator<ChatCompletionChunk>).
    for adapter in &streaming_adapters {
        gen_streaming_method(&mut body, adapter, prefix, &type_snake, main_class, to_json_type_names);
    }

    // Emit non-streaming instance methods (chat, embed, moderate, …).
    for method in &instance_methods {
        gen_instance_method(
            &mut body,
            method,
            prefix,
            &type_snake,
            main_class,
            opaque_type_names,
            to_json_type_names,
        );
    }

    // Emit static factory methods (constructors / preset factories with no receiver).
    // These give callers a clean `Parser.ofDefault()`, `LanguageRegistry.ofDefault()`,
    // `DownloadManager.create(version)` API without exposing the raw FFI handle.
    if has_static_factories {
        for method in &static_factory_methods {
            gen_static_factory_method(
                &mut body,
                method,
                class_name,
                prefix,
                &type_snake,
                main_class,
                enum_names,
            );
        }
    }

    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
    body.push_str(&crate::backends::java::template_env::render(
        "opaque_handle_close.jinja",
        minijinja::context! {
            free_handle => free_handle,
            class_name => class_name,
        },
    ));

    if needs_helpers {
        gen_streaming_helpers(&mut body, prefix, main_class);
    }

    body.push_str("}\n");

    let mut imports: Vec<&str> = vec!["java.lang.foreign.MemorySegment"];
    if needs_helpers || has_static_factories {
        // `Arena.ofShared()` is only referenced when method bodies actually use it
        // (e.g., string parameters that allocate via Arena).
        if body.contains("Arena") {
            imports.push("java.lang.foreign.Arena");
        }
        // `ValueLayout` only appears when an instance method, streaming helper, or static
        // factory actually marshals memory; stub methods never reference it.
        if body.contains("ValueLayout") {
            imports.push("java.lang.foreign.ValueLayout");
        }
        // Same reasoning for ObjectMapper — STREAM_MAPPER references it, but
        // not all paths reach STREAM_MAPPER.
        if body.contains("ObjectMapper") {
            imports.push("com.fasterxml.jackson.databind.ObjectMapper");
        }
        // JsonNode is needed when method parameters or returns use it (e.g., requestSchemaJson(JsonNode)).
        if body.contains("JsonNode") {
            imports.push("com.fasterxml.jackson.databind.JsonNode");
        }
    }
    // Streaming method bodies reference java.util.stream.Stream<T> and
    // java.util.stream.StreamSupport via fully-qualified names in the template,
    // so no short-form import is needed. Adding one would trigger Checkstyle's
    // UnusedImports rule (confirmed in sample_crate DefaultClient.java:12).
    let _ = has_streaming;
    // Import collection types from actual body usage (params AND returns), not just return types —
    // e.g. a builder method taking `List<String>` needs the import even with no List return.
    if body.contains("List<") {
        imports.push("java.util.List");
    }
    if body.contains("Optional<") {
        imports.push("java.util.Optional");
    }
    if body.contains("Map<") {
        imports.push("java.util.Map");
    }

    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&body);
    out
}

/// Emit a non-streaming instance method on an opaque-handle owner.
fn gen_instance_method(
    out: &mut String,
    method: &MethodDef,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    opaque_type_names: &AHashSet<String>,
    to_json_type_names: &AHashSet<String>,
) {
    let method_name = safe_java_method_name(&method.name);
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
        java_return_type(&method.return_type).to_string()
    };

    // Emit Javadoc derived from the IR method.doc above the method declaration
    // so opaque-handle instance methods carry their source-level rustdoc into
    // the generated Java surface.
    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public ");
    out.push_str(&return_type_java);
    out.push(' ');
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push(')');

    // Methods named "clone" cannot declare throws because they override Object.clone()
    // which only throws CloneNotSupportedException. All other methods on opaque types
    // may call FFI functions that fail, so they declare throws.
    if method.name != "clone" {
        out.push_str(" throws ");
        out.push_str(&exception_class);
    }

    out.push_str(" {\n");

    for p in &method.params {
        if !p.optional && param_needs_null_check(&p.ty) {
            let pname = p.name.to_lower_camel_case();
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => pname },
            ));
        }
    }

    // Check if any parameters require Arena allocation (String, Path, Named types, etc.)
    let needs_arena = method.params.iter().any(|p| match &p.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner)
            if matches!(
                inner.as_ref(),
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_)
            ) =>
        {
            true
        }
        _ => false,
    });

    out.push_str("        try {\n");
    if needs_arena {
        out.push_str("            Arena arena = Arena.ofShared();\n");
    }

    let mut named_ptr_frees: Vec<(String, String)> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();

    for p in &method.params {
        let pname = p.name.to_lower_camel_case();
        let cname = format!("c{}", to_class_name(&p.name));
        match &p.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Json => {
                // Object (polymorphic JSON) passed directly without marshalling.
                call_args.push(pname);
            }
            TypeRef::Path => {
                // Path → C string requires `.toString()` because Java's SegmentAllocator.allocateFrom
                // accepts String, not java.nio.file.Path. Reuse marshal_path.jinja which already
                // emits the conversion.
                out.push_str(&crate::backends::java::template_env::render(
                    "marshal_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_optional_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Json) => {
                // Optional<Object> (polymorphic JSON) passed directly without marshalling.
                call_args.push(pname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
                // Optional Path also needs `.toString()` because SegmentAllocator.allocateFrom
                // accepts String, not java.nio.file.Path.
                out.push_str(&crate::backends::java::template_env::render(
                    "marshal_optional_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
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
                    out.push_str(&crate::backends::java::template_env::render(
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
                    out.push_str(&crate::backends::java::template_env::render(
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
                out.push_str(&crate::backends::java::template_env::render(
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
                out.push_str(&crate::backends::java::template_env::render(
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
            frees.push_str(&crate::backends::java::template_env::render(
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
        out.push_str(&crate::backends::java::template_env::render(
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

        // Check if the return type is opaque or lacks _to_json in the FFI
        if opaque_type_names.contains(&return_type_name) || !to_json_type_names.contains(&return_type_name) {
            // For opaque types, wrap the pointer in a new instance of the return type.
            // For value types without _to_json (shouldn't happen but be defensive), stub the method.
            if opaque_type_names.contains(&return_type_name) {
                // Wrap pointer in new instance: `return new TypeName(resultPtr);`
                let ret_type_snake = return_type_name.to_snake_case();
                let ret_type_upper = ret_type_snake.to_uppercase();
                let empty_return = if is_optional_return {
                    "java.util.Optional.empty()".to_string()
                } else {
                    "null".to_string()
                };
                let success_return = if is_optional_return {
                    format!("java.util.Optional.of(new {return_type_name}(resultPtr))")
                } else {
                    format!("new {return_type_name}(resultPtr)")
                };
                let ret_free = format!("NativeLib.{prefix_upper}_{ret_type_upper}_FREE");
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_opaque_handle_result.jinja",
                    minijinja::context! {
                        ffi_handle => ffi_handle,
                        args_joined => args_joined,
                        named_frees => render_named_frees("            "),
                        empty_return => empty_return,
                        success_return => success_return,
                        ret_free => ret_free,
                    },
                ));
            } else {
                // Value type without _to_json (defensive stub)
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_unsupported_return.jinja",
                    minijinja::context! {
                        named_frees => render_named_frees("            "),
                        method_name => method_name,
                        exception_class => exception_class,
                    },
                ));
            }
        } else {
            // Normal value type with _to_json — deserialize from JSON
            let ret_snake = return_type_name.to_snake_case();
            let ret_upper = ret_snake.to_uppercase();
            let ret_free = format!("NativeLib.{prefix_upper}_{ret_upper}_FREE");
            let ret_to_json = format!("NativeLib.{prefix_upper}_{ret_upper}_TO_JSON");
            // When the declared return is `Optional<NamedDto>`, the method signature
            // is `Optional<NamedDto>` (from `java_return_type`) but the body builds
            // a bare `NamedDto`; wrap each return site through `Optional.of` /
            // `Optional.empty` so the body matches the signature.  Non-optional
            // named returns keep the historical bare-return shape.
            let (empty_return, success_return) = if is_optional_return {
                (
                    "java.util.Optional.empty()".to_string(),
                    format!("return java.util.Optional.of(STREAM_MAPPER.readValue(json, {return_type_name}.class));"),
                )
            } else {
                (
                    "null".to_string(),
                    format!("return STREAM_MAPPER.readValue(json, {return_type_name}.class);"),
                )
            };

            out.push_str(&crate::backends::java::template_env::render(
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
                    empty_return => empty_return,
                    success_return => success_return,
                },
            ));
        }
    } else if is_ffi_string_return(&dispatch_return) {
        let template = if is_optional_return {
            "stream_method_optional_string_result.jinja"
        } else {
            "stream_method_string_result.jinja"
        };
        out.push_str(&crate::backends::java::template_env::render(
            template,
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                prefix_upper => prefix_upper,
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Primitive(_) | TypeRef::Duration) {
        let template = if is_optional_return {
            "stream_method_optional_primitive_result.jinja"
        } else {
            "stream_method_primitive_result.jinja"
        };
        out.push_str(&crate::backends::java::template_env::render(
            template,
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
                java_primitive_type => java_ffi_return_cast(&dispatch_return),
                java_primitive_expr => java_ffi_return_expr(&dispatch_return, "result"),
                is_optional_long => matches!(dispatch_return, TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Isize | PrimitiveType::Usize) | TypeRef::Duration),
            },
        ));
    } else if matches!(dispatch_return, TypeRef::Unit) {
        out.push_str(&crate::backends::java::template_env::render(
            "stream_method_unit_result.jinja",
            minijinja::context! {
                ffi_handle => ffi_handle,
                args_joined => args_joined,
                named_frees => render_named_frees("            "),
            },
        ));
    } else {
        out.push_str(&crate::backends::java::template_env::render(
            "stream_method_unsupported_return.jinja",
            minijinja::context! {
                named_frees => render_named_frees("            "),
                method_name => method_name,
                exception_class => exception_class,
            },
        ));
    }

    // For clone() methods, wrap exceptions in RuntimeException since the method
    // cannot declare throws (it overrides Object.clone() which only throws
    // CloneNotSupportedException). All other methods can declare throws.
    let catch_template = if method.name == "clone" {
        "stream_method_catch_unchecked.jinja"
    } else {
        "stream_method_catch.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        catch_template,
        minijinja::context! {
            exception_class => exception_class,
            method_name => method_name,
        },
    ));
}

/// Emit a static factory method on an opaque-handle class.
///
/// Static factories have no `self` receiver — they allocate a new native object and
/// return it wrapped in the Java class.  Examples: `Parser::default()`,
/// `LanguageRegistry::default()`, `DownloadManager::new(version)`.
///
/// The pattern mirrors `gen_instance_method` but:
///  - the NIF call does NOT prepend `this.handle` to `call_args`
///  - the result is wrapped in `new ClassName(handle)` rather than returned raw
fn gen_static_factory_method(
    out: &mut String,
    method: &MethodDef,
    class_name: &str,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    enum_names: &AHashSet<String>,
) {
    let method_name = safe_java_method_name(&method.name);
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

    emit_javadoc(out, &method.doc, "    ");
    out.push_str("    public static ");
    out.push_str(class_name);
    out.push(' ');
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push_str(") throws ");
    out.push_str(&exception_class);
    out.push_str(" {\n");

    // Null checks for non-optional reference params.
    for p in &method.params {
        if !p.optional && param_needs_null_check(&p.ty) {
            let pname = p.name.to_lower_camel_case();
            out.push_str(&crate::backends::java::template_env::render(
                "stream_method_null_check.jinja",
                minijinja::context! { param_name => pname },
            ));
        }
    }

    out.push_str("        try {\n");

    // Check if any parameters require Arena allocation (String, Path, Named types, etc.)
    let needs_arena = method.params.iter().any(|p| match &p.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner)
            if matches!(
                inner.as_ref(),
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_)
            ) =>
        {
            true
        }
        _ => false,
    });

    if needs_arena {
        out.push_str("            Arena arena = Arena.ofShared();\n");
    }

    let mut named_ptr_frees: Vec<(String, String)> = Vec::new();
    let mut call_args: Vec<String> = Vec::new();

    // Marshal parameters (same logic as gen_instance_method but no receiver in call_args).
    for p in &method.params {
        let pname = p.name.to_lower_camel_case();
        let cname = format!("c{}", to_class_name(&p.name));
        match &p.ty {
            TypeRef::String | TypeRef::Char => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Json => {
                // Object (polymorphic JSON) passed directly without marshalling.
                call_args.push(pname);
            }
            TypeRef::Path => {
                out.push_str(&crate::backends::java::template_env::render(
                    "marshal_path.jinja",
                    minijinja::context! { cname => &cname, name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char | TypeRef::Json) => {
                out.push_str(&crate::backends::java::template_env::render(
                    "stream_method_optional_string_param.jinja",
                    minijinja::context! { c_name => cname, param_name => pname },
                ));
                call_args.push(cname);
            }
            TypeRef::Named(type_name) => {
                // Check if this is an enum type (Wave 2 FFI backend emits enums as i32 discriminants)
                if enum_names.contains(type_name.as_str()) {
                    // Enum parameter: convert to ordinal/discriminant value
                    // For Method enum: method.ordinal() gives the i32 discriminant
                    let enum_expr = if p.optional {
                        format!("{pname} != null ? {pname}.ordinal() : -1")
                    } else {
                        format!("{pname}.ordinal()")
                    };
                    out.push_str(&crate::backends::java::template_env::render(
                        "stream_method_enum_param.jinja",
                        minijinja::context! {
                            c_name => cname,
                            enum_expr => enum_expr,
                        },
                    ));
                    call_args.push(cname);
                } else {
                    // Struct/record parameter: JSON-serialize via _from_json
                    let req_snake = type_name.to_snake_case();
                    let req_upper = req_snake.to_uppercase();
                    let from_json = format!("NativeLib.{prefix_upper}_{req_upper}_FROM_JSON");
                    let req_free = format!("NativeLib.{prefix_upper}_{req_upper}_FREE");
                    if p.optional {
                        out.push_str(&crate::backends::java::template_env::render(
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
                        out.push_str(&crate::backends::java::template_env::render(
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
            }
            TypeRef::Primitive(_) | TypeRef::Duration => {
                call_args.push(pname);
            }
            _ => {
                // Unsupported param type for static factory — emit stub that throws.
                out.push_str(&crate::backends::java::template_env::render(
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
            frees.push_str(&crate::backends::java::template_env::render(
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

    let args_joined = call_args.join(", ");

    // The return type for a static factory is always Self (the class being constructed).
    // Emit: call FFI → check non-null → wrap in new ClassName(handle).
    let named_frees_str = render_named_frees("            ");
    out.push_str(&crate::backends::java::template_env::render(
        "static_factory_return_handle.jinja",
        minijinja::context! {
            ffi_handle => ffi_handle,
            args_joined => args_joined,
            named_frees => named_frees_str,
            exception_class => exception_class,
            method_name => method_name,
            class_name => class_name,
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
///
/// NOTE: Streaming item types must have serde derives in the Rust source.
/// This codegen always emits the `{PREFIX}_{ITEM}_TO_JSON` symbol name, which must
/// exist in the C FFI layer. If a cfg-gated type (e.g. `#[cfg(not(wasm32))]`)
/// lacks the symbol, that indicates a C FFI generation failure, not a Java codegen issue.
fn gen_streaming_method(
    out: &mut String,
    adapter: &AdapterConfig,
    prefix: &str,
    owner_snake: &str,
    main_class: &str,
    _to_json_type_names: &AHashSet<String>,
) {
    let method_name = adapter.name.to_lower_camel_case();
    let item_type = adapter.item_type.as_deref().unwrap_or("Object");
    let request_type_full = adapter.params[0].ty.as_str();
    // Strip any leading module path (e.g. `sample_llm::ChatCompletionRequest` → `ChatCompletionRequest`).
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
    // For streaming item types, always derive the to_json symbol from the item type name.
    // Streaming items must have serde derives (checked at adapter validation time);
    // if the FFI symbol is missing, that's a C FFI generation issue, not Java codegen.
    let item_to_json = format!("{prefix_upper}_{item_upper}_TO_JSON");
    let item_free = format!("{prefix_upper}_{item_upper}_FREE");

    out.push_str(&crate::backends::java::template_env::render(
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
    let needs_stream_mapper = out.contains("STREAM_MAPPER");

    out.push_str(&crate::backends::java::template_env::render(
        "streaming_helpers.jinja",
        minijinja::context! {
            exception_class => exception_class,
            prefix_upper => prefix_upper,
            needs_read_bytes_result => needs_read_bytes_result,
            free_bytes => free_bytes,
            needs_stream_mapper => needs_stream_mapper,
        },
    ));
}

// ---------------------------------------------------------------------------
// Record types (Java records)
// ---------------------------------------------------------------------------

/// Threshold for auto-emit builder: emit when field count >= this value.
const BUILDER_AUTO_THRESHOLD: usize = 8;

/// Check if a field type is complex (nested object, collection of complex types, etc.).
fn is_complex_field_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Json
    )
}

/// Decide whether to emit a builder for this type based on its field count and configuration.
fn should_emit_builder(typ: &TypeDef, builder_mode: JavaBuilderMode) -> bool {
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
fn gen_builder_nested_class(
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
        let has_serde_default = field.default == Some("/* serde(default) */".to_string());

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
            // The "/* serde(default) */" placeholder is a signal value set by the
            // extractor when a field carries #[serde(default)] but no other explicit
            // default — it must NOT be emitted as a Java expression. Treat it as
            // "no real default, use Optional.empty()".
            if let Some(default) = &field.default
                && default != "/* serde(default) */"
            {
                // If there's an explicit default, wrap it in Optional.of()
                format_optional_value(&field.ty, default)
            } else {
                // If no default, use Optional.empty()
                "Optional.empty()".to_string()
            }
        } else {
            // For non-Optional fields, use regular defaults.
            // Same placeholder filter as above — fall through to the type-driven
            // default match arm so Vec emits `List.of()`, Map emits `Map.of()`, etc.
            if let Some(default) = &field.default
                && default != "/* serde(default) */"
            {
                default.clone()
            } else if field.default == Some("/* serde(default) */".to_string()) {
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

        // Add @Nullable for fields that are boxed for serde(default) or Duration
        // When a non-optional field uses a boxed type to represent "not set" via null,
        // it needs the @Nullable annotation for proper static analysis.
        let needs_nullable_annotation =
            has_serde_default && matches!(&field.ty, TypeRef::Named(_)) || matches!(field.ty, TypeRef::Duration);

        if needs_nullable_annotation {
            body.push_str("        @Nullable ");
        }

        body.push_str("private ");
        body.push_str(&field_type);
        body.push(' ');
        body.push_str(&field_name);
        body.push_str(" = ");
        body.push_str(&default_value);
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
        let has_serde_default = field.default == Some("/* serde(default) */".to_string());

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
    body.push_str("        /** Builds the ");
    body.push_str(&typ.name);
    body.push_str(" instance. */\n");
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
    let mut out = crate::backends::java::template_env::render(
        "java_file_header.jinja",
        minijinja::context! { header => header, package => package, imports => &imports },
    );
    out.push('\n');
    out.push_str(&crate::backends::java::template_env::render(
        "byte_array_serializer.jinja",
        minijinja::context! {},
    ));
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
            out.push('(');
            // For String inner types, convert the entire node to JSON string
            if inner_type.as_ref() == "String" {
                out.push_str("node.toString()");
            } else {
                out.push_str("ctx.readTreeAsValue(node, ");
                out.push_str(inner_type.as_ref());
                out.push_str(".class)");
            }
            out.push_str(");\n");
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
    out.push_str(&crate::backends::java::template_env::render(
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
    use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, PrimitiveType, TypeRef};
    use ahash::AHashSet;
    use std::collections::HashSet;

    fn make_config_type_with_duration_default() -> TypeDef {
        TypeDef {
            name: "CrawlConfig".to_string(),
            rust_path: "sample_crate::CrawlConfig".to_string(),
            original_rust_path: "sample_crate::CrawlConfig".to_string(),
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    fn make_request_type_with_multiword_fields() -> TypeDef {
        TypeDef {
            name: "ChatCompletionRequest".to_string(),
            rust_path: "sample_llm::ChatCompletionRequest".to_string(),
            original_rust_path: "sample_llm::ChatCompletionRequest".to_string(),
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
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        }
    }

    /// Single-word builder fields like `model` MUST get `@JsonProperty`
    /// Jackson's BuilderBasedDeserializer requires @JsonProperty on every setter
    /// to correctly map JSON properties to setters.
    #[test]
    fn single_word_builder_field_gets_json_property() {
        let typ = make_request_type_with_multiword_fields();
        let out = gen_record_type(
            "dev.sample_crate",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            &[],
            "SampleLlmRs",
            JavaBuilderMode::Auto,
            &ahash::AHashMap::default(),
            &AHashSet::default(),
            &HashSet::default(),
        );
        // `model` is single-word: Jackson still requires @JsonProperty on the builder setter
        // to map JSON fields to setters correctly.
        assert!(
            out.contains("@JsonProperty(\"model\")"),
            "single-word builder field must get @JsonProperty; got:\n{out}"
        );
    }

    /// Multi-word snake_case fields like `max_tokens` → `maxTokens` MUST get
    /// `@JsonProperty("max_tokens")` so Jackson sends the snake_case wire name
    /// that Rust's serde expects.
    #[test]
    fn multiword_snake_case_field_gets_json_property_annotation() {
        let typ = make_request_type_with_multiword_fields();
        let out = gen_record_type(
            "dev.sample_crate",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            &[],
            "SampleLlmRs",
            JavaBuilderMode::Auto,
            &ahash::AHashMap::default(),
            &AHashSet::default(),
            &HashSet::default(),
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
            "dev.sample_crate",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            &[],
            "SampleCrawler",
            JavaBuilderMode::Auto,
            &ahash::AHashMap::default(),
            &AHashSet::default(),
            &HashSet::default(),
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

    /// A type with only 2 visible fields but one carrying `#[serde(flatten)]` on a
    /// `serde_json::Value` field must still emit a Builder (with `@JsonAnySetter`)
    /// regardless of the Auto field-count threshold.  Without the Builder, Jackson
    /// cannot absorb unknown sibling keys and throws
    /// `Unrecognized field "..." not marked as ignorable`.
    #[test]
    fn flatten_json_field_forces_builder_emission_below_auto_threshold() {
        use crate::core::ir::CoreWrapper;
        let typ = TypeDef {
            name: "ResponseTool".to_string(),
            rust_path: "sample_llm::ResponseTool".to_string(),
            original_rust_path: "sample_llm::ResponseTool".to_string(),
            fields: vec![
                FieldDef {
                    name: "tool_type".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: Some("\"\"".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: CoreWrapper::None,
                    vec_inner_core_wrapper: CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: Some("type".to_string()),
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "config".to_string(),
                    ty: TypeRef::Json,
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
                    serde_flatten: true,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };
        let out = gen_record_type(
            "dev.sample_crate.samplellm",
            &typ,
            &AHashSet::default(),
            &AHashSet::default(),
            "SNAKE_CASE",
            &[],
            "SampleLlmRs",
            JavaBuilderMode::Auto,
            &ahash::AHashMap::default(),
            &AHashSet::default(),
            &HashSet::default(),
        );
        // Builder must be emitted so @JsonAnySetter can absorb unknown sibling fields.
        assert!(
            out.contains("@JsonDeserialize(builder = ResponseTool.Builder.class)"),
            "flatten+Json type must emit Builder even with < 5 fields"
        );
        assert!(
            out.contains("@com.fasterxml.jackson.annotation.JsonAnySetter"),
            "Builder must have @JsonAnySetter to absorb unknown sibling fields"
        );
        // The record field itself should still use @JsonAnyGetter for serialization.
        assert!(
            out.contains("@com.fasterxml.jackson.annotation.JsonAnyGetter"),
            "record field must still carry @JsonAnyGetter for serialization"
        );
    }
}
