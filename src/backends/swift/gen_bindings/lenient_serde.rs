//! `[crates.swift] lenient_serde_types`: a per-type opt-out of strict Swift decoding.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;

/// Clears `serde(deny_unknown_fields)` on every listed type, so its Swift DTO is generated like a
/// permissive serde struct: unknown keys are ignored and custom input codecs (`deserialize_with`,
/// `with`) are no longer a strict-policy error. The Rust type keeps its own policy.
///
/// Entries are checked against the whole extracted API, before feature and exclusion filtering, so
/// one entry serves every feature set; a name that matches no strict type fails generation.
pub(super) fn apply(mut api: ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<ApiSurface> {
    let names = config
        .swift
        .as_ref()
        .map_or(&[][..], |swift| &swift.lenient_serde_types[..]);
    for name in names {
        let ty = api
            .types
            .iter_mut()
            .find(|ty| &ty.name == name && ty.serde_deserialization.deny_unknown_fields)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "[crates.swift] lenient_serde_types entry '{name}' names no type with serde(deny_unknown_fields)"
                )
            })?;
        ty.serde_deserialization.deny_unknown_fields = false;
    }
    Ok(api)
}
