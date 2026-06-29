use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::helpers::{
    core_prim_str, core_type_path_remapped, field_references_excluded_type, is_newtype, needs_f64_cast, needs_i32_cast,
    needs_i64_cast,
};
use crate::core::ir::{CoreWrapper, TypeDef, TypeRef};

use super::fields::field_conversion_to_core_cfg;
use super::wrappers::apply_core_wrapper_to_core;

/// Generate `impl From<BindingType> for core::Type` (binding -> core).
/// Sanitized fields use `Default::default()` unless the sanitizer only removed a
/// core wrapper that can be reconstructed losslessly from the binding value.
pub fn gen_from_binding_to_core(typ: &TypeDef, core_import: &str) -> String {
    gen_from_binding_to_core_cfg(typ, core_import, &ConversionConfig::default())
}

/// Generate `impl From<BindingType> for core::Type` with backend-specific config.
pub fn gen_from_binding_to_core_cfg(typ: &TypeDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_type_path_remapped(typ, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, typ.name);

    // Types with an explicit static `new()` method may have private fields not exposed in the
    // binding IR. The struct literal construction path would fail to compile because it cannot
    // set the private fields. Flag these types so the template emits an explicit compile-time
    // config requirement instead of a runtime placeholder.
    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");

    // Newtype structs: generate tuple constructor Self(val._0)
    if is_newtype(typ) {
        let field = &typ.fields[0];
        let newtype_inner_expr = match &field.ty {
            TypeRef::Named(_) => "val._0.into()".to_string(),
            TypeRef::Path => "val._0.into()".to_string(),
            TypeRef::Duration => "std::time::Duration::from_millis(val._0)".to_string(),
            _ => "val._0".to_string(),
        };
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => core_path,
                binding_name => binding_name,
                has_lifetime_params => typ.has_lifetime_params,
                is_newtype => true,
                has_explicit_static_new => false,
                newtype_inner_expr => newtype_inner_expr,
                builder_mode => false,
                uses_builder_pattern => false,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => vec![] as Vec<String>,
                fields => vec![] as Vec<String>,
            },
        );
    }

    // Types with lifetime parameters have private fields that forbid struct-literal construction.
    // Find a suitable static factory method from the IR (a method with no receiver whose params
    // are a superset of the type's fields) and emit a constructor call instead.
    if typ.has_lifetime_params {
        if let Some(constructor_call) = gen_from_lifetime_type_constructor(typ, &core_path, &binding_name, config) {
            return constructor_call;
        }
        // No suitable constructor found; emit an explicit compile-time config requirement to
        // avoid generating a broken struct literal (binding fields are String while core fields
        // may be &str or other borrowed types).
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => &core_path,
                binding_name => &binding_name,
                has_lifetime_params => true,
                is_newtype => false,
                has_explicit_static_new => true,
                newtype_inner_expr => String::new(),
                builder_mode => false,
                uses_builder_pattern => false,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => vec![] as Vec<String>,
                fields => vec![] as Vec<String>,
            },
        );
    }

    // Types with an explicit static `new()` constructor (private fields) but no lifetime params:
    // attempt to synthesise a constructor call; fall back to compile_error! if no suitable
    // constructor is found in the IR.
    if has_explicit_static_new {
        if let Some(call) = gen_from_explicit_new_constructor(typ, &core_path, &binding_name, config) {
            return call;
        }
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => &core_path,
                binding_name => &binding_name,
                has_lifetime_params => false,
                is_newtype => false,
                has_explicit_static_new => true,
                newtype_inner_expr => String::new(),
                builder_mode => false,
                uses_builder_pattern => false,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => vec![] as Vec<String>,
                fields => vec![] as Vec<String>,
            },
        );
    }

    // Determine if we're using the builder pattern
    let uses_builder_pattern = (config.option_duration_on_defaults
        && typ.has_default
        && typ
            .fields
            .iter()
            .any(|f| !f.optional && matches!(f.ty, TypeRef::Duration)))
        || (config.optionalize_defaults && typ.has_default);

    // When option_duration_on_defaults is set for a has_default type, non-optional Duration
    // fields are stored as Option<u64> in the binding struct.  We use the builder pattern
    // so that None falls back to the core type's Default (giving the real field default,
    // e.g. Duration::from_millis(30000)) rather than Duration::ZERO).

    // Determine if we're using the builder pattern
    let has_optionalized_fields = config.option_duration_on_defaults
        && typ.has_default
        && typ
            .fields
            .iter()
            .any(|f| !f.optional && matches!(f.ty, TypeRef::Duration));

    if has_optionalized_fields {
        // Builder pattern: start from core default, override explicitly-set fields.
        let optionalized = config.optionalize_defaults && typ.has_default;
        let mut statements = Vec::new();
        for field in &typ.fields {
            if field.binding_excluded {
                continue;
            }
            if field.sanitized && field.core_wrapper != CoreWrapper::Cow {
                // sanitized fields keep the default value — skip
                continue;
            }
            // Fields referencing excluded types keep their default value — skip
            if !config.exclude_types.is_empty() && field_references_excluded_type(&field.ty, config.exclude_types) {
                continue;
            }
            // Duration field stored as Option<u64/i64>: only override when Some
            let binding_name_field = config.binding_field_name_owned(&typ.name, &field.name);
            if !field.optional && matches!(field.ty, TypeRef::Duration) {
                let cast = if config.cast_large_ints_to_i64 { " as u64" } else { "" };
                statements.push(format!(
                    "if let Some(__v) = val.{binding_name_field} {{ __result.{} = std::time::Duration::from_millis(__v{cast}); }}",
                    field.name
                ));
                continue;
            }
            // Determine if this field was Option-wrapped by config for ergonomics.
            // Two cases:
            // 1. optionalize_defaults=true: all non-optional IR fields become Option<T> in binding
            // 2. option_duration_on_defaults=true: non-optional Duration IR fields become Option<u64> in binding
            //
            // Core field optionality matters:
            // - If core is non-optional (T): unwrap binding Option, use if-let to preserve defaults
            // - If core is optional (Option<T>): both binding and core are Option, skip if-let
            let field_is_optionalized_by_duration = config.option_duration_on_defaults
                && typ.has_default
                && !field.optional
                && matches!(field.ty, TypeRef::Duration);
            let field_is_config_optionalized = (optionalized && !field.optional) || field_is_optionalized_by_duration;

            // Genuinely-optional fields (both binding and core are Option<T>).
            // These should NOT use if-let unwrapping.
            let _field_is_genuinely_optional = config.option_duration_on_defaults && typ.has_default && field.optional;

            let conversion = if field_is_config_optionalized {
                // Field was Option-wrapped by optionalize_defaults or option_duration_on_defaults;
                // core field is non-optional (T). Compute conversion for the unwrapped value.
                field_conversion_to_core_cfg(&field.name, &field.ty, false, config)
            } else {
                // Standard path: either not optionalized, or genuinely-optional (Option<T>→Option<T>).
                field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config)
            };
            // Apply binding field name substitution for keyword-escaped fields.
            let conversion = if binding_name_field != field.name {
                conversion.replace(&format!("val.{}", field.name), &format!("val.{binding_name_field}"))
            } else {
                conversion
            };
            // Strip the "name: " prefix to get just the expression, then assign
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                if field_is_config_optionalized {
                    // Emit `if let Some(__v) = val.field { __result.field = <expr with __v>; }`
                    // so omitted fields preserve core's Default value rather than being
                    // overwritten with the primitive zero from `.unwrap_or_default()`.
                    statements.push(format!(
                        "if let Some(__v) = val.{binding_name_field} {{ __result.{} = {}; }}",
                        field.name,
                        expr.replace(&format!("val.{binding_name_field}"), "__v")
                    ));
                } else {
                    statements.push(format!("__result.{} = {};", field.name, expr));
                }
            }
        }

        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => core_path,
                binding_name => binding_name,
                has_lifetime_params => typ.has_lifetime_params,
                is_newtype => false,
                has_explicit_static_new => false,
                newtype_inner_expr => "",
                builder_mode => true,
                uses_builder_pattern => uses_builder_pattern,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => statements,
                fields => vec![] as Vec<String>,
            },
        );
    }

    let optionalized = config.optionalize_defaults && typ.has_default;

    // Pre-compute all fields
    let mut fields = Vec::new();
    let mut statements = Vec::new();
    // Track whether any binding-excluded field was skipped — when so, force the
    // `..Default::default()` trailer so the core type's Default impl fills those
    // fields in (preserves invariants like `SsrfPolicy::from_env`, which an
    // explicit field-level `Default::default()` on a sub-type would bypass).
    //
    // Exception: when the core type does not implement Default, the spread
    // trailer would fail to compile. In that case, fall back to emitting
    // per-field `Default::default()` for each binding-excluded field — there
    // is no core Default to bypass.
    let core_has_default = typ.has_default;
    let mut skipped_binding_excluded = false;

    for field in &typ.fields {
        if field.binding_excluded {
            // Skip the field entirely and rely on `..Default::default()` to
            // populate it. Emitting `field: Default::default()` here would call
            // the sub-type's `Default` directly, bypassing any core-type Default
            // that intentionally departs from per-field defaults (for example
            // a `Config::default()` that reads an environment variable to pick
            // a non-zero policy, whereas the embedded sub-policy's own
            // `default()` hardcodes a stricter value).
            //
            // BUT: when the core type does not derive/impl Default, the spread
            // trailer (`..Default::default()`) does not compile. Emit a per-field
            // `Default::default()` so the From impl still works — there is no
            // bespoke core Default whose semantics we could be bypassing.
            if !core_has_default {
                fields.push(format!("{}: Default::default()", field.name));
                continue;
            }
            skipped_binding_excluded = true;
            continue;
        }
        // Cfg-gated fields: emit the assignment with `#[cfg(...)]` so it only applies when
        // the same feature is enabled on the binding crate. Force-restored (never_skip) fields
        // skip the gate — they're always emitted (used by trait-bridge bind_via = "options_field").
        // Pre-stripped types still have the field in IR; we just don't emit the cfg gate here
        // since the binding struct definition has already been gated.
        // Fields referencing excluded types don't exist in the binding struct.
        // When the type has stripped cfg-gated fields, these fields may also be
        // cfg-gated and absent from the core struct — skip them entirely and let
        // ..Default::default() fill them in.
        // Otherwise, use Default::default() to fill them in the core type.
        // Sanitized fields also use Default::default() (lossy but functional).
        let references_excluded =
            !config.exclude_types.is_empty() && field_references_excluded_type(&field.ty, config.exclude_types);
        if references_excluded && typ.has_stripped_cfg_fields {
            continue;
        }
        // When the binding crate strips cfg-gated fields from the struct
        // (typically because the backend doesn't carry feature gates into the binding
        // crate's Cargo.toml — e.g. extendr), the From impl cannot reference
        // val.<field> because the field doesn't exist in the binding struct.
        // Skip these entirely; ..Default::default() in the template handles them.
        if field.cfg.is_some()
            && !config.never_skip_cfg_field_names.contains(&field.name)
            && config.strip_cfg_fields_from_binding_struct
        {
            continue;
        }
        if optionalized && ((field.sanitized && field.core_wrapper != CoreWrapper::Cow) || references_excluded) {
            continue;
        }
        let field_was_optionalized = optionalized && !field.optional;
        let conversion = if (field.sanitized && field.core_wrapper != CoreWrapper::Cow) || references_excluded {
            format!("{}: Default::default()", field.name)
        } else if field_was_optionalized {
            // Field was wrapped in Option<T> for JS ergonomics but core expects T.
            // Convert the supplied value as T; omitted fields keep the core type's Default value.
            field_conversion_to_core_cfg(&field.name, &field.ty, false, config)
        } else {
            field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config)
        };
        // Newtype wrapping: when the field was resolved from a newtype (e.g. NodeIndex → u32),
        // wrap the binding value back into the newtype for the core struct.
        // e.g. `source: val.source` → `source: sample_core::NodeIndex(val.source)`
        //      `parent: val.parent` → `parent: val.parent.map(sample_core::NodeIndex)`
        //      `children: val.children` → `children: val.children.into_iter().map(sample_core::NodeIndex).collect()`
        let conversion = if let Some(newtype_path) = &field.newtype_wrapper {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                // When `optional=true` and `ty` is a plain Primitive (not TypeRef::Optional), the core
                // field is actually `Option<NewtypeT>`, so we must use `.map(NewtypeT)` not `NewtypeT(...)`.
                match &field.ty {
                    TypeRef::Optional(_) => format!("{}: ({expr}).map({newtype_path})", field.name),
                    TypeRef::Vec(_) => {
                        // When the inner expr already ends with .collect() (e.g. because of a
                        // primitive cast), the compiler cannot infer the intermediate Vec type
                        // without an explicit type annotation. Use collect::<Vec<_>>() to make
                        // the intermediate collection type unambiguous before mapping to newtype.
                        let inner_expr = if let Some(prefix) = expr.strip_suffix(".collect()") {
                            format!("{prefix}.collect::<Vec<_>>()")
                        } else {
                            expr.to_string()
                        };
                        format!(
                            "{}: ({inner_expr}).into_iter().map({newtype_path}).collect()",
                            field.name
                        )
                    }
                    _ if field.optional => format!("{}: ({expr}).map({newtype_path})", field.name),
                    _ => format!("{}: {newtype_path}({expr})", field.name),
                }
            } else {
                conversion
            }
        } else {
            conversion
        };
        // Box<T> fields: wrap the converted value in Box::new()
        let conversion = if field.is_boxed && matches!(&field.ty, TypeRef::Named(_)) {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                if field.optional {
                    // Option<Box<T>> field: map inside the Option
                    format!("{}: {}.map(Box::new)", field.name, expr)
                } else {
                    format!("{}: Box::new({})", field.name, expr)
                }
            } else {
                conversion
            }
        } else {
            conversion
        };
        // CoreWrapper: apply Cow/Arc/Bytes wrapping for binding→core direction.
        //
        // Special case: opaque Named field with CoreWrapper::Arc.
        // The binding wrapper already holds `inner: Arc<CoreT>`, so the correct
        // conversion is to extract `.inner` directly rather than calling `.into()`
        // (which requires `From<BindingType> for CoreT`, a non-existent impl) and
        // then wrapping in `Arc::new` (which would double-wrap the Arc).
        let is_opaque_arc_field = field.core_wrapper == CoreWrapper::Arc
            && matches!(&field.ty, TypeRef::Named(n) if config
                .opaque_types
                .is_some_and(|opaque| opaque.contains(n.as_str())));
        // Opaque Named fields without CoreWrapper::Arc (e.g. visitor: Object<'static>) cannot be
        // auto-converted via Into — the binding stores a raw JS object that needs a bridge.
        // Emit Default::default() and let the caller (e.g. the convert function) set it separately.
        let is_opaque_no_wrapper_field = field.core_wrapper == CoreWrapper::None
            && matches!(&field.ty, TypeRef::Named(n) if config
                .opaque_types
                .is_some_and(|opaque| opaque.contains(n.as_str())));
        let conversion = if is_opaque_arc_field {
            if field.optional {
                format!("{}: val.{}.map(|v| v.inner)", field.name, field.name)
            } else {
                format!("{}: val.{}.inner", field.name, field.name)
            }
        } else if is_opaque_no_wrapper_field {
            // Trait-bridge OptionsField fields: the binding wrapper holds `inner: Arc<core::T>`.
            // Clone out of the Arc so the visitor (or other bridge handle) is forwarded instead
            // of silently dropped. Fall back to Default::default() when no Arc wrapper is present.
            if config.trait_bridge_field_is_arc_wrapper(&field.name) {
                if field.optional {
                    format!("{}: val.{}.map(|v| (*v.inner).clone())", field.name, field.name)
                } else {
                    format!("{}: (*val.{}.inner).clone()", field.name, field.name)
                }
            } else {
                format!("{}: Default::default()", field.name)
            }
        } else {
            apply_core_wrapper_to_core(
                &conversion,
                &field.name,
                &field.core_wrapper,
                &field.vec_inner_core_wrapper,
                field.optional,
            )
        };
        // When the binding struct uses a keyword-escaped field name (e.g. `class_` for `class`),
        // replace `val.{field.name}` access patterns in the conversion expression with
        // `val.{binding_name}` so the generated From impl compiles.
        let binding_name_field = config.binding_field_name_owned(&typ.name, &field.name);
        let conversion = if binding_name_field != field.name {
            conversion.replace(&format!("val.{}", field.name), &format!("val.{binding_name_field}"))
        } else {
            conversion
        };
        if optionalized {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                if field_was_optionalized {
                    statements.push(format!(
                        "if let Some(__v) = val.{binding_name_field} {{ __result.{} = {}; }}",
                        field.name,
                        expr.replace(&format!("val.{binding_name_field}"), "__v")
                    ));
                } else {
                    statements.push(format!("__result.{} = {};", field.name, expr));
                }
            }
        } else {
            fields.push(conversion);
        }
    }

    // Note: ..Default::default() for cfg-gated fields is emitted by the template
    // via the has_stripped_cfg_fields context variable — do not push it here.
    let emit_trailer = typ.has_stripped_cfg_fields || skipped_binding_excluded;

    crate::codegen::template_env::render(
        "conversions/binding_to_core_impl",
        minijinja::context! {
            core_path => core_path,
            binding_name => binding_name,
            has_lifetime_params => typ.has_lifetime_params,
            is_newtype => false,
            has_explicit_static_new => has_explicit_static_new,
            newtype_inner_expr => "",
            builder_mode => optionalized,
            uses_builder_pattern => uses_builder_pattern,
            has_stripped_cfg_fields => emit_trailer,
            statements => statements,
            fields => fields,
        },
    )
}

