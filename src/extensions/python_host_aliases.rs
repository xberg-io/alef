//! Explicit Python callback aliases without changing public config or IR layouts.

use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::extension::{Extension, ExtensionConfig};
use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef};
use crate::core::validation::ValidatedApiSurface;
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

mod projection;
#[cfg(test)]
mod tests;
mod workspace;

pub(crate) use projection::{read_non_python_surface, require_active_python_bridge};
pub(crate) use workspace::read_for_crate;

/// Parsed callback aliases for one source crate or direct backend instance.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PythonHostAliases {
    #[serde(default)]
    traits: BTreeMap<String, BTreeMap<String, HostAlias>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum HostAlias {
    Legacy(String),
    Configured(HostAliasOptions),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostAliasOptions {
    fallback: String,
    #[serde(default)]
    python_only: bool,
}

impl HostAlias {
    fn fallback(&self) -> &str {
        match self {
            Self::Legacy(fallback) => fallback,
            Self::Configured(options) => &options.fallback,
        }
    }

    fn python_only(&self) -> bool {
        matches!(self, Self::Configured(options) if options.python_only)
    }
}

impl PythonHostAliases {
    /// Parse explicit Rust callback → Python host method mappings.
    pub fn from_raw(raw: &toml::Value) -> Result<Self> {
        raw.clone()
            .try_into()
            .context("invalid extensions.python_host_aliases configuration")
    }

    pub(crate) fn for_trait(&self, typ: &TypeDef) -> BTreeMap<String, String> {
        self.traits
            .get(&typ.rust_path)
            .or_else(|| self.traits.get(&typ.name))
            .map(|aliases| {
                aliases
                    .iter()
                    .map(|(source, alias)| (source.clone(), alias.fallback().to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn has_python_only_methods(&self) -> bool {
        self.traits
            .values()
            .flat_map(|aliases| aliases.values())
            .any(HostAlias::python_only)
    }

    pub(crate) fn validate(&self, api: &ApiSurface) -> Result<()> {
        self.validate_with(api, |_, _, _| {})
    }

    fn validate_with(&self, api: &ApiSurface, mut visit: impl FnMut(&TypeDef, &MethodDef, &HostAlias)) -> Result<()> {
        let mut seen = BTreeSet::new();
        for (name, aliases) in &self.traits {
            let matches: Vec<_> = api
                .types
                .iter()
                .filter(|typ| typ.is_trait && (typ.rust_path == *name || typ.name == *name))
                .collect();
            ensure!(!matches.is_empty(), "Python host aliases: unknown trait `{name}`");
            ensure!(
                matches.len() == 1,
                "Python host aliases: ambiguous trait `{name}`; use its qualified path"
            );
            let typ = matches[0];
            ensure!(
                seen.insert(&typ.rust_path),
                "Python host aliases: trait `{name}` configured twice"
            );
            ensure!(!typ.binding_excluded, "Python host aliases: trait `{name}` is excluded");
            for (source, alias) in aliases {
                let source = own_method(typ, source)?;
                let target = own_method(typ, alias.fallback())?;
                validate_pair(typ, source, target)?;
                ensure!(
                    !alias.python_only() || !typ.rust_path.is_empty(),
                    "Python-only host aliases require a qualified Rust trait identity"
                );
                visit(typ, source, alias);
            }
        }
        Ok(())
    }
}

fn own_method<'a>(typ: &'a TypeDef, name: &str) -> Result<&'a MethodDef> {
    let methods: Vec<_> = typ
        .methods
        .iter()
        .filter(|m| m.name == name && m.trait_source.is_none())
        .collect();
    ensure!(
        methods.len() == 1,
        "Python host aliases: unknown or ambiguous method `{}.{name}`",
        typ.rust_path
    );
    ensure!(
        !methods[0].binding_excluded,
        "Python host aliases: method `{}.{name}` is excluded",
        typ.rust_path
    );
    Ok(methods[0])
}

fn validate_pair(typ: &TypeDef, source: &MethodDef, target: &MethodDef) -> Result<()> {
    let context = format!(
        "Python host alias `{}.{}` → `{}`",
        typ.rust_path, source.name, target.name
    );
    ensure!(
        source.has_default_impl && source.error_type.is_some(),
        "{context}: callback must be defaulted and fallible"
    );
    ensure!(
        !target.has_default_impl && target.error_type.is_none(),
        "{context}: host target must be required and infallible"
    );
    for method in [source, target] {
        ensure!(
            !method.is_async
                && !method.is_static
                && method.receiver == Some(ReceiverKind::Ref)
                && !method.returns_ref
                && !method.returns_cow
                && !method.sanitized,
            "{context}: only unsanitized synchronous &self callbacks with owned results are supported"
        );
    }
    ensure!(
        source.return_type == target.return_type,
        "{context}: incompatible Ok-return type"
    );
    ensure!(
        source.params.len() == target.params.len(),
        "{context}: incompatible argument count"
    );
    for (left, right) in source.params.iter().zip(&target.params) {
        ensure!(
            left.ty == right.ty
                && left.is_ref == right.is_ref
                && left.is_mut == right.is_mut
                && left.optional == right.optional
                && !left.sanitized
                && !right.sanitized
                && left.newtype_wrapper == right.newtype_wrapper
                && left.original_type == right.original_type,
            "{context}: incompatible argument shape"
        );
    }
    Ok(())
}

pub(crate) fn generate_bindings(
    backend: &dyn Backend,
    api: ValidatedApiSurface<'_>,
    config: &ResolvedCrateConfig,
    aliases: Option<&PythonHostAliases>,
) -> Result<Vec<GeneratedFile>> {
    let Some(aliases) = aliases.filter(|_| backend.language() == Language::Python) else {
        return backend.generate_bindings_checked(api, config);
    };
    ensure!(
        backend.name() == "pyo3",
        "python_host_aliases requires the PyO3 backend"
    );
    crate::backends::pyo3::Pyo3Backend::with_parsed_host_aliases(aliases.clone()).generate_bindings_checked(api, config)
}

/// Built-in configuration owner; emission occurs in the configured PyO3 backend.
pub struct PythonHostAliasesExtension;

impl Extension for PythonHostAliasesExtension {
    fn name(&self) -> &str {
        "python_host_aliases"
    }

    fn parse_config(&self, raw: Option<&toml::Value>) -> Result<ExtensionConfig> {
        match raw {
            Some(raw) => Ok(ExtensionConfig::with_typed(workspace::parse(raw)?, Some(raw.clone()))),
            None => Ok(ExtensionConfig::empty()),
        }
    }
}
