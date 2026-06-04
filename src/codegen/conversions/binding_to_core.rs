use crate::core::ir::{CoreWrapper, PrimitiveType, TypeDef, TypeRef};

use super::ConversionConfig;
use super::helpers::{
    core_prim_str, core_type_path_remapped, is_newtype, is_tuple_type_name, needs_f64_cast, needs_i32_cast,
    needs_i64_cast,
};

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
            if !config.exclude_types.is_empty()
                && super::helpers::field_references_excluded_type(&field.ty, config.exclude_types)
            {
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

    for field in &typ.fields {
        if field.binding_excluded {
            if field.cfg.is_some()
                && !config.never_skip_cfg_field_names.contains(&field.name)
                && (typ.has_stripped_cfg_fields || config.strip_cfg_fields_from_binding_struct)
            {
                continue;
            }
            fields.push(format!("{}: Default::default()", field.name));
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
        let references_excluded = !config.exclude_types.is_empty()
            && super::helpers::field_references_excluded_type(&field.ty, config.exclude_types);
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
            has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
            statements => statements,
            fields => fields,
        },
    )
}

/// Determine the field conversion expression for binding -> core.
pub fn field_conversion_to_core(name: &str, ty: &TypeRef, optional: bool) -> String {
    match ty {
        // Primitives, String, Unit -- direct assignment
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Unit => {
            format!("{name}: val.{name}")
        }
        // Bytes: binding may use Vec<u8> or napi `Buffer`; core uses `bytes::Bytes`
        // (or `Vec<u8>` for some targets). `.to_vec().into()` works in all cases:
        // Buffer → Vec<u8> via `From<Buffer> for Vec<u8>`, then `Vec<u8> → Bytes`
        // via `From<Vec<u8>> for Bytes` (or identity From for Vec<u8>→Vec<u8>).
        TypeRef::Bytes => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_vec().into())")
            } else {
                format!("{name}: val.{name}.to_vec().into()")
            }
        }
        // Json: binding uses String, core uses serde_json::Value — parse or default
        TypeRef::Json => {
            if optional {
                format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())")
            } else {
                format!("{name}: serde_json::from_str(&val.{name}).unwrap_or_default()")
            }
        }
        // Char: binding uses String, core uses char — convert first character
        TypeRef::Char => {
            if optional {
                format!("{name}: val.{name}.and_then(|s| s.chars().next())")
            } else {
                format!("{name}: val.{name}.chars().next().unwrap_or('*')")
            }
        }
        // Duration: binding uses u64 (millis), core uses std::time::Duration
        TypeRef::Duration => {
            if optional {
                format!("{name}: val.{name}.map(std::time::Duration::from_millis)")
            } else {
                format!("{name}: std::time::Duration::from_millis(val.{name})")
            }
        }
        // Path needs .into() — binding uses String, core uses PathBuf
        TypeRef::Path => {
            if optional {
                format!("{name}: val.{name}.map(Into::into)")
            } else {
                format!("{name}: val.{name}.into()")
            }
        }
        // Named type -- needs .into() to convert between binding and core types
        // Tuple types (e.g., "(String, String)") are passthrough — no conversion needed
        TypeRef::Named(type_name) if is_tuple_type_name(type_name) => {
            format!("{name}: val.{name}")
        }
        TypeRef::Named(_) => {
            if optional {
                format!("{name}: val.{name}.map(Into::into)")
            } else {
                format!("{name}: val.{name}.into()")
            }
        }
        // Map with Json value type: binding uses HashMap<K, String>, core uses HashMap<K, Value>.
        // Use `k.into()` for non-Json keys so String→String is a no-op while still converting
        // String→Cow<'_, str>/Box<str>/Arc<str> when the core type uses one of those wrappers.
        TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Json) => {
            let k_expr = if matches!(k.as_ref(), TypeRef::Json) {
                "serde_json::from_str(&k).unwrap_or_default()"
            } else {
                "k.into()"
            };
            if optional {
                format!(
                    "{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({k_expr}, serde_json::from_str(&v).unwrap_or_default())).collect())"
                )
            } else {
                format!(
                    "{name}: val.{name}.into_iter().map(|(k, v)| ({k_expr}, serde_json::from_str(&v).unwrap_or_default())).collect()"
                )
            }
        }
        // Map<K, Bytes>: binding uses Vec<u8> or napi Buffer, core uses bytes::Bytes (or Vec<u8>).
        // `.to_vec().into()` converts Buffer→Vec<u8> (napi) or is identity for Vec<u8>→Vec<u8>.
        TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Bytes) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect()")
            }
        }
        // Optional with inner
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Json => format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())"),
            TypeRef::Named(_) | TypeRef::Path => format!("{name}: val.{name}.map(Into::into)"),
            TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
            }
            TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Json) => {
                let k_expr = if matches!(k.as_ref(), TypeRef::Json) {
                    "serde_json::from_str(&k).unwrap_or_default()"
                } else {
                    "k.into()"
                };
                format!(
                    "{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({k_expr}, serde_json::from_str(&v).unwrap_or_default())).collect())"
                )
            }
            // Optional<Vec<Primitive/String/Bytes>>: the core type may be a Set.
            // Use .into_iter().collect() for Set→Vec conversion compatibility.
            TypeRef::Vec(_) => {
                format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
            }
            _ => format!("{name}: val.{name}"),
        },
        // Vec of named or Json types -- map each element
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Json => {
                if optional {
                    format!(
                        "{name}: val.{name}.map(|v| v.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect())"
                    )
                } else {
                    format!("{name}: val.{name}.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect()")
                }
            }
            // Vec<(T1, T2)> — tuples are passthrough
            TypeRef::Named(type_name) if is_tuple_type_name(type_name) => {
                format!("{name}: val.{name}")
            }
            TypeRef::Named(_) => {
                if optional {
                    format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(Into::into).collect()")
                }
            }
            // Vec<Primitive>, Vec<String>, Vec<Bytes>, etc.
            // The core type may be a Set (HashSet, AHashSet, BTreeSet, etc.) which the type resolver
            // maps to Vec in the IR. Emit .into_iter().collect() which works for both Vec→Vec (identity)
            // and Vec→Set (convert ordered collection to uniqueness-guaranteed set) conversions.
            _ => {
                if optional {
                    format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().collect()")
                }
            }
        },
        // Map -- collect to handle HashMap↔BTreeMap conversion;
        // additionally convert Named keys/values via Into, Json values via serde.
        TypeRef::Map(k, v) => {
            let has_named_key = matches!(k.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n));
            let has_named_val = matches!(v.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n));
            let has_json_val = matches!(v.as_ref(), TypeRef::Json);
            let has_json_key = matches!(k.as_ref(), TypeRef::Json);
            // Vec<Named> values: each vector element needs Into conversion.
            let has_vec_named_val = matches!(v.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n)));
            // Vec<Json> values: each element needs serde deserialization.
            let has_vec_json_val = matches!(v.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Json));
            if has_json_val || has_json_key || has_named_key || has_named_val || has_vec_named_val || has_vec_json_val {
                // `k.into()` is a no-op for `String`→`String` and the canonical conversion for
                // wrapped string keys (`Cow`, `Box<str>`, `Arc<str>`) which the type resolver
                // collapses to `TypeRef::String`.
                let k_expr = if has_json_key {
                    "serde_json::from_str(&k).unwrap_or(serde_json::Value::String(k))"
                } else {
                    "k.into()"
                };
                let v_expr = if has_json_val {
                    "serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v))"
                } else if has_named_val {
                    "v.into()"
                } else if has_vec_named_val {
                    "v.into_iter().map(Into::into).collect()"
                } else if has_vec_json_val {
                    "v.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect()"
                } else {
                    "v"
                };
                if optional {
                    format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({k_expr}, {v_expr})).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|(k, v)| ({k_expr}, {v_expr})).collect()")
                }
            } else {
                // Map<String, String>: binding may have String keys/values, core may have Box<str>/Cow<str>.
                // Emit .map(|(k, v)| (k.into(), v.into())) which is a no-op when both sides are String.
                // This handles cases like HashMap<String, String> (binding) → HashMap<Box<str>, Box<str>> (core).
                let is_string_map = matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String);
                if is_string_map {
                    if optional {
                        format!(
                            "{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())"
                        )
                    } else {
                        format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
                    }
                } else {
                    // No conversion needed for keys/values — just collect for potential
                    // HashMap↔BTreeMap type change. Still apply per-value .into() when the value
                    // type is a Named wrapper that requires conversion (e.g. a binding-side newtype).
                    if optional {
                        if has_named_val {
                            format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())")
                        } else {
                            format!("{name}: val.{name}.map(|m| m.into_iter().collect())")
                        }
                    } else {
                        format!("{name}: val.{name}.into_iter().collect()")
                    }
                }
            }
        }
    }
}

