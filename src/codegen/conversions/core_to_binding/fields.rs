use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::config::WasmCamelRecasedEnum;
use crate::codegen::conversions::field_conversion_to_core;
use crate::codegen::conversions::helpers::{binding_prim_str, needs_f64_cast, needs_i32_cast, needs_i64_cast};
use crate::codegen::json_wire_types::out_pipeline_expr;
use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::AHashSet;

/// `core_expr` must evaluate to something borrowable as the core enum (see
/// `json_wire_types::out_pipeline_expr`). Produces a Rust expression of type `serde_json::Value`.
fn camel_json_value(core_expr: &str, rec: &WasmCamelRecasedEnum) -> String {
    out_pipeline_expr(
        core_expr,
        rec.retag_fn_name,
        rec.out_wire_type,
        rec.core_tag_key,
        rec.js_tag_key,
    )
}

/// Bridge a `serde_json::Value` to `JsValue` as a PLAIN JavaScript object, falling back to
/// `JsValue::NULL`.
///
/// `serde_wasm_bindgen`'s default `Serializer` renders every serde *map* as a JS `Map`, not an
/// object, and a recased enum's pipeline hands it a `serde_json::Value::Object`. The field
/// therefore arrived in JavaScript as a `Map`, on which property access returns `undefined`,
/// while the `.d.ts` `ts_union` emits for it declares a plain object with camelCase keys:
/// a downstream wasm e2e read `metadata.format.sheetCount` as `NaN` and `metadata.format.title` as
/// `''`. `JSON.parse` of the serialized text always yields plain objects, and is already this
/// module's idiom for `Map<String, String>` fields.
///
/// Deliberately NOT applied to the raw (non-recased) `tagged_data_enum_names` passthrough: for an
/// UNTAGGED data enum `ts_union` derives the declared union FROM what `serde_wasm_bindgen`
/// produces, so moving the bridge without moving the declaration would put the two out of step --
/// and those shapes (a bare string, an array) are not maps, so they never had the defect. ~keep
fn json_object_jsvalue(value_expr: &str) -> String {
    format!("js_sys::JSON::parse(&serde_json::to_string(&{value_expr}).unwrap_or_default()).unwrap_or(JsValue::NULL)")
}

/// Same as `camel_json_value` but wraps the result for wasm's `JsValue` field boundary, falling
/// back to `JsValue::NULL` exactly like the raw (non-recased) `tagged_data_enum_names` path's own
/// `.unwrap_or(JsValue::NULL)`.
fn camel_jsvalue(core_expr: &str, rec: &WasmCamelRecasedEnum) -> String {
    json_object_jsvalue(&camel_json_value(core_expr, rec))
}

/// Same as `camel_jsvalue` but for a `Vec<CoreEnum>`: `items_expr` must evaluate to something
/// whose `.iter()` yields items borrowable as the core enum (e.g. `&Vec<CoreEnum>` or
/// `Vec<CoreEnum>`).
fn camel_jsvalue_vec(items_expr: &str, rec: &WasmCamelRecasedEnum) -> String {
    json_object_jsvalue(&format!(
        "{items_expr}.iter().map(|item| {}).collect::<Vec<serde_json::Value>>()",
        camel_json_value("item", rec)
    ))
}

/// Looks up `name` in `tagged_names` under each of the four shapes the raw `tagged_data_enum_names`
/// path already special-cases (bare, `Option<Named>`, `Vec<Named>`, `Option<Vec<Named>>`), also
/// returning the matched enum NAME (not just a bool) so the caller can look it up in
/// `ConversionConfig::wasm_camel_recased_enums`.
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

