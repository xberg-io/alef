//! Explicit Python callback aliases without changing public config or IR layouts.

use crate::core::extension::{Extension, ExtensionConfig};
use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef};
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;
mod workspace;

pub(crate) use workspace::read_for_crate;

/// Parsed callback aliases for one source crate or direct backend instance.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PythonHostAliases {
    #[serde(default)]
    traits: BTreeMap<String, BTreeMap<String, String>>,
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
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn validate(&self, api: &ApiSurface) -> Result<()> {
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
            for (source, target) in aliases {
                let source = own_method(typ, source)?;
                let target = own_method(typ, target)?;
                validate_pair(typ, source, target)?;
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

/// Built-in configuration owner; emission occurs in the configured PyO3 backend.
pub struct PythonHostAliasesExtension;

impl Extension for PythonHostAliasesExtension {
    fn name(&self) -> &str {
        "python_host_aliases"
    }

    fn parse_config(&self, raw: Option<&toml::Value>) -> Result<ExtensionConfig> {
        match raw {
            Some(raw) => Ok(ExtensionConfig::with_typed(workspace::parse(raw)?, raw.clone())),
            None => Ok(ExtensionConfig::empty()),
        }
    }
}
