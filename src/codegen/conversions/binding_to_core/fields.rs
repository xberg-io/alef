use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::helpers::{
    core_prim_str, is_tuple_type_name, needs_f64_cast, needs_i32_cast, needs_i64_cast,
};
use crate::core::ir::{PrimitiveType, TypeRef};

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

    // Text-field content union (binding holds the display text String, core holds the typed enum):
    // deserialise the string into the union's text variant via serde (an untagged content union
    // accepts a bare JSON string).  Handles direct and Optional wrappings.
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
