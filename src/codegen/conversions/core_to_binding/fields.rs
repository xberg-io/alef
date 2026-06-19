use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::field_conversion_to_core;
use crate::codegen::conversions::helpers::{binding_prim_str, needs_f64_cast, needs_i32_cast, needs_i64_cast};
use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::AHashSet;

/// Same but for core -> binding direction.
/// Some types are asymmetric (PathBuf→String, sanitized fields need .to_string()).
pub fn field_conversion_from_core(
    name: &str,
    ty: &TypeRef,
    optional: bool,
    sanitized: bool,
    opaque_types: &AHashSet<String>,
) -> String {
    // Sanitized fields: the binding type differs from core (e.g. Box<str>→String, Cow<str>→String).
    // Box<str>, Cow<str>, and Arc<str> all implement Display, so use .to_string() not {:?}.
    // {:?} on string-like types produces debug-escaped output with surrounding quotes.
    if sanitized {
        // Vec<Primitive>: sanitized from tuple types like (u32, u32) → Vec<u32>.
        // Core has a tuple, binding expects Vec — destructure the tuple.
        if let TypeRef::Vec(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Primitive(_)) {
                if optional {
                    return format!(
                        "{name}: val.{name}.map(|t| {{ let arr: Vec<_> = [t.0, t.1].into_iter().map(|v| v as _).collect(); arr }})"
                    );
                }
                return format!("{name}: vec![val.{name}.0 as _, val.{name}.1 as _]");
            }
        }
        // Optional(Vec<Primitive>): sanitized from Option<(T, T)> → Option<Vec<T>>.
        if let TypeRef::Optional(opt_inner) = ty {
            if let TypeRef::Vec(vec_inner) = opt_inner.as_ref() {
                if matches!(vec_inner.as_ref(), TypeRef::Primitive(_)) {
                    return format!("{name}: val.{name}.map(|t| vec![t.0 as _, t.1 as _])");
                }
            }
        }
        // Map(String, String): sanitized from Map(Box<str>, Box<str>) etc.
        if let TypeRef::Map(k, v) = ty {
            if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String) {
                if optional {
                    return format!(
                        "{name}: val.{name}.as_ref().map(|m| m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())"
                    );
                }
                return format!(
                    "{name}: val.{name}.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()"
                );
            }
        }
        // Vec<Vec<String>>: sanitized from Vec<(String, String)> (homogeneous tuple of strings).
        // parse_homogeneous_tuple lifts (String, String) → Vec<String>; the outer Vec is preserved,
        // so the IR shape is Vec<Vec<String>>. Core still holds Vec<(String, String)>, so we
        // destructure each tuple item into a 2-element Vec.
        if let TypeRef::Vec(outer_inner) = ty {
            if let TypeRef::Vec(inner) = outer_inner.as_ref() {
                if matches!(inner.as_ref(), TypeRef::String) {
                    if optional {
                        return format!(
                            "{name}: val.{name}.as_ref().map(|v| v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())"
                        );
                    }
                    return format!(
                        "{name}: val.{name}.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()"
                    );
                }
            }
        }
        // Optional<Vec<Vec<String>>>: sanitized from Option<Vec<(String, String)>>.
        if let TypeRef::Optional(opt_inner) = ty {
            if let TypeRef::Vec(outer_inner) = opt_inner.as_ref() {
                if let TypeRef::Vec(inner) = outer_inner.as_ref() {
                    if matches!(inner.as_ref(), TypeRef::String) {
                        return format!(
                            "{name}: val.{name}.as_ref().map(|v| v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())"
                        );
                    }
                }
            }
        }
        // Vec<String>: sanitized from Vec<Box<str>>, Vec<Cow<str>>, Vec<Named>, etc.
        // Use Debug formatting — the original core type may not implement Display.
        if let TypeRef::Vec(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::String) {
                if optional {
                    return format!(
                        "{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| format!(\"{{:?}}\", i)).collect())"
                    );
                }
                return format!("{name}: val.{name}.iter().map(|i| format!(\"{{:?}}\", i)).collect()");
            }
        }
        // Optional<Vec<String>>: sanitized from Optional<Vec<Box<str>>>, Optional<Vec<Cow<str>>>, etc.
        if let TypeRef::Optional(opt_inner) = ty {
            if let TypeRef::Vec(vec_inner) = opt_inner.as_ref() {
                if matches!(vec_inner.as_ref(), TypeRef::String) {
                    return format!(
                        "{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| format!(\"{{:?}}\", i)).collect())"
                    );
                }
            }
        }
        // String: sanitized from Box<str>, Cow<str>, (u32, u32), etc.
        // Use Debug formatting — it works for all types (including tuples) and avoids Display
        // trait bound failures when the original core type doesn't implement Display.
        // Note: Cow<str> is handled before this point via the CoreWrapper::Cow path above.
        if matches!(ty, TypeRef::String) {
            if optional {
                return format!("{name}: val.{name}.as_ref().map(|v| format!(\"{{v:?}}\"))");
            }
            return format!("{name}: format!(\"{{:?}}\", val.{name})");
        }
        // Fallback for truly unknown sanitized types — the core type may not implement Display,
        // so use Debug formatting which is always available (required by the sanitized field's derive).
        if optional {
            return format!("{name}: val.{name}.as_ref().map(|v| format!(\"{{v:?}}\"))");
        }
        return format!("{name}: format!(\"{{:?}}\", val.{name})");
    }
    match ty {
        // Duration: core uses std::time::Duration, binding uses u64 (millis)
        TypeRef::Duration => {
            if optional {
                return format!("{name}: val.{name}.map(|d| d.as_millis() as u64)");
            }
            format!("{name}: val.{name}.as_millis() as u64")
        }
        // Path: core uses PathBuf, binding uses String — PathBuf→String needs special handling
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
        // Char: core uses char, binding uses String — convert char to string
        TypeRef::Char => {
            if optional {
                format!("{name}: val.{name}.map(|c| c.to_string())")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        // Bytes: core uses bytes::Bytes, binding uses Vec<u8> or napi `Buffer`.
        // `.into()` is a no-op when destination is Vec<u8> (identity From) and
        // a Vec→Buffer wrap when destination is `napi::bindgen_prelude::Buffer`.
        TypeRef::Bytes => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_vec().into())")
            } else {
                format!("{name}: val.{name}.to_vec().into()")
            }
        }
        // Opaque Named types: wrap in Arc to create the binding wrapper
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if optional {
                format!("{name}: val.{name}.map(|v| {n} {{ inner: Arc::new(v) }})")
            } else {
                format!("{name}: {n} {{ inner: Arc::new(val.{name}) }}")
            }
        }
        // Json: core uses serde_json::Value, binding uses String — use .to_string()
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
        // Vec<Optional<Json>>: each element is Option<Value> → Option<String>
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Optional(oi) if matches!(oi.as_ref(), TypeRef::Json)) => {
            if optional {
                format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().map(|i| i.as_ref().map(ToString::to_string)).collect())"
                )
            } else {
                format!("{name}: val.{name}.iter().map(|i| i.as_ref().map(ToString::to_string)).collect()")
            }
        }
        // Map with Json values: core uses HashMap<K, serde_json::Value>, binding uses HashMap<K, String>.
        // Always emit `k.to_string()` so Cow<'_, str> / Box<str> / Arc<str> keys (which the type
        // resolver normalizes to TypeRef::String) convert correctly. For an actual `String` key
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
        // Map with Json keys: core uses HashMap<serde_json::Value, V>, binding uses HashMap<String, V>
        TypeRef::Map(k, _v) if matches!(k.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.to_string(), v)).collect()")
            }
        }
        // Map<String, String>: core may have Box<str> keys/values, binding has String keys/values.
        // Emit .map() with .into() conversions, which are no-ops when both sides are String.
        // This handles cases like HashMap<Box<str>, Box<str>> (core) → HashMap<String, String> (binding).
        TypeRef::Map(k, v) if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
            }
        }
        // Map<K, Bytes>: core uses bytes::Bytes (or Vec<u8>), binding uses Vec<u8> or napi Buffer.
        // `.to_vec().into()` converts Bytes→Vec<u8> (identity for Vec<u8>) or Bytes→Buffer (napi).
        TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Bytes) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v.to_vec().into())).collect()")
            }
        }
        // Map<K, Named>: each value needs .into() to convert core→binding
        TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Named(_)) => {
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| (k, v.into())).collect()")
            }
        }
        // Optional(Map<K, Named>): same but wrapped in Option
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Map(_k, v) if matches!(v.as_ref(), TypeRef::Named(_))) =>
        {
            format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())")
        }
        // Vec<Named>: each element needs .into() to convert core→binding
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(Into::into).collect()")
            }
        }
        // Optional(Vec<Named>): same but wrapped in Option
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_))) =>
        {
            format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
        }
        // Vec<Primitive>, Vec<String>, Vec<Bytes>, etc.
        // The core type may be a Set (HashSet, AHashSet, BTreeSet, etc.) which the type resolver
        // maps to Vec in the IR. Emit .into_iter().collect() which works for both Vec→Vec (identity)
        // and Set→Vec (uniqueness guarantee → ordered collection) conversions.
        TypeRef::Vec(_) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
            } else {
                format!("{name}: val.{name}.into_iter().collect()")
            }
        }
        // Optional(Vec<T>): same but wrapped in Option
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)) => {
            format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
        }
        // String: core may be &str (lifetime-param types like Segment<'_>), binding is always
        // String. Use .to_string() which works for both owned String (no-op clone) and &str.
        TypeRef::String => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_string())")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        // Everything else is symmetric
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
    // Sanitized fields: for WASM (map_uses_jsvalue), Map and Vec<Json> fields target JsValue
    // and need serde_wasm_bindgen::to_value() instead of iterator-based .collect().
    // Note: Vec<String> sanitized does NOT use the JsValue path because Vec<String> maps to
    // Vec<String> in WASM (not JsValue) — use the normal sanitized iterator path instead.
    if sanitized {
        if config.map_uses_jsvalue {
            // Map(String, String) sanitized → JsValue (HashMap maps to JsValue in WASM)
            // Use js_sys::JSON::parse(json_str) to get a plain JS object (not ES6 Map).
            if let TypeRef::Map(k, v) = ty {
                if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String) {
                    if optional {
                        return format!(
                            "{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok()).and_then(|s| js_sys::JSON::parse(&s).ok())"
                        );
                    }
                    return format!(
                        "{name}: js_sys::JSON::parse(&serde_json::to_string(&val.{name}).unwrap_or_default()).unwrap_or(JsValue::NULL)"
                    );
                }
            }
            // Vec<Json> sanitized → JsValue (Vec<Json> maps to JsValue in WASM via nested-vec path)
            if let TypeRef::Vec(inner) = ty {
                if matches!(inner.as_ref(), TypeRef::Json) {
                    if optional {
                        return format!(
                            "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())"
                        );
                    }
                    return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
                }
            }
            // Vec<Vec<String>> sanitized from Vec<(String, String)> → JsValue (nested vec maps to JsValue in WASM)
            // Destructure tuples to 2-element Vecs and serialize to JsValue via serde_wasm_bindgen.
            if let TypeRef::Vec(outer_inner) = ty {
                if let TypeRef::Vec(inner) = outer_inner.as_ref() {
                    if matches!(inner.as_ref(), TypeRef::String) {
                        if optional {
                            return format!(
                                "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(&v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()).ok())"
                            );
                        }
                        return format!(
                            "{name}: serde_wasm_bindgen::to_value(&val.{name}.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()).unwrap_or(JsValue::NULL)"
                        );
                    }
                }
            }
            // Optional<Vec<Vec<String>>> sanitized from Option<Vec<(String, String)>> → Option<JsValue>
            if let TypeRef::Optional(opt_inner) = ty {
                if let TypeRef::Vec(outer_inner) = opt_inner.as_ref() {
                    if let TypeRef::Vec(inner) = outer_inner.as_ref() {
                        if matches!(inner.as_ref(), TypeRef::String) {
                            return format!(
                                "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(&v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>()).ok())"
                            );
                        }
                    }
                }
            }
        }
        return field_conversion_from_core(name, ty, optional, sanitized, opaque_types);
    }

    // Tagged-data enum field (WASM only; binding stores JsValue / Option<JsValue> instead of
    // WasmEnum / Option<WasmEnum>). Handles bare Named, Option<Named>, Vec<Named>, and
    // Option<Vec<Named>> so the JS side always receives the serde-tagged wire shape
    // (plain objects) rather than wasm-bindgen class instances.
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
                // Bare TaggedDataEnum: serialize to JsValue directly.
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
                }
                return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
            }
            if optional_named {
                // Option<TaggedDataEnum>: serialize when Some, yield Option<JsValue>.
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
            }
            if vec_named {
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
                }
                return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
            }
            if optional_vec_named {
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
            }
        }
    }

    // Text-field content union (core holds the typed enum, binding holds the display text String):
    // render via the core type's Display impl.  Handles direct and Optional wrappings.
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

    // Untagged data enum field (core holds the typed enum, binding holds serde_json::Value):
    // serialize via serde_json::to_value.  Handles direct, Optional, and Vec wrappings.
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
        if vec_named {
            if optional {
                return format!(
                    "{name}: val.{name}.as_ref().map(|v| v.iter().filter_map(|x| serde_json::to_value(x).ok()).collect())"
                );
            }
            return format!("{name}: val.{name}.iter().filter_map(|x| serde_json::to_value(x).ok()).collect()");
        }
        if optional_vec_named {
            return format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().filter_map(|x| serde_json::to_value(x).ok()).collect())"
            );
        }
    }

    // Vec<Named>→String core→binding: binding holds JSON string, core has Vec<Named>.
    // Only apply serde round-trip for Vec<Named> types (complex structs that can't cross FFI).
    // Vec<String>, Vec<Primitive>, etc. stay as-is since they map directly.
    if config.vec_named_to_string {
        if let TypeRef::Vec(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Named(_)) {
                if optional {
                    return format!("{name}: val.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok())");
                }
                return format!("{name}: serde_json::to_string(&val.{name}).unwrap_or_default()");
            }
        }
    }

    // Map→String core→binding: binding holds Debug-formatted string, core has HashMap.
    // Used by Rustler (Elixir NIFs) where HashMap cannot cross the NIF boundary directly.
    if config.map_as_string && matches!(ty, TypeRef::Map(_, _)) {
        if optional {
            return format!("{name}: val.{name}.as_ref().map(|m| format!(\"{{m:?}}\"))");
        }
        return format!("{name}: format!(\"{{:?}}\", val.{name})");
    }
    if config.map_as_string {
        if let TypeRef::Optional(inner) = ty {
            if matches!(inner.as_ref(), TypeRef::Map(_, _)) {
                return format!("{name}: val.{name}.as_ref().map(|m| format!(\"{{m:?}}\"))");
            }
        }
    }

    // WASM JsValue: use js_sys::JSON::parse for Map types (produces plain JS objects, not ES6
    // Maps which serde_wasm_bindgen would produce for serialize_map calls). Use
    // serde_wasm_bindgen for nested Vec types.
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
        // i64 casting for large int primitives
        TypeRef::Primitive(p) if config.cast_large_ints_to_i64 && needs_i64_cast(p) => {
            let cast_to = binding_prim_str(p);
            if optional {
                format!("{name}: val.{name}.map(|v| v as {cast_to})")
            } else {
                format!("{name}: val.{name} as {cast_to}")
            }
        }
        // Optional(large_int) with i64 casting
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
        // i32 casting for small uint primitives (extendr/R only)
        TypeRef::Primitive(p) if config.cast_uints_to_i32 && needs_i32_cast(p) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as i32)")
            } else {
                format!("{name}: val.{name} as i32")
            }
        }
        // Optional(small_uint) with i32 casting
        TypeRef::Optional(inner)
            if config.cast_uints_to_i32 && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i32_cast(p)) =>
        {
            format!("{name}: val.{name}.map(|v| v as i32)")
        }
        // Vec<u8/u16/u32/i8/i16> needs element-wise core→i32 casting (extendr/R only)
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
        // f64 casting for large int primitives (extendr/R only)
        TypeRef::Primitive(p) if config.cast_large_ints_to_f64 && needs_f64_cast(p) => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as f64)")
            } else {
                format!("{name}: val.{name} as f64")
            }
        }
        // Optional(large_int) with f64 casting
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            format!("{name}: val.{name}.map(|v| v as f64)")
        }
        // Vec<usize/u64/i64/isize/f32> needs element-wise f64 cast for extendr/R backend
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
        // Optional(Vec(usize/u64/i64/isize/f32)) needs element-wise f64 cast
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p))) =>
        {
            format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
        }
        // Vec<Vec<usize/u64/i64/isize/f32>> needs nested element-wise f64 cast (embeddings)
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
        // Optional(Vec<Vec<usize/u64/i64/isize/f32>>) needs nested element-wise f64 cast
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(prim) if matches!(prim.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)))) =>
        {
            format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect())"
            )
        }
        // Map values that are usize/u64/i64/isize/f32 stored as f64 in binding → cast when reading core
        TypeRef::Map(_k, v)
            if config.cast_large_ints_to_f64 && matches!(v.as_ref(), TypeRef::Primitive(p) if needs_f64_cast(p)) =>
        {
            if optional {
                format!("{name}: val.{name}.as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), *v as f64)).collect())")
            } else {
                format!("{name}: val.{name}.iter().map(|(k, v)| (k.clone(), *v as f64)).collect()")
            }
        }
        // Duration with f64 casting (R: no u64, use f64 millis)
        TypeRef::Duration if config.cast_large_ints_to_f64 => {
            if optional {
                format!("{name}: val.{name}.map(|d| d.as_millis() as f64)")
            } else {
                format!("{name}: val.{name}.as_millis() as f64")
            }
        }
        // f32→f64 casting (NAPI only)
        TypeRef::Primitive(PrimitiveType::F32) if config.cast_f32_to_f64 => {
            if optional {
                format!("{name}: val.{name}.map(|v| v as f64)")
            } else {
                format!("{name}: val.{name} as f64")
            }
        }
        // Duration with i64 casting
        TypeRef::Duration if config.cast_large_ints_to_i64 => {
            if optional {
                format!("{name}: val.{name}.map(|d| d.as_millis() as u64 as i64)")
            } else {
                format!("{name}: val.{name}.as_millis() as u64 as i64")
            }
        }
        // Opaque Named types with prefix: wrap in Arc with prefixed binding name
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) && !prefix.is_empty() => {
            let prefixed = format!("{prefix}{n}");
            if optional {
                format!("{name}: val.{name}.map(|v| {prefixed} {{ inner: Arc::new(v) }})")
            } else {
                format!("{name}: {prefixed} {{ inner: Arc::new(val.{name}) }}")
            }
        }
        // Enum-to-String Named types (PHP pattern)
        TypeRef::Named(n) if is_enum_string(n) => {
            // Use serde serialization to get the correct serde(rename) value, not Debug format.
            // serde_json::to_value gives Value::String("auto") which we extract.
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
        // Vec<Enum-to-String> Named types: element-wise serde serialization
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
        // Optional(Vec<Enum-to-String>) Named types (PHP pattern)
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(n) if is_enum_string(n))) =>
        {
            format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|x| serde_json::to_value(x).ok().and_then(|s| s.as_str().map(String::from)).unwrap_or_default()).collect())"
            )
        }
        // Vec<f32> needs element-wise cast to f64 when f32→f64 mapping is active
        TypeRef::Vec(inner)
            if config.cast_f32_to_f64 && matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::F32)) =>
        {
            if optional {
                format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
            } else {
                format!("{name}: val.{name}.iter().map(|&v| v as f64).collect()")
            }
        }
        // Optional(Vec(f32)) needs element-wise cast to f64
        TypeRef::Optional(inner)
            if config.cast_f32_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(PrimitiveType::F32))) =>
        {
            format!("{name}: val.{name}.as_ref().map(|v| v.iter().map(|&x| x as f64).collect())")
        }
        // Optional(Vec(u64/usize/isize)) needs element-wise i64 casting
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p))) =>
        {
            if let TypeRef::Vec(vi) = inner.as_ref() {
                if let TypeRef::Primitive(p) = vi.as_ref() {
                    let cast_to = binding_prim_str(p);
                    if sanitized {
                        // Sanitized from Option<(T, T)> → Option<Vec<T>>: destructure tuple
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
        // Vec<Vec<f32>> needs nested element-wise cast to f64 (for embeddings, etc.)
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
        // Optional(Vec<Vec<f32>>) needs nested element-wise cast to f64
        TypeRef::Optional(inner)
            if config.cast_f32_to_f64
                && matches!(inner.as_ref(), TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(prim) if matches!(prim.as_ref(), TypeRef::Primitive(PrimitiveType::F32)))) =>
        {
            format!(
                "{name}: val.{name}.as_ref().map(|v| v.iter().map(|inner| inner.iter().map(|&x| x as f64).collect()).collect())"
            )
        }
        // Optional with i64-cast inner
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
        // HashMap value type casting: when value type needs i64 casting
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
        // Vec<u64/usize/isize> needs element-wise i64 casting (core→binding)
        TypeRef::Vec(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let cast_to = binding_prim_str(p);
                if sanitized {
                    // Sanitized from tuple (T, T) → Vec<T>: destructure tuple into vec
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
        // Vec<Vec<u64/usize/isize>> needs nested element-wise i64 casting (core→binding)
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
        // Json→String: core uses serde_json::Value, binding uses String (PHP)
        TypeRef::Json if config.json_to_string => {
            if optional {
                format!("{name}: val.{name}.as_ref().map(ToString::to_string)")
            } else {
                format!("{name}: val.{name}.to_string()")
            }
        }
        // Json stays as serde_json::Value: identity passthrough.
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
        // Json→JsValue: core uses serde_json::Value, binding uses JsValue (WASM)
        TypeRef::Json if config.map_uses_jsvalue => {
            if optional {
                format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())")
            } else {
                format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)")
            }
        }
        // Vec<Json>→JsValue: core uses Vec<serde_json::Value>, binding uses JsValue (WASM)
        TypeRef::Vec(inner) if config.map_uses_jsvalue && matches!(inner.as_ref(), TypeRef::Json) => {
            if optional {
                format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())")
            } else {
                format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)")
            }
        }
        // Optional(Vec<Json>)→JsValue (WASM)
        TypeRef::Optional(inner)
            if config.map_uses_jsvalue
                && matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Json)) =>
        {
            format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())")
        }
        // Fall through to default (handles paths, opaque without prefix, etc.)
        _ => field_conversion_from_core(name, ty, optional, sanitized, opaque_types),
    }
}
