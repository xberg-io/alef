use alef_core::ir::{CoreWrapper, PrimitiveType, TypeDef, TypeRef};
use std::fmt::Write;

use super::ConversionConfig;
use super::helpers::{core_prim_str, core_type_path, is_newtype, is_tuple_type_name, needs_i64_cast};

/// Generate `impl From<BindingType> for core::Type` (binding -> core).
/// Sanitized fields use `Default::default()` (lossy but functional).
pub fn gen_from_binding_to_core(typ: &TypeDef, core_import: &str) -> String {
    gen_from_binding_to_core_cfg(typ, core_import, &ConversionConfig::default())
}

/// Generate `impl From<BindingType> for core::Type` with backend-specific config.
pub fn gen_from_binding_to_core_cfg(typ: &TypeDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_type_path(typ, core_import);
    let binding_name = format!("{}{}", config.type_name_prefix, typ.name);
    let mut out = String::with_capacity(256);
    // When cfg-gated fields exist, ..Default::default() fills them when the feature is enabled.
    // When disabled, all fields are already specified and the update has no effect — suppress lint.
    if typ.has_stripped_cfg_fields {
        writeln!(out, "#[allow(clippy::needless_update)]").ok();
    }
    // Suppress clippy when we use the builder pattern (Default + field reassignment)
    let uses_builder_pattern = config.option_duration_on_defaults
        && typ.has_default
        && typ
            .fields
            .iter()
            .any(|f| !f.optional && matches!(f.ty, TypeRef::Duration));
    if uses_builder_pattern {
        writeln!(out, "#[allow(clippy::field_reassign_with_default)]").ok();
    }
    writeln!(out, "impl From<{binding_name}> for {core_path} {{").ok();
    writeln!(out, "    fn from(val: {binding_name}) -> Self {{").ok();

    // Newtype structs: generate tuple constructor Self(val._0)
    if is_newtype(typ) {
        let field = &typ.fields[0];
        let inner_expr = match &field.ty {
            TypeRef::Named(_) => "val._0.into()".to_string(),
            TypeRef::Path => "val._0.into()".to_string(),
            TypeRef::Duration => "std::time::Duration::from_millis(val._0)".to_string(),
            _ => "val._0".to_string(),
        };
        writeln!(out, "        Self({inner_expr})").ok();
        writeln!(out, "    }}").ok();
        write!(out, "}}").ok();
        return out;
    }

    // When option_duration_on_defaults is set for a has_default type, non-optional Duration
    // fields are stored as Option<u64> in the binding struct.  We use the builder pattern
    // so that None falls back to the core type's Default (giving the real field default,
    // e.g. Duration::from_millis(30000)) rather than Duration::ZERO.
    let has_optionalized_duration = config.option_duration_on_defaults
        && typ.has_default
        && typ
            .fields
            .iter()
            .any(|f| !f.optional && matches!(f.ty, TypeRef::Duration));

    if has_optionalized_duration {
        // Builder pattern: start from core default, override explicitly-set fields.
        writeln!(out, "        let mut __result = {core_path}::default();").ok();
        let optionalized = config.optionalize_defaults && typ.has_default;
        for field in &typ.fields {
            // Skip cfg-gated fields — they don't exist in the binding struct.
            if field.cfg.is_some() {
                continue;
            }
            if field.sanitized {
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
            if !field.optional && matches!(field.ty, TypeRef::Duration) {
                let cast = if config.cast_large_ints_to_i64 { " as u64" } else { "" };
                writeln!(
                    out,
                    "        if let Some(__v) = val.{} {{ __result.{} = std::time::Duration::from_millis(__v{cast}); }}",
                    field.name, field.name
                )
                .ok();
                continue;
            }
            let conversion = if optionalized && !field.optional {
                gen_optionalized_field_to_core(&field.name, &field.ty, config)
            } else {
                field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config)
            };
            // Strip the "name: " prefix to get just the expression, then assign
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                writeln!(out, "        __result.{} = {};", field.name, expr).ok();
            }
        }
        writeln!(out, "        __result").ok();
        writeln!(out, "    }}").ok();
        write!(out, "}}").ok();
        return out;
    }

    writeln!(out, "        Self {{").ok();
    let optionalized = config.optionalize_defaults && typ.has_default;
    for field in &typ.fields {
        // Skip cfg-gated fields — they don't exist in the binding struct.
        // When the binding is compiled, these fields are absent, and accessing them would fail.
        // The ..Default::default() at the end fills in these fields when the core type is compiled
        // with the required feature enabled.
        if field.cfg.is_some() {
            continue;
        }
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
        let conversion = if field.sanitized || references_excluded {
            format!("{}: Default::default()", field.name)
        } else if optionalized && !field.optional {
            // Field was wrapped in Option<T> for JS ergonomics but core expects T.
            // Use unwrap_or_default() for simple types, unwrap_or_default() + into for Named.
            gen_optionalized_field_to_core(&field.name, &field.ty, config)
        } else {
            field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config)
        };
        // Newtype wrapping: when the field was resolved from a newtype (e.g. NodeIndex → u32),
        // wrap the binding value back into the newtype for the core struct.
        // e.g. `source: val.source` → `source: kreuzberg::NodeIndex(val.source)`
        //      `parent: val.parent` → `parent: val.parent.map(kreuzberg::NodeIndex)`
        //      `children: val.children` → `children: val.children.into_iter().map(kreuzberg::NodeIndex).collect()`
        let conversion = if let Some(newtype_path) = &field.newtype_wrapper {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                // When `optional=true` and `ty` is a plain Primitive (not TypeRef::Optional), the core
                // field is actually `Option<NewtypeT>`, so we must use `.map(NewtypeT)` not `NewtypeT(...)`.
                match &field.ty {
                    TypeRef::Optional(_) => format!("{}: ({expr}).map({newtype_path})", field.name),
                    TypeRef::Vec(_) => {
                        format!("{}: ({expr}).into_iter().map({newtype_path}).collect()", field.name)
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
        // CoreWrapper: apply Cow/Arc/Bytes wrapping for binding→core direction
        let conversion = apply_core_wrapper_to_core(
            &conversion,
            &field.name,
            &field.core_wrapper,
            &field.vec_inner_core_wrapper,
            field.optional,
        );
        writeln!(out, "            {conversion},").ok();
    }
    // Use ..Default::default() to fill cfg-gated fields stripped from the IR
    if typ.has_stripped_cfg_fields {
        writeln!(out, "            ..Default::default()").ok();
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
}

/// Generate field conversion for a non-optional field that was optionalized
/// (wrapped in Option<T>) in the binding struct for JS ergonomics.
pub(super) fn gen_optionalized_field_to_core(name: &str, ty: &TypeRef, config: &ConversionConfig) -> String {
    match ty {
        TypeRef::Json => {
            format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()")
        }
        TypeRef::Named(_) => {
            // Named type: unwrap Option, convert via .into(), or use Default
            format!("{name}: val.{name}.map(Into::into).unwrap_or_default()")
        }
        TypeRef::Primitive(PrimitiveType::F32) if config.cast_f32_to_f64 => {
            format!("{name}: val.{name}.map(|v| v as f32).unwrap_or(0.0)")
        }
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => {
            format!("{name}: val.{name}.unwrap_or(0.0)")
        }
        TypeRef::Primitive(p) if config.cast_large_ints_to_i64 && needs_i64_cast(p) => {
            let core_ty = core_prim_str(p);
            format!("{name}: val.{name}.map(|v| v as {core_ty}).unwrap_or_default()")
        }
        TypeRef::Optional(inner)
            if config.cast_large_ints_to_i64
                && matches!(inner.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) =>
        {
            if let TypeRef::Primitive(p) = inner.as_ref() {
                let core_ty = core_prim_str(p);
                format!("{name}: val.{name}.map(|v| v as {core_ty})")
            } else {
                field_conversion_to_core(name, ty, false)
            }
        }
        TypeRef::Duration if config.cast_large_ints_to_i64 => {
            format!("{name}: val.{name}.map(|v| std::time::Duration::from_millis(v as u64)).unwrap_or_default()")
        }
        TypeRef::Duration => {
            format!("{name}: val.{name}.map(std::time::Duration::from_millis).unwrap_or_default()")
        }
        TypeRef::Path => {
            format!("{name}: val.{name}.map(Into::into).unwrap_or_default()")
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Path) => {
            // Binding has Option<String>, core has Option<PathBuf>
            format!("{name}: val.{name}.map(|s| std::path::PathBuf::from(s))")
        }
        // Char: binding uses Option<String>, core uses char
        TypeRef::Char => {
            format!("{name}: val.{name}.and_then(|s| s.chars().next()).unwrap_or('*')")
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Json => {
                format!(
                    "{name}: val.{name}.map(|v| v.into_iter().filter_map(|s| serde_json::from_str(&s).ok()).collect()).unwrap_or_default()"
                )
            }
            TypeRef::Named(_) => {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect()).unwrap_or_default()")
            }
            TypeRef::Primitive(p) if config.cast_large_ints_to_i64 && needs_i64_cast(p) => {
                let core_ty = core_prim_str(p);
                format!(
                    "{name}: val.{name}.map(|v| v.into_iter().map(|x| x as {core_ty}).collect()).unwrap_or_default()"
                )
            }
            _ => format!("{name}: val.{name}.unwrap_or_default()"),
        },
        TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Json) => {
            // Map with Json values: binding uses HashMap<K, String>, core uses HashMap<K, serde_json::Value>
            let k_is_json = matches!(k.as_ref(), TypeRef::Json);
            let k_expr = if k_is_json {
                "serde_json::from_str(&k).unwrap_or_default()"
            } else {
                "k"
            };
            format!(
                "{name}: val.{name}.unwrap_or_default().into_iter().map(|(k, v)| ({k_expr}, serde_json::from_str(&v).unwrap_or(serde_json::json!(v)))).collect()"
            )
        }
        TypeRef::Map(k, _v) if matches!(k.as_ref(), TypeRef::Json) => {
            // Map with Json keys: binding uses HashMap<String, V>, core uses HashMap<serde_json::Value, V>
            format!(
                "{name}: val.{name}.unwrap_or_default().into_iter().map(|(k, v)| (serde_json::from_str(&k).unwrap_or_default(), v)).collect()"
            )
        }
        TypeRef::Map(_, _) => {
            // Collect to handle HashMap↔BTreeMap conversion
            format!("{name}: val.{name}.unwrap_or_default().into_iter().collect()")
        }
        _ => {
            // Simple types (primitives, String, etc): unwrap_or_default()
            format!("{name}: val.{name}.unwrap_or_default()")
        }
    }
}

