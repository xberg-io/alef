use std::path::{Path, PathBuf};

use ahash::{AHashMap, AHashSet};
use anyhow::{Context, Result};

use super::helpers::{collect_reexport_map, is_pub, is_test_gated, resolve_result_alias_scope};
use super::paths::{derive_module_path, module_children_dir};
use crate::extract::type_resolver::{self, ResultAliasScope};

#[derive(Default)]
struct AliasIndex {
    declarations: AHashMap<String, String>,
    imports: AHashMap<String, ResultAliasScope>,
    visited: AHashSet<PathBuf>,
}

/// Index Rust alias namespaces before lowering any dependent function signatures.
pub(super) fn collect_sources(sources: &[&Path], crate_src_dir: Option<&Path>) -> Result<()> {
    let mut index = AliasIndex::default();
    for source in sources {
        let module_path = derive_module_path(source, crate_src_dir);
        index.collect_file(source, module_path.is_empty(), &module_path)?;
    }
    index.install();
    Ok(())
}

#[cfg(test)]
pub(super) fn collect_inline_items(items: &[syn::Item]) -> Result<()> {
    let mut index = AliasIndex::default();
    index.collect_items(items, Path::new("test.rs"), true, "")?;
    index.install();
    Ok(())
}

impl AliasIndex {
    fn collect_file(&mut self, source: &Path, is_crate_root: bool, module_path: &str) -> Result<()> {
        let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
        if !self.visited.insert(canonical) {
            return Ok(());
        }
        let content = std::fs::read_to_string(source)
            .with_context(|| format!("Failed to read Result alias source: {}", source.display()))?;
        let parsed = syn::parse_file(&content)
            .with_context(|| format!("Failed to parse Result alias source: {}", source.display()))?;
        self.collect_items(&parsed.items, source, is_crate_root, module_path)
    }

    fn collect_items(
        &mut self,
        items: &[syn::Item],
        source: &Path,
        is_crate_root: bool,
        module_path: &str,
    ) -> Result<()> {
        if let Some(scope) = resolve_result_alias_scope(items, module_path) {
            self.imports.insert(module_path.to_string(), scope);
        }
        let reexports = collect_reexport_map(items);
        for item in items {
            match item {
                syn::Item::Type(alias) if alias.ident == "Result" && !is_test_gated(&alias.attrs) => {
                    if let Some(error_type) =
                        type_resolver::extract_result_error_type_from_alias(&alias.ty, &alias.generics)
                    {
                        self.declarations.insert(module_path.to_string(), error_type);
                    }
                }
                syn::Item::Mod(module)
                    if !is_test_gated(&module.attrs)
                        && (is_pub(&module.vis) || reexports.contains_key(&module.ident.to_string())) =>
                {
                    self.collect_module(module, source, is_crate_root, module_path)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_module(
        &mut self,
        module: &syn::ItemMod,
        source: &Path,
        is_crate_root: bool,
        parent_path: &str,
    ) -> Result<()> {
        let name = module.ident.to_string();
        let module_path = if parent_path.is_empty() {
            name.clone()
        } else {
            format!("{parent_path}::{name}")
        };
        if let Some((_, items)) = &module.content {
            return self.collect_items(items, source, is_crate_root, &module_path);
        }
        let parent = module_children_dir(source, is_crate_root);
        let file_name = name.strip_prefix("r#").unwrap_or(&name);
        let candidates = [
            parent.join(format!("{file_name}.rs")),
            parent.join(file_name).join("mod.rs"),
        ];
        if let Some(candidate) = candidates.iter().find(|path| path.exists()) {
            self.collect_file(candidate, false, &module_path)?;
        }
        Ok(())
    }

    fn install(mut self) {
        loop {
            let before = self.declarations.len();
            for (module, scope) in &self.imports {
                if !self.declarations.contains_key(module)
                    && let ResultAliasScope::Crate(target) = scope
                    && let Some(error_type) = self.declarations.get(target).cloned()
                {
                    self.declarations.insert(module.clone(), error_type);
                }
            }
            if self.declarations.len() == before {
                break;
            }
        }
        for (module, error_type) in self.declarations {
            type_resolver::record_result_error_hint(&module, error_type);
        }
    }
}
