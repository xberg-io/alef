//! Helpers for resolving default values for enum types.
//!
//! When a struct field has `#[serde(default)]` and its type is an enum,
//! backends need to materialize the default enum variant. This module provides
//! a shared function to extract the default variant name from the IR.

use crate::core::ir::{ApiSurface, EnumDef};
use ahash::AHashMap;

/// Metadata for materializing a default enum variant value in a target language.
#[derive(Clone, Debug)]
pub struct DefaultEnumVariant {
    /// The name of the default variant.
    pub variant_name: String,
    /// True if the variant has zero fields (unit/record with no args).
    /// Used by Java to determine whether to emit `new EnumName.Variant()` or just `EnumName.Variant`.
    pub is_zero_field: bool,
}

/// Find the default variant name for an enum.
///
/// Returns the name of the variant marked with `#[default]` if any,
/// otherwise returns the name of the first variant.
///
/// # Arguments
/// - `enum_def`: The enum definition from the IR
///
/// # Returns
/// The name of the default enum variant, or `None` if the enum has no variants.
pub fn default_variant_name(enum_def: &EnumDef) -> Option<String> {
    // First, try to find a variant explicitly marked with `#[default]`
    enum_def
        .variants
        .iter()
        .find(|v| v.is_default)
        .or_else(|| enum_def.variants.first())
        .map(|v| v.name.clone())
}

/// Build a lookup map of enum names to their default variant names.
///
/// This is useful for backends that need to materialize default enum values
/// when a struct field has `#[serde(default)]` but no explicit default value.
///
/// # Arguments
/// - `api`: The API surface containing enum definitions
///
/// # Returns
/// A map from enum name to default variant name. Only enums with variants are included.
pub fn enum_default_variants_map(api: &ApiSurface) -> AHashMap<String, String> {
    api.enums
        .iter()
        .filter_map(|enum_def| default_variant_name(enum_def).map(|variant_name| (enum_def.name.clone(), variant_name)))
        .collect()
}

/// Build a lookup map of enum names to their default variant metadata.
///
/// This variant includes information about whether the default variant has zero fields,
/// which is needed by Java to determine whether to emit `new EnumName.Variant()` or
/// just `EnumName.Variant` as a static reference.
///
/// # Arguments
/// - `api`: The API surface containing enum definitions
///
/// # Returns
/// A map from enum name to `DefaultEnumVariant` metadata. Only enums with variants are included.
pub fn enum_default_variants_map_with_metadata(api: &ApiSurface) -> AHashMap<String, DefaultEnumVariant> {
    api.enums
        .iter()
        .filter_map(|enum_def| {
            let default_variant = enum_def
                .variants
                .iter()
                .find(|v| v.is_default)
                .or_else(|| enum_def.variants.first())?;

            Some((
                enum_def.name.clone(),
                DefaultEnumVariant {
                    variant_name: default_variant.name.clone(),
                    is_zero_field: default_variant.fields.is_empty(),
                },
            ))
        })
        .collect()
}
