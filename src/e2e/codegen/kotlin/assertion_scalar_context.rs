//! Kotlin `render_assertion` scalar-pipeline context: the field-shape facts and derived
//! accessor expressions computed once a fixture field survives every early gate in
//! `assertion_field_gates.rs`, and is about to be dispatched through
//! `assertion_scalar_dispatch::render_scalar_assertion`.
//!
//! Split out of `assertions.rs` at the concept boundary -- each function here is a verbatim
//! extraction of one `let` binding `render_assertion` used to compute inline; no behavior
//! change, only a function boundary around statements that already existed. ~keep

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Determine if this field is an enum type, whether its resolved type is an untyped JSON scalar
/// (Kotlin `Any?`, from Rust `Option<serde_json::Value>`), and whether it is a display_as_text
/// field (e.g., AssistantContent) with a `.text()` accessor. `enum_fields` carries the effective
/// hand-maintained config (merged with the call-level `type_enum_fields` auto-detect in
/// test_method.rs, which itself requires a `result_type` override to anchor); when it is silent,
/// `field_resolver.is_enum` falls back to the IR-derived classification. This is purely
/// additive -- it only turns a `false` into `true`. ~keep
///
/// A payload-carrying union never reaches here: `assertion_field_gates::
/// try_skip_payload_union_scalar_lowering` refused it, because neither target's scalar accessor
/// (`toWire()` / the Java facade's `getValue()`) exists on the sealed class the binding emitted,
/// and the generic string fallback would compare a wrapper object to a `String`. ~keep
pub(super) fn compute_field_shape_flags(
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    enum_fields: &std::collections::HashSet<String>,
    json_scalar_fields: &std::collections::HashSet<String>,
) -> (bool, bool, bool) {
    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)) || field_resolver.is_enum(f)
    });
    let field_is_json_scalar = assertion
        .field
        .as_deref()
        .is_some_and(|field| field_resolver.is_json_scalar(field, json_scalar_fields));
    let field_is_display_as_text = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_display_as_text(f));
    (field_is_enum, field_is_json_scalar, field_is_display_as_text)
}

/// The raw field accessor -- may end with nullable type if field is optional. `accessor_lang` is
/// "kotlin_android" or "kotlin" since kotlin_android data classes expose properties (no parens).
pub(super) fn resolve_field_expr(
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
    result_is_simple: bool,
    accessor_lang: &str,
) -> String {
    if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, accessor_lang, result_var),
            _ => result_var.to_string(),
        }
    }
}

/// Whether the accessor may return a nullable type in Kotlin. This is true when the leaf field
/// OR any intermediate segment in the path is optional (the `?.` safe-call propagates null
/// through the whole chain).
///
/// Additionally, if the generated accessor expression itself contains `?.` then the return type
/// is `T?` regardless of what the path-resolver says — sticky nullability means any `?.` in the
/// chain makes the whole expression nullable. This handles cases like
/// `toolCalls()?.first()?.function()?.name()` where the `is_optional` prefix lookup misses due
/// to index notation mismatch.
pub(super) fn resolve_field_is_optional(
    result_is_simple: bool,
    field_expr: &str,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    kotlin_android_style: bool,
) -> bool {
    !result_is_simple
        && (field_expr.contains("?.")
            || assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                if field_resolver.has_map_access(f) {
                    // Kotlin's `Map<K, V>.get(key)` always returns `V?`. In the
                    // kotlin_android target, DTOs are pure Kotlin data classes so
                    // the nullable propagates through and string operations on
                    // the result must coalesce or safe-call. In the kotlin/JVM
                    // target the same map field flows through Java records and
                    // appears as a platform type, so adding `.orEmpty()` is
                    // unnecessary but harmless — keep the legacy behaviour for
                    // JVM to avoid churning unrelated snapshots.
                    return kotlin_android_style;
                }
                // Check the leaf field itself.
                if field_resolver.is_optional(resolved) {
                    return true;
                }
                // Also check every prefix segment: if any intermediate field is
                // optional the ?.  chain propagates null to the final result.
                let mut prefix = String::new();
                for part in resolved.split('.') {
                    // Strip array notation for the lookup key.
                    let key = part.split('[').next().unwrap_or(part);
                    if !prefix.is_empty() {
                        prefix.push('.');
                    }
                    prefix.push_str(key);
                    if field_resolver.is_optional(&prefix) {
                        return true;
                    }
                }
                false
            }))
}

