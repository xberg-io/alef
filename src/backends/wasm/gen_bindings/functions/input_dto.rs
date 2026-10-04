//! Input DTO generation and conversion helpers for WASM function parameters.

use crate::backends::wasm::gen_bindings::{cfg_condition_enabled, field_references_excluded_type};
use crate::codegen::naming::to_node_name;

/// Check if a struct should have an Input DTO for JS object deserialization.
///
/// Input DTOs are needed to properly handle camelCase field name mapping via per-field
/// #[serde(rename)] attributes. This is necessary because serde_wasm_bindgen does not
/// honor container-level `rename_all` directives when deserializing from JsValue objects.
/// The decision is based on extracted wire metadata and field shape, not type-name suffixes.
pub(in crate::backends::wasm::gen_bindings) fn should_have_input_dto(type_def: &crate::core::ir::TypeDef) -> bool {
    type_def.has_default
        && type_def.has_serde
        && crate::codegen::shared::binding_fields(&type_def.fields).any(|field| {
            field.serde_rename.is_some()
                || type_def.serde_rename_all.is_some()
                || crate::codegen::naming::to_node_name(&field.name) != field.name
        })
}

/// Generate an Input DTO struct that deserializes from camelCase and converts to the core type.
/// Returns (input_dto_code, input_dto_name).
/// Reads actual struct fields from the `ApiSurface` TypeDef.
/// Accepts exclude_types and enabled_features to properly gate fields whose types
/// are not available in the target's feature set.
#[cfg(test)]
pub(in crate::backends::wasm::gen_bindings) fn gen_input_dto_for_type(
    type_name: &str,
    core_import: &str,
    type_def: &crate::core::ir::TypeDef,
) -> (String, String) {
    gen_input_dto_for_type_with_cfg(
        type_name,
        core_import,
        type_def,
        &[],
        &[],
        &std::collections::HashSet::new(),
    )
}

pub(in crate::backends::wasm::gen_bindings) struct InputDtoConfig<'a> {
    pub core_path: &'a str,
    pub core_import: &'a str,
    pub core_type_paths: &'a ahash::AHashMap<String, String>,
    pub exclude_types: &'a [String],
    pub enabled_features: &'a [String],
    pub non_deserializable_type_names: &'a std::collections::HashSet<String>,
}

pub(in crate::backends::wasm::gen_bindings) fn gen_input_dto_for_type_at_path(
    type_name: &str,
    core_path: &str,
    core_import: &str,
    core_type_paths: &ahash::AHashMap<String, String>,
    type_def: &crate::core::ir::TypeDef,
) -> (String, String) {
    gen_input_dto_for_type_with_cfg_at_path(
        type_name,
        type_def,
        InputDtoConfig {
            core_path,
            core_import,
            core_type_paths,
            exclude_types: &[],
            enabled_features: &[],
            non_deserializable_type_names: &std::collections::HashSet::new(),
        },
    )
}

/// Generate an Input DTO struct with feature-gate awareness.
/// exclude_types: list of types that don't compile in the target (e.g., LayoutDetectionConfig on WASM)
/// enabled_features: list of features enabled in the target's feature set
/// non_deserializable_type_names: names of IR types whose Rust definition does not
///   implement `serde::Deserialize` — typically trait objects, type aliases over
///   `dyn Trait`, or opaque handles. Fields referencing one of these by Named type
///   are emitted with `#[serde(skip)]` so the DTO derives `Deserialize` cleanly.
#[cfg(test)]
pub(in crate::backends::wasm::gen_bindings) fn gen_input_dto_for_type_with_cfg(
    type_name: &str,
    core_import: &str,
    type_def: &crate::core::ir::TypeDef,
    exclude_types: &[String],
    enabled_features: &[String],
    non_deserializable_type_names: &std::collections::HashSet<String>,
) -> (String, String) {
    let core_path = format!("{}::{}", core_import, type_name);
    gen_input_dto_for_type_with_cfg_at_path(
        type_name,
        type_def,
        InputDtoConfig {
            core_path: &core_path,
            core_import,
            core_type_paths: &ahash::AHashMap::new(),
            exclude_types,
            enabled_features,
            non_deserializable_type_names,
        },
    )
}

pub(in crate::backends::wasm::gen_bindings) fn gen_input_dto_for_type_with_cfg_at_path(
    type_name: &str,
    type_def: &crate::core::ir::TypeDef,
    config: InputDtoConfig<'_>,
) -> (String, String) {
    let input_name = format!("{}Input", type_name);

    let fields: Vec<_> = crate::codegen::shared::binding_fields(&type_def.fields)
        .map(|field| input_dto_field_context(field, &config))
        .collect::<Vec<_>>();

    let code = if !fields.is_empty() || !type_def.fields.is_empty() {
        crate::backends::wasm::template_env::render(
            "gen_input_dto",
            minijinja::context! {
                input_name => &input_name,
                core_path => config.core_path,
                fields => &fields,
                has_default => type_def.has_default,
            },
        )
    } else {
        String::new()
    };

    (code, input_name)
}

