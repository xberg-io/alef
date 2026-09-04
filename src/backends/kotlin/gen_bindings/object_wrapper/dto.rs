use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use std::collections::BTreeSet;

use super::types::{escape_kotlin_string, fits_single_line, kotlin_field_default, kotlin_type_with_string_imports};
use crate::backends::kotlin::gen_bindings::helpers::emit_cleaned_kdoc;
use crate::backends::kotlin::gen_bindings::shared::{ValueMethodBridge, kotlin_field_name, to_lower_camel};
use crate::core::jni::{bridge_method_name, bridgeable_value_methods, is_functional_ref_mut_value_method};

/// File-private Jackson mapper used to marshal `this` and the parameter object
/// across the JNI boundary for value-type instance methods.
const VALUE_METHOD_MAPPER: &str = "VALUE_METHOD_MAPPER";

/// Whether Jackson's default bean-property name derivation would silently disagree with
/// `PropertyNamingStrategies.SNAKE_CASE`'s intended translation of `camel_name`.
///
/// Kotlin data-class properties without an explicit `@JsonProperty` are named by Jackson
/// from the auto-generated JavaBean getter (`getFoo()` for property `foo`). Deriving the
/// property name back from that getter goes through `java.beans.Introspector.decapitalize`,
/// which refuses to lowercase the leading character when the name's first two characters
/// are *both* upper-case (its acronym heuristic, e.g. `URL` staying `URL` rather than
/// becoming `uRL`). A property whose camelCase form is a single lower-case letter followed
/// immediately by an upper-case letter (e.g. `kClusters`) capitalizes to a getter suffix
/// (`KClusters`) that itself starts with two upper-case letters, so `decapitalize` leaves it
/// as `KClusters` instead of `kClusters`. `SnakeCaseStrategy.translate` then never inserts an
/// underscore between those two positions (the first can't take a leading `_`, and the second
/// is skipped because the first already flagged "just translated"), producing `kclusters`
/// instead of `k_clusters` on the wire.
///
/// This is a name-shape predicate derived from that documented Introspector/Jackson
/// behaviour, not a lookup against any particular property or project name: it fires for
/// every `[a-z][A-Z]...` camelCase name, matching whichever properties happen to have that
/// shape in a given crate. ~keep
fn kotlin_property_name_defeats_bean_introspection(camel_name: &str) -> bool {
    let mut chars = camel_name.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) => first.is_ascii_lowercase() && second.is_ascii_uppercase(),
        _ => false,
    }
}

