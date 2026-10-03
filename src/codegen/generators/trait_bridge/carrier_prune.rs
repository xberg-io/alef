//! Per-language removal of an `options_field` bridge's carrier field.
//!
//! `exclude_languages` suppresses the **bridge** for a target but said nothing about the
//! **field** the bridge binds to, so an excluded backend still emitted the carrier on its
//! config DTO and linked update DTOs -- typed as a handle it had no bridge to construct.
//! Reported for node, elixir and dart, and measured for pyo3, where the public dataclass advertised
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
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

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

fn direct_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => direct_named_type(inner),
        _ => None,
    }
}

fn is_matching_carrier(candidate: &FieldDef, carrier: &FieldDef) -> bool {
    candidate.name == carrier.name && candidate.ty == carrier.ty && candidate.type_rust_path == carrier.type_rust_path
}

fn carrier_owner_names(api: &ApiSurface, options_type: &str, field_name: &str) -> Vec<String> {
    let Some(options) = api.types.iter().find(|typ| typ.name == options_type) else {
        return Vec::new();
    };
    let Some(carrier) = options.fields.iter().find(|field| field.name == field_name) else {
        return Vec::new();
    };

    let mut owners = vec![options.name.clone()];
    for related_name in options
        .methods
        .iter()
        .flat_map(|method| &method.params)
        .filter_map(|param| direct_named_type(&param.ty))
    {
        let Some(related) = api.types.iter().find(|typ| typ.name == related_name) else {
            continue;
        };
        if related.fields.iter().any(|field| is_matching_carrier(field, carrier))
            && !owners.iter().any(|owner| owner == related_name)
        {
            owners.push(related_name.to_string());
        }
    }
    owners
}

fn exclude_carrier(typ: &mut TypeDef, field_name: &str) -> bool {
    let Some(field) = typ
        .fields
        .iter_mut()
        .find(|field| field.name == field_name && !field.binding_excluded)
    else {
        return false;
    };
    field.binding_excluded = true;
    field.binding_exclusion_reason = Some("trait bridge excluded for this language via exclude_languages".to_string());
    true
}

/// A copy of `api` with every inactive bridge's carrier field marked `binding_excluded`.
///
/// The carrier is removed from the configured options type and from DTOs passed to its methods
/// when those DTOs contain the same field and core type, covering partial-update inputs without
/// relying on an `Update` naming convention.
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

    let owners = carriers
        .iter()
        .flat_map(|(options_type, field_name)| {
            carrier_owner_names(api, options_type, field_name)
                .into_iter()
                .map(|owner| (owner, *field_name))
        })
        .collect::<Vec<_>>();
    let mut pruned = api.clone();
    let mut marked = false;
    for typ in &mut pruned.types {
        for (owner, field_name) in &owners {
            if typ.name != *owner {
                continue;
            }
            marked |= exclude_carrier(typ, field_name);
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