/// Binding→core field conversion with backend-specific config (i64 casts, etc.).
pub fn field_conversion_to_core_cfg(name: &str, ty: &TypeRef, optional: bool, config: &ConversionConfig) -> String {
    // When optional=true and ty=Optional(T), the binding field was flattened from
    // Option<Option<T>> to Option<T>. Core expects Option<Option<T>>, so wrap with .map(Some).
    // This applies regardless of cast config; handle before any other dispatch.
    if optional && matches!(ty, TypeRef::Optional(_)) {
        // Delegate to get the inner Optional(T) → Option<T> conversion (with optional=false,
        // since the outer Option is handled by the .map(Some) we add here).
        let inner_expr = field_conversion_to_core_cfg(name, ty, false, config);
        // inner_expr is "name: <expr-for-Option<T>>"; wrap it with .map(Some)
        if let Some(expr) = inner_expr.strip_prefix(&format!("{name}: ")) {
            return format!("{name}: ({expr}).map(Some)");
        }
        return inner_expr;
    }

    // WASM JsValue: use serde_wasm_bindgen for Map, nested Vec, and Vec<Json> types
    if config.map_uses_jsvalue {
        let is_nested_vec = matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)));
        let is_vec_json = matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Json));
        let is_map = matches!(ty, TypeRef::Map(_, _));
        if is_nested_vec || is_map || is_vec_json {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                );
            }
            return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
        }
        if let TypeRef::Optional(inner) = ty {
            let is_inner_nested = matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Vec(_)));
            let is_inner_vec_json = matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Json));
            let is_inner_map = matches!(inner.as_ref(), TypeRef::Map(_, _));
            if is_inner_nested || is_inner_map || is_inner_vec_json {
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                );
            }
        }
    }

    // Vec<Named>→String binding→core: binding holds JSON string, core expects Vec<Named>.
    // Only apply serde round-trip for Vec<Named> types (complex structs that can't cross FFI).
    // Vec<String>, Vec<Primitive>, etc. stay as-is since they map directly.
    if config.vec_named_to_string {
        if let TypeRef::Vec(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Named(_)) {
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())");
                }
                return format!("{name}: serde_json::from_str(&val.{name}).unwrap_or_default()");
            }
        }
    }
    // Map→String binding→core: use Default::default() (lossy — can't reconstruct HashMap from Debug string)
    if config.map_as_string && matches!(ty, TypeRef::Map(_, _)) {
        return format!("{name}: Default::default()");
    }
    if config.map_as_string {
        if let TypeRef::Optional(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Map(_, _)) {
                return format!("{name}: Default::default()");
            }
        }
    }
    // Tagged-data enum field (WASM only; binding holds JsValue / Option<JsValue>, core holds the
    // typed enum). Handles bare Named, Option<Named>, Vec<Named>, and Option<Vec<Named>> shapes.
    // All four shapes are stored as JsValue or Option<JsValue> in the binding struct so that
    // callers can pass plain JS objects without constructing explicit wasm-bindgen class instances.
    if config.map_uses_jsvalue {
        if let Some(tagged_names) = config.tagged_data_enum_names {
            let bare_named = matches!(ty, TypeRef::Named(n) if tagged_names.contains(n));
            let optional_named = matches!(ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_names.contains(n)));
            let vec_named = matches!(ty, TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_names.contains(n)));
            let optional_vec_named = matches!(ty, TypeRef::Optional(outer)
                if matches!(outer.as_ref(), TypeRef::Vec(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_names.contains(n))));
            if bare_named {
                if optional {
                    // Optional bare TaggedDataEnum (field.optional=true, ty=Named): binding holds
                    // Option<JsValue>; core expects Option<T>.
                    return format!(
                        "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                    );
                }
                // Required bare TaggedDataEnum stored as JsValue: deserialize directly.
                return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
            }
            if optional_named {
                // Option<TaggedDataEnum> (ty=Optional(Named)) stored as Option<JsValue>: deserialize when Some.
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                );
            }
            if vec_named {
                return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
            }
            if optional_vec_named {
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                );
            }
        }
    }

    // Untagged data enum field (binding holds serde_json::Value, core holds the typed enum):
    // convert via serde_json::from_value.  Handles direct, Optional, and Vec wrappings.
    if let Some(untagged_names) = config.untagged_data_enum_names {
        let direct_named = matches!(ty, TypeRef::Named(n) if untagged_names.contains(n));
        let optional_named = matches!(ty, TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n) if untagged_names.contains(n)));
        let vec_named = matches!(ty, TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n) if untagged_names.contains(n)));
        let optional_vec_named = matches!(ty, TypeRef::Optional(outer)
            if matches!(outer.as_ref(), TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if untagged_names.contains(n))));
        if direct_named {
            if optional {
                return format!("{name}: val.{name}.and_then(|v| serde_json::from_value(v).ok())");
            }
            return format!("{name}: serde_json::from_value(val.{name}).unwrap_or_default()");
        }
        if optional_named {
            return format!("{name}: val.{name}.and_then(|v| serde_json::from_value(v).ok())");
        }
        if vec_named {
            if optional {
                return format!(
                    "{name}: val.{name}.map(|v| v.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect())"
                );
            }
            return format!("{name}: val.{name}.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect()");
        }
        if optional_vec_named {
            return format!(
                "{name}: val.{name}.map(|v| v.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect())"
            );
        }
    }
    // Json→String binding→core: use Default::default() (lossy — can't parse String back)
    if config.json_to_string && matches!(ty, TypeRef::Json) {
        return format!("{name}: Default::default()");
    }
    // Json stays as serde_json::Value: identity passthrough.
    if config.json_as_value && matches!(ty, TypeRef::Json) {
        return format!("{name}: val.{name}");
    }
    if config.json_as_value {
        if let TypeRef::Optional(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Json) {
                return format!("{name}: val.{name}");
            }
        }
        if let TypeRef::Vec(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Json) {
                if optional {
                    return format!("{name}: val.{name}.unwrap_or_default()");
                }
                return format!("{name}: val.{name}");
            }
        }
        if let TypeRef::Map(_k, v) = ty {
            if matches!(v.as_ref(), TypeRef::Json) {
                if optional {
                    return format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), v)).collect())");
                }
                return format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.into(), v)).collect()");
            }
        }
    }
    // Json→JsValue binding→core: use serde_wasm_bindgen to convert (WASM)
    if config.map_uses_jsvalue && matches!(ty, TypeRef::Json) {
        if optional {
            return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())");
        }
        return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
    }
    if !config.cast_large_ints_to_i64
        && !config.cast_large_ints_to_f64
        && !config.cast_uints_to_i32
        && !config.cast_f32_to_f64
        && !config.json_to_string
        && !config.vec_named_to_string
        && !config.map_as_string
        && config.from_binding_skip_types.is_empty()
    {
        return field_conversion_to_core(name, ty, optional);
    }
    // Cast mode: handle primitives and Duration differently
    match ty {
        TypeRef::Primitive(p) if config.cast_large_ints_to_i64 && needs_i64_cast(p) => {
            let core_ty = core_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                format!("{name}: val.{name} as {core_ty}")
            }
        }
        // f64→f32 cast (NAPI binding f64 → core f32)
        TypeRef::Primitive(PrimitiveType::F32) if config.cast_f32_to_f64 => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as f32)")
            } else {
                format!("{name}: val.{name} as f32")
            }
        }
        TypeRef::Duration if config.cast_large_ints_to_i64 => {
            if optional {
                format!("{name}: val.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
            } else {
                format!("{name}: std::time::Duration::from_millis(val.{name} as u64)")
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) => {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // Vec<u64/usize/isize> needs element-wise i64→core casting
        TypeRef::Vec(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                if optional {
                    format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|v| v as {core_ty}).collect()")
                }
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // HashMap value type casting: when value type needs i64→core casting
        TypeRef::Map(_k, v)
            if config.cast_large_ints_to_i64 && matches!(v.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = v.as_ref() {
                let core_ty = core_prim_str(p);
                if optional {
                    format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v as {core_ty})).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v as {core_ty})).collect()")
                }
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // Vec<f32> needs element-wise cast when f32→f64 mapping is active (NAPI)
        TypeRef::Vec(inner)
            if config.cast_f32_to_f64 && matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::F32)) =>
        {
            if optional {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as f32).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|v| v as f32).collect()")
            }
        }
        // Optional(Vec(f32)) needs element-wise cast (NAPI only)
        TypeRef::Optional(inner)
            if config.cast_f32_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(PrimitiveType::F32))) =>
        {
            format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as f32).collect())")
        }
        // i32→u8/u16/u32/i8/i16 casts (extendr — R maps small ints to i32)
        TypeRef::Primitive(p) if config.cast_uints_to_i32 && needs_i32_cast(p) => {
            let core_ty = core_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                format!("{name}: val.{name} as {core_ty}")
            }
        }
        // Optional(i32-needs-cast) with cast_uints_to_i32
        TypeRef::Optional(inner)
            if config.cast_uints_to_i32 && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i32_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // Vec<u8/u16/u32/i8/i16> needs element-wise i32→core casting
        TypeRef::Vec(inner)
            if config.cast_uints_to_i32 && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i32_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                if optional {
                    format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|v| v as {core_ty}).collect()")
                }
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // f64→u64/usize/isize casts (extendr — R maps large ints to f64)
        TypeRef::Primitive(p) if config.cast_large_ints_to_f64 && needs_f64_cast(p) => {
            let core_ty = core_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                format!("{name}: val.{name} as {core_ty}")
            }
        }
        // Optional(f64-needs-cast) with cast_large_ints_to_f64
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // Vec<u64/usize/isize> needs element-wise f64→core casting
        TypeRef::Vec(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                if optional {
                    format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|v| v as {core_ty}).collect()")
                }
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // Map<K, usize/u64/i64/isize/f32> needs value-wise f64→core casting (extendr)
        TypeRef::Map(_k, v)
            if config.cast_large_ints_to_f64 && matches!(v.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = v.as_ref() {
                let core_ty = core_prim_str(p);
                if optional {
                    format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v as {core_ty})).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v as {core_ty})).collect()")
                }
            } else {
                field_conversion_to_core(name, ty, optional)
            }
        }
        // Skip-type: Named types that can't be auto-converted via Into in the binding→core From
        // impl (e.g. PHP VisitorHandle which is handled separately by bridge machinery).
        TypeRef::Named(n) if config.from_binding_skip_types.iter().any(|s| s == n) => {
            format!("{name}: Default::default()")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if config.from_binding_skip_types.iter().any(|s| s == n) => {
                format!("{name}: Default::default()")
            }
            _ => field_conversion_to_core(name, ty, optional),
        },
        // Fall through to default for everything else
        _ => field_conversion_to_core(name, ty, optional),
    }
}