pub(crate) fn emit_type_with_imports(
    ty: &TypeDef,
    out: &mut String,
    imports: &mut BTreeSet<String>,
    enum_defaults: &std::collections::HashMap<String, String>,
    sealed_class_names: &std::collections::HashSet<String>,
    default_constructible_types: &std::collections::HashSet<String>,
    value_method_bridge: Option<ValueMethodBridge<'_>>,
) {
    emit_cleaned_kdoc(out, &ty.doc, "");
    if ty.fields.is_empty() {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "empty_class.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
        return;
    }

    // Enumerate before filtering so `original_idx` stays stable for field naming, then drop
    // `binding_excluded` fields entirely — matching every other backend. Keeping them (as the
    // legacy nullable `= null` branch did) leaks force-controlled / internal knobs into the public
    // DTO; a `[crates.exclude].fields` entry must remove the field, not just null its type.
    let visible_fields: Vec<(usize, &crate::core::ir::FieldDef)> = ty
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.binding_excluded)
        .collect();

    let field_sealed_annotations: Vec<Option<String>> = visible_fields
        .iter()
        .map(|(_, f)| sealed_class_field_annotation(&f.ty, sealed_class_names))
        .collect();

    let has_field_docs = visible_fields.iter().any(|(_, f)| !f.doc.is_empty());
    // Detect `#[serde(flatten)]` fields. In Rust these collect all unknown
    let has_flatten_field = visible_fields.iter().any(|(_, f)| f.serde_flatten);

    let mut field_strings: Vec<String> = Vec::with_capacity(visible_fields.len());
    let mut field_names: Vec<String> = Vec::with_capacity(visible_fields.len());
    for (original_idx, field) in visible_fields.iter() {
        let ty_str = kotlin_type_with_string_imports(&field.ty, field.optional, imports);
        let name = kotlin_field_name(&field.name, *original_idx);
        field_names.push(name.clone());
        // collections (`#[serde(skip_serializing_if = "...")]`) or skip a
        // field entirely under a feature gate (`#[serde(skip)]`). Without a
        let (effective_ty_str, default_suffix) = if field.serde_flatten {
            let nullable_ty = if ty_str.ends_with('?') {
                ty_str.clone()
            } else {
                format!("{ty_str}?")
            };
            (nullable_ty, " = null".to_string())
        } else {
            let default_suffix = kotlin_field_default(
                &field.ty,
                field.optional,
                field.typed_default.as_ref(),
                enum_defaults,
                default_constructible_types,
            );
            if default_suffix.contains(".milliseconds") {
                imports.insert("import kotlin.time.Duration.Companion.milliseconds".to_string());
            }
            (ty_str, default_suffix)
        };
        field_strings.push(format!("val {name}: {effective_ty_str}{default_suffix}"));
    }

    let has_field_annotations = visible_fields.iter().any(|(_, f)| f.serde_rename.is_some())
        || field_sealed_annotations.iter().any(Option::is_some)
        || field_names
            .iter()
            .any(|name| kotlin_property_name_defeats_bean_introspection(name));

    // Instance methods are only emitted when a JNI bridge is available to back them.
    // Without one they are dropped rather than stubbed out, so no method that compiles
    // can fail at runtime.
    let bridged_methods: Vec<&MethodDef> = value_method_bridge
        .map(|bridge| bridgeable_value_methods(ty, bridge.serde_type_names))
        .unwrap_or_default();
    let has_instance_methods = !bridged_methods.is_empty();

    let prefix = format!("data class {}", ty.name);
    let use_single_line = !has_field_docs
        && !has_field_annotations
        && !has_flatten_field
        && !has_instance_methods
        && fits_single_line("", &prefix, &field_strings, "");

    if has_flatten_field {
        out.push_str("@com.fasterxml.jackson.annotation.JsonIgnoreProperties(ignoreUnknown = true)\n");
    }

    if use_single_line {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "data_class_inline.jinja",
            minijinja::context! {
                prefix => prefix,
                fields => field_strings.join(", "),
            },
        ));
    } else {
        out.push_str(&crate::backends::kotlin::template_env::render(
            "data_class_header_only.jinja",
            minijinja::context! {
                prefix => prefix,
            },
        ));
        for (idx, (((_, field), field_str), name)) in visible_fields
            .iter()
            .zip(field_strings.iter())
            .zip(field_names.iter())
            .enumerate()
        {
            emit_cleaned_kdoc(out, &field.doc, "    ");
            // Emit @JsonProperty when the Rust field carries #[serde(rename = "...")], or when
            // the plain wire name (`field.name`) would come back mangled through Jackson's
            // default bean-property derivation (see `kotlin_property_name_defeats_bean_
            // introspection`) — in that case the wire name is `field.name` itself, since there
            // is no serde rename overriding it. ~keep
            let wire_name = field
                .serde_rename
                .clone()
                .or_else(|| kotlin_property_name_defeats_bean_introspection(name).then(|| field.name.clone()));
            if let Some(wire_name) = &wire_name {
                out.push_str(&crate::backends::kotlin::template_env::render(
                    "json_property_annotation.jinja",
                    minijinja::context! {
                        indent => "    ",
                        value => escape_kotlin_string(wire_name),
                    },
                ));
            }
            if let Some(annotation) = &field_sealed_annotations[idx] {
                out.push_str("    ");
                out.push_str(annotation);
                out.push('\n');
            }
            out.push_str(&crate::backends::kotlin::template_env::render(
                "data_class_field_line.jinja",
                minijinja::context! {
                    indent => "    ",
                    field => field_str,
                },
            ));
        }
        out.push_str(&crate::backends::kotlin::template_env::render(
            "data_class_close.jinja",
            minijinja::context! {
                indent => "",
                suffix => if has_instance_methods { " {" } else { "" },
            },
        ));
    }

    if let Some(bridge) = value_method_bridge {
        for &method in &bridged_methods {
            emit_value_method(out, ty, method, imports, bridge);
        }
    }

    if has_instance_methods {
        out.push_str("}\n");
        emit_value_method_mapper(out);
    }
}

