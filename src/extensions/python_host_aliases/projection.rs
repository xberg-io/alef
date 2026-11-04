use super::PythonHostAliases;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, MethodDef};
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type HiddenMethods = BTreeMap<String, BTreeSet<String>>;

pub(crate) fn read_non_python_surface(
    api: &ApiSurface,
    path: &Path,
    config: &ResolvedCrateConfig,
) -> Result<Option<ApiSurface>> {
    match super::read_for_crate(path, &config.name)? {
        Some(aliases) => aliases.non_python_surface(api, config),
        None => Ok(None),
    }
}

pub(crate) fn require_active_python_bridge(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    trait_path: &str,
) -> Result<()> {
    ensure!(
        config.trait_bridges_for(Language::Python).any(|bridge| {
            crate::backends::pyo3::trait_bridge::active_bridge_trait(bridge, api)
                .is_some_and(|active| active.rust_path == trait_path)
        }),
        "Python host aliases: trait `{trait_path}` has no active Python bridge"
    );
    Ok(())
}

impl PythonHostAliases {
    pub(crate) fn non_python_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Result<Option<ApiSurface>> {
        if !self.has_python_only_methods() {
            return Ok(None);
        }

        let mut hidden = HiddenMethods::new();
        self.validate_with(api, |typ, method, alias| {
            if alias.python_only() {
                hidden
                    .entry(typ.rust_path.clone())
                    .or_default()
                    .insert(method.name.clone());
            }
        })?;
        for trait_path in hidden.keys() {
            require_active_python_bridge(api, config, trait_path)?;
        }

        let mut projected = api.clone();
        for typ in &mut projected.types {
            let trait_path = typ.is_trait.then_some(typ.rust_path.as_str());
            typ.methods.retain(|method| !is_hidden(method, trait_path, &hidden));
        }
        for enm in &mut projected.enums {
            enm.methods.retain(|method| !is_hidden(method, None, &hidden));
        }
        Ok(Some(projected))
    }
}

fn is_hidden(method: &MethodDef, own_trait_path: Option<&str>, hidden: &HiddenMethods) -> bool {
    method
        .trait_source
        .as_deref()
        .or(own_trait_path)
        .and_then(|path| hidden.get(path))
        .is_some_and(|names| names.contains(&method.name))
}
