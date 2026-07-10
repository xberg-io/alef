use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::helpers::{
    core_prim_str, is_tuple_type_name, needs_f64_cast, needs_i32_cast, needs_i64_cast,
};
use crate::core::ir::{PrimitiveType, TypeRef};

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
            TypeRef::Vec(_) => {
                format!("{name}: val.{name}.map(|v| v.into_iter().collect())")
            }
            _ => format!("{name}: val.{name}"),
        },
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
                    return format!(
                        "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())"
                    );
                }
                return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
            }
            if optional_named {
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
    if config.json_to_string && matches!(ty, TypeRef::Json) {
        return format!("{name}: Default::default()");
    }
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