/// Generate `impl From<BindingType> for CoreType` using a static constructor method.
///
/// Used for types without lifetime params but with private fields (indicated by
/// `has_explicit_static_new`). Finds a static method (no receiver) whose parameters
/// cover all binding fields, then emits `Self::<method>(...)` using
/// `field_conversion_to_core_cfg` for each argument.
///
/// Returns `None` when no suitable constructor is found.
pub fn gen_from_explicit_new_constructor(
    typ: &TypeDef,
    core_path: &str,
    binding_name: &str,
    config: &ConversionConfig,
) -> Option<String> {
    let field_names: std::collections::HashSet<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| f.name.as_str())
        .collect();

    // Find a static method whose params cover all binding fields.
    // Prefer non-borrowed variants (same heuristic as gen_from_lifetime_type_constructor).
    let constructor = typ
        .methods
        .iter()
        .find(|m| {
            m.receiver.is_none()
                && !m.name.contains("borrowed")
                && field_names
                    .iter()
                    .all(|fname| m.params.iter().any(|p| p.name == *fname))
        })
        .or_else(|| {
            typ.methods.iter().find(|m| {
                m.receiver.is_none()
                    && field_names
                        .iter()
                        .all(|fname| m.params.iter().any(|p| p.name == *fname))
            })
        })?;

    // Build args in param order using standard field conversion expressions.
    let mut args: Vec<String> = Vec::new();
    for param in &constructor.params {
        if let Some(field) = typ.fields.iter().find(|f| f.name == param.name) {
            let binding_field = config.binding_field_name_owned(&typ.name, &field.name);
            // Use standard field conversion (owned — no lifetime constraint needed here).
            let expr = field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config);
            // Strip "name: " prefix to obtain just the value expression.
            let expr = if let Some(e) = expr.strip_prefix(&format!("{}: ", field.name)) {
                e.to_string()
            } else {
                expr
            };
            // Apply keyword-escaped field name substitution.
            let expr = if binding_field != field.name {
                expr.replace(&format!("val.{}", field.name), &format!("val.{binding_field}"))
            } else {
                expr
            };
            args.push(expr);
        } else {
            // No matching binding field — use Default or empty collection.
            match &param.ty {
                TypeRef::Map(_, _) => args.push("Default::default()".to_string()),
                _ => {
                    if param.is_ref {
                        args.push("&Default::default()".to_string());
                    } else {
                        args.push("Default::default()".to_string());
                    }
                }
            }
        }
    }

    let args_str = args.join(",\n        ");
    Some(format!(
        "#[allow(clippy::redundant_closure, clippy::useless_conversion)]\n\
         impl From<{binding_name}> for {core_path} {{\n\
             fn from(val: {binding_name}) -> Self {{\n\
                 Self::{constructor_name}(\n\
                     {args_str},\n\
                 )\n\
             }}\n\
         }}\n",
        constructor_name = constructor.name,
    ))
}