/// Same but for core -> binding direction.
/// Some types are asymmetric (PathBuf→String, sanitized fields need .to_string()).
pub fn field_conversion_from_core(
    name: &str,
    ty: &TypeRef,
    optional: bool,
    sanitized: bool,
    opaque_types: &AHashSet<String>,
) -> String {
    if sanitized {
        if let TypeRef::Vec(inner) = ty
            && matches!(inner.as_ref(), TypeRef::Primitive(_))
        {
            if optional {
                return format!(
                    "{name}: val.{name}.map(|t| {{ let arr: Vec<_> = [t.0, t.1].into_iter().map(|v| v as _).collect(); arr }})"
                );
            }
            return format!("{name}: vec![val.{name}.0 as _, val.{name}.1 as _]");
        }
        if let TypeRef::Optional(opt_inner) = ty
            && let TypeRef::Vec(vec_inner) = opt_inner.as_ref()
            && matches!(vec_inner.as_ref(), TypeRef::Primitive(_))
        {
            return format!("{name}: val.{name}.map(|t| vec![t.0 as _, t.1 as _])");
        }
        if let TypeRef::Map(k, v) = ty
            && matches!(k.as_ref(), TypeRef::String)
            && matches!(v.as_ref(), TypeRef::String)
        {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().map(|m| m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())"
                );
            }
            return format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()");
        }
        if let TypeRef::Vec(outer_inner) = ty
            && let TypeRef::Vec(inner) = outer_inner.as_ref()
            && matches!(inner.as_ref(), TypeRef::String)
        {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())"
                );
            }
            return format!(
                "{name}: val.{name}.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()"
            );
        }
        if let TypeRef::Optional(opt_inner) = ty
            && let TypeRef::Vec(outer_inner) = opt_inner.as_ref()
            && let TypeRef::Vec(inner) = outer_inner.as_ref()
            && matches!(inner.as_ref(), TypeRef::String)
        {
            return format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())"
            );
        }
        if let TypeRef::Vec(inner) = ty
            && matches!(inner.as_ref(), TypeRef::String)
        {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| format!(\"{{:?}}\", i)).collect())"
                );
            }
            return format!("{name}: val.{name}.iter().map(|i| format!(\"{{:?}}\", i)).collect()");
        }
        if let TypeRef::Optional(opt_inner) = ty
            && let TypeRef::Vec(vec_inner) = opt_inner.as_ref()
            && matches!(vec_inner.as_ref(), TypeRef::String)
        {
            return format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| format!(\"{{:?}}\", i)).collect())");
        }
        if matches!(ty, TypeRef::String) {
            if optional {
                return format!("{name}: val.{name}.as_ref().map(|v| format!(\"{{v:?}}\"))");
            }
            return format!("{name}: format!(\"{{:?}}\", val.{name})");
        }
        if optional {
            return format!("{name}: val.{name}.as_ref().map(|v| format!(\"{{v:?}}\"))");
        }
        return format!("{name}: format!(\"{{:?}}\", val.{name})");
    }
    match ty {
        TypeRef::Duration => {
            if optional {
                return format!("{name}: val.{name}.map(|d| d.as_millis() as u64)");
            }
            format!("{name}: val.{name}.as_millis() as u64")
        }
        TypeRef::Path => {
            if optional {
                format!("{name}: val.{name}.map(|p| p.to_string_lossy().to_string())")
            } else {
                format!("{name}: val.{name}.to_string_lossy().to_string()")
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
            format!("{name}: val.{name}.map(|p| p.to_string_lossy().to_string())")
        }
        TypeRef::Char => {
            if optional {
                format!("{name}: val.{name}.map(|c| c.to_string())")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        TypeRef::Bytes => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_vec().into())")
            } else {
                format!("{name}: val.{name}.to_vec().into()")
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if optional {
                format!("{name}: val.{name}.map(|v| {n} {{ inner: Arc::new(v) }})")
            } else {
                format!("{name}: {n} {{ inner: Arc::new(val.{name}) }}")
            }
        }
        TypeRef::Json => {
            if optional {
                format!("{name}: val.{name}.as_ref().map(ToString::to_string)")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Json) => {
            format!("{name}: val.{name}.as_ref().map(ToString::to_string)")
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| i.to_string()).collect())")
            } else {
                format!("{name}: val.{name}.iter().map(ToString::to_string).collect()")
            }
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Optional(oi) if matches!(oi.as_ref(), TypeRef::Json)) => {
            if optional {
                format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| i.as_ref().map(ToString::to_string)).collect())"
                )
            } else {
                format!("{name}: val.{name}.iter().map(|i| i.as_ref().map(ToString::to_string)).collect()")
            }
        }
        // this is a clone, accepted under the existing `#[allow(clippy::useless_conversion)]`.
        TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Json) => {
            if optional {
                format!(
                    "{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())"
                )
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()")
            }
        }
        TypeRef::Map(k, _v) if matches!(k.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.to_string(), v)).collect()")
            }
        }
        TypeRef::Map(k, v) if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
            }
        }
        TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Bytes) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect()")
            }
        }
        TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Named(_)) => {
            let key = if matches!(k.as_ref(), TypeRef::Named(_)) {
                "k.into()"
            } else {
                "k"
            };
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({key}, v.into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| ({key}, v.into())).collect()")
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Named(_))) => {
            field_conversion_from_core(name, inner, true, false, opaque_types)
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Map(k, v) if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::Json)) =>
        {
            let converted =
                "items.into_iter().map(|m| m.into_iter().map(|(k, v)| (k, v.to_string())).collect()).collect()";
            if optional {
                format!("{name}: val.{name}.map(|items| {converted})")
            } else {
                format!("{name}: {}", converted.replacen("items", &format!("val.{name}"), 1))
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(v) if matches!(v.as_ref(), TypeRef::Map(k, value) if matches!(k.as_ref(), TypeRef::String) && matches!(value.as_ref(), TypeRef::Json))) => {
            field_conversion_from_core(name, inner, true, false, opaque_types)
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(Into::into).collect()")
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_))) =>
        {
            format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
        }
        TypeRef::Vec(_) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
            } else {
                format!("{name}: val.{name}.into_iter().collect()")
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)) => {
            format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
        }
        TypeRef::String => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_string())")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        _ => field_conversion_to_core(name, ty, optional),
    }
}

