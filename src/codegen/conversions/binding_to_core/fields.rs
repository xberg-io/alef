use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::config::WasmCamelRecasedEnum;
use crate::codegen::conversions::helpers::{
    core_prim_str, is_tuple_type_name, needs_f64_cast, needs_i32_cast, needs_i64_cast,
};
use crate::codegen::json_wire_types::in_pipeline_expr;
use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::AHashSet;

/// `raw_js_expr` must evaluate to a `JsValue` (owned, per `serde_wasm_bindgen::from_value`'s
/// signature -- callers pass `val.{name}.clone()` or `v.clone()`). Produces a bare core-value
/// Rust expression, falling back to `Default::default()` at every stage exactly like the raw
/// (non-recased) `tagged_data_enum_names` path's own `.unwrap_or_default()`. The outer
/// `serde_json::from_value(...)` has no explicit turbofish: the target type is inferred from the
/// surrounding struct-literal field position, the same convention this file's
/// `untagged_data_enum_names` branch below already relies on. ~keep
fn camel_core_value(raw_js_expr: &str, rec: &WasmCamelRecasedEnum) -> String {
    let decoded = format!("serde_wasm_bindgen::from_value::<serde_json::Value>({raw_js_expr}).unwrap_or_default()");
    let inner = in_pipeline_expr(
        &decoded,
        rec.retag_fn_name,
        rec.in_wire_type,
        rec.core_tag_key,
        rec.js_tag_key,
    );
    format!("serde_json::from_value({inner}).unwrap_or_default()")
}

/// Same as `camel_core_value` but for a `Vec<CoreEnum>` stored as a single `JsValue` (a JS
/// array): decodes the whole array as `Vec<serde_json::Value>` once, then retags and decodes each
/// element through the same per-element pipeline. A parse failure on one element falls back to
/// that element's `Default`, matching every other Vec-of-fallible-decode conversion in this file
/// (see the `untagged_data_enum_names` `vec_named` branch below) rather than shrinking the Vec.
fn camel_core_value_vec(raw_js_expr: &str, rec: &WasmCamelRecasedEnum) -> String {
    let decoded_vec =
        format!("serde_wasm_bindgen::from_value::<Vec<serde_json::Value>>({raw_js_expr}).unwrap_or_default()");
    let per_item = in_pipeline_expr(
        "item",
        rec.retag_fn_name,
        rec.in_wire_type,
        rec.core_tag_key,
        rec.js_tag_key,
    );
    format!("{decoded_vec}.into_iter().map(|item| serde_json::from_value({per_item}).unwrap_or_default()).collect()")
}

/// See the identical-purpose helper of the same name in `core_to_binding::fields`.
enum TaggedShape<'t> {
    Bare(&'t str),
    Optional(&'t str),
    Vec(&'t str),
    OptionalVec(&'t str),
}

fn tagged_shape<'t>(ty: &'t TypeRef, tagged_names: &AHashSet<String>) -> Option<TaggedShape<'t>> {
    match ty {
        TypeRef::Named(n) if tagged_names.contains(n) => Some(TaggedShape::Bare(n)),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if tagged_names.contains(n) => Some(TaggedShape::Optional(n)),
            TypeRef::Vec(vi) => match vi.as_ref() {
                TypeRef::Named(n) if tagged_names.contains(n) => Some(TaggedShape::OptionalVec(n)),
                _ => None,
            },
            _ => None,
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if tagged_names.contains(n) => Some(TaggedShape::Vec(n)),
            _ => None,
        },
        _ => None,
    }
}