/// Generate a `From<Binding> for CoreType<'_>` impl using a static constructor method.
///
/// For types with `has_lifetime_params=true`, struct-literal construction is forbidden
/// (private fields). This function locates a static method (no receiver) in `typ.methods`
/// whose parameters are a superset of the type's binding fields, then emits a call to that
/// constructor, using field conversion expressions for params that match a binding field and
/// `Default::default()` for any extra params not present in the binding struct.
///
/// Returns `None` when no suitable constructor is found.
pub fn gen_from_lifetime_type_constructor(
    typ: &TypeDef,
    core_path: &str,
    binding_name: &str,
    config: &ConversionConfig,
) -> Option<String> {
    // Field names present in the binding struct.
    let field_names: std::collections::HashSet<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| f.name.as_str())
        .collect();

    // Find a static method whose params include all binding field names.
    // Prefer with_owned_* methods over with_borrowed_* because the From impl
    // cannot provide the lifetime required by borrowed variants (temporaries can't be borrowed).
    let constructor = typ
        .methods
        .iter()
        .find(|m| {
            // Must be static (no receiver).
            m.receiver.is_none()
            // All binding fields must appear as a param.
            && field_names.iter().all(|fname| m.params.iter().any(|p| p.name == *fname))
            // Prefer owned variants over borrowed for From impl context
            && !m.name.contains("borrowed")
        })
        .or_else(|| {
            // Fallback: accept borrowed variants if no owned variant exists
            typ.methods.iter().find(|m| {
                m.receiver.is_none()
                    && field_names
                        .iter()
                        .all(|fname| m.params.iter().any(|p| p.name == *fname))
            })
        })?;

    // Build the argument list in param order.
    let mut args: Vec<String> = Vec::new();
    for param in &constructor.params {
        if let Some(field) = typ.fields.iter().find(|f| f.name == param.name) {
            // Binding field exists — generate conversion expression.
            let binding_field = config.binding_field_name_owned(&typ.name, &field.name);
            let expr = match &field.ty {
                TypeRef::String if matches!(field.core_wrapper, CoreWrapper::Cow | CoreWrapper::Box) => {
                    if field.optional {
                        format!("val.{binding_field}.map(Into::into)")
                    } else {
                        format!("val.{binding_field}.into()")
                    }
                }
                TypeRef::Map(_k, _v) => {
                    // Map fields: convert HashMap to BTreeMap (owned, since From impl context can't provide lifetime)
                    format!(
                        "val.{binding_field}.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<std::collections::BTreeMap<_, _>>()"
                    )
                }
                TypeRef::Named(type_name) => {
                    // When the binding stores the enum as a String (PHP enum_string_names),
                    // use serde_json deserialization to convert String → Enum.
                    // The core→binding path serialises via `serde_json::to_value(enum_val)`
                    // which yields `Value::String("VariantName")`. Reverse with
                    // `from_value(Value::String(...))` rather than `from_str` (which would
                    // require a JSON-quoted string like `"\"VariantName\"`). We use
                    // `.expect(...)` because an unrecognised variant name indicates a bug
                    // in the calling code — there is no safe fallback and the enum may not
                    // implement Default.
                    let is_enum_string = config
                        .enum_string_names
                        .is_some_and(|names| names.contains(type_name.as_str()));
                    if is_enum_string {
                        if field.optional {
                            format!(
                                "val.{binding_field}.map(|s| serde_json::from_value(serde_json::Value::String(s)).expect(\"valid {type_name}\"))"
                            )
                        } else {
                            format!(
                                "serde_json::from_value(serde_json::Value::String(val.{binding_field}.clone())).expect(\"valid {type_name}\")"
                            )
                        }
                    } else if field.optional {
                        format!("val.{binding_field}.map(Into::into)")
                    } else {
                        format!("val.{binding_field}.into()")
                    }
                }
                TypeRef::Primitive(p) => {
                    // When the binding stores the value as a remapped primitive (i64 in
                    // NAPI/PHP, f64 in extendr/R, i32 in extendr for u32), cast back to the
                    // core type (e.g. usize) when constructing the core value. Without the
                    // cast, the From impl emits e.g. `val.depth` of type `f64` into a `usize`
                    // parameter, producing an E0308 type mismatch.
                    let needs_cast = (config.cast_large_ints_to_i64 && needs_i64_cast(p))
                        || (config.cast_large_ints_to_f64 && needs_f64_cast(p))
                        || (config.cast_uints_to_i32 && needs_i32_cast(p));
                    if needs_cast {
                        let core_ty = core_prim_str(p);
                        format!("val.{binding_field} as {core_ty}")
                    } else {
                        format!("val.{binding_field}")
                    }
                }
                TypeRef::String | TypeRef::Unit => {
                    format!("val.{binding_field}")
                }
                TypeRef::Optional(_) => format!("val.{binding_field}.map(Into::into)"),
                _ => format!("val.{binding_field}.into()"),
            };
            args.push(expr);
        } else {
            // No binding field for this param — use Default::default() or empty collection
            match &param.ty {
                TypeRef::Map(_, _) => {
                    // For Map parameters with no binding field, create an empty BTreeMap
                    args.push("std::collections::BTreeMap::new()".to_string());
                }
                _ => {
                    if param.is_ref {
                        args.push("&Default::default()".to_string());
                    } else {
                        args.push("Default::default()".to_string());
                    }
                }
            }
        }
    }

    let args_str = args.join(",\n        ");
    let code = format!(
        "#[allow(clippy::redundant_closure, clippy::useless_conversion)]\n\
         impl From<{binding_name}> for {core_path}<'_> {{\n\
             fn from(val: {binding_name}) -> Self {{\n\
                 {core_path}::{constructor_name}(\n\
                     {args_str},\n\
                 )\n\
             }}\n\
         }}\n",
        constructor_name = constructor.name,
    );
    Some(code)
}
