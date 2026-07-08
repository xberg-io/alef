use crate::codegen::conversions::helpers::{core_prim_str, needs_f64_cast, needs_i32_cast};
use crate::core::ir::{CoreWrapper, TypeDef, TypeRef};
use ahash::AHashSet;

/// Generate a lossy binding→core struct literal for non-opaque delegation.
/// Sanitized fields use `Default::default()`, non-sanitized fields are cloned and converted.
/// Fields are accessed via `self.` (behind &self), so all non-Copy types need `.clone()`.
///
/// `opaque_types` is the set of opaque type names (Arc-wrapped handles, trait bridge aliases,
/// etc.). Fields whose `TypeRef::Named` type is in this set have no `From` impl in the binding
/// layer, so `Default::default()` is emitted for them instead of `.clone().into()`.
///
/// NOTE: This assumes all binding struct fields implement Clone. If a field type does not
/// implement Clone (e.g., `Mutex<T>`), it should be marked as `sanitized=true` so that
/// `Default::default()` is used instead of calling `.clone()`. Backends that exclude types
/// should mark such fields appropriately.
pub fn gen_lossy_binding_to_core_fields(
    typ: &TypeDef,
    core_import: &str,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
    skip_types: &[String],
) -> String {
    gen_lossy_binding_to_core_fields_inner(
        typ,
        core_import,
        false,
        option_duration_on_defaults,
        opaque_types,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
        skip_types,
    )
}

/// Same as `gen_lossy_binding_to_core_fields` but declares `core_self` as mutable.
pub fn gen_lossy_binding_to_core_fields_mut(
    typ: &TypeDef,
    core_import: &str,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
    skip_types: &[String],
) -> String {
    gen_lossy_binding_to_core_fields_inner(
        typ,
        core_import,
        true,
        option_duration_on_defaults,
        opaque_types,
        cast_uints_to_i32,
        cast_large_ints_to_f64,
        skip_types,
    )
}

