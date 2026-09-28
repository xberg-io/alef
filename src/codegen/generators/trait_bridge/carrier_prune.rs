//! Per-language removal of an `options_field` bridge's carrier field.
//!
//! `exclude_languages` suppresses the **bridge** for a target but said nothing about the
//! **field** the bridge binds to, so an excluded backend still emitted the carrier on its
//! config DTO -- typed as a handle it had no bridge to construct. Reported for node, elixir
//! and dart, and measured for pyo3, where the public dataclass advertised
//! `on_progress: ProgressHandle | None` for a class with no constructor exposed to the host
//! (alef #480).
//!
//! The prune derives a per-language `ApiSurface` rather than adding a language dimension to
//! `FieldDef`: `binding_excluded` is already honoured by every emitter through
//! [`crate::codegen::shared::binding_fields`], so marking the carrier on a language's own copy
//! of the surface reaches all of them without touching a single read site.
//!
//! It is deliberately narrow. Only a carrier field of a bridge that is inactive for the target
//! is marked, and only when the bridge binds via `options_field`. Language-scoped
//! `exclude_types` / `exclude_functions` stay where each backend applies them today.

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

use super::bridge_targets_language;

/// The carrier fields that `target_spellings` must not see, as `(options_type, field)` pairs.
fn inactive_carriers<'a>(trait_bridges: &'a [TraitBridgeConfig], target_spellings: &[&str]) -> Vec<(&'a str, &'a str)> {
    trait_bridges
        .iter()
        .filter(|bridge| !bridge_targets_language(bridge, target_spellings))
        .filter_map(|bridge| {
            let options_type = bridge.options_type.as_deref()?;
            let field = bridge.resolved_options_field()?;
            Some((options_type, field))
        })
        .collect()
}

/// A copy of `api` with every inactive bridge's carrier field marked `binding_excluded`.
///
/// Returns `None` when nothing needs marking, so the common case -- no bridges, or every
/// bridge active for this target -- does not clone the surface. This runs once per language
/// inside the generation pipeline's parallel fan-out, so the empty case has to stay free.
pub fn prune_inactive_bridge_carriers(
    api: &ApiSurface,
    trait_bridges: &[TraitBridgeConfig],
    target_spellings: &[&str],
) -> Option<ApiSurface> {
    let carriers = inactive_carriers(trait_bridges, target_spellings);
    if carriers.is_empty() {
        return None;
    }

    let mut pruned = api.clone();
    let mut marked = false;
    for typ in &mut pruned.types {
        for (options_type, field_name) in &carriers {
            if typ.name != *options_type {
                continue;
            }
            for field in &mut typ.fields {
                if field.name == *field_name && !field.binding_excluded {
                    field.binding_excluded = true;
                    field.binding_exclusion_reason =
                        Some("trait bridge excluded for this language via exclude_languages".to_string());
                    marked = true;
                }
            }
        }
    }

    // A configured carrier that matches no field is not this pass's error to report --
    // `trait_bridge_carrier_diagnostics` and the backends' own lookups already speak to an
    // unresolvable `options_type`/`options_field`. Returning the untouched surface keeps the
    // caller on its borrowed copy instead of paying for a clone that changed nothing. ~keep
    marked.then_some(pruned)
}

/// [`prune_inactive_bridge_carriers`] for one `Language`, resolving its own
/// `exclude_languages` spellings.
///
/// A caller holding a `Language` -- the generation pipeline and the docs renderer both do --
/// cannot reach a backend's `TARGET_SPELLINGS`, three of which are private to their modules.
/// [`Language::bridge_spellings`] is the shared answer, pinned against those constants by
/// `tests/trait_bridge_exclude_language_spellings.rs`. ~keep
pub fn language_surface(
    api: &ApiSurface,
    trait_bridges: &[TraitBridgeConfig],
    language: crate::core::config::Language,
) -> Option<ApiSurface> {
    prune_inactive_bridge_carriers(api, trait_bridges, language.bridge_spellings())
}

/// Re-wrap [`language_surface`]'s output, falling back to the whole-set surface.
///
/// Exists because the pruned surface is owned by the caller's stack frame while
/// [`ValidatedApiSurface`] borrows it, so the two cannot be produced by one call. Each of the
/// generation pipeline's four entry points binds the `Option` and then calls this, which is
/// what keeps the bindings, the type stubs, the service API and the public API reading the
/// same per-language surface -- a subset of them applying it reproduces exactly the
/// stub-disagrees-with-binding defect this release removed elsewhere. ~keep
pub fn validated_language_surface<'a>(
    pruned: &'a Option<ApiSurface>,
    whole: crate::core::validation::ValidatedApiSurface<'a>,
) -> crate::core::validation::ValidatedApiSurface<'a> {
    pruned.as_ref().map_or(
        whole,
        crate::core::validation::ValidatedApiSurface::from_subtractive_projection,
    )
}
