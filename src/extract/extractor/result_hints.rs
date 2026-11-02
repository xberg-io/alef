use std::path::{Path, PathBuf};

use ahash::{AHashMap, AHashSet};
use anyhow::{Context, Result};

use super::helpers::is_test_gated;
use super::helpers::result_alias_scope::{
    ResultAliasImport, collect_result_glob_scopes, collect_result_module_imports, resolve_named_result_alias_import,
    resolve_result_module_scope, result_alias_target_scope, visibility_scope,
};
use super::paths::{derive_module_path, module_children_dir};
use crate::extract::type_resolver::{self, ResultAliasHint, ResultAliasScope};

#[derive(Clone, PartialEq, Eq)]
struct AliasBinding {
    hint: ResultAliasHint,
    visible_from: String,
}

#[derive(Default)]
struct AliasIndex {
    bindings: AHashMap<String, AliasBinding>,
    alias_visibility: AHashMap<String, String>,
    alias_targets: AHashMap<String, ResultAliasScope>,
    imports: AHashMap<String, ResultAliasImport>,
    globs: AHashMap<String, Vec<ResultAliasImport>>,
    module_imports: AHashMap<String, ResultAliasScope>,
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
        if let Some(import) = resolve_named_result_alias_import(items, module_path) {
            self.imports.insert(module_path.to_string(), import);
        }
        let globs = collect_result_glob_scopes(items, module_path);
        if !globs.is_empty() {
            self.globs.insert(module_path.to_string(), globs);
        }
        self.module_imports
            .extend(collect_result_module_imports(items, module_path));
        for item in items {
            match item {
                syn::Item::Type(alias) if alias.ident == "Result" && !is_test_gated(&alias.attrs) => {
                    self.collect_alias(alias, items, module_path);
                }
                // Private aliases can feed public signatures; item lowering still owns IR visibility.
                syn::Item::Mod(module) if !is_test_gated(&module.attrs) => {
                    self.collect_module(module, source, is_crate_root, module_path)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_alias(&mut self, alias: &syn::ItemType, items: &[syn::Item], module_path: &str) {
        let visible_from = visibility_scope(&alias.vis, module_path);
        self.alias_visibility
            .insert(module_path.to_string(), visible_from.clone());
        let hint =
            if let Some(error_type) = type_resolver::extract_result_error_type_from_alias(&alias.ty, &alias.generics) {
                ResultAliasHint::Error(error_type)
            } else if let Some(scope) = result_alias_target_scope(&alias.ty, items, module_path) {
                self.alias_targets.insert(module_path.to_string(), scope);
                return;
            } else {
                ResultAliasHint::Foreign
            };
        self.bindings
            .insert(module_path.to_string(), AliasBinding { hint, visible_from });
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

    fn normalize_module_scopes(&mut self) {
        let modules = &self.module_imports;
        let normalize =
            |scope: &ResultAliasScope| resolve_result_module_scope(scope, |prefix| modules.get(prefix).cloned());
        // A renamed import can name Result even when its written RHS identifier does not.
        for scope in self.alias_targets.values_mut() {
            *scope = result_alias_owner(normalize(scope));
        }
        for import in self.imports.values_mut() {
            import.scope = normalize(&import.scope);
        }
        for import in self.globs.values_mut().flatten() {
            import.scope = normalize(&import.scope);
        }
    }

    fn source_binding(&self, scope: &ResultAliasScope, destination: &str) -> Option<AliasBinding> {
        match scope {
            ResultAliasScope::Foreign => Some(AliasBinding {
                hint: ResultAliasHint::Foreign,
                visible_from: String::new(),
            }),
            ResultAliasScope::Crate(target) => self
                .bindings
                .get(target)
                .filter(|binding| is_within(destination, &binding.visible_from))
                .cloned(),
        }
    }

    fn resolve_alias_targets(&mut self) -> bool {
        let mut changed = false;
        for (module, scope) in &self.alias_targets {
            if self.bindings.contains_key(module) {
                continue;
            }
            if let Some(mut binding) = self.source_binding(scope, module) {
                binding.visible_from = self.alias_visibility[module].clone();
                changed |= update_binding(&mut self.bindings, module, binding);
            }
        }
        changed
    }

    fn resolve_named_imports(&mut self) -> bool {
        let mut changed = false;
        for (module, import) in &self.imports {
            if self.alias_visibility.contains_key(module) {
                continue;
            }
            if let Some(mut binding) = self.source_binding(&import.scope, module) {
                binding.visible_from = narrower_scope(&binding.visible_from, &import.visible_from);
                changed |= update_binding(&mut self.bindings, module, binding);
            }
        }
        changed
    }

    fn resolve_known_globs(&mut self) -> bool {
        let mut changed = false;
        for (module, imports) in &self.globs {
            if self.alias_visibility.contains_key(module) || self.imports.contains_key(module) {
                continue;
            }
            let mut selected = self.bindings.get(module).cloned();
            for import in imports {
                if matches!(import.scope, ResultAliasScope::Foreign) {
                    continue;
                }
                let Some(mut binding) = self.source_binding(&import.scope, module) else {
                    continue;
                };
                binding.visible_from = narrower_scope(&binding.visible_from, &import.visible_from);
                match &mut selected {
                    Some(existing) if existing.hint == binding.hint => {
                        existing.visible_from = broader_scope(&existing.visible_from, &binding.visible_from);
                    }
                    None => selected = Some(binding),
                    _ => {}
                }
            }
            if let Some(binding) = selected {
                changed |= update_binding(&mut self.bindings, module, binding);
            }
        }
        changed
    }

    fn resolve_known_bindings(&mut self) {
        loop {
            let mut changed = self.resolve_alias_targets();
            changed |= self.resolve_named_imports();
            changed |= self.resolve_known_globs();
            if !changed {
                break;
            }
        }
    }

    fn install_foreign_glob_fallbacks(&mut self) {
        for (module, imports) in &self.globs {
            if self.bindings.contains_key(module)
                || self.alias_visibility.contains_key(module)
                || self.imports.contains_key(module)
            {
                continue;
            }
            let visible_from = imports
                .iter()
                .filter(|import| matches!(import.scope, ResultAliasScope::Foreign))
                .map(|import| import.visible_from.clone())
                .reduce(|left, right| broader_scope(&left, &right));
            if let Some(visible_from) = visible_from {
                self.bindings.insert(
                    module.clone(),
                    AliasBinding {
                        hint: ResultAliasHint::Foreign,
                        visible_from,
                    },
                );
            }
        }
    }

    fn install(mut self) {
        self.normalize_module_scopes();
        self.resolve_known_bindings();
        // Foreign globs are opaque: first let known local paths establish which globs supply Result.
        self.install_foreign_glob_fallbacks();
        self.resolve_known_bindings();
        for module in self.alias_targets.keys() {
            self.bindings.entry(module.clone()).or_insert_with(|| AliasBinding {
                hint: ResultAliasHint::Foreign,
                visible_from: self.alias_visibility[module].clone(),
            });
        }
        self.resolve_known_bindings();
        for (module, target) in self.module_imports {
            type_resolver::record_result_alias_hint(&module, ResultAliasHint::Module(target));
        }
        for (module, binding) in self.bindings {
            type_resolver::record_result_alias_hint(&module, binding.hint);
        }
    }
}

fn is_within(module: &str, boundary: &str) -> bool {
    boundary.is_empty()
        || module == boundary
        || module.strip_prefix(boundary).is_some_and(|tail| tail.starts_with("::"))
}

fn narrower_scope(left: &str, right: &str) -> String {
    if is_within(left, right) {
        left.to_string()
    } else {
        right.to_string()
    }
}

fn broader_scope(left: &str, right: &str) -> String {
    if is_within(left, right) {
        right.to_string()
    } else {
        left.to_string()
    }
}

fn update_binding(bindings: &mut AHashMap<String, AliasBinding>, module: &str, binding: AliasBinding) -> bool {
    if bindings.get(module) == Some(&binding) {
        return false;
    }
    bindings.insert(module.to_string(), binding);
    true
}

fn result_alias_owner(scope: ResultAliasScope) -> ResultAliasScope {
    match scope {
        ResultAliasScope::Crate(path) => match path.rsplit_once("::") {
            Some((module, "Result")) => ResultAliasScope::Crate(module.to_string()),
            None if path == "Result" => ResultAliasScope::Crate(String::new()),
            _ => ResultAliasScope::Foreign,
        },
        ResultAliasScope::Foreign => ResultAliasScope::Foreign,
    }
}
