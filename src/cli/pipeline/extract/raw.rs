use crate::cli::pipeline::version_core::read_version;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use std::path::Path;
use tracing::info;

pub(super) fn extract_raw(config: &ResolvedCrateConfig, _config_path: &Path) -> anyhow::Result<ApiSurface> {
    info!("Extracting API surface from Rust source...");
    let version = read_version(&config.version_from)?;
    let workspace_root = config.workspace_root.as_deref();
    let default_name = &config.name;

    // Build source groups: use explicit primary source_crates config when
    // available, otherwise derive crate names from file paths in the flat
    // sources list. Source-crate entries with `roots` are external type-only
    // seeds and are merged later by the external-types pass; they must not
    // replace the host crate sources.
    let mut groups: std::collections::BTreeMap<String, Vec<&Path>> = std::collections::BTreeMap::new();
    let primary_source_crates: Vec<_> = config
        .source_crates
        .iter()
        .filter(|source_crate| source_crate.roots.is_empty())
        .collect();
    if !primary_source_crates.is_empty() {
        for sc in primary_source_crates {
            let crate_name = sc.name.replace('-', "_");
            for source in &sc.sources {
                groups.entry(crate_name.clone()).or_default().push(source.as_path());
            }
        }
    } else {
        for source in &config.sources {
            let crate_name = derive_crate_name_from_path(source, default_name);
            groups.entry(crate_name).or_default().push(source.as_path());
        }
    }

    // Extract each group with its own crate name, then merge
    let mut merged = ApiSurface {
        crate_name: default_name.to_string(),
        version: version.clone(),
        ..ApiSurface::default()
    };

    for (crate_name, sources) in &groups {
        let api = crate::extract::extractor::extract(sources, crate_name, &version, workspace_root)
            .with_context(|| format!("failed to extract API surface from crate {crate_name}"))?;
        merged.types.extend(api.types);
        merged.functions.extend(api.functions);
        merged.enums.extend(api.enums);
        merged.errors.extend(api.errors);
        merged.excluded_type_paths.extend(api.excluded_type_paths);
        merged.excluded_trait_names.extend(api.excluded_trait_names);
        merged.unsupported_public_items.extend(api.unsupported_public_items);
    }

    // Re-run the return-type marking against the merged surface so that a
    // function in crate A that returns a type whose canonical home is crate B
    // (a common pattern when the public facade `pub use`s items from internal
    // crates) still gets its TypeDef.is_return_type flagged. The per-crate
    // extractor only marks types that share its own surface, so cross-crate
    // function→type pairs would otherwise stay false here.
    let return_type_names: ahash::AHashSet<String> = merged
        .functions
        .iter()
        .filter_map(|f| match &f.return_type {
            crate::core::ir::TypeRef::Named(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    for typ in &mut merged.types {
        if return_type_names.contains(&typ.name) {
            typ.is_return_type = true;
        }
    }

    Ok(merged)
}

/// Derive the crate name from a source file path.
///
/// Matches `crates/{name}/src/` pattern and converts hyphens to underscores.
/// Falls back to the provided default name if the pattern doesn't match.
fn derive_crate_name_from_path(path: &Path, default: &str) -> String {
    let path_str = path.to_string_lossy();
    // Match both "crates/foo-bar/src/" and "/abs/path/crates/foo-bar/src/"
    if let Some(after_crates) = path_str.split("crates/").nth(1) {
        if let Some(name) = after_crates.split('/').next() {
            if path_str.contains(&format!("crates/{name}/src/")) {
                return name.replace('-', "_");
            }
        }
    }
    default.to_string()
}