/// Determine the field conversion expression for binding -> core.
pub fn field_conversion_to_core(name: &str, ty: &TypeRef, optional: bool) -> String {
    match ty {
        // Primitives, String, Bytes, Unit -- direct assignment
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Bytes | TypeRef::Unit => {
            format!("{name}: val.{name}")
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
        // Map with Json value type: binding uses HashMap<K, String>, core uses HashMap<K, Value>
        TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Json) => {
            let k_expr = if matches!(k.as_ref(), TypeRef::Json) {
                "serde_json::from_str(&k).unwrap_or_default()"
            } else {
                "k"
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
        // Optional with inner
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Json => format!("{name}: val.{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())"),
            TypeRef::Named(_) | TypeRef::Path => format!("{name}: val.{name}.map(Into::into)"),
            TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                format!("{name}: val.{name}.map(|v| v.into_iter().map(Into::into).collect())")
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
            _ => format!("{name}: val.{name}"),
        },
        // Map -- collect to handle HashMap↔BTreeMap conversion;
        // additionally convert Named keys/values via Into, Json values via serde.
        TypeRef::Map(k, v) => {
            let has_named_key = matches!(k.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n));
            let has_named_val = matches!(v.as_ref(), TypeRef::Named(n) if !is_tuple_type_name(n));
            let has_json_val = matches!(v.as_ref(), TypeRef::Json);
            let has_json_key = matches!(k.as_ref(), TypeRef::Json);
            if has_json_val || has_json_key || has_named_key || has_named_val {
                let k_expr = if has_json_key {
                    "serde_json::from_str(&k).unwrap_or(serde_json::Value::String(k))"
                } else if has_named_key {
                    "k.into()"
                } else {
                    "k"
                };
                let v_expr = if has_json_val {
                    "serde_json::from_str(&v).unwrap_or(serde_json::Value::String(v))"
                } else if has_named_val {
                    "v.into()"
                } else {
                    "v"
                };
                if optional {
                    format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({k_expr}, {v_expr})).collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().map(|(k, v)| ({k_expr}, {v_expr})).collect()")
                }
            } else {
                // No conversion needed — just collect for potential HashMap↔BTreeMap type change
                if optional {
                    format!("{name}: val.{name}.map(|m| m.into_iter().collect())")
                } else {
                    format!("{name}: val.{name}.into_iter().collect()")
                }
            }
        }
    }
}

