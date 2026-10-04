//! Parameter type formatting and serde recovery call argument helpers.

use crate::codegen::generators;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};

use crate::backends::wasm::type_map::WasmMapper;

pub(super) fn mapped_type_contains_jsvalue(mapped_ty: &str) -> bool {
    if crate::codegen::shared::maps_to_js_value(mapped_ty) {
        return true;
    }
    let mapped_ty = mapped_ty.trim();
    ["Option<", "Vec<"].iter().any(|prefix| {
        mapped_ty
            .strip_prefix(prefix)
            .and_then(|inner| inner.strip_suffix('>'))
            .is_some_and(mapped_type_contains_jsvalue)
    })
}

fn mapped_type_is_optional_jsvalue(mapped_ty: &str) -> bool {
    let mapped_ty = mapped_ty.trim();
    mapped_ty
        .strip_prefix("Option<")
        .and_then(|inner| inner.strip_suffix('>'))
        .is_some_and(crate::codegen::shared::maps_to_js_value)
}

pub(in crate::backends::wasm::gen_bindings) fn typeref_to_core_type_str(
    ty: &TypeRef,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &AHashMap<String, String>,
) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
            PrimitiveType::Usize => "usize".to_string(),
            PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Vec(inner) => format!(
            "Vec<{}>",
            typeref_to_core_type_str(inner, core_import, source_crate_remaps, type_paths)
        ),
        TypeRef::Optional(inner) => format!(
            "Option<{}>",
            typeref_to_core_type_str(inner, core_import, source_crate_remaps, type_paths)
        ),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_core_type_str(k, core_import, source_crate_remaps, type_paths),
            typeref_to_core_type_str(v, core_import, source_crate_remaps, type_paths)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => {
            let path = if n.contains("::") {
                n.clone()
            } else {
                type_paths
                    .get(n)
                    .cloned()
                    .unwrap_or_else(|| format!("{core_import}::{n}"))
            };
            crate::codegen::conversions::apply_crate_remaps(&path, source_crate_remaps)
        }
        TypeRef::Unit => "()".to_string(),
    }
}

pub(in crate::backends::wasm::gen_bindings) fn wasm_type_path_map(
    api: &ApiSurface,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
) -> AHashMap<String, String> {
    let mut paths = AHashMap::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        paths.insert(
            typ.name.clone(),
            crate::codegen::conversions::core_type_path_remapped(typ, core_import, source_crate_remaps),
        );
    }
    for enum_def in &api.enums {
        paths.insert(
            enum_def.name.clone(),
            crate::codegen::conversions::core_enum_path_remapped(enum_def, core_import, source_crate_remaps),
        );
    }
    paths
}

pub(in crate::backends::wasm::gen_bindings) fn wasm_mapped_param_type(param: &ParamDef, mapper: &WasmMapper) -> String {
    if param
        .newtype_wrapper
        .as_deref()
        .is_some_and(super::super::types::types_helpers::complex_newtype_wrapper_uses_jsvalue)
    {
        "JsValue".to_string()
    } else {
        mapper.map_type(&param.ty)
    }
}

/// Prepare parameters whose binding shape differs from the core call shape.
///
/// `JsValue`-mapped containers must first be deserialized into their resolved binding-native
/// representation. Borrowed wrappers additionally need a named local because the core API takes
/// `&Wrapper` / `&mut Wrapper`, and borrowing a conversion temporary is not a stable call shape. ~keep
pub(in crate::backends::wasm::gen_bindings) fn wasm_newtype_param_bindings(
    params: &[crate::core::ir::ParamDef],
    mapper: &WasmMapper,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &AHashMap<String, String>,
) -> String {
    let mut bindings = String::new();
    for param in params {
        let mapped_ty = wasm_mapped_param_type(param, mapper);
        let contains_jsvalue = mapped_type_contains_jsvalue(&mapped_ty);
        let is_jsvalue_container =
            matches!(param.ty, TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Optional(_)) && contains_jsvalue;
        if contains_jsvalue && (param.newtype_wrapper.is_some() || is_jsvalue_container) {
            let core_ty = typeref_to_core_type_str(&param.ty, core_import, source_crate_remaps, type_paths);
            let intrinsic_optional = mapped_type_is_optional_jsvalue(&mapped_ty);
            if param.optional && intrinsic_optional {
                bindings.push_str(&format!(
                    "let {name}: Option<{core_ty}> = {name}.map(|value| value.map(|value| \
                     serde_wasm_bindgen::from_value(value).unwrap_or_else(|error| \
                     wasm_bindgen::throw_val(wasm_bindgen::JsValue::from_str(&error.to_string()))));\n    ",
                    name = param.name,
                ));
            } else if param.optional {
                bindings.push_str(&format!(
                    "let {name}: Option<{core_ty}> = {name}.map(|value| \
                     serde_wasm_bindgen::from_value(value).unwrap_or_else(|error| \
                     wasm_bindgen::throw_val(wasm_bindgen::JsValue::from_str(&error.to_string()))));\n    ",
                    name = param.name,
                ));
            } else if intrinsic_optional {
                bindings.push_str(&format!(
                    "let {name}: {core_ty} = {name}.map(|value| serde_wasm_bindgen::from_value(value)\
                     .unwrap_or_else(|error| wasm_bindgen::throw_val(\
                     wasm_bindgen::JsValue::from_str(&error.to_string())));\n    ",
                    name = param.name,
                ));
            } else {
                bindings.push_str(&format!(
                    "let {name}: {core_ty} = serde_wasm_bindgen::from_value({name})\
                     .unwrap_or_else(|error| wasm_bindgen::throw_val(\
                     wasm_bindgen::JsValue::from_str(&error.to_string())));\n    ",
                    name = param.name,
                ));
            }
        }

        if param.is_ref
            && let Some(wrapper) = param.newtype_wrapper.as_deref()
        {
            let converted = super::super::types::types_helpers::apply_newtype_to_core(
                &param.name,
                &param.ty,
                param.optional,
                wrapper,
            );
            let mut_kw = if param.is_mut { "mut " } else { "" };
            bindings.push_str(&format!(
                "let {mut_kw}{name}_newtype = {converted};\n    ",
                name = param.name,
            ));
        }
    }
    bindings
}

