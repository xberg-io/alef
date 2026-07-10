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

/// Generate an Input DTO struct with feature-gate awareness.
/// exclude_types: list of types that don't compile in the target (e.g., LayoutDetectionConfig on WASM)
/// enabled_features: list of features enabled in the target's feature set
/// non_deserializable_type_names: names of IR types whose Rust definition does not
///   implement `serde::Deserialize` — typically trait objects, type aliases over
///   `dyn Trait`, or opaque handles. Fields referencing one of these by Named type
///   are emitted with `#[serde(skip)]` so the DTO derives `Deserialize` cleanly.
pub(in crate::backends::wasm::gen_bindings) fn gen_input_dto_for_type_with_cfg(
    type_name: &str,
    core_import: &str,
    type_def: &crate::core::ir::TypeDef,
    exclude_types: &[String],
    enabled_features: &[String],
    non_deserializable_type_names: &std::collections::HashSet<String>,
) -> (String, String) {
    let input_name = format!("{}Input", type_name);
    let core_path = format!("{}::{}", core_import, type_name);

    let fields: Vec<_> = crate::codegen::shared::binding_fields(&type_def.fields)
        .map(|f| {
            let field_references_excluded = field_references_excluded_type(&f.ty, exclude_types);
            let field_cfg = f.cfg.as_deref();

            let cfg_satisfied = if let Some(cfg_str) = field_cfg {
                cfg_condition_enabled(cfg_str, enabled_features)
            } else {
                true
            };

            let inner_ty = match &f.ty {
                crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            let field_references_non_deserializable = matches!(
                inner_ty,
                crate::core::ir::TypeRef::Named(name) if non_deserializable_type_names.contains(name)
            );

            let is_skipped = field_references_excluded || !cfg_satisfied || field_references_non_deserializable;

            let dto_ty = format!("Option<{}>", type_ref_to_dto_type(&f.ty, core_import));
            let camel_case_name = to_node_name(&f.name);

            minijinja::context! {
                name => &f.name,
                ty => &dto_ty,
                core_name => &f.name,
                serde_rename => &camel_case_name,
                conv => dto_field_conversion(&f.ty, f.sanitized, f.optional),
                cfg => field_cfg,
                is_skipped => is_skipped,
            }
        })
        .collect::<Vec<_>>();

    let code = if !fields.is_empty() || !type_def.fields.is_empty() {
        crate::backends::wasm::template_env::render(
            "gen_input_dto",
            minijinja::context! {
                input_name => &input_name,
                core_path => &core_path,
                fields => &fields,
                has_default => type_def.has_default,
            },
        )
    } else {
        String::new()
    };

    (code, input_name)
}

/// Convert a TypeRef to a DTO field type string.
///
/// `Named` types are core-qualified (`{core_import}::{name}`) because the DTO is
/// deserialized via serde and converted into the core type: the core type already
/// derives `Deserialize`, and emitting the bare name would leave it unresolved in
/// the binding crate (the wasm-mapped wrapper enum is not the DTO field type).
pub(super) fn type_ref_to_dto_type(ty: &crate::core::ir::TypeRef, core_import: &str) -> String {
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
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_to_dto_type(inner, core_import)),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_to_dto_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_dto_type(k, core_import),
            type_ref_to_dto_type(v, core_import)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
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