/// String-context expression: append .orEmpty() for nullable string fields so string operations
/// (contains, trim) don't require a safe-call chain. Note: this is only sound when the leaf type
/// is `String?`. For enum-typed optional fields (`T?` where `T` is an enum class), `.orEmpty()`
/// is undefined; `resolve_string_expr` handles those by going through `?.getValue()` first. For
/// fields in `json_scalar_fields` (`Any?`, e.g. `Option<serde_json::Value>`), `.orEmpty()` is
/// likewise undefined; stringify through `?.toString()` first. For display_as_text fields (e.g.,
/// AssistantContent), call `.text()` to extract the textual representation, which returns
/// `String` (non-nullable). Also handle the case where the bare result (no field specified) is
/// nullable due to `result_is_option` being true.
pub(super) fn resolve_string_field_expr(
    field_expr: &str,
    field_is_display_as_text: bool,
    field_is_json_scalar: bool,
    bare_result_is_nullable: bool,
    field_is_optional: bool,
) -> String {
    if field_is_display_as_text {
        // display_as_text fields have a .text() accessor returning String
        if field_is_optional {
            format!("{field_expr}?.text().orEmpty()")
        } else {
            format!("{field_expr}.text()")
        }
    } else if field_is_json_scalar {
        // `.orEmpty()` is a `String?`/`CharSequence?` extension and is undefined on
        // `Any?` — stringify through a null-safe call first (`Any?.toString()` is
        // always defined), then coalesce the resulting `String?` the same way.
        format!("{field_expr}?.toString().orEmpty()")
    } else if bare_result_is_nullable {
        format!("{field_expr}?.toString().orEmpty()")
    } else if field_is_optional {
        format!("{field_expr}.orEmpty()")
    } else {
        field_expr.to_string()
    }
}

/// For enum fields, convert to string for comparison.
///
/// - JVM (kotlin) mode: The Java facade wraps enums in a Java enum type that exposes a
///   `.getValue()` accessor. Use `.getValue()` (with optional-safe variant when the field is
///   nullable), mirroring the Java codegen pattern
///   `Optional.ofNullable(...).map(v -> v.getValue()).orElse("")`.
///
/// - kotlin_android mode: every fieldless `enum class` carries a `fun toWire(): String`
///   returning the exact `wire_variant_value` per constant, the same string `@JsonValue`
///   serializes. A prior `.name.lowercase()` wrongly assumed every wire value is the Kotlin
///   constant name lowercased (`IN_PROGRESS` -> `"in_progress"`); that fails for `DataNodeKind`
///   (no `rename_all`): `KEY_VALUE` -> `"keyvalue"`, not `"KeyValue"`. ~keep
pub(super) fn resolve_string_expr(
    field_expr: &str,
    field_is_enum: bool,
    field_is_optional: bool,
    string_field_expr: &str,
    kotlin_android_style: bool,
) -> String {
    if kotlin_android_style {
        match (field_is_enum, field_is_optional) {
            (true, true) => format!("{field_expr}?.toWire().orEmpty()"),
            (true, false) => format!("{field_expr}.toWire()"),
            (false, _) => string_field_expr.to_string(),
        }
    } else {
        match (field_is_enum, field_is_optional) {
            (true, true) => format!("{field_expr}?.getValue().orEmpty()"),
            (true, false) => format!("{field_expr}.getValue()"),
            (false, _) => string_field_expr.to_string(),
        }
    }
}

/// Determine if this assertion field maps to a 64-bit C type (uint64_t / int64_t), which
/// corresponds to Kotlin `Long`. When true, integer literals must be suffixed with `L` to avoid
/// a type mismatch between Kotlin `Int` and `Long`.
pub(super) fn compute_field_is_long(
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    fields_c_types: &std::collections::HashMap<String, String>,
) -> bool {
    assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        matches!(
            fields_c_types.get(resolved).map(String::as_str),
            Some("uint64_t") | Some("int64_t")
        )
    })
}