fn input_dto_field_context(field: &crate::core::ir::FieldDef, config: &InputDtoConfig<'_>) -> minijinja::Value {
    let field_cfg = field.cfg.as_deref();
    let cfg_satisfied = field_cfg.is_none_or(|cfg| cfg_condition_enabled(cfg, config.enabled_features));
    let inner_ty = match &field.ty {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    let is_skipped = field_references_excluded_type(&field.ty, config.exclude_types)
        || !cfg_satisfied
        || matches!(
            inner_ty,
            crate::core::ir::TypeRef::Named(name) if config.non_deserializable_type_names.contains(name)
        );
    let dto_ty = format!(
        "Option<{}>",
        type_ref_to_dto_type_with_paths(&field.ty, config.core_import, config.core_type_paths)
    );
    let wire_name = field.serde_rename.clone().unwrap_or_else(|| to_node_name(&field.name));
    let conversion = input_dto_field_conversion(field);

    minijinja::context! {
        name => &field.name,
        ty => &dto_ty,
        core_name => &field.name,
        serde_rename => &wire_name,
        conv => conversion,
        cfg => field_cfg,
        is_skipped => is_skipped,
    }
}

pub(super) fn input_dto_field_conversion(field: &crate::core::ir::FieldDef) -> String {
    let Some(wrapper) = field.newtype_wrapper.as_deref() else {
        return dto_field_conversion(&field.ty, field.sanitized, field.optional);
    };
    let source = if field.optional { "Some(v)" } else { "v" };
    let converted = crate::backends::wasm::gen_bindings::types::types_helpers::apply_newtype_to_core(
        source,
        &field.ty,
        field.optional,
        wrapper,
    );
    if !field.is_boxed || !crate::codegen::conversions::helpers::is_explicit_newtype(wrapper) {
        return converted;
    }
    if field.optional || matches!(field.ty, crate::core::ir::TypeRef::Optional(_)) {
        format!("({converted}).map(Box::new)")
    } else {
        format!("Box::new({converted})")
    }
}

fn type_ref_to_dto_type_with_paths(
    ty: &crate::core::ir::TypeRef,
    core_import: &str,
    core_type_paths: &ahash::AHashMap<String, String>,
) -> String {
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "bool".to_string(),
            crate::core::ir::PrimitiveType::U8 => "u8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "u16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "u32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "u64".to_string(),
            crate::core::ir::PrimitiveType::I8 => "i8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "i16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "i32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "i64".to_string(),
            crate::core::ir::PrimitiveType::F32 => "f32".to_string(),
            crate::core::ir::PrimitiveType::F64 => "f64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "usize".to_string(),
            crate::core::ir::PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Vec(inner) => format!(
            "Vec<{}>",
            type_ref_to_dto_type_with_paths(inner, core_import, core_type_paths)
        ),
        TypeRef::Optional(inner) => format!(
            "Option<{}>",
            type_ref_to_dto_type_with_paths(inner, core_import, core_type_paths)
        ),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_dto_type_with_paths(k, core_import, core_type_paths),
            type_ref_to_dto_type_with_paths(v, core_import, core_type_paths)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => core_type_paths
            .get(n)
            .cloned()
            .unwrap_or_else(|| format!("{core_import}::{n}")),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Build the conversion expression turning a present DTO field value (bound as
/// the variable `v`) into the core struct field value.
///
/// Most field types convert with a plain `v.into()`: identity for matching
/// types, and `Option<T>: From<T>` papers over a core field that is `Option<_>`
/// while the DTO holds the bare `T`. Two core types have no such blanket `From`
/// from their DTO spelling and need an explicit constructor first:
/// `Duration` (DTO `u64` milliseconds) and `PathBuf` (DTO `String`). Wrapping
/// the constructed value in `Into::into` keeps the same optional-field papering
/// as the default branch, so the expression is valid whether the core field is
/// `T` or `Option<T>`.
///
/// When a field is sanitized (e.g., `Option<ConcurrencyConfig>` represented as
/// `Option<String>` for JSON serialization), use JSON deserialization instead
/// of `.into()`, which doesn't impl for the target type.
pub(super) fn dto_field_conversion(ty: &crate::core::ir::TypeRef, sanitized: bool, optional: bool) -> String {
    use crate::core::ir::TypeRef;
    let wrap_optional = |expr: &str| -> String {
        if optional {
            format!("Some({expr})")
        } else {
            expr.to_string()
        }
    };
    match ty {
        TypeRef::Duration => "Into::into(std::time::Duration::from_millis(v))".to_string(),
        TypeRef::Path => "Into::into(std::path::PathBuf::from(v))".to_string(),
        TypeRef::Char => "Into::into(v.chars().next().unwrap_or('\\0'))".to_string(),
        TypeRef::String if sanitized => "serde_json::from_str(&v).unwrap_or_default()".to_string(),
        TypeRef::Vec(_) => wrap_optional("v.into_iter().collect()"),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)) => {
            "v.map(|items| items.into_iter().collect())".to_string()
        }
        _ => "v.into()".to_string(),
    }
}