/// Apply CoreWrapper transformations to a binding→core conversion expression.
/// Wraps the value expression with Arc::new(), .into() for Cow, etc.
pub fn apply_core_wrapper_to_core(
    conversion: &str,
    name: &str,
    core_wrapper: &CoreWrapper,
    vec_inner_core_wrapper: &CoreWrapper,
    optional: bool,
) -> String {
    // Handle Vec<Arc<T>>: replace .map(Into::into) with .map(|v| std::sync::Arc::new(v.into()))
    if *vec_inner_core_wrapper == CoreWrapper::Arc {
        return conversion
            .replace(
                ".map(Into::into).collect()",
                ".map(|v| std::sync::Arc::new(v.into())).collect()",
            )
            .replace(
                "map(|v| v.into_iter().map(Into::into)",
                "map(|v| v.into_iter().map(|v| std::sync::Arc::new(v.into()))",
            );
    }

    match core_wrapper {
        CoreWrapper::None => conversion.to_string(),
        CoreWrapper::Cow | CoreWrapper::Box => {
            // Cow<str> / Box<str>: binding String → core wrapper via .into().
            // Both wrappers have the same conversion shape — binding is `String`
            // and core is `Cow<'_, str>` or `Box<str>`, so `String -> wrapper`
            // goes through the same `.into()` path. The field_conversion already
            // emits "name: val.name" for strings; we add .into() to wrap.
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(Into::into)")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.into()")
                } else if expr == "Default::default()" {
                    // Sanitized field: Default::default() already resolves to the correct core type
                    // (e.g. Cow<'static, str> — adding .into() breaks type inference).
                    conversion.to_string()
                } else {
                    format!("{name}: ({expr}).into()")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Arc => {
            // Arc<T>: wrap with Arc::new()
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if expr == "Default::default()" {
                    // Sanitized field: Default::default() resolves to the correct core type;
                    // wrapping in Arc::new() would change the type.
                    conversion.to_string()
                } else if optional {
                    format!("{name}: {expr}.map(|v| std::sync::Arc::new(v))")
                } else {
                    format!("{name}: std::sync::Arc::new({expr})")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Bytes => {
            // Bytes: binding Vec<u8> → core bytes::Bytes via .into().
            // When TypeRef::Bytes already emitted a conversion (e.g. `val.{name}.into()` or
            // `val.{name}.map(Into::into)`), applying another .into() creates an ambiguous
            // double-into chain. Detect and dedup: use the already-generated expression as-is
            // when it fully covers the conversion, or emit a fresh single .into() for bare fields.
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                let already_converted_non_opt =
                    expr == format!("val.{name}.into()") || expr == format!("val.{name}.to_vec().into()");
                let already_converted_opt = expr
                    .strip_prefix(&format!("val.{name}"))
                    .map(|s| s == ".map(Into::into)" || s == ".map(|v| v.to_vec().into())")
                    .unwrap_or(false);
                if already_converted_non_opt || already_converted_opt {
                    // The base conversion already handles Bytes — pass through unchanged.
                    conversion.to_string()
                } else if optional {
                    format!("{name}: {expr}.map(Into::into)")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.into()")
                } else if expr == "Default::default()" {
                    // Sanitized field: Default::default() already resolves to the correct core type
                    // (e.g. bytes::Bytes — adding .into() breaks type inference).
                    conversion.to_string()
                } else {
                    format!("{name}: ({expr}).into()")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::ArcMutex => {
            // ArcMutex: binding T → core Arc<Mutex<T>> via Arc::new(Mutex::new())
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(|v| std::sync::Arc::new(std::sync::Mutex::new(v.into())))")
                } else if expr == format!("val.{name}") {
                    format!("{name}: std::sync::Arc::new(std::sync::Mutex::new(val.{name}.into()))")
                } else {
                    format!("{name}: std::sync::Arc::new(std::sync::Mutex::new(({expr}).into()))")
                }
            } else {
                conversion.to_string()
            }
        }
    }
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
                TypeRef::Named(_) => {
                    if field.optional {
                        format!("val.{binding_field}.map(Into::into)")
                    } else {
                        format!("val.{binding_field}.into()")
                    }
                }
                TypeRef::Primitive(p) => {
                    // When cast_large_ints_to_i64 is active (NAPI/PHP), the binding field
                    // stores the value as i64. Cast back to the core type (e.g. usize).
                    if config.cast_large_ints_to_i64 && super::helpers::needs_i64_cast(p) {
                        let core_ty = super::helpers::core_prim_str(p);
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

#[cfg(test)]
mod tests {
    use super::gen_from_binding_to_core;
    use super::gen_from_binding_to_core_cfg;
    use crate::codegen::conversions::ConversionConfig;
    use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, TypeDef, TypeRef};
    use ahash::AHashSet;

    fn type_with_field(field: FieldDef) -> TypeDef {
        TypeDef {
            name: "ProcessConfig".to_string(),
            rust_path: "crate::ProcessConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![field],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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
        }
    }

    #[test]
    fn sanitized_cow_string_field_converts_to_core() {
        let field = FieldDef {
            name: "language".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: true,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::Empty),
            core_wrapper: CoreWrapper::Cow,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        };

        let out = gen_from_binding_to_core(&type_with_field(field), "crate");

        assert!(out.contains("language: val.language.into()"));
        assert!(!out.contains("language: Default::default()"));
    }

    #[test]
    fn binding_excluded_cfg_field_is_not_emitted_into_core_literal() {
        let field = FieldDef {
            name: "di_container".to_string(),
            ty: TypeRef::String,
            optional: true,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: Some("feature = \"di\"".to_string()),
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: true,
            binding_exclusion_reason: Some("internal implementation detail".to_string()),
            original_type: None,
        };
        let mut typ = type_with_field(field);
        typ.has_stripped_cfg_fields = true;

        let out = gen_from_binding_to_core(&typ, "crate");

        assert!(
            !out.contains("di_container:"),
            "cfg-gated binding-excluded fields may not exist in the core struct; got:\n{out}"
        );
        assert!(
            out.contains("..Default::default()"),
            "stripped cfg fields should be filled by the default update; got:\n{out}"
        );
    }

    /// Trait-bridge OptionsField field with Arc wrapper: the binding→core From impl must
    /// emit `val.visitor.map(|v| (*v.inner).clone())` and must NOT fall back to
    /// `visitor: Default::default()`, which would silently drop the visitor handle.
    #[test]
    fn trait_bridge_arc_wrapper_field_forwards_value_not_default() {
        let opaque_type_name = "VisitorHandle".to_string();
        let mut opaque_set = AHashSet::new();
        opaque_set.insert(opaque_type_name.clone());

        let field = FieldDef {
            name: "visitor".to_string(),
            ty: TypeRef::Named(opaque_type_name.clone()),
            optional: true,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: Some("feature = \"visitor\"".to_string()),
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        };

        let never_skip = vec!["visitor".to_string()];
        let arc_wrapper = vec!["visitor".to_string()];

        let config = ConversionConfig {
            opaque_types: Some(&opaque_set),
            never_skip_cfg_field_names: &never_skip,
            trait_bridge_arc_wrapper_field_names: &arc_wrapper,
            ..ConversionConfig::default()
        };

        let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

        assert!(
            out.contains("val.visitor.map(|v| (*v.inner).clone())"),
            "expected arc-wrapper clone forwarding, got:\n{out}"
        );
        assert!(
            !out.contains("visitor: Default::default()"),
            "must not emit Default::default() for arc-wrapper trait-bridge field, got:\n{out}"
        );
    }

    /// When `trait_bridge_arc_wrapper_field_names` is empty (default), the old
    /// `Default::default()` fallback is preserved for opaque-no-wrapper fields.
    #[test]
    fn opaque_no_wrapper_field_without_arc_flag_emits_default() {
        let opaque_type_name = "OpaqueHandle".to_string();
        let mut opaque_set = AHashSet::new();
        opaque_set.insert(opaque_type_name.clone());

        let field = FieldDef {
            name: "handle".to_string(),
            ty: TypeRef::Named(opaque_type_name.clone()),
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
        };

        let config = ConversionConfig {
            opaque_types: Some(&opaque_set),
            // trait_bridge_arc_wrapper_field_names left empty (default)
            ..ConversionConfig::default()
        };

        let out = gen_from_binding_to_core_cfg(&type_with_field(field), "crate", &config);

        assert!(
            out.contains("handle: Default::default()"),
            "expected Default::default() for non-arc-wrapper opaque field, got:\n{out}"
        );
        assert!(
            !out.contains("(*val.handle.inner).clone()"),
            "must not emit arc-clone for non-arc-wrapper opaque field, got:\n{out}"
        );
    }
}
