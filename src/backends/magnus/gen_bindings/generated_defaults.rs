use std::collections::HashSet;

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeRef};

use super::classes;

/// Every type name the magnus backend gives the *generated* crate a `Default` impl for.
///
/// This is a strict superset of the `has_default` set, which only answers "did the core type derive
/// or hand-write `Default`" and so cannot decide whether a `Default::default()` fallback resolves at
/// a mirrored type. The cases below mirror, one for one, the backend's emission sites:
/// `gen_magnus_default_impl` for a `has_default` struct; `gen_struct_default_impl_explicit` for a
/// struct whose own field-level defaults earn one; and `enum_magnus.rs.jinja`'s two impls —
/// unconditional for a unit enum, `has_default`-gated for a data enum. Opaque (`#[magnus::wrap]`)
/// structs get none, which is why they are filtered out rather than merely absent.
///
/// Keep this in step with those sites: a name wrongly present here is an `E0277` in the generated
/// crate, a name wrongly absent turns an optional Ruby keyword into a required one.
pub(super) fn generated_default_types<'a>(
    api: &'a ApiSurface,
    default_types: &HashSet<&str>,
    map_type: &dyn Fn(&TypeRef) -> String,
    trait_bridges: &[TraitBridgeConfig],
) -> HashSet<&'a str> {
    api.types
        .iter()
        .filter(|typ| !typ.is_trait && !typ.is_opaque)
        .filter(|typ| {
            typ.has_default
                || !typ.fields.is_empty()
                    && classes::gen_struct_default_impl_explicit(typ, map_type, trait_bridges, default_types).is_some()
        })
        .map(|typ| typ.name.as_str())
        .chain(
            api.enums
                .iter()
                .filter(|enum_def| enum_def.has_default || enum_def.variants.iter().all(|v| v.fields.is_empty()))
                .map(|enum_def| enum_def.name.as_str()),
        )
        .collect()
}
