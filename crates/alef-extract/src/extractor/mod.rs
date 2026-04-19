mod defaults;
mod functions;
mod helpers;
mod reexports;
mod types;

use std::path::{Path, PathBuf};

use ahash::AHashMap;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use anyhow::{Context, Result};

use crate::type_resolver;

use self::functions::{detect_receiver, extract_function, extract_impl_block, extract_params, resolve_return_type};
use self::helpers::{build_rust_path, collect_reexport_map, extract_doc_comments, is_pub, is_thiserror_enum};
use self::reexports::{extract_module, resolve_use_tree};
use self::types::{extract_enum, extract_error_enum, extract_struct};

/// Extract the public API surface from Rust source files.
///
/// `sources` should be the root source files (e.g., `lib.rs`) of the crate.
/// Submodules referenced via `mod` declarations are resolved and extracted recursively.
/// `workspace_root` enables resolution of `pub use` re-exports from workspace sibling crates.
pub fn extract(
    sources: &[&Path],
    crate_name: &str,
    version: &str,
    workspace_root: Option<&Path>,
) -> Result<ApiSurface> {
    let mut surface = ApiSurface {
        crate_name: crate_name.to_string(),
        version: version.to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let mut visited = Vec::<PathBuf>::new();

    // Determine the crate source root directory from the first source (typically lib.rs).
    // This enables deriving correct module_path for other source files in the hierarchy.
    let crate_src_dir = sources.first().and_then(|s| s.parent()).map(|p| p.to_path_buf());

    for source in sources {
        let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

        // Skip source files already visited via `pub mod` traversal from an earlier
        // source (typically lib.rs). Re-processing them with module_path="" would
        // produce incorrect rust_paths (e.g. `kreuzberg::CustomProperties` instead
        // of `kreuzberg::extraction::CustomProperties`).
        if visited.contains(&canonical) {
            continue;
        }
        visited.push(canonical);

        let content = std::fs::read_to_string(source)
            .with_context(|| format!("Failed to read source file: {}", source.display()))?;
        let file =
            syn::parse_file(&content).with_context(|| format!("Failed to parse source file: {}", source.display()))?;

        // Derive module_path from the source file's location relative to the crate
        // source root. For example, `src/cache/core.rs` relative to `src/` gives
        // module_path `cache::core`. This ensures types get correct rust_paths even
        // when they're listed as explicit sources rather than discovered via `pub mod`.
        let module_path = derive_module_path(source, crate_src_dir.as_deref());

        let types_before = surface.types.len();
        let enums_before = surface.enums.len();
        let fns_before = surface.functions.len();

        let mut result_wrapping_aliases = ahash::AHashSet::new();
        extract_items(
            &file.items,
            source,
            crate_name,
            &module_path,
            &mut surface,
            workspace_root,
            &mut visited,
            &mut result_wrapping_aliases,
        )?;

        // For non-root source files, apply re-export shortening from the parent module.
        // When `cache/core.rs` is processed with module_path="cache::core", items get
        // paths like `kreuzberg::cache::core::GenericCache`. If the parent `cache/mod.rs`
        // has `pub use core::{GenericCache, ...}`, we shorten to `kreuzberg::cache::GenericCache`.
        if !module_path.is_empty() {
            apply_parent_reexport_shortening(
                source,
                crate_name,
                &module_path,
                &mut surface,
                types_before,
                enums_before,
                fns_before,
            );
        }
    }

    // Post-processing: resolve unresolved trait sources.
    // When a file containing `impl Trait for Type` is processed before the file defining
    // the Trait, the `trait_source` on methods will be `None`. Now that all files are
    // processed we can retroactively resolve them against the complete trait type list.
    resolve_trait_sources(&mut surface);

    // Post-processing: resolve newtype wrappers.
    // Single-field tuple structs like `pub struct Foo(String)` are detected by having
    // exactly one field named `_0`. We replace all `TypeRef::Named("Foo")` references
    // with the inner type, then remove the newtype TypeDefs from the surface.
    resolve_newtypes(&mut surface);

    // After newtype resolution, any remaining types with `_0` fields are tuple structs
    // that weren't resolved (because they have methods or complex inner types).
    // Keep them as newtypes (with the _0 field) so codegen can generate proper
    // From impls using tuple constructors. They're not opaque — they have a known inner type.

    // Mark types that appear as function return types.
    // These may use a different DTO style (e.g., TypedDict in Python).
    let return_type_names: ahash::AHashSet<String> = surface
        .functions
        .iter()
        .filter_map(|f| match &f.return_type {
            TypeRef::Named(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    for typ in &mut surface.types {
        if return_type_names.contains(&typ.name) {
            typ.is_return_type = true;
        }
    }

    Ok(surface)
}

/// Apply named re-export shortening from the parent module file.
///
/// When a source file like `cache/core.rs` produces items with paths like
/// `kreuzberg::cache::core::GenericCache`, and the parent `cache/mod.rs` has
/// `pub use core::{GenericCache, ...}`, this shortens the path to
/// `kreuzberg::cache::GenericCache`.
fn apply_parent_reexport_shortening(
    source: &Path,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    types_before: usize,
    enums_before: usize,
    fns_before: usize,
) {
    use self::helpers::collect_reexport_map;
    use self::reexports::collect_use_names;

    // Find the parent module file (mod.rs in parent directory, or parent.rs)
    let parent_dir = match source.parent() {
        Some(p) => p,
        None => return,
    };

    // Check if there's a mod.rs in the same directory (for files like cache/core.rs,
    // the parent module is cache/mod.rs)
    let parent_mod = parent_dir.join("mod.rs");
    let parent_lib = parent_dir.join("lib.rs");
    let parent_content = if parent_mod.exists() && parent_mod != source {
        std::fs::read_to_string(&parent_mod).ok()
    } else if parent_lib.exists() && parent_lib != source {
        std::fs::read_to_string(&parent_lib).ok()
    } else {
        None
    };

    let Some(content) = parent_content else {
        return;
    };

    let Ok(parent_file) = syn::parse_file(&content) else {
        return;
    };

    // Get the module name of the source file (e.g., "core" for cache/core.rs)
    let mod_name = source.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if mod_name.is_empty() || mod_name == "mod" {
        return;
    }

    // Collect re-exports from the parent module
    let reexport_map = collect_reexport_map(&parent_file.items);

    // Also check for `pub use mod_name::{A, B}` statements directly
    let mut reexported_names = std::collections::HashSet::new();
    for item in &parent_file.items {
        if let syn::Item::Use(item_use) = item {
            if helpers::is_pub(&item_use.vis) {
                if let syn::UseTree::Path(use_path) = &item_use.tree {
                    if use_path.ident == mod_name {
                        match collect_use_names(&use_path.tree) {
                            reexports::UseFilter::All => {
                                // Glob re-export — all items are re-exported
                                // Shorten all items to parent path
                                let parent_module_path = module_path.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
                                let parent_prefix = if parent_module_path.is_empty() {
                                    crate_name.to_string()
                                } else {
                                    format!("{crate_name}::{parent_module_path}")
                                };
                                for ty in &mut surface.types[types_before..] {
                                    ty.rust_path = format!("{parent_prefix}::{}", ty.name);
                                }
                                for en in &mut surface.enums[enums_before..] {
                                    en.rust_path = format!("{parent_prefix}::{}", en.name);
                                }
                                for func in &mut surface.functions[fns_before..] {
                                    func.rust_path = format!("{parent_prefix}::{}", func.name);
                                }
                                return;
                            }
                            reexports::UseFilter::Names(names) => {
                                reexported_names.extend(names);
                            }
                        }
                    }
                }
            }
        }
    }

    // Also include names from the reexport_map
    if let Some(helpers::ReexportKind::Names(names)) = reexport_map.get(mod_name) {
        reexported_names.extend(names.iter().cloned());
    } else if matches!(reexport_map.get(mod_name), Some(helpers::ReexportKind::Glob)) {
        // Glob — shorten all
        let parent_module_path = module_path.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
        let parent_prefix = if parent_module_path.is_empty() {
            crate_name.to_string()
        } else {
            format!("{crate_name}::{parent_module_path}")
        };
        for ty in &mut surface.types[types_before..] {
            ty.rust_path = format!("{parent_prefix}::{}", ty.name);
        }
        for en in &mut surface.enums[enums_before..] {
            en.rust_path = format!("{parent_prefix}::{}", en.name);
        }
        for func in &mut surface.functions[fns_before..] {
            func.rust_path = format!("{parent_prefix}::{}", func.name);
        }
        return;
    }

    if reexported_names.is_empty() {
        return;
    }

    // Apply shortening for named re-exports
    let parent_module_path = module_path.rsplit_once("::").map(|(p, _)| p).unwrap_or("");
    let parent_prefix = if parent_module_path.is_empty() {
        crate_name.to_string()
    } else {
        format!("{crate_name}::{parent_module_path}")
    };

    for ty in &mut surface.types[types_before..] {
        if reexported_names.contains(&ty.name) {
            ty.rust_path = format!("{parent_prefix}::{}", ty.name);
        }
    }
    for en in &mut surface.enums[enums_before..] {
        if reexported_names.contains(&en.name) {
            en.rust_path = format!("{parent_prefix}::{}", en.name);
        }
    }
    for func in &mut surface.functions[fns_before..] {
        if reexported_names.contains(&func.name) {
            func.rust_path = format!("{parent_prefix}::{}", func.name);
        }
    }
}

/// Derive the module path from a source file's location relative to the crate source root.
///
/// For `lib.rs` (the root), returns `""`.
/// For `src/cache/core.rs` relative to `src/`, returns `"cache::core"`.
/// For `src/types/mod.rs` relative to `src/`, returns `"types"`.
/// Falls back to `""` if the path can't be derived (e.g. file is outside the crate tree).
fn derive_module_path(source: &Path, crate_src_dir: Option<&Path>) -> String {
    let Some(root) = crate_src_dir else {
        return String::new();
    };

    // Canonicalize both paths for reliable comparison
    let root_canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let source_canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());

    let Ok(relative) = source_canonical.strip_prefix(&root_canonical) else {
        return String::new();
    };

    // Convert path components to module segments.
    // `lib.rs` → "" (root), `cache/core.rs` → "cache::core", `types/mod.rs` → "types"
    let mut segments = Vec::new();
    for component in relative.iter() {
        let s = component.to_string_lossy();
        if s == "lib.rs" || s == "main.rs" {
            // Root file — no module path
            return String::new();
        } else if s == "mod.rs" {
            // mod.rs doesn't add a segment (the parent directory is the module name)
            continue;
        } else if let Some(stem) = s.strip_suffix(".rs") {
            segments.push(stem.to_string());
        } else {
            // Directory component
            segments.push(s.to_string());
        }
    }

    segments.join("::")
}

/// Returns `true` if the type is a simple leaf type (primitive, String, Bytes, Path, etc.)
/// rather than a complex Named, collection, or Optional type.
fn is_simple_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Primitive(_)
            | TypeRef::String
            | TypeRef::Bytes
            | TypeRef::Path
            | TypeRef::Unit
            | TypeRef::Duration
            | TypeRef::Json
    )
}

/// Resolve newtype wrappers in the API surface.
///
/// Single-field tuple structs (`pub struct Foo(T)`) are identified by having exactly
/// one field named `_0`, no methods, and a simple inner type (primitive, String, etc.).
/// For each such newtype, all `TypeRef::Named("Foo")` references throughout the surface
/// are replaced with the inner type `T`, and the newtype TypeDef itself is removed.
/// This makes newtypes fully transparent to backends.
///
/// Tuple structs wrapping complex Named types (e.g., builders) are kept as-is.
fn resolve_newtypes(surface: &mut ApiSurface) {
    // Build a map of newtype name → inner TypeRef.
    let newtype_map: AHashMap<String, TypeRef> = surface
        .types
        .iter()
        .filter(|t| t.fields.len() == 1 && t.fields[0].name == "_0" && is_simple_type(&t.fields[0].ty))
        .map(|t| (t.name.clone(), t.fields[0].ty.clone()))
        .collect();

    if newtype_map.is_empty() {
        return;
    }

    // Capture the full rust_path for each newtype before removing them.
    // This is needed by codegen to re-wrap resolved primitives when calling core methods.
    let newtype_rust_paths: AHashMap<String, String> = surface
        .types
        .iter()
        .filter(|t| newtype_map.contains_key(&t.name))
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .collect();

    // Remove newtype TypeDefs from the surface.
    surface.types.retain(|t| !newtype_map.contains_key(&t.name));

    // Walk all TypeRefs in the surface and replace Named references to newtypes.
    for typ in &mut surface.types {
        for field in &mut typ.fields {
            // Record the newtype wrapper path before resolving, so codegen can wrap/unwrap correctly.
            if let alef_core::ir::TypeRef::Named(name) = &field.ty {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    field.newtype_wrapper = Some(rust_path.clone());
                }
            }
            // Also handle Optional<NewtypeT> — record wrapper on the optional field
            if let alef_core::ir::TypeRef::Optional(inner) = &field.ty {
                if let alef_core::ir::TypeRef::Named(name) = inner.as_ref() {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        field.newtype_wrapper = Some(rust_path.clone());
                    }
                }
            }
            // And Vec<NewtypeT>
            if let alef_core::ir::TypeRef::Vec(inner) = &field.ty {
                if let alef_core::ir::TypeRef::Named(name) = inner.as_ref() {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        field.newtype_wrapper = Some(rust_path.clone());
                    }
                }
            }
            resolve_typeref(&newtype_map, &mut field.ty);
        }
        for method in &mut typ.methods {
            for param in &mut method.params {
                // Record the newtype wrapper path before resolving, so codegen can re-wrap when calling core.
                if let alef_core::ir::TypeRef::Named(name) = &param.ty {
                    if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                        param.newtype_wrapper = Some(rust_path.clone());
                    }
                }
                resolve_typeref(&newtype_map, &mut param.ty);
            }
            // Record return newtype wrapper before resolving — only for direct Named returns
            // (not Optional/Vec wrappers; those would require different unwrap patterns).
            if let alef_core::ir::TypeRef::Named(name) = &method.return_type {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    method.return_newtype_wrapper = Some(rust_path.clone());
                }
            }
            resolve_typeref(&newtype_map, &mut method.return_type);
        }
    }
    for func in &mut surface.functions {
        for param in &mut func.params {
            if let alef_core::ir::TypeRef::Named(name) = &param.ty {
                if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                    param.newtype_wrapper = Some(rust_path.clone());
                }
            }
            resolve_typeref(&newtype_map, &mut param.ty);
        }
        // Record return newtype wrapper for free functions too
        if let alef_core::ir::TypeRef::Named(name) = &func.return_type {
            if let Some(rust_path) = newtype_rust_paths.get(name.as_str()) {
                func.return_newtype_wrapper = Some(rust_path.clone());
            }
        }
        resolve_typeref(&newtype_map, &mut func.return_type);
    }
    for enum_def in &mut surface.enums {
        for variant in &mut enum_def.variants {
            for field in &mut variant.fields {
                resolve_typeref(&newtype_map, &mut field.ty);
            }
        }
    }
}