/// Core→binding field conversion with backend-specific config.
pub fn field_conversion_from_core_cfg(
    name: &str,
    ty: &TypeRef,
    optional: bool,
    sanitized: bool,
    opaque_types: &AHashSet<String>,
    config: &ConversionConfig,
) -> String {
    if sanitized {
        if config.map_uses_jsvalue {
            if let TypeRef::Map(k, v) = ty
                && matches!(k.as_ref(), TypeRef::String)
                && matches!(v.as_ref(), TypeRef::String)
            {
                if optional {
                    return format!(
                        "{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok()).and_then(|s| js_sys::JSON::parse(&s).ok())"
                    );
                }
                return format!(
                    "{name}: js_sys::JSON::parse(&serde_json::to_string(&val.{name}).unwrap_or_default()).unwrap_or(JsValue::NULL)"
                );
            }
            if let TypeRef::Vec(inner) = ty
                && matches!(inner.as_ref(), TypeRef::Json)
            {
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
                }
                return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
            }
            if let TypeRef::Vec(outer_inner) = ty
                && let TypeRef::Vec(inner) = outer_inner.as_ref()
                && matches!(inner.as_ref(), TypeRef::String)
            {
                if optional {
                    return format!(
                        "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(&v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()).ok())"
                    );
                }
                return format!(
                    "{name}: serde_wasm_bindgen::to_value(&val.{name}.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()).unwrap_or(JsValue::NULL)"
                );
            }
            if let TypeRef::Optional(opt_inner) = ty
                && let TypeRef::Vec(outer_inner) = opt_inner.as_ref()
                && let TypeRef::Vec(inner) = outer_inner.as_ref()
                && matches!(inner.as_ref(), TypeRef::String)
            {
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(&v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()).ok())"
                );
            }
        }
        return field_conversion_from_core(name, ty, optional, sanitized, opaque_types);
    }

    if config.map_uses_jsvalue
        && let Some(tagged_names) = config.tagged_data_enum_names
    {
        let recased = |n: &str| config.wasm_camel_recased_enums.and_then(|m| m.get(n));
        match tagged_shape(ty, tagged_names) {
            Some(TaggedShape::Bare(n)) => {
                if let Some(rec) = recased(n) {
                    return if optional {
                        format!("{name}: val.{name}.as_ref().map(|v| {})", camel_jsvalue("v", rec))
                    } else {
                        format!("{name}: {}", camel_jsvalue(&format!("val.{name}"), rec))
                    };
                }
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
                }
                return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
            }
            Some(TaggedShape::Optional(n)) => {
                if let Some(rec) = recased(n) {
                    return format!("{name}: val.{name}.as_ref().map(|v| {})", camel_jsvalue("v", rec));
                }
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
            }
            Some(TaggedShape::Vec(n)) => {
                if let Some(rec) = recased(n) {
                    return if optional {
                        format!("{name}: val.{name}.as_ref().map(|v| {})", camel_jsvalue_vec("v", rec))
                    } else {
                        format!("{name}: {}", camel_jsvalue_vec(&format!("val.{name}"), rec))
                    };
                }
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
                }
                return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
            }
            Some(TaggedShape::OptionalVec(n)) => {
                if let Some(rec) = recased(n) {
                    return format!("{name}: val.{name}.as_ref().map(|v| {})", camel_jsvalue_vec("v", rec));
                }
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
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
                return format!("{name}: val.{name}.as_ref().map(|v| v.to_string())");
            }
            return format!("{name}: val.{name}.to_string()");
        }
        if optional_named {
            return format!("{name}: val.{name}.as_ref().map(|v| v.to_string())");
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
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_json::to_value(v).ok())");
            }
            return format!("{name}: serde_json::to_value(&val.{name}).unwrap_or(serde_json::Value::Null)");
        }
        if optional_named {
            return format!("{name}: val.{name}.as_ref().and_then(|v| serde_json::to_value(v).ok())");
        }
        // `.map(...).collect()`, not `.filter_map(...).collect()`: a serialize failure on one
        // Vec element must not silently shrink the Vec (that shifts every later index).
        // `unwrap_or(serde_json::Value::Null)` mirrors `direct_named`/`optional_named` above,
        // which already fall back to `serde_json::Value::Null` on failure. ~keep
        if vec_named {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|x| serde_json::to_value(x).unwrap_or(serde_json::Value::Null)).collect())"
                );
            }
            return format!(
                "{name}: val.{name}.iter().map(|x| serde_json::to_value(x).unwrap_or(serde_json::Value::Null)).collect()"
            );
        }
        if optional_vec_named {
            return format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|x| serde_json::to_value(x).unwrap_or(serde_json::Value::Null)).collect())"
            );
        }
    }

    if config.vec_named_to_string
        && let TypeRef::Vec(inner) = ty
        && matches!(inner.as_ref(), TypeRef::Named(_))
    {
        if optional {
            return format!("{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok())");
        }
        return format!("{name}: serde_json::to_string(&val.{name}).unwrap_or_default()");
    }

    if config.map_flatten_to_string
        && let TypeRef::Map(_, _) = ty
    {
        if optional {
            return format!("{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok())");
        }
        return format!("{name}: serde_json::to_string(&val.{name}).unwrap_or_default()");
    }
    if config.map_as_string && matches!(ty, TypeRef::Map(_, _)) {
        if optional {
            return format!("{name}: val.{name}.as_ref().map(|m| format!(\"{{m:?}}\"))");
        }
        return format!("{name}: format!(\"{{:?}}\", val.{name})");
    }
    if config.map_as_string
        && let TypeRef::Optional(inner) = ty
        && matches!(inner.as_ref(), TypeRef::Map(_, _))
    {
        return format!("{name}: val.{name}.as_ref().map(|m| format!(\"{{m:?}}\"))");
    }

    if config.map_uses_jsvalue {
        let is_nested_vec = matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)));
        let is_map = matches!(ty, TypeRef::Map(_, _));
        if is_map {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok()).and_then(|s| js_sys::JSON::parse(&s).ok())"
                );
            }
            return format!(
                "{name}: js_sys::JSON::parse(&serde_json::to_string(&val.{name}).unwrap_or_default()).unwrap_or(JsValue::NULL)"
            );
        }
        if is_nested_vec {
            if optional {
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
            }
            return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
        }
        if let TypeRef::Optional(inner) = ty {
            let is_inner_nested = matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Vec(_)));
            let is_inner_map = matches!(inner.as_ref(), TypeRef::Map(_, _));
            if is_inner_map {
                return format!(
                    "{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok()).and_then(|s| js_sys::JSON::parse(&s).ok())"
                );
            }
            if is_inner_nested {
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
            }
        }
    }

    let prefix = config.type_name_prefix;
    let is_enum_string = |n: &str| -> bool { config.enum_string_names.as_ref().is_some_and(|names| names.contains(n)) };

    match ty {
        TypeRef::Primitive(p) if config.cast_large_ints_to_i64 && needs_i64_cast(p) => {
            let cast_to = binding_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {cast_to})")
            } else {
                format!("{name}: val.{name} as {cast_to}")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let cast_to = binding_prim_str(p);
                format!("{name}: val.{name}.map(|v| v as {cast_to})")
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Primitive(p) if config.cast_uints_to_i32 && needs_i32_cast(p) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as i32)")
            } else {
                format!("{name}: val.{name} as i32")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_uints_to_i32 && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i32_cast(p)) =>
        {
            format!("{name}: val.{name}.map(|v| v as i32)")
        }
        TypeRef::Vec(inner)
            if config.cast_uints_to_i32 && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i32_cast(p)) =>
        {
            if let TypeRef::Primitive(_p) = inner.as_ref() {
                if optional {
                    format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as i32).collect())")
                } else {
                    format!("{name}: val.{name}.iter().map(|&v| v as i32).collect()")
                }
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Primitive(p) if config.cast_large_ints_to_f64 && needs_f64_cast(p) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as f64)")
            } else {
                format!("{name}: val.{name} as f64")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            format!("{name}: val.{name}.map(|v| v as f64)")
        }
        TypeRef::Vec(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            if optional {
                format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
            } else {
                format!("{name}: val.{name}.iter().map(|&v| v as f64).collect()")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p))) =>
        {
            format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
        }
        TypeRef::Vec(outer)
            if config.cast_large_ints_to_f64
                && matches!(outer.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p))) =>
        {
            if optional {
                format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect())"
                )
            } else {
                format!("{name}: val.{name}.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect()")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(prim) if matches!(prim.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)))) =>
        {
            format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect())"
            )
        }
        TypeRef::Map(_k, v)
            if config.cast_large_ints_to_f64 && matches!(v.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            if optional {
                format!("{name}: val.{name}.as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), *v as f64)).collect())")
            } else {
                format!("{name}: val.{name}.iter().map(|(k, v)| (k.clone(), *v as f64)).collect()")
            }
        }
        TypeRef::Duration if config.cast_large_ints_to_f64 => {
            if optional {
                format!("{name}: val.{name}.map(|d| d.as_millis() as f64)")
            } else {
                format!("{name}: val.{name}.as_millis() as f64")
            }
        }
        TypeRef::Primitive(PrimitiveType::F32) if config.cast_f32_to_f64 => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as f64)")
            } else {
                format!("{name}: val.{name} as f64")
            }
        }
        TypeRef::Duration if config.cast_large_ints_to_i64 => {
            if optional {
                format!("{name}: val.{name}.map(|d| d.as_millis() as u64 as i64)")
            } else {
                format!("{name}: val.{name}.as_millis() as u64 as i64")
            }
        }
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) && !prefix.is_empty() => {
            let prefixed = format!("{prefix}{n}");
            if optional {
                format!("{name}: val.{name}.map(|v| {prefixed} {{ inner: Arc::new(v) }})")
            } else {
                format!("{name}: {prefixed} {{ inner: Arc::new(val.{name}) }}")
            }
        }
        TypeRef::Named(n) if is_enum_string(n) => {
            if optional {
                format!(
                    "{name}: val.{name}.as_ref().map(|v| serde_json::to_value(v).ok().and_then(|s| s.as_str().map(String::from)).unwrap_or_default())"
                )
            } else {
                format!(
                    "{name}: serde_json::to_value(val.{name}).ok().and_then(|s| s.as_str().map(String::from)).unwrap_or_default()"
                )
            }
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if is_enum_string(n)) => {
            if optional {
                format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|x| serde_json::to_value(x).ok().and_then(|s| s.as_str().map(String::from)).unwrap_or_default()).collect())"
                )
            } else {
                format!(
                    "{name}: val.{name}.iter().map(|v| serde_json::to_value(v).ok().and_then(|s| s.as_str().map(String::from)).unwrap_or_default()).collect()"
                )
            }
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(n) if is_enum_string(n))) =>
        {
            format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|x| serde_json::to_value(x).ok().and_then(|s| s.as_str().map(String::from)).unwrap_or_default()).collect())"
            )
        }
        TypeRef::Vec(inner)
            if config.cast_f32_to_f64 && matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::F32)) =>
        {
            if optional {
                format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
            } else {
                format!("{name}: val.{name}.iter().map(|&v| v as f64).collect()")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_f32_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(PrimitiveType::F32))) =>
        {
            format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p))) =>
        {
            if let TypeRef::Vec(vi) = inner.as_ref() {
                if let TypeRef::Primitive(p) = vi.as_ref() {
                    let cast_to = binding_prim_str(p);
                    if sanitized {
                        format!("{name}: val.{name}.map(|(a, b)| vec![a as {cast_to}, b as {cast_to}])")
                    } else {
                        format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as {cast_to}).collect())")
                    }
                } else {
                    field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
                }
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Vec(outer)
            if config.cast_f32_to_f64
                && matches!(outer.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::F32))) =>
        {
            if optional {
                format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect())"
                )
            } else {
                format!("{name}: val.{name}.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect()")
            }
        }
        TypeRef::Optional(inner)
            if config.cast_f32_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(prim) if matches!(prim.as_ref(), TypeRef::Primitive(PrimitiveType::F32)))) =>
        {
            format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect())"
            )
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let cast_to = binding_prim_str(p);
                format!("{name}: val.{name}.map(|v| v as {cast_to})")
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Map(_k, v)
            if config.cast_large_ints_to_i64 && matches!(v.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = v.as_ref() {
                let cast_to = binding_prim_str(p);
                if optional {
                    format!(
                        "{name}: val.{name}.as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), *v as {cast_to})).collect())"
                    )
                } else {
                    format!("{name}: val.{name}.iter().map(|(k, v)| (k.clone(), *v as {cast_to})).collect()")
                }
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Vec(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let cast_to = binding_prim_str(p);
                if sanitized {
                    if optional {
                        format!("{name}: val.{name}.map(|(a, b)| vec![a as {cast_to}, b as {cast_to}])")
                    } else {
                        format!("{name}: {{ let (a, b) = val.{name}; vec![a as {cast_to}, b as {cast_to}] }}")
                    }
                } else if optional {
                    format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as {cast_to}).collect())")
                } else {
                    format!("{name}: val.{name}.iter().map(|&v| v as {cast_to}).collect()")
                }
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Vec(outer)
            if config.cast_large_ints_to_i64
                && matches!(outer.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p))) =>
        {
            if let TypeRef::Vec(inner) = outer.as_ref() {
                if let TypeRef::Primitive(p) = inner.as_ref() {
                    let cast_to = binding_prim_str(p);
                    if optional {
                        format!(
                            "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as {cast_to}).collect()).collect())"
                        )
                    } else {
                        format!(
                            "{name}: val.{name}.iter().map(|inner| inner.iter().map(|&x| x as {cast_to}).collect()).collect()"
                        )
                    }
                } else {
                    field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
                }
            } else {
                field_conversion_from_core(name, ty, optional, sanitized, opaque_types)
            }
        }
        TypeRef::Json if config.json_to_string => {
            if optional {
                format!("{name}: val.{name}.as_ref().map(ToString::to_string)")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        TypeRef::Json if config.json_as_value => {
            format!("{name}: val.{name}")
        }
        TypeRef::Optional(inner) if config.json_as_value && matches!(inner.as_ref(), TypeRef::Json) => {
            format!("{name}: val.{name}")
        }
        TypeRef::Vec(inner) if config.json_as_value && matches!(inner.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: Some(val.{name})")
            } else {
                format!("{name}: val.{name}")
            }
        }
        TypeRef::Map(_k, v) if config.json_as_value && matches!(v.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), v)).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.into(), v)).collect()")
            }
        }
        TypeRef::Json if config.map_uses_jsvalue => {
            if optional {
                format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())")
            } else {
                format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)")
            }
        }
        TypeRef::Vec(inner) if config.map_uses_jsvalue && matches!(inner.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())")
            } else {
                format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)")
            }
        }
        TypeRef::Optional(inner)
            if config.map_uses_jsvalue
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Json)) =>
        {
            format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())")
        }
        _ => field_conversion_from_core(name, ty, optional, sanitized, opaque_types),
    }
}