/// Determine the field conversion expression for binding -> core.
pub fn field_conversion_to_core(name: &str, ty: &TypeRef, optional: bool) -> String {
    match ty {
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Unit => {
            format!("{name}: val.{name}")
        }
        TypeRef::Bytes => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_vec().into())")
            } else {
                format!("{name}: val.{name}.to_vec().into()")
            }
        }
        TypeRef::Json => {
            if optional {
                format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())")
            } else {
                format!("{name}: serde_json::from_str(&val.{name}).unwrap_or_default()")
            }
        }
        TypeRef::Char => {
            if optional {
                format!("{name}: val.{name}.and_then(|s| s.chars().next())")
            } else {
                format!("{name}: val.{name}.chars().next().unwrap_or('*')")
            }
        }
        TypeRef::Duration => {
            if optional {
                format!("{name}: val.{name}.map(std::time::Duration::from_millis)")
            } else {
                format!("{name}: std::time::Duration::from_millis(val.{name})")
            }
        }
        TypeRef::Path => {
            if optional {
                format!("{name}: val.{name}.map(Into::into)")
            } else {
                format!("{name}: val.{name}.into()")
            }
        }
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
        TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Bytes) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect()")
            }
        }
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
            TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Named(_)) => {
                field_conversion_to_core(name, inner, true)
            }
            TypeRef::Vec(_) => field_conversion_to_core(name, inner, true),
            _ => format!("{name}: val.{name}"),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Map(k, v) if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::Json) => {
                let converted = "items.into_iter().map(|m| m.into_iter().map(|(k, v)| (k, serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v)))).collect()).collect()";
                if optional {
                    format!("{name}: val.{name}.map(|items| {converted})")
                } else {
                    format!("{name}: {}", converted.replacen("items", &format!("val.{name}"), 1))
                }
            }
            TypeRef::Json => {
                // `.map(...).collect()`, not `.filter_map(...).collect()`: filter_map would
                // silently shrink the Vec on any element that fails to parse, shifting every
                // later element's index. `unwrap_or_default()` keeps the element count aligned
                // with the source, matching the scalar `TypeRef::Json` conversion above, which
                // already assumes `T: Default`. ~keep
                if optional {
                    format!(
                        "{name}: val.{name}.map(|v| v.into_iter().map(|s| serde_json::from_str(&s).unwrap_or_default()).collect())"
                    )
                } else {
                    format!(
                        "{name}: val.{name}.into_iter().map(|s| serde_json::from_str(&s).unwrap_or_default()).collect()"
                    )
                }
            }
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
            _ => {
                if optional {
                    format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().collect()")
                }
            }
        },
        TypeRef::Map(k, v) => {
            let has_named_key = matches!(k.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n));
            let has_named_val = matches!(v.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n));
            let has_json_val = matches!(v.as_ref(), TypeRef::Json);
            let has_json_key = matches!(k.as_ref(), TypeRef::Json);
            let has_vec_named_val = matches!(v.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n)));
            let has_vec_json_val = matches!(v.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Json));
            if has_json_val || has_json_key || has_named_key || has_named_val || has_vec_named_val || has_vec_json_val {
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
                    // Preserve element count on parse failure (see the TypeRef::Vec(Json) arm
                    // above for rationale); the map value type is assumed `Default` already, as
                    // the scalar map-value conversions in this match rely on it too. ~keep
                    "v.into_iter().map(|s| serde_json::from_str(&s).unwrap_or_default()).collect()"
                } else {
                    "v"
                };
                if optional {
                    format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({k_expr}, {v_expr})).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|(k, v)| ({k_expr}, {v_expr})).collect()")
                }
            } else {
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
    if optional && matches!(ty, TypeRef::Optional(_)) {
        let inner_expr = field_conversion_to_core_cfg(name, ty, false, config);
        if let Some(expr) = inner_expr.strip_prefix(&format!("{name}: ")) {
            return format!("{name}: ({expr}).map(Some)");
        }
        return inner_expr;
    }

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

    if config.vec_named_to_string
        && let TypeRef::Vec(inner) = ty
        && matches!(inner.as_ref(), TypeRef::Named(_))
    {
        if optional {
            return format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())");
        }
        return format!("{name}: serde_json::from_str(&val.{name}).unwrap_or_default()");
    }
    if config.map_flatten_to_string
        && let TypeRef::Map(_, _) = ty
    {
        if optional {
            return format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())");
        }
        return format!("{name}: serde_json::from_str(&val.{name}).unwrap_or_default()");
    }
    if config.map_as_string && matches!(ty, TypeRef::Map(_, _)) {
        return format!("{name}: Default::default()");
    }
    if config.map_as_string
        && let TypeRef::Optional(inner) = ty
        && matches!(inner.as_ref(), TypeRef::Map(_, _))
    {
        return format!("{name}: Default::default()");
    }
    if config.map_uses_jsvalue
        && let Some(tagged_names) = config.tagged_data_enum_names
    {
        let recased = |n: &str| config.wasm_camel_recased_enums.and_then(|m| m.get(n));
        match tagged_shape(ty, tagged_names) {
            Some(TaggedShape::Bare(n)) => {
                if let Some(rec) = recased(n) {
                    return if optional {
                        format!(
                            "{name}: val.{name}.as_ref().map(|v| {})",
                            camel_core_value("v.clone()", rec)
                        )
                    } else {
                        format!("{name}: {}", camel_core_value(&format!("val.{name}.clone()"), rec))
                    };
                }
                if optional {
                    return format!(
                        "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                    );
                }
                return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
            }
            Some(TaggedShape::Optional(n)) => {
                if let Some(rec) = recased(n) {
                    return format!(
                        "{name}: val.{name}.as_ref().map(|v| {})",
                        camel_core_value("v.clone()", rec)
                    );
                }
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                );
            }
            Some(TaggedShape::Vec(n)) => {
                if let Some(rec) = recased(n) {
                    return format!("{name}: {}", camel_core_value_vec(&format!("val.{name}.clone()"), rec));
                }
                return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
            }
            Some(TaggedShape::OptionalVec(n)) => {
                if let Some(rec) = recased(n) {
                    return format!(
                        "{name}: val.{name}.as_ref().map(|v| {})",
                        camel_core_value_vec("v.clone()", rec)
                    );
                }
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                );
            }
            None => {}
        }
    }

    if let Some(text_names) = config.text_field_enum_names {
        let direct_named = matches!(ty, TypeRef::Named(n) if text_names.contains(n));
        let optional_named = matches!(ty, TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n) if text_names.contains(n)));
        if direct_named {
            if optional {
                return format!(
                    "{name}: val.{name}.map(|s| serde_json::from_value(serde_json::Value::String(s)).unwrap_or_default())"
                );
            }
            return format!(
                "{name}: serde_json::from_value(serde_json::Value::String(val.{name})).unwrap_or_default()"
            );
        }
        if optional_named {
            return format!(
                "{name}: val.{name}.map(|s| serde_json::from_value(serde_json::Value::String(s)).unwrap_or_default())"
            );
        }
    }

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
        // `.map(...).collect()`, not `.filter_map(...).collect()`: a deserialize failure on one
        // Vec element must not silently shrink the Vec (that shifts every later index). This
        // mirrors `direct_named`/`optional_named` above, which already assume `T: Default`. ~keep
        if vec_named {
            if optional {
                return format!(
                    "{name}: val.{name}.map(|v| v.into_iter().map(|x| serde_json::from_value(x).unwrap_or_default()).collect())"
                );
            }
            return format!(
                "{name}: val.{name}.into_iter().map(|x| serde_json::from_value(x).unwrap_or_default()).collect()"
            );
        }
        if optional_vec_named {
            return format!(
                "{name}: val.{name}.map(|v| v.into_iter().map(|x| serde_json::from_value(x).unwrap_or_default()).collect())"
            );
        }
    }
    if config.json_to_string && matches!(ty, TypeRef::Json) {
        return format!("{name}: Default::default()");
    }
    if config.json_as_value && matches!(ty, TypeRef::Json) {
        return format!("{name}: val.{name}");
    }
    if config.json_as_value {
        if let TypeRef::Optional(inner) = ty
            && matches!(inner.as_ref(), TypeRef::Json)
        {
            return format!("{name}: val.{name}");
        }
        if let TypeRef::Vec(inner) = ty
            && matches!(inner.as_ref(), TypeRef::Json)
        {
            if optional {
                return format!("{name}: val.{name}.unwrap_or_default()");
            }
            return format!("{name}: val.{name}");
        }
        if let TypeRef::Map(_k, v) = ty
            && matches!(v.as_ref(), TypeRef::Json)
        {
            if optional {
                return format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), v)).collect())");
            }
            return format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.into(), v)).collect()");
        }
    }
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
        && !config.map_flatten_to_string
        && config.from_binding_skip_types.is_empty()
    {
        return field_conversion_to_core(name, ty, optional);
    }
    match ty {
        TypeRef::Primitive(p) if config.cast_large_ints_to_i64 && needs_i64_cast(p) => {
            let core_ty = core_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                format!("{name}: val.{name} as {core_ty}")
            }
        }
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
        TypeRef::Vec(inner)
            if config.cast_f32_to_f64 && matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::F32)) =>
        {
            if optional {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as f32).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|v| v as f32).collect()")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_f32_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(PrimitiveType::F32))) =>
        {
            format!("{name}: val.{name}.map(|v| v.into_iter().map(|x| x as f32).collect())")
        }
        TypeRef::Primitive(p) if config.cast_uints_to_i32 && needs_i32_cast(p) => {
            let core_ty = core_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                format!("{name}: val.{name} as {core_ty}")
            }
        }
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
        TypeRef::Primitive(p) if config.cast_large_ints_to_f64 && needs_f64_cast(p) => {
            let core_ty = core_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                format!("{name}: val.{name} as {core_ty}")
            }
        }
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
        TypeRef::Named(n) if config.from_binding_skip_types.iter().any(|s| s == n) => {
            format!("{name}: Default::default()")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if config.from_binding_skip_types.iter().any(|s| s == n) => {
                format!("{name}: Default::default()")
            }
            _ => field_conversion_to_core(name, ty, optional),
        },
        _ => field_conversion_to_core(name, ty, optional),
    }
}

