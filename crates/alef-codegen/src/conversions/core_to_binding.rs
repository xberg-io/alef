use ahash::AHashSet;
use alef_core::ir::{CoreWrapper, PrimitiveType, TypeDef, TypeRef};
use std::fmt::Write;

use super::ConversionConfig;
use super::binding_to_core::field_conversion_to_core;
use super::helpers::is_newtype;
use super::helpers::{binding_prim_str, core_type_path, needs_i64_cast};

/// Generate `impl From<core::Type> for BindingType` (core -> binding).
pub fn gen_from_core_to_binding(typ: &TypeDef, core_import: &str, opaque_types: &AHashSet<String>) -> String {
    gen_from_core_to_binding_cfg(typ, core_import, opaque_types, &ConversionConfig::default())
}

/// Generate `impl From<core::Type> for BindingType` with backend-specific config.
pub fn gen_from_core_to_binding_cfg(
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    config: &ConversionConfig,
) -> String {
    let core_path = core_type_path(typ, core_import);
    let binding_name = format!("{}{}", config.type_name_prefix, typ.name);
    let mut out = String::with_capacity(256);
    writeln!(out, "#[allow(clippy::redundant_closure, clippy::useless_conversion)]").ok();
    writeln!(out, "impl From<{core_path}> for {binding_name} {{").ok();
    writeln!(out, "    fn from(val: {core_path}) -> Self {{").ok();

    // Newtype structs: extract inner value with val.0
    if is_newtype(typ) {
        let field = &typ.fields[0];
        let inner_expr = match &field.ty {
            TypeRef::Named(_) => "val.0.into()".to_string(),
            TypeRef::Path => "val.0.to_string_lossy().to_string()".to_string(),
            TypeRef::Duration => "val.0.as_millis() as u64".to_string(),
            _ => "val.0".to_string(),
        };
        writeln!(out, "        Self {{ _0: {inner_expr} }}").ok();
        writeln!(out, "    }}").ok();
        write!(out, "}}").ok();
        return out;
    }

    let optionalized = config.optionalize_defaults && typ.has_default;
    writeln!(out, "        Self {{").ok();
    for field in &typ.fields {
        // Fields referencing excluded types are not present in the binding struct — skip
        if !config.exclude_types.is_empty()
            && super::helpers::field_references_excluded_type(&field.ty, config.exclude_types)
        {
            continue;
        }
        let base_conversion = field_conversion_from_core_cfg(
            &field.name,
            &field.ty,
            field.optional,
            field.sanitized,
            opaque_types,
            config,
        );
        // Box<T> fields: dereference before conversion.
        let base_conversion = if field.is_boxed && matches!(&field.ty, TypeRef::Named(_)) {
            if field.optional {
                // Optional<Box<T>>: replace .map(Into::into) with .map(|v| (*v).into())
                let src = format!("{}: val.{}.map(Into::into)", field.name, field.name);
                let dst = format!("{}: val.{}.map(|v| (*v).into())", field.name, field.name);
                if base_conversion == src { dst } else { base_conversion }
            } else {
                // Box<T>: replace `val.{name}` with `(*val.{name})`
                base_conversion.replace(&format!("val.{}", field.name), &format!("(*val.{})", field.name))
            }
        } else {
            base_conversion
        };
        // Newtype unwrapping: when the field was resolved from a newtype (e.g. NodeIndex → u32),
        // unwrap the core newtype by accessing `.0`.
        // e.g. `source: val.source` → `source: val.source.0`
        //      `parent: val.parent` → `parent: val.parent.map(|v| v.0)`
        //      `children: val.children` → `children: val.children.iter().map(|v| v.0).collect()`
        let base_conversion = if field.newtype_wrapper.is_some() {
            match &field.ty {
                TypeRef::Optional(_) => {
                    // Replace `val.{name}` with `val.{name}.map(|v| v.0)` in the generated expression
                    base_conversion.replace(
                        &format!("val.{}", field.name),
                        &format!("val.{}.map(|v| v.0)", field.name),
                    )
                }
                TypeRef::Vec(_) => {
                    // Replace `val.{name}` with `val.{name}.iter().map(|v| v.0).collect()` in expression
                    base_conversion.replace(
                        &format!("val.{}", field.name),
                        &format!("val.{}.iter().map(|v| v.0).collect::<Vec<_>>()", field.name),
                    )
                }
                // When `optional=true` and `ty` is a plain Primitive (not TypeRef::Optional), the core
                // field is actually `Option<NewtypeT>`, so we must use `.map(|v| v.0)` not `.0`.
                _ if field.optional => base_conversion.replace(
                    &format!("val.{}", field.name),
                    &format!("val.{}.map(|v| v.0)", field.name),
                ),
                _ => {
                    // Direct field: append `.0` to access the inner primitive
                    base_conversion.replace(&format!("val.{}", field.name), &format!("val.{}.0", field.name))
                }
            }
        } else {
            base_conversion
        };
        // When field.optional=true AND field.ty=Optional(T), the binding struct flattens
        // Option<Option<T>> to Option<T>. Core produces Option<Option<T>>, binding needs
        // Option<T>. Generate the conversion by treating the pre-flattened field as Option<T>:
        // call the standard conversion for the inner type T with optional=true, substituting
        // val.{name}.flatten() for val.{name} so all cast/conversion logic applies to T.
        let is_flattened_optional = field.optional && matches!(field.ty, TypeRef::Optional(_));
        let base_conversion = if is_flattened_optional {
            if let TypeRef::Optional(inner) = &field.ty {
                // Produce the conversion as if the field is Option<inner> with value val.name.flatten()
                let inner_conv = field_conversion_from_core_cfg(
                    &field.name,
                    inner.as_ref(),
                    true,
                    field.sanitized,
                    opaque_types,
                    config,
                );
                // inner_conv references val.{name}; replace with val.{name}.flatten()
                inner_conv.replace(&format!("val.{}", field.name), &format!("val.{}.flatten()", field.name))
            } else {
                base_conversion
            }
        } else {
            base_conversion
        };
        // Optionalized non-optional fields need Some() wrapping in core→binding direction.
        // This covers both NAPI-style full optionalization and PyO3-style Duration optionalization.
        // Flattened-optional fields are already handled above with the correct type.
        let needs_some_wrap = !is_flattened_optional
            && ((optionalized && !field.optional)
                || (config.option_duration_on_defaults
                    && typ.has_default
                    && !field.optional
                    && matches!(field.ty, TypeRef::Duration)));
        let conversion = if needs_some_wrap {
            // Extract the value expression after "name: " and wrap in Some()
            if let Some(expr) = base_conversion.strip_prefix(&format!("{}: ", field.name)) {
                format!("{}: Some({})", field.name, expr)
            } else {
                base_conversion
            }
        } else {
            base_conversion
        };
        // CoreWrapper: unwrap Arc, convert Cow→String, Bytes→Vec<u8>
        // Skip for sanitized fields since their conversion already handles the type mismatch via format!("{:?}", ...)
        let conversion = if !field.sanitized {
            apply_core_wrapper_from_core(
                &conversion,
                &field.name,
                &field.core_wrapper,
                &field.vec_inner_core_wrapper,
                field.optional,
            )
        } else {
            conversion
        };
        // Skip cfg-gated fields — they don't exist in the binding struct
        if field.cfg.is_some() {
            continue;
        }
        // In core→binding direction, the binding struct field may be keyword-escaped
        // (e.g. `class_` for `class`). The generated conversion has `field.name: expr`
        // on the left side — rename it to `binding_name: expr` when needed.
        let binding_field = config.binding_field_name_owned(&typ.name, &field.name);
        let conversion = if binding_field != field.name {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                format!("{binding_field}: {expr}")
            } else {
                conversion
            }
        } else {
            conversion
        };
        writeln!(out, "            {conversion},").ok();
    }

    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    write!(out, "}}").ok();
    out
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
        // Bytes: core uses bytes::Bytes, binding uses Vec<u8>
        TypeRef::Bytes => {
            if optional {
                format!("{name}: val.{name}.map(|v| v.to_vec())")
            } else {
                format!("{name}: val.{name}.to_vec()")
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
        // Map with Json values: core uses HashMap<K, serde_json::Value>, binding uses HashMap<K, String>
        TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Json) => {
            let k_is_json = matches!(k.as_ref(), TypeRef::Json);
            let k_expr = if k_is_json { "k.to_string()" } else { "k" };
            if optional {
                format!("{name}: val.{name}.map(|m| m.into_iter().map(|(k, v)| ({k_expr}, v.to_string())).collect())")
            } else {
                format!("{name}: val.{name}.into_iter().map(|(k, v)| ({k_expr}, v.to_string())).collect()")
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
            if let TypeRef::Map(k, v) = ty {
                if matches!(k.as_ref(), TypeRef::String) && matches!(v.as_ref(), TypeRef::String) {
                    if optional {
                        return format!(
                            "{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())"
                        );
                    }
                    return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
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
        }
        return field_conversion_from_core(name, ty, optional, sanitized, opaque_types);
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

    // WASM JsValue: use serde_wasm_bindgen for Map and nested Vec types
    if config.map_uses_jsvalue {
        let is_nested_vec = matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)));
        let is_map = matches!(ty, TypeRef::Map(_, _));
        if is_nested_vec || is_map {
            if optional {
                return format!("{name}: val.{name}.as_ref().and_then(|v| serde_wasm_bindgen::to_value(v).ok())");
            }
            return format!("{name}: serde_wasm_bindgen::to_value(&val.{name}).unwrap_or(JsValue::NULL)");
        }
        if let TypeRef::Optional(inner) = ty {
            let is_inner_nested = matches!(inner.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Vec(_)));
            let is_inner_map = matches!(inner.as_ref(), TypeRef::Map(_, _));
            if is_inner_nested || is_inner_map {
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

/// Apply CoreWrapper transformations for core→binding direction.
/// Unwraps Arc, converts Cow→String, Bytes→Vec<u8>.
fn apply_core_wrapper_from_core(
    conversion: &str,
    name: &str,
    core_wrapper: &CoreWrapper,
    vec_inner_core_wrapper: &CoreWrapper,
    optional: bool,
) -> String {
    // Handle Vec<Arc<T>>: unwrap Arc elements
    if *vec_inner_core_wrapper == CoreWrapper::Arc {
        return conversion
            .replace(".map(Into::into).collect()", ".map(|v| (*v).clone().into()).collect()")
            .replace(
                "map(|v| v.into_iter().map(Into::into)",
                "map(|v| v.into_iter().map(|v| (*v).clone().into())",
            );
    }

    match core_wrapper {
        CoreWrapper::None => conversion.to_string(),
        CoreWrapper::Cow => {
            // Cow<str> → String: core val.name is Cow, binding needs String
            // The conversion already emits "name: val.name" for strings which works
            // since Cow<str> derefs to &str and String: From<Cow<str>> exists.
            // But if it's "val.name" directly, add .into_owned() or .to_string()
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    // Already handled by map
                    conversion.to_string()
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.into_owned()")
                } else {
                    conversion.to_string()
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Arc => {
            // Arc<T> → T: unwrap via clone.
            //
            // Special case: opaque Named types build the binding wrapper with
            // `{ inner: Arc::new(v) }` in the base conversion, but when the core
            // field is `Arc<T>`, `v` IS already the `Arc<T>` — wrapping it again
            // with `Arc::new` produces `Arc<Arc<T>>`.  Detect this pattern and
            // replace `Arc::new(v)` with `v`, and `Arc::new(val.{name})` with
            // `val.{name}`, then return without adding an extra unwrap chain.
            if conversion.contains("{ inner: Arc::new(") {
                return conversion.replace("{ inner: Arc::new(v) }", "{ inner: v }").replace(
                    &format!("{{ inner: Arc::new(val.{name}) }}"),
                    &format!("{{ inner: val.{name} }}"),
                );
            }
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(|v| (*v).clone().into())")
                } else {
                    let unwrapped = expr.replace(&format!("val.{name}"), &format!("(*val.{name}).clone()"));
                    format!("{name}: {unwrapped}")
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::Bytes => {
            // Bytes → Vec<u8>: .to_vec()
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(|v| v.to_vec())")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.to_vec()")
                } else {
                    conversion.to_string()
                }
            } else {
                conversion.to_string()
            }
        }
        CoreWrapper::ArcMutex => {
            // Arc<Mutex<T>> → T: lock and clone
            if let Some(expr) = conversion.strip_prefix(&format!("{name}: ")) {
                if optional {
                    format!("{name}: {expr}.map(|v| v.lock().unwrap().clone().into())")
                } else if expr == format!("val.{name}") {
                    format!("{name}: val.{name}.lock().unwrap().clone().into()")
                } else {
                    conversion.to_string()
                }
            } else {
                conversion.to_string()
            }
        }
    }
}