pub(in crate::backends::wasm::gen_bindings) fn wasm_call_args(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    use_named_let_bindings: bool,
) -> String {
    let has_borrowed_wrapper = params
        .iter()
        .any(|param| param.newtype_wrapper.is_some() && param.is_ref);
    let has_root_optional_owned_wrapper = params.iter().enumerate().any(|(index, param)| {
        !param.is_ref
            && (param.optional || matches!(param.ty, TypeRef::Optional(_)))
            && !crate::codegen::shared::is_promoted_optional(params, index)
            && param
                .newtype_wrapper
                .as_deref()
                .and_then(super::super::types::types_helpers::root_optional_transparent_constructor)
                .is_some()
    });
    if !has_borrowed_wrapper && !has_root_optional_owned_wrapper {
        return if use_named_let_bindings {
            generators::gen_call_args_with_let_bindings_no_promote(params, opaque_types)
        } else {
            generators::gen_call_args_no_promote(params, opaque_types)
        };
    }

    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            if param.newtype_wrapper.is_some() && param.is_ref {
                if param.optional {
                    let accessor = if param.is_mut { "as_mut" } else { "as_ref" };
                    format!("{}_newtype.{accessor}()", param.name)
                } else if param.is_mut {
                    format!("&mut {}_newtype", param.name)
                } else {
                    format!("&{}_newtype", param.name)
                }
            } else if (param.optional || matches!(param.ty, TypeRef::Optional(_)))
                && !crate::codegen::shared::is_promoted_optional(params, index)
                && let Some(wrapper) = param.newtype_wrapper.as_deref()
                && super::super::types::types_helpers::root_optional_transparent_constructor(wrapper).is_some()
            {
                super::super::types::types_helpers::apply_newtype_to_core(
                    &param.name,
                    &param.ty,
                    param.optional,
                    wrapper,
                )
            } else if use_named_let_bindings {
                generators::gen_call_args_with_let_bindings_no_promote(std::slice::from_ref(param), opaque_types)
            } else {
                generators::gen_call_args_no_promote(std::slice::from_ref(param), opaque_types)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Helper: format a parameter, prefixing with _ if unused
pub(in crate::backends::wasm::gen_bindings) fn format_param_unused(name: &str, ty: &str, unused: bool) -> String {
    let prefix = if unused { "_" } else { "" };
    format!("{}{}: {}", prefix, name, ty)
}

/// Returns a type name in turbofish form for use before `::from(expr)`.
///
/// Rust requires turbofish when a type has generic parameters and sits before `::`:
///   `Vec<T>::from(x)` is a syntax error — `Vec::<T>::from(x)` is required.
pub(super) fn wasm_serde_recovery_call_args(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| {
            if p.newtype_wrapper.is_some() && p.is_ref {
                return if p.optional {
                    let accessor = if p.is_mut { "as_mut" } else { "as_ref" };
                    format!("{}_newtype.{accessor}()", p.name)
                } else if p.is_mut {
                    format!("&mut {}_newtype", p.name)
                } else {
                    format!("&{}_newtype", p.name)
                };
            }
            match &p.ty {
                TypeRef::Vec(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref && !p.optional =>
                {
                    format!("&{}_refs", p.name)
                }
                _ => generators::gen_call_args_with_let_bindings(std::slice::from_ref(p), opaque_types),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Prefix `&` onto an opaque-handle parameter's mapped type.
///
/// wasm-bindgen's JS glue for a by-value exported-struct argument calls
/// `arg.__destroy_into_raw()`, which nulls the JS object's `__wbg_ptr`. A handle passed by value is
/// therefore dead after a single call and every later use throws `null pointer passed to rust`
/// after a single call. Taking it by reference makes the glue pass `arg.__wbg_ptr` instead and
/// leaves the object alive. This holds for exported `async fn`s too: wasm-bindgen routes `&T`
/// through `LongRefFromWasmAbi`, which needs neither `Clone` nor a borrow held across the await,
/// and the emitted `.d.ts` signature is byte-identical to the by-value one. Verified against
/// wasm-bindgen 0.2.128. Mirrors the NAPI backend's `&{prefix}{n}` in
/// `backends::napi::gen_bindings::functions`. ~keep
///
/// `Option<T>` is left alone: `Option<&T>` is not a supported wasm-bindgen argument shape, and the
/// call-argument generator already emits `{name}.as_ref().map(..)` for that case.
pub(in crate::backends::wasm::gen_bindings) fn borrow_opaque_param(
    param: &crate::core::ir::ParamDef,
    mapped_type: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    match &param.ty {
        TypeRef::Named(name)
            if opaque_types.contains(name.as_str())
                && !param.optional
                && !crate::codegen::shared::maps_to_js_value(mapped_type) =>
        {
            format!("&{mapped_type}")
        }
        _ => mapped_type.to_string(),
    }
}