/// Recursively replace `TypeRef::Named(name)` with the newtype's inner type.
fn resolve_typeref(newtype_map: &AHashMap<String, TypeRef>, ty: &mut TypeRef) {
    match ty {
        TypeRef::Named(name) => {
            if let Some(inner) = newtype_map.get(name.as_str()) {
                *ty = inner.clone();
            }
        }
        TypeRef::Optional(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Vec(inner) => resolve_typeref(newtype_map, inner),
        TypeRef::Map(k, v) => {
            resolve_typeref(newtype_map, k);
            resolve_typeref(newtype_map, v);
        }
        _ => {}
    }
}

/// Resolve unresolved `trait_source` on methods after all source files have been processed.
///
/// When `impl Trait for Type` is encountered before the trait definition has been extracted
/// (e.g., `pub mod extractors` comes before `pub mod plugins` in lib.rs), the single-segment
/// trait name lookup fails because the trait `TypeDef` doesn't exist yet. This pass retroactively
/// resolves those methods by matching method names against trait types' method lists.
fn resolve_trait_sources(surface: &mut ApiSurface) {
    // Build a map of trait method names -> trait rust_path for all known trait types.
    let mut trait_method_map: AHashMap<String, Vec<(String, String)>> = AHashMap::new();
    // Also build a map of trait_name -> set of method names, for disambiguation.
    let mut trait_methods_set: AHashMap<String, Vec<String>> = AHashMap::new();

    for typ in &surface.types {
        if !typ.is_trait {
            continue;
        }
        let method_names: Vec<String> = typ.methods.iter().map(|m| m.name.clone()).collect();
        trait_methods_set.insert(typ.name.clone(), method_names.clone());
        for method_name in &method_names {
            trait_method_map
                .entry(method_name.clone())
                .or_default()
                .push((typ.name.clone(), typ.rust_path.replace('-', "_")));
        }
    }

    if trait_method_map.is_empty() {
        return;
    }

    // For each non-trait type, collect unresolved method names first, then resolve.
    for typ in &mut surface.types {
        if typ.is_trait {
            continue;
        }

        // Collect the names of all unresolved methods on this type (for disambiguation).
        let unresolved_names: Vec<String> = typ
            .methods
            .iter()
            .filter(|m| m.trait_source.is_none())
            .map(|m| m.name.clone())
            .collect();

        for method in &mut typ.methods {
            if method.trait_source.is_some() {
                continue;
            }
            let Some(candidates) = trait_method_map.get(&method.name) else {
                continue;
            };

            if candidates.len() == 1 {
                method.trait_source = Some(candidates[0].1.clone());
            } else {
                // Pick the trait whose method set has the most overlap with this type's unresolved methods.
                let best = candidates.iter().max_by_key(|(trait_name, _)| {
                    trait_methods_set
                        .get(trait_name)
                        .map(|trait_ms| trait_ms.iter().filter(|m| unresolved_names.contains(m)).count())
                        .unwrap_or(0)
                });
                if let Some((_, rust_path)) = best {
                    method.trait_source = Some(rust_path.clone());
                }
            }
        }
    }
}

/// Extract items from a parsed syn file or module.
#[allow(clippy::too_many_arguments)]
fn extract_items(
    items: &[syn::Item],
    source_path: &Path,
    crate_name: &str,
    module_path: &str,
    surface: &mut ApiSurface,
    workspace_root: Option<&Path>,
    visited: &mut Vec<PathBuf>,
    result_wrapping_aliases: &mut ahash::AHashSet<String>,
) -> Result<()> {
    // Collect pub use re-exports at this level (for path flattening).
    // When a `pub use submod::*` or `pub use submod::TypeName` is found,
    // items defined in that submodule should get a shorter path (this level's path).
    let reexport_map = collect_reexport_map(items);

    // Pre-scan: detect generic type aliases whose definition wraps Result<T>.
    // e.g. `pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;`
    // When such an alias is used as `BoxFuture<'_, SomeType>`, the extractor should
    // mark the return as is_result=true even though `SomeType` isn't `Result<...>`.
    for item in items {
        if let syn::Item::Type(item_type) = item {
            if is_pub(&item_type.vis) && !item_type.generics.params.is_empty() {
                let name = item_type.ident.to_string();
                // Check if the RHS contains `Result<` — a heuristic that works for
                // `Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>` patterns.
                let rhs = quote::quote!(#item_type).to_string();
                if rhs.contains("Result <") || rhs.contains("Result<") {
                    result_wrapping_aliases.insert(name);
                }
            }
        }
    }

    // First pass: collect all structs/enums (no impl blocks yet)
    for item in items {
        match item {
            syn::Item::Struct(item_struct) if is_pub(&item_struct.vis) => {
                if let Some(td) = extract_struct(item_struct, crate_name, module_path) {
                    surface.types.push(td);
                }
            }
            syn::Item::Enum(item_enum) if is_pub(&item_enum.vis) => {
                if is_thiserror_enum(&item_enum.attrs) {
                    if let Some(ed) = extract_error_enum(item_enum, crate_name, module_path) {
                        surface.errors.push(ed);
                    }
                } else if let Some(ed) = extract_enum(item_enum, crate_name, module_path) {
                    surface.enums.push(ed);
                }
            }
            syn::Item::Fn(item_fn) if is_pub(&item_fn.vis) => {
                if let Some(fd) = extract_function(item_fn, crate_name, module_path) {
                    surface.functions.push(fd);
                }
            }
            syn::Item::Type(item_type) if is_pub(&item_type.vis) && item_type.generics.params.is_empty() => {
                // Type alias: pub type Foo = Bar;
                // Extract as a TypeDef with the aliased type
                let name = item_type.ident.to_string();
                let _ty = type_resolver::resolve_type(&item_type.ty);
                let rust_path = build_rust_path(crate_name, module_path, &name);
                let doc = extract_doc_comments(&item_type.attrs);
                surface.types.push(TypeDef {
                    name,
                    rust_path,
                    fields: vec![],
                    methods: vec![],
                    is_opaque: true, // type aliases are opaque (no fields)
                    is_clone: false,
                    is_trait: false,
                    has_default: false,
                    has_stripped_cfg_fields: false,
                    is_return_type: false,
                    doc,
                    cfg: None,
                    serde_rename_all: None,
                    has_serde: false,
                });
            }
            syn::Item::Trait(item_trait) if is_pub(&item_trait.vis) && item_trait.generics.params.is_empty() => {
                let name = item_trait.ident.to_string();
                let rust_path = build_rust_path(crate_name, module_path, &name);
                let doc = extract_doc_comments(&item_trait.attrs);

                // Extract trait methods
                let methods: Vec<MethodDef> = item_trait
                    .items
                    .iter()
                    .filter_map(|item| {
                        if let syn::TraitItem::Fn(method) = item {
                            let method_name = method.sig.ident.to_string();
                            let method_doc = extract_doc_comments(&method.attrs);
                            let mut is_async = method.sig.asyncness.is_some();
                            let (mut return_type, mut error_type, returns_ref) =
                                resolve_return_type(&method.sig.output);

                            // Check for BoxFuture async pattern
                            if !is_async {
                                if let Some((inner, future_error_type)) =
                                    functions::unwrap_future_return(&method.sig.output, result_wrapping_aliases)
                                {
                                    is_async = true;
                                    return_type = inner;
                                    // If the future's output is Result<T, E>, propagate the error type.
                                    if future_error_type.is_some() {
                                        error_type = future_error_type;
                                    }
                                }
                            }

                            // Skip generic methods
                            if !method.sig.generics.params.is_empty() {
                                return None;
                            }

                            let (receiver, is_static) = detect_receiver(&method.sig.inputs);
                            let params = extract_params(&method.sig.inputs);

                            Some(MethodDef {
                                name: method_name,
                                params,
                                return_type,
                                is_async,
                                is_static,
                                error_type,
                                doc: method_doc,
                                receiver,
                                sanitized: false,
                                trait_source: None,
                                returns_ref,
                                returns_cow: false,
                                return_newtype_wrapper: None,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                surface.types.push(TypeDef {
                    name,
                    rust_path,
                    fields: vec![],
                    methods,
                    is_opaque: true,
                    is_clone: false,
                    is_trait: true,
                    has_default: false,
                    has_stripped_cfg_fields: false,
                    is_return_type: false,
                    doc,
                    cfg: None,
                    serde_rename_all: None,
                    has_serde: false,
                });
            }
            syn::Item::Mod(item_mod) => {
                // Follow pub modules unconditionally.
                // Also follow non-pub modules whose items are re-exported via `pub use`
                // at this level (e.g., `mod ocr; pub use ocr::{OcrBackend, ...}`).
                // Without this, traits defined in private submodules wouldn't be extracted,
                // causing unresolved trait_source on methods in downstream types.
                let mod_name = item_mod.ident.to_string();
                let is_reexported = reexport_map.contains_key(&mod_name);
                if is_pub(&item_mod.vis) || is_reexported {
                    extract_module(
                        item_mod,
                        source_path,
                        crate_name,
                        module_path,
                        &reexport_map,
                        surface,
                        workspace_root,
                        visited,
                    )?;
                }
            }
            syn::Item::Use(item_use) if is_pub(&item_use.vis) => {
                resolve_use_tree(&item_use.tree, crate_name, surface, workspace_root, visited)?;
            }
            _ => {}
        }
    }

    // Build type name to index map for O(1) lookup
    let type_index: AHashMap<String, usize> = surface
        .types
        .iter()
        .enumerate()
        .map(|(idx, typ)| (typ.name.clone(), idx))
        .collect();

    // Second pass: process impl blocks using the index
    for item in items {
        if let syn::Item::Impl(item_impl) = item {
            extract_impl_block(
                item_impl,
                crate_name,
                module_path,
                surface,
                &type_index,
                result_wrapping_aliases,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