#[cfg(test)]
mod wasm_camel_recase_tests {
    //! Regression coverage for the wasm-only camelCase recasing branch in
    //! `field_conversion_from_core_cfg`. Every test builds a `ConversionConfig` by hand (no wasm
    //! backend, no `ApiSurface`) so these are true unit tests of the string-building logic, not an
    //! end-to-end codegen run -- the wasm backend wiring that populates
    //! `wasm_camel_recased_enums` from a real `JsonWireTypes` instance is a separate, not-yet-made
    //! change (see the task assessment).

    use super::field_conversion_from_core_cfg;
    use crate::codegen::conversions::ConversionConfig;
    use crate::codegen::conversions::config::WasmCamelRecasedEnum;
    use crate::core::ir::TypeRef;
    use ahash::AHashSet;
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
        tagged_names: &'a AHashSet<String>,
        recased: Option<&'a HashMap<String, WasmCamelRecasedEnum<'a>>>,
    ) -> ConversionConfig<'a> {
        ConversionConfig {
            map_uses_jsvalue: true,
            tagged_data_enum_names: Some(tagged_names),
            wasm_camel_recased_enums: recased,
            ..ConversionConfig::default()
        }
    }

    /// CONTROL: with no `wasm_camel_recased_enums` entry at all, a bare tagged-enum field must
    /// keep emitting the exact raw expression the pre-existing `tagged_data_enum_names` path
    /// always has -- proving the new branch is a strict opt-in with zero effect until a backend
    /// actually populates the map.
    #[test]
    fn bare_field_with_no_recased_map_keeps_the_raw_snake_case_passthrough() {
        let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let cfg = config(&tagged, None);
        let out = field_conversion_from_core_cfg(
            "format",
            &TypeRef::Named("FormatMetadata".to_string()),
            false,
            false,
            &AHashSet::new(),
            &cfg,
        );
        assert_eq!(
            out, "format: serde_wasm_bindgen::to_value(&val.format).unwrap_or(JsValue::NULL)",
            "unexpected output: {out}"
        );
    }

    /// A name present in `tagged_data_enum_names` but ABSENT from `wasm_camel_recased_enums`
    /// (e.g. a plain tagged-discriminator enum, or a genuinely untagged one) must also keep the
    /// raw passthrough -- this is the safety property behind shape 2 of the assessment: only
    /// enums explicitly registered as fully-flattened get recased.
    #[test]
    fn name_absent_from_recased_map_keeps_the_raw_passthrough_even_with_map_present() {
        let tagged: AHashSet<String> = ["FormatMetadata".to_string(), "ChatRole".to_string()]
            .into_iter()
            .collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_from_core_cfg(
            "role",
            &TypeRef::Named("ChatRole".to_string()),
            false,
            false,
            &AHashSet::new(),
            &cfg,
        );
        assert_eq!(
            out, "role: serde_wasm_bindgen::to_value(&val.role).unwrap_or(JsValue::NULL)",
            "unexpected output: {out}"
        );
    }

    /// The reported defect's actual shape: a bare `FormatMetadata` field, recased.
    #[test]
    fn bare_recased_field_routes_through_the_wire_type_pipeline() {
        let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_from_core_cfg(
            "format",
            &TypeRef::Named("FormatMetadata".to_string()),
            false,
            false,
            &AHashSet::new(),
            &cfg,
        );
        assert_eq!(
            out,
            "format: js_sys::JSON::parse(&serde_json::to_string(&__alef_wire_retag_Wasm(serde_json::to_value(&val.format).ok()\
             .and_then(|raw| serde_json::from_value::<__AlefWireOutWasmFormatMetadata>(raw).ok())\
             .and_then(|wire| serde_json::to_value(wire).ok())\
             .unwrap_or_default(), \"format_type\", \"formatType\")).unwrap_or_default()).unwrap_or(JsValue::NULL)",
            "unexpected output: {out}"
        );
    }

    #[test]
    fn optional_recased_field_maps_over_the_option() {
        let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_from_core_cfg(
            "format",
            &TypeRef::Named("FormatMetadata".to_string()),
            true,
            false,
            &AHashSet::new(),
            &cfg,
        );
        assert_eq!(
            out,
            "format: val.format.as_ref().map(|v| js_sys::JSON::parse(&serde_json::to_string(&__alef_wire_retag_Wasm(serde_json::to_value(&v).ok()\
             .and_then(|raw| serde_json::from_value::<__AlefWireOutWasmFormatMetadata>(raw).ok())\
             .and_then(|wire| serde_json::to_value(wire).ok())\
             .unwrap_or_default(), \"format_type\", \"formatType\")).unwrap_or_default()).unwrap_or(JsValue::NULL))",
            "unexpected output: {out}"
        );
    }

    #[test]
    fn vec_recased_field_maps_each_element_through_the_pipeline() {
        let tagged: AHashSet<String> = ["FormatMetadata".to_string()].into_iter().collect();
        let mut recased = HashMap::new();
        recased.insert("FormatMetadata".to_string(), format_metadata_rec());
        let cfg = config(&tagged, Some(&recased));
        let out = field_conversion_from_core_cfg(
            "formats",
            &TypeRef::Vec(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
            false,
            false,
            &AHashSet::new(),
            &cfg,
        );
        assert!(
            out.starts_with("formats: js_sys::JSON::parse(&serde_json::to_string(&val.formats.iter().map(|item| "),
            "unexpected output: {out}"
        );
        assert!(
            out.contains("serde_json::from_value::<__AlefWireOutWasmFormatMetadata>(raw)"),
            "must decode each element through the wire type, got: {out}"
        );
        assert!(
            out.ends_with(").collect::<Vec<serde_json::Value>>()).unwrap_or_default()).unwrap_or(JsValue::NULL)"),
            "unexpected output: {out}"
        );
    }
}