/// Emit one data-class instance method backed by a JNI value-method shim.
///
/// The receiver is marshalled as JSON (`this`), parameters as a JSON object
/// keyed by the Rust parameter name, and the result comes back as either a JNI
/// primitive or a JSON string that Jackson reads into the declared Kotlin type.
/// Delegating rather than reimplementing keeps behaviour the Kotlin side cannot
/// see — argument clamping, validation messages — identical to the core library.
fn emit_value_method(
    out: &mut String,
    ty: &TypeDef,
    method: &MethodDef,
    imports: &mut BTreeSet<String>,
    bridge: ValueMethodBridge<'_>,
) {
    let method_name = to_lower_camel(&method.name);
    let returns_receiver = is_functional_ref_mut_value_method(method);
    let return_type = if returns_receiver {
        TypeRef::Named(ty.name.clone())
    } else {
        method.return_type.clone()
    };
    let return_type_str = kotlin_type_with_string_imports(&return_type, false, imports);

    let params_sig: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ptype = kotlin_type_with_string_imports(&p.ty, p.optional, imports);
            let pname = to_lower_camel(&p.name);
            format!("{pname}: {ptype}")
        })
        .collect();

    out.push('\n');
    emit_cleaned_kdoc(out, &method.doc, "    ");
    out.push_str("    fun ");
    out.push_str(&method_name);
    out.push('(');
    out.push_str(&params_sig.join(", "));
    out.push_str("): ");
    out.push_str(&return_type_str);
    out.push_str(" {\n");
    out.push_str(&format!(
        "        val selfJson = {VALUE_METHOD_MAPPER}.writeValueAsString(this)\n"
    ));

    let mut call_args = vec!["selfJson".to_string()];
    if !method.params.is_empty() {
        out.push_str(&format!(
            "        val requestJson = {VALUE_METHOD_MAPPER}.writeValueAsString(\n            mapOf(\n"
        ));
        for param in &method.params {
            let key = escape_kotlin_string(&param.name.replace('-', "_"));
            let value = to_lower_camel(&param.name);
            out.push_str(&format!("                \"{key}\" to {value},\n"));
        }
        out.push_str("            ),\n        )\n");
        call_args.push("requestJson".to_string());
    }

    let native_call = format!(
        "{}.{}({})",
        bridge.bridge_class,
        bridge_method_name(&ty.name, &method.name),
        call_args.join(", ")
    );

    match &return_type {
        TypeRef::Unit => out.push_str(&format!("        {native_call}\n")),
        TypeRef::Primitive(_) | TypeRef::String => out.push_str(&format!("        return {native_call}\n")),
        _ => {
            out.push_str(&format!("        val resultJson = {native_call}\n"));
            out.push_str(&format!("        return {VALUE_METHOD_MAPPER}.readValue(\n"));
            out.push_str("            resultJson,\n");
            out.push_str(&format!(
                "            object : com.fasterxml.jackson.core.type.TypeReference<{return_type_str}>() {{}},\n"
            ));
            out.push_str("        )\n");
        }
    }
    out.push_str("    }\n");
}

