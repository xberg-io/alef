use alef_core::ir::ApiSurface;

/// Build the set of variant names that collide across enums, error types, and top-level types.
///
/// Gleam requires constructor names to be unique module-wide. Any variant name
/// that appears more than once across enums + error types, or that matches an
/// existing top-level type name, must be prefixed with the parent type name.
pub(crate) fn build_collision_set(api: &ApiSurface) -> std::collections::HashSet<String> {
    let mut variant_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for en in &api.enums {
        for v in &en.variants {
            *variant_counts.entry(v.name.clone()).or_insert(0) += 1;
        }
    }
    for err in &api.errors {
        for v in &err.variants {
            *variant_counts.entry(v.name.clone()).or_insert(0) += 1;
        }
    }
    // Also flag variants whose name collides with an existing top-level type.
    for ty in &api.types {
        *variant_counts.entry(ty.name.clone()).or_insert(0) += 1;
    }
    variant_counts
        .into_iter()
        .filter_map(|(n, c)| if c > 1 { Some(n) } else { None })
        .collect()
}

/// Resolve a variant name within its parent type. If the variant name is also
/// used by another type's variant in the same module (Gleam requires unique
/// constructor names module-wide), prefix it with the parent type name.
pub(crate) fn variant_constructor_name(
    parent_type: &str,
    variant_name: &str,
    collisions: &std::collections::HashSet<String>,
) -> String {
    if collisions.contains(variant_name) {
        format!("{parent_type}{variant_name}")
    } else {
        variant_name.to_string()
    }
}
