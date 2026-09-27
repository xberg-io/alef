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

    let mut groups: std::collections::BTreeMap<String, Vec<&Path>> = std::collections::BTreeMap::new();
    let primary_source_crates: Vec<_> = config
        .resolved_source_crates()?
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
        // `sources` is the only thing here that knows which `sources` entries backed this
        // extraction group -- `extract_module` itself has no visibility into `alef.toml`, so
        // `declared_sources` is stamped on here, per group, rather than at the point of
        // discovery. ~keep
        let declared_sources: Vec<std::path::PathBuf> = sources.iter().map(|source| source.to_path_buf()).collect();
        merged
            .unresolved_modules
            .extend(api.unresolved_modules.into_iter().map(|mut unresolved| {
                unresolved.declared_sources = declared_sources.clone();
                unresolved
            }));
    }

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
    if let Some(after_crates) = path_str.split("crates/").nth(1)
        && let Some(name) = after_crates.split('/').next()
        && path_str.contains(&format!("crates/{name}/src/"))
    {
        return name.replace('-', "_");
    }
    default.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_manifest(tmp: &Path) {
        fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest fixture");
    }

    /// Regression coverage for the `raw.rs` merge point: two crate groups each carry their own
    /// unresolved `mod` declaration, and `extract_raw` must stamp each recorded entry with only
    /// the `sources` that backed *its own* group -- never the union across groups.
    #[test]
    fn extract_raw_stamps_each_unresolved_module_with_its_own_crate_sources() {
        let tmp = std::env::temp_dir().join("alef_test_raw_unresolved_modules_sources");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("crates/crate_a/src")).expect("create crate_a dir");
        fs::create_dir_all(tmp.join("crates/crate_b/src")).expect("create crate_b dir");
        write_manifest(&tmp);
        fs::write(tmp.join("crates/crate_a/src/lib.rs"), "pub mod missing_a;\n").expect("write crate_a lib.rs");
        fs::write(tmp.join("crates/crate_b/src/lib.rs"), "pub mod missing_b;\n").expect("write crate_b lib.rs");

        let crate_a_lib = tmp.join("crates/crate_a/src/lib.rs");
        let crate_b_lib = tmp.join("crates/crate_b/src/lib.rs");

        let config = ResolvedCrateConfig {
            name: "sample".to_string(),
            sources: vec![crate_a_lib.clone(), crate_b_lib.clone()],
            version_from: tmp.join("Cargo.toml").to_string_lossy().into_owned(),
            ..ResolvedCrateConfig::default()
        };
        let config_path = tmp.join("alef.toml");

        let surface = extract_raw(&config, &config_path).expect("extract_raw must succeed");

        assert_eq!(
            surface.unresolved_modules.len(),
            2,
            "each crate group's own unresolved `mod` declaration must be recorded once; got {:?}",
            surface.unresolved_modules
        );

        let missing_a = surface
            .unresolved_modules
            .iter()
            .find(|module| module.module_path == "missing_a")
            .expect("missing_a must be recorded");
        assert_eq!(
            missing_a.declared_sources,
            vec![crate_a_lib.clone()],
            "missing_a's declared_sources must be crate_a's own sources only, not the union with crate_b"
        );

        let missing_b = surface
            .unresolved_modules
            .iter()
            .find(|module| module.module_path == "missing_b")
            .expect("missing_b must be recorded");
        assert_eq!(
            missing_b.declared_sources,
            vec![crate_b_lib.clone()],
            "missing_b's declared_sources must be crate_b's own sources only, not the union with crate_a"
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