/// Emit the file-private Jackson mapper backing value-method marshalling.
///
/// Emitted at most once per file; the Kotlin visibility keeps it from colliding
/// with the mapper any other generated file declares.
fn emit_value_method_mapper(out: &mut String) {
    let declaration = crate::backends::kotlin::template_env::render(
        "value_method_mapper.jinja",
        minijinja::context! {
            name => VALUE_METHOD_MAPPER,
        },
    );
    if !out.contains(&declaration) {
        out.push('\n');
        out.push_str(&declaration);
    }
}

/// Return the `@field:JsonSerialize(...)` annotation source needed for a
/// field whose declared type references a sealed class, or `None` if the
/// type does not reference a sealed class.
///
/// Recognised shapes (Optional layers are unwrapped first):
/// - `Named(sealed)` → `@field:JsonSerialize(\`as\` = sealed::class)`
/// - `Vec<Named(sealed)>` → `@field:JsonSerialize(contentAs = sealed::class)`
/// - `Map<_, Named(sealed)>` → `@field:JsonSerialize(contentAs = sealed::class)`
///
/// Other shapes (nested generics, sealed inside `Map` key, …) are ignored —
/// they don't appear in the current codebase, and `contentAs` cannot express
/// them anyway.
fn sealed_class_field_annotation(
    ty: &TypeRef,
    sealed_class_names: &std::collections::HashSet<String>,
) -> Option<String> {
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    match base {
        TypeRef::Named(name) if sealed_class_names.contains(name) => Some(format!(
            "@field:com.fasterxml.jackson.databind.annotation.JsonSerialize(`as` = {name}::class)"
        )),
        TypeRef::Vec(inner) => {
            let inner_base = match inner.as_ref() {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            };
            if let TypeRef::Named(name) = inner_base
                && sealed_class_names.contains(name)
            {
                return Some(format!(
                    "@field:com.fasterxml.jackson.databind.annotation.JsonSerialize(contentAs = {name}::class)"
                ));
            }
            None
        }
        TypeRef::Map(_, value) => {
            let value_base = match value.as_ref() {
                TypeRef::Optional(i) => i.as_ref(),
                other => other,
            };
            if let TypeRef::Named(name) = value_base
                && sealed_class_names.contains(name)
            {
                return Some(format!(
                    "@field:com.fasterxml.jackson.databind.annotation.JsonSerialize(contentAs = {name}::class)"
                ));
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{CoreWrapper, TypeDef};

    fn make_field(name: &str, ty: TypeRef, serde_rename: Option<&str>) -> crate::core::ir::FieldDef {
        crate::core::ir::FieldDef {
            version: Default::default(),
            name: name.to_string(),
            ty,
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
            serde_rename: serde_rename.map(str::to_string),
            serde_flatten: false,
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    /// Regression: Kotlin now warns that an annotation on a primary-constructor `val` with no
    /// explicit use-site target "currently applies to a value parameter only [and] in a future
    /// release it will apply to the property/field" and suggests `@param:` or `@field:`. Jackson
    /// creator-based deserialization (via jackson-module-kotlin) needs `@JsonProperty` on the
    /// constructor parameter, which is exactly today's implicit behavior — so the fix locks that
    /// in with an explicit `@param:` rather than silently also targeting the backing field.
    #[test]
    fn json_property_annotation_targets_the_constructor_parameter_explicitly() {
        let timeout_field = make_field(
            "timeout_ms",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
            Some("timeoutMs"),
        );
        let ty = TypeDef {
            name: "ClientConfig".to_string(),
            rust_path: "crate::ClientConfig".to_string(),
            fields: vec![timeout_field],
            has_serde: true,
            ..Default::default()
        };

        let mut out = String::new();
        let mut imports = std::collections::BTreeSet::new();
        emit_type_with_imports(
            &ty,
            &mut out,
            &mut imports,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            None,
        );

        assert!(
            out.contains("@param:com.fasterxml.jackson.annotation.JsonProperty(\"timeoutMs\")"),
            "renamed field must carry an explicit @param: use-site target: {out}"
        );
        assert!(
            !out.contains("\n    @com.fasterxml.jackson.annotation.JsonProperty("),
            "the annotation must not be emitted with no explicit use-site target: {out}"
        );
    }

    /// Regression: the value-method JNI mapper (`VALUE_METHOD_MAPPER`) is Jackson-configured
    /// the same way as the module facade's mapper and carries the same deprecated
    /// `setSerializationInclusion(...)` call (Jackson deprecated it since 2.13 in favor of
    /// `setDefaultPropertyInclusion(...)`) until fixed.
    #[test]
    fn value_method_mapper_uses_the_non_deprecated_default_property_inclusion_setter() {
        let mut out = String::new();
        emit_value_method_mapper(&mut out);

        assert!(
            out.contains(".setDefaultPropertyInclusion(com.fasterxml.jackson.annotation.JsonInclude.Include.NON_NULL)"),
            "value-method mapper must configure inclusion via the non-deprecated setter: {out}"
        );
        assert!(
            !out.contains(".setSerializationInclusion("),
            "value-method mapper must not call the deprecated (since Jackson 2.13) setSerializationInclusion: {out}"
        );
    }

    /// Direct unit coverage of the name-shape predicate itself, independent of any
    /// particular crate's field names. `n_widgets` is a stand-in shaped like the pattern the
    /// predicate targets (single lower-case letter, then an upper-case letter once
    /// camelCased); `max_depth` and `text` are ordinary shapes the predicate must leave alone.
    #[test]
    fn bean_introspection_predicate_fires_only_on_the_documented_shape() {
        assert!(kotlin_property_name_defeats_bean_introspection("nWidgets"));
        assert!(kotlin_property_name_defeats_bean_introspection("xOffset"));
        assert!(!kotlin_property_name_defeats_bean_introspection("maxDepth"));
        assert!(!kotlin_property_name_defeats_bean_introspection("text"));
        assert!(!kotlin_property_name_defeats_bean_introspection(""));
        assert!(!kotlin_property_name_defeats_bean_introspection("a"));
    }

    /// Regression for a real deserialization break: a Kotlin data-class property with no
    /// `#[serde(rename = "...")]` but whose camelCased name is a single lower-case letter
    /// followed by an upper-case letter (e.g. `n_widgets` -> `nWidgets`) is silently mangled
    /// by Jackson's default bean-property derivation into the wrong wire name (`nwidgets`
    /// instead of `n_widgets`) once `PropertyNamingStrategies.SNAKE_CASE` is applied, because
    /// `java.beans.Introspector.decapitalize` refuses to lowercase a leading run of two
    /// upper-case letters (`getNWidgets` -> `NWidgets`, not `nWidgets`). The generator must
    /// emit an explicit `@JsonProperty` naming the real wire field so serialization survives
    /// regardless of what naming strategy a consumer's ObjectMapper is configured with.
    ///
    /// A neighboring ordinary field (`max_depth`) proves the fix does not vacuously annotate
    /// every field regardless of shape.
    #[test]
    fn json_property_is_emitted_for_names_that_defeat_bean_introspection() {
        let vulnerable_field = make_field(
            "n_widgets",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
            None,
        );
        let ordinary_field = make_field(
            "max_depth",
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
            None,
        );
        let ty = TypeDef {
            name: "WidgetConfig".to_string(),
            rust_path: "crate::WidgetConfig".to_string(),
            fields: vec![vulnerable_field, ordinary_field],
            has_serde: true,
            ..Default::default()
        };

        let mut out = String::new();
        let mut imports = std::collections::BTreeSet::new();
        emit_type_with_imports(
            &ty,
            &mut out,
            &mut imports,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            None,
        );

        assert!(
            out.contains("@param:com.fasterxml.jackson.annotation.JsonProperty(\"n_widgets\")"),
            "a name shaped like [a-z][A-Z]... must get an explicit wire-name annotation: {out}"
        );
        assert!(
            !out.contains("\"max_depth\""),
            "an ordinary field name must not be annotated: {out}"
        );
    }
}