/// Binding→core field conversion with backend-specific config (i64 casts, etc.).
pub fn field_conversion_to_core_cfg(name: &str, ty: &TypeRef, optional: bool, config: &ConversionConfig) -> String {
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

    // Json→String binding→core: use Default::default() (lossy — can't parse String back)
    if config.json_to_string && matches!(ty, TypeRef::Json) {
        return format!("{name}: Default::default()");
    }
    // Json→JsValue binding→core: use serde_wasm_bindgen to convert (WASM)
    if config.map_uses_jsvalue && matches!(ty, TypeRef::Json) {
        if optional {
            return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::from_value(v.clone()).ok())");
        }
        return format!("{name}: serde_wasm_bindgen::from_value(val.{name}.clone()).unwrap_or_default()");
    }
    if !config.cast_large_ints_to_i64 && !config.cast_f32_to_f64 && !config.json_to_string {
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
        TypeRef::Map(k, v)
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
        // Fall through to default for everything else
        _ => field_conversion_to_core(name, ty, optional),
    }
}

/// Apply CoreWrapper transformations to a binding→core conversion expression.
/// Wraps the value expression with Arc::new(), .into() for Cow, etc.
fn apply_core_wrapper_to_core(
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
        CoreWrapper::Cow => {
            // Cow<str>: binding String → core Cow via .into()
            // The field_conversion already emits "name: val.name" for strings,
            // we need to add .into() to convert String → Cow<'static, str>
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
            // Bytes: binding Vec<u8> → core Bytes via .into()
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
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
    }
}