#[allow(clippy::too_many_arguments)]
fn gen_lossy_binding_to_core_fields_inner(
    typ: &TypeDef,
    core_import: &str,
    needs_mut: bool,
    option_duration_on_defaults: bool,
    opaque_types: &AHashSet<String>,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
    skip_types: &[String],
) -> String {
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let mut_kw = if needs_mut { "mut " } else { "" };

    // Types with lifetime parameters (e.g. `NodeContext<'a>`) have private fields that make
    // struct-literal construction impossible. Delegate to the `From` impl (generated separately
    // via `gen_from_binding_to_core_cfg`) which uses the appropriate constructor.
    // The `mut` qualifier is not needed here because method bodies call immutable methods on
    // `core_self` and use `into_owned()` for the owned-receiver case.
    if typ.has_lifetime_params {
        return format!("let {mut_kw}core_self = {core_path}::from(self.clone());\n        ");
    }

    // The struct literal ends with ..Default::default() whenever the trailer can
    // compile (see the emission condition at the bottom). Suppress
    // clippy::needless_update because the trailer is intentionally emitted even
    // when the field list looks complete — clippy would flag the spread as
    // redundant on a fully-mirrored literal.
    let allow = if typ.has_stripped_cfg_fields || typ.has_default {
        "#[allow(clippy::needless_update)]\n        "
    } else {
        ""
    };
    let mut out = format!("{allow}let {mut_kw}core_self = {core_path} {{\n");
    // When the core type does NOT implement Default, the `..Default::default()`
    // trailer would not compile (E0277), so emit `field: Default::default()`
    // per-field for binding-excluded fields instead — there is no bespoke core
    // Default whose semantics we could be bypassing. Mirrors the parallel logic
    // in `codegen/conversions/binding_to_core/render.rs`.
    let core_has_default = typ.has_default;
    for field in &typ.fields {
        if field.binding_excluded {
            if !core_has_default {
                // Core type has no Default — emit per-field fallback so the method
                // body compiles. For example, a struct can carry a binding-excluded
                // field whose type does not implement Default, while the struct itself
                // only derives Clone/Debug/Serialize/Deserialize.
                out.push_str(&crate::codegen::template_env::render(
                    "binding_helpers/struct_field_default.jinja",
                    minijinja::context! {
                        name => &field.name,
                    },
                ));
                out.push('\n');
                continue;
            }
            // Skip binding_excluded fields entirely; the trailing `..Default::default()`
            // spread fills them with the CORE type's Default impl, preserving custom
            // defaults that derive field values from environment or runtime configuration.
            // Emitting `<field>: Default::default()` would shadow that with the sub-type's
            // (often stricter) default value.
            continue;
        }
        // Skip cfg-gated fields — they are absent from the binding struct.
        // The ..Default::default() spread below fills them when the feature is enabled.
        if field.cfg.is_some() {
            continue;
        }
        let name = &field.name;
        if field.sanitized && field.core_wrapper != CoreWrapper::Cow {
            out.push_str(&crate::codegen::template_env::render(
                "binding_helpers/struct_field_default.jinja",
                minijinja::context! {
                    name => &field.name,
                },
            ));
            out.push('\n');
            continue;
        }
        // Opaque-type fields (Arc-wrapped handles, trait bridge aliases) have no From impl
        // in the binding layer. Emit Default::default() so the apply_update / clone-mutate
        // paths compile without needing From<Arc<Py<PyAny>>> for VisitorHandle, etc.
        // This covers both bare Named opaque fields and Optional<Named opaque> fields.
        let is_opaque_named = match &field.ty {
            TypeRef::Named(n) => opaque_types.contains(n.as_str()),
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if opaque_types.contains(n.as_str()))
            }
            _ => false,
        };
        if is_opaque_named {
            out.push_str(&crate::codegen::template_env::render(
                "binding_helpers/struct_field_default.jinja",
                minijinja::context! {
                    name => &field.name,
                },
            ));
            out.push('\n');
            continue;
        }
        // Skip types: output-only types (e.g. flat data enums) that have no From impl
        // from the binding layer. Emit Default::default() so method body compiles.
        let is_skip_named = match &field.ty {
            TypeRef::Named(n) => skip_types.contains(n),
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if skip_types.contains(n))
            }
            _ => false,
        };
        if is_skip_named {
            out.push_str(&crate::codegen::template_env::render(
                "binding_helpers/default_field.jinja",
                minijinja::context! {
                    name => &name,
                },
            ));
            continue;
        }
        let expr = match &field.ty {
            TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                let core_ty = core_prim_str(p);
                if field.optional {
                    format!("self.{name}.map(|v| v as {core_ty})")
                } else {
                    format!("self.{name} as {core_ty}")
                }
            }
            TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                let core_ty = core_prim_str(p);
                if field.optional {
                    format!("self.{name}.map(|v| v as {core_ty})")
                } else {
                    format!("self.{name} as {core_ty}")
                }
            }
            TypeRef::Primitive(_) => format!("self.{name}"),
            TypeRef::Duration => {
                if field.optional {
                    format!("self.{name}.map(std::time::Duration::from_millis)")
                } else if option_duration_on_defaults && typ.has_default {
                    // When option_duration_on_defaults is true, non-optional Duration fields
                    // on has_default types are stored as Option<u64> in the binding struct.
                    // Use .map(...).unwrap_or_default() so that None falls back to the core
                    // type's Default (e.g. Duration::from_secs(30)) rather than Duration::ZERO.
                    format!("self.{name}.map(std::time::Duration::from_millis).unwrap_or_default()")
                } else {
                    format!("std::time::Duration::from_millis(self.{name})")
                }
            }
            TypeRef::String => {
                // Cow<'_, str> and Box<str> both need `.into()` to convert
                // back to the wrapper from the binding-side `String`.
                // When the field is optional, use `.map(Into::into)` so that
                // Option<String> converts to Option<Cow<'_, str>> correctly.
                if matches!(field.core_wrapper, CoreWrapper::Cow | CoreWrapper::Box) {
                    if field.optional {
                        format!("self.{name}.clone().map(Into::into)")
                    } else {
                        format!("self.{name}.clone().into()")
                    }
                } else {
                    format!("self.{name}.clone()")
                }
            }
            // Bytes: binding stores Vec<u8>. When core_wrapper == Bytes, core expects
            // bytes::Bytes so we must call .into() to convert Vec<u8> → Bytes.
            // When core_wrapper == None, the core field is also Vec<u8> (plain clone).
            TypeRef::Bytes => {
                if field.core_wrapper == CoreWrapper::Bytes {
                    format!("self.{name}.clone().into()")
                } else {
                    format!("self.{name}.clone()")
                }
            }
            TypeRef::Char => {
                if field.optional {
                    format!("self.{name}.as_ref().and_then(|s| s.chars().next())")
                } else {
                    format!("self.{name}.chars().next().unwrap_or('*')")
                }
            }
            TypeRef::Path => {
                if field.optional {
                    format!("self.{name}.clone().map(Into::into)")
                } else {
                    format!("self.{name}.clone().into()")
                }
            }
            TypeRef::Named(_) => {
                if field.optional {
                    format!("self.{name}.clone().map(Into::into)")
                } else {
                    format!("self.{name}.clone().into()")
                }
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(_) => {
                    if field.optional {
                        // Option<Vec<Named(T)>>: map over the Option, then convert each element
                        format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(Into::into).collect()")
                    }
                }
                // Vec<u8/u16/u32/i8/i16> stored as Vec<i32> in binding → cast each element back
                TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(|v| v as {core_ty}).collect()")
                    }
                }
                // Vec<usize/u64/i64/isize/f32> stored as Vec<f64> in binding → cast each element back
                TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(|v| v as {core_ty}).collect()")
                    }
                }
                _ => format!("self.{name}.clone()"),
            },
            TypeRef::Optional(inner) => {
                // When field.optional is also true, the binding field was flattened from
                // Option<Option<T>> to Option<T>. Core expects Option<Option<T>>, so wrap
                // with .map(Some) to reconstruct the double-optional.
                let base = match inner.as_ref() {
                    TypeRef::Named(_) => {
                        format!("self.{name}.clone().map(Into::into)")
                    }
                    TypeRef::Duration => {
                        format!("self.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
                    }
                    TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                        format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                    }
                    // Option<Vec<u8/u16/u32/i8/i16>> stored as Option<Vec<i32>> → cast elements back
                    TypeRef::Vec(vi) => match vi.as_ref() {
                        TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                            let core_ty = core_prim_str(p);
                            format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                        }
                        // Option<Vec<usize/u64/i64/f32>> stored as Option<Vec<f64>> → cast elements back
                        TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                            let core_ty = core_prim_str(p);
                            format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                        }
                        _ => format!("self.{name}.clone()"),
                    },
                    _ => format!("self.{name}.clone()"),
                };
                if field.optional {
                    format!("({base}).map(Some)")
                } else {
                    base
                }
            }
            TypeRef::Map(_, v) => match v.as_ref() {
                TypeRef::Json => {
                    // HashMap<String, String> (binding) → HashMap<K, Value> (core).
                    // Emit `k.into()` so wrapped string keys (`Cow`, `Box<str>`, `Arc<str>`)
                    // — which the type resolver collapses to `TypeRef::String` — convert
                    // correctly. For a real `String` core key it is a no-op.
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| \
                                 (k.into(), serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v)))).collect())"
                        )
                    } else {
                        format!(
                            "self.{name}.clone().into_iter().map(|(k, v)| \
                                 (k.into(), serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v)))).collect()"
                        )
                    }
                }
                // Named values: each value needs Into conversion to bridge the binding wrapper
                // type into the core type (e.g. PyExtractionPattern → ExtractionPattern).
                TypeRef::Named(_) => {
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
                    }
                }
                // Map values that are u8/u16/u32/i8/i16 stored as i32 in binding → cast back
                TypeRef::Primitive(p) if cast_uints_to_i32 && needs_i32_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect()")
                    }
                }
                // Map values that are usize/u64/i64/isize/f32 stored as f64 in binding → cast back
                TypeRef::Primitive(p) if cast_large_ints_to_f64 && needs_f64_cast(p) => {
                    let core_ty = core_prim_str(p);
                    if field.optional {
                        format!(
                            "self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect())"
                        )
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v as {core_ty})).collect()")
                    }
                }
                // Collect to handle HashMap↔BTreeMap conversion
                _ => {
                    if field.optional {
                        format!("self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k.into(), v)).collect())")
                    } else {
                        format!("self.{name}.clone().into_iter().map(|(k, v)| (k.into(), v)).collect()")
                    }
                }
            },
            TypeRef::Unit => format!("self.{name}.clone()"),
            TypeRef::Json => {
                // String (binding) → serde_json::Value (core)
                if field.optional {
                    format!("self.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())")
                } else {
                    format!("serde_json::from_str(&self.{name}).unwrap_or_default()")
                }
            }
        };
        // Newtype wrapping: when the field was resolved from a newtype (e.g. NodeIndex → u32),
        // re-wrap the binding value into the newtype for the core struct literal.
        // When `optional=true` and `ty` is a plain Primitive (not TypeRef::Optional), the core
        // field is actually `Option<NewtypeT>`, so we must use `.map(NewtypeT)` not `NewtypeT(...)`.
        let expr = if let Some(newtype_path) = &field.newtype_wrapper {
            match &field.ty {
                TypeRef::Optional(_) => format!("({expr}).map({newtype_path})"),
                TypeRef::Vec(_) => format!("({expr}).into_iter().map({newtype_path}).collect::<Vec<_>>()"),
                _ if field.optional => format!("({expr}).map({newtype_path})"),
                _ => format!("{newtype_path}({expr})"),
            }
        } else {
            expr
        };
        out.push_str(&crate::codegen::template_env::render(
            "binding_helpers/struct_field_line.jinja",
            minijinja::context! {
                name => &field.name,
                expr => &expr,
            },
        ));
        out.push('\n');
    }
    // Emit the ..Default::default() trailer for every has_default core type — it
    // fills binding-excluded fields (alef(skip)) with the core's Default, and it
    // keeps the literal compiling (E0063) when an additive field lands on the core
    // struct after generation. Without Default the trailer would fail E0277, so
    // the per-field fallback above covers binding-excluded fields instead; the
    // cfg-stripped trailer stays unconditional because the `#[cfg(...)]` gates
    // make the spread a no-op when the feature is disabled and the gated paths
    // rely on the core type being Default-constructible when it is enabled.
    if typ.has_stripped_cfg_fields || typ.has_default {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str("        };\n        ");
    out
}