#[cfg(test)]
mod wasm_camel_recase_tests {
    //! Mirrors `core_to_binding::fields::wasm_camel_recase_tests` for the JS -> core direction.
    //! See that module's doc comment for why these are pure string-building unit tests, not an
    //! end-to-end wasm backend run.

    use super::field_conversion_to_core_cfg;
    use crate::codegen::conversions::ConversionConfig;
    use crate::codegen::conversions::config::WasmCamelRecasedEnum;
    use crate::core::ir::TypeRef;
    use std::collections::HashMap;

    fn format_metadata_rec() -> WasmCamelRecasedEnum<'static> {
        WasmCamelRecasedEnum {
            out_wire_type: "__AlefWireOutWasmFormatMetadata",
            in_wire_type: "__AlefWireInWasmFormatMetadata",
            retag_fn_name: "__alef_wire_retag_Wasm",
            core_tag_key: "format_type",
            js_tag_key: "formatType",
        }
    }

    fn config<'a>(
        tagged_names: &'a ahash::AHashSet<String>,
        recased: Option<&'a HashMap<String, WasmCamelRecasedEnum<'a>>>,
    ) -> ConversionConfig<'a> {
        ConversionConfig {
            map_uses_jsvalue: true,
            tagged_data_enum_names: Some(tagged_names),
            wasm_camel_recased_enums: recased,
            ..ConversionConfig::default()
        }
    }

    /// CONTROL: no `wasm_camel_recased_enums` entry -> the exact pre-existing raw expression,
    /// byte for byte.
    #[test]
    fn bare_field_with_no_recased_map_keeps_the_raw_passthrough() {
        let tagged: ahash::AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let cfg = config(&tagged, None);
        let out = field_conversion_to_core_cfg("format", &TypeRef::Named("FormatMetadata".to_string()), false, &cfg);
        assert_eq!(
            out, "format: serde_wasm_bindgen::from_value(val.format.clone()).unwrap_or_default()",
            "unexpected output: {out}"
        );
    }

    #[test]
    fn bare_recased_field_decodes_through_the_wire_type_pipeline() {
        let tagged: ahash::AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_to_core_cfg("format", &TypeRef::Named("FormatMetadata".to_string()), false, &cfg);
        assert_eq!(
            out,
            "format: serde_json::from_value(__alef_wire_retag_Wasm(serde_json::from_value::<__AlefWireInWasmFormatMetadata>(\
             serde_wasm_bindgen::from_value::<serde_json::Value>(val.format.clone()).unwrap_or_default()).ok()\
             .and_then(|wire| serde_json::to_value(wire).ok())\
             .unwrap_or_default(), \"formatType\", \"format_type\")).unwrap_or_default()",
            "unexpected output: {out}"
        );
    }

    #[test]
    fn optional_recased_field_maps_over_the_option() {
        let tagged: ahash::AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_to_core_cfg("format", &TypeRef::Named("FormatMetadata".to_string()), true, &cfg);
        assert_eq!(
            out,
            "format: val.format.as_ref().map(|v| serde_json::from_value(__alef_wire_retag_Wasm(serde_json::from_value::<__AlefWireInWasmFormatMetadata>(\
             serde_wasm_bindgen::from_value::<serde_json::Value>(v.clone()).unwrap_or_default()).ok()\
             .and_then(|wire| serde_json::to_value(wire).ok())\
             .unwrap_or_default(), \"formatType\", \"format_type\")).unwrap_or_default())",
            "unexpected output: {out}"
        );
    }

    #[test]
    fn vec_recased_field_decodes_each_element_through_the_pipeline() {
        let tagged: ahash::AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_to_core_cfg(
            "formats",
            &TypeRef::Vec(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
            false,
            &cfg,
        );
        assert!(
            out.starts_with(
                "formats: serde_wasm_bindgen::from_value::<Vec<serde_json::Value>>(val.formats.clone()).unwrap_or_default()\
                 .into_iter().map(|item| serde_json::from_value("
            ),
            "unexpected output: {out}"
        );
        assert!(
            out.contains("serde_json::from_value::<__AlefWireInWasmFormatMetadata>(item)"),
            "must decode each element through the wire type, got: {out}"
        );
        assert!(out.ends_with(").collect()"), "unexpected output: {out}");
    }
}