/// Determine whether the field's underlying type is a list/collection. For `contains` /
/// `contains_all` / `not_contains` assertions on `List<String>` fields Kotlin requires a cast to
/// `List<String>` so the `@OnlyInputTypes` annotation on `Collection.contains()` can infer `T`.
/// For plain `String` fields (e.g. `result.text` on TranscribeTest) the assertion is a substring
/// check on a `String` — emitting `(s as List<String>).contains` throws ClassCastException at
/// runtime, so the cast must be gated on the field actually being a collection.
/// `field_resolver.is_array` is true for paths in `fields_array`; `is_collection_root` is true
/// when the field is a top-level collection accessor (e.g. `tags` whose entries are tracked as
/// `tags[0]` in `fields_array`).
pub(super) fn compute_field_is_collection(assertion: &Assertion, field_resolver: &FieldResolver) -> bool {
    assertion.field.as_deref().filter(|f| !f.is_empty()).is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        field_resolver.is_array(f)
            || field_resolver.is_array(resolved)
            || field_resolver.is_collection_root(f)
            || field_resolver.is_collection_root(resolved)
    })
}

/// Inputs to [`compute_scalar_context`], grouped to keep the function under poly's
/// `too-many-parameters` limit. Field order matches the parameter order it replaces -- no
/// behavior change.
pub(super) struct ScalarContextInputs<'a> {
    pub(super) assertion: &'a Assertion,
    pub(super) field_resolver: &'a FieldResolver,
    pub(super) result_var: &'a str,
    pub(super) result_is_simple: bool,
    pub(super) result_is_option: bool,
    pub(super) enum_fields: &'a std::collections::HashSet<String>,
    pub(super) json_scalar_fields: &'a std::collections::HashSet<String>,
    pub(super) fields_c_types: &'a std::collections::HashMap<String, String>,
    pub(super) kotlin_android_style: bool,
}

/// Compute every scalar-pipeline context value `render_scalar_assertion` needs, in the same
/// order `render_assertion` used to compute them inline. Bundles the six `compute_*`/`resolve_*`
/// helpers above into the one call `render_assertion` makes, so it stays under the file's
/// function-length cap -- verbatim orchestration, no behavior change. ~keep
pub(super) fn compute_scalar_context(
    inputs: &ScalarContextInputs,
) -> (String, String, String, String, bool, bool, bool) {
    let ScalarContextInputs {
        assertion,
        field_resolver,
        result_var,
        result_is_simple,
        result_is_option,
        enum_fields,
        json_scalar_fields,
        fields_c_types,
        kotlin_android_style,
    } = *inputs;
    let (field_is_enum, field_is_json_scalar, field_is_display_as_text) =
        compute_field_shape_flags(assertion, field_resolver, enum_fields, json_scalar_fields);
    let accessor_lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let field_expr = resolve_field_expr(assertion, field_resolver, result_var, result_is_simple, accessor_lang);
    let field_is_optional = resolve_field_is_optional(
        result_is_simple,
        &field_expr,
        assertion,
        field_resolver,
        kotlin_android_style,
    );
    let bare_result_is_nullable = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    let string_field_expr = resolve_string_field_expr(
        &field_expr,
        field_is_display_as_text,
        field_is_json_scalar,
        bare_result_is_nullable,
        field_is_optional,
    );
    let nonnull_field_expr = if field_is_optional {
        format!("{field_expr}!!")
    } else {
        field_expr.clone()
    };
    let string_expr = resolve_string_expr(
        &field_expr,
        field_is_enum,
        field_is_optional,
        &string_field_expr,
        kotlin_android_style,
    );
    let field_is_long = compute_field_is_long(assertion, field_resolver, fields_c_types);
    let field_is_collection = compute_field_is_collection(assertion, field_resolver);
    (
        field_expr,
        string_field_expr,
        nonnull_field_expr,
        string_expr,
        field_is_optional,
        field_is_collection,
        field_is_long,
    )
}
