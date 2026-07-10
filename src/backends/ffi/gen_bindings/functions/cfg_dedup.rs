//! Same-named function entry deduplication for the FFI emitter.
//!
//! When a Rust crate exposes the same public function under two disjoint `cfg` gates — typically
//! a real implementation re-exported under `#[cfg(feature = "X")]` plus a stub fallback under
//! `#[cfg(all(feature = "X-presets", not(feature = "X")))]` — the extractor preserves both
//! entries in the shared `ApiSurface`. The shared surface is intentionally NOT collapsed
//! because:
//!
//! 1. The two entries usually carry distinct `rust_path` values (the crate-root stub path vs.
//!    the real-module re-export path), and downstream codegen plus the e2e call-export
//!    validator depend on both being visible.
//! 2. Collapsing in the extract pass would inherit `#[cfg_attr(alef, alef(skip))]` from
//!    whichever entry was selected as canonical, causing the merged result to be silently
//!    stripped by the exclusion filter and disappearing from every backend.
//!
//! At the FFI layer, however, both entries map to the *same* C symbol (`{prefix}_<name>`). Emitting
//! the symbol twice produces a duplicate-definition linker error; emitting only one entry gates the
//! symbol behind the cfg of whichever entry was iterated first, leaving the other build flavor
//! unresolved.
//!
//! `dedup_same_name_functions` resolves this locally for the FFI backend: it groups by `name`,
//! picks the canonical entry (preferring real impls — entries whose param names are not all
//! `_`-prefixed), and rewrites its `cfg` to the OR of every group member's cfg. All other
//! backends and the e2e validator see the original surface untouched.

use crate::core::ir::FunctionDef;
use ahash::{AHashMap, AHashSet};

/// Returns a deduplicated `Vec<FunctionDef>` derived from `functions`.
///
/// Functions whose `name` is unique in the input pass through unchanged.  Functions sharing a
/// `name` with at least one other entry are collapsed into a single canonical entry whose
/// `cfg` is the OR (`any(...)`) of every group member's cfg.  See the module-level docs for the
/// canonical-pick heuristic and the merge rules.
///
/// The relative order of canonical entries follows the position of each group's first member
/// in the input slice, matching the behavior the FFI emitter previously got from the extract
/// post-pass.
pub(in crate::backends::ffi::gen_bindings) fn dedup_same_name_functions(functions: &[FunctionDef]) -> Vec<FunctionDef> {
    let groups = collect_function_groups(functions);
    let groups_to_merge = groups_to_merge(&groups, functions);
    if groups_to_merge.is_empty() {
        return functions.to_vec();
    }

    let mut canonical_by_first_index: AHashMap<usize, FunctionDef> = AHashMap::new();
    let mut skipped_indices: AHashSet<usize> = AHashSet::new();
    for indices in &groups_to_merge {
        let merged_cfg = merge_cfgs(indices.iter().map(|&i| functions[i].cfg.as_deref()));
        let canonical_idx = pick_canonical_entry(indices, functions);
        let mut canonical = functions[canonical_idx].clone();
        canonical.cfg = merged_cfg;

        let first_idx = *indices.iter().min().expect("merge group indices are non-empty");
        canonical_by_first_index.insert(first_idx, canonical);

        for &idx in indices {
            if idx != first_idx {
                skipped_indices.insert(idx);
            }
        }
    }

    let mut merged_functions = Vec::with_capacity(functions.len() - skipped_indices.len());
    for (idx, function) in functions.iter().cloned().enumerate() {
        if let Some(canonical) = canonical_by_first_index.remove(&idx) {
            merged_functions.push(canonical);
        } else if !skipped_indices.contains(&idx) {
            merged_functions.push(function);
        }
    }
    merged_functions
}

fn collect_function_groups(functions: &[FunctionDef]) -> AHashMap<String, Vec<usize>> {
    let mut name_to_indices: AHashMap<String, Vec<usize>> = AHashMap::new();
    for (idx, func) in functions.iter().enumerate() {
        name_to_indices.entry(func.name.clone()).or_default().push(idx);
    }
    name_to_indices
}

fn groups_to_merge(groups: &AHashMap<String, Vec<usize>>, functions: &[FunctionDef]) -> Vec<Vec<usize>> {
    groups
        .values()
        .filter(|indices| should_merge_cfg_group(indices, functions))
        .cloned()
        .collect()
}

fn should_merge_cfg_group(indices: &[usize], functions: &[FunctionDef]) -> bool {
    if indices.len() <= 1 {
        return false;
    }
    let first_cfg = &functions[indices[0]].cfg;
    indices.iter().any(|&idx| &functions[idx].cfg != first_cfg)
}

/// Compute the OR-merge of a set of cfg strings.
///
/// - If any cfg is `None` (unconditional), returns `None`.
/// - If there is exactly one distinct value, returns it unchanged.
/// - Otherwise wraps all distinct values in `any(...)`.
fn merge_cfgs<'a>(cfgs: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut distinct: Vec<&str> = Vec::new();
    for cfg in cfgs {
        match cfg {
            None => return None,
            Some(s) => {
                if !distinct.contains(&s) {
                    distinct.push(s);
                }
            }
        }
    }
    match distinct.len() {
        0 => None,
        1 => Some(distinct[0].to_string()),
        _ => Some(format!("any({})", distinct.join(", "))),
    }
}

/// Pick the index of the "canonical" (real) entry from a group.
///
/// Prefers an entry whose params are NOT all underscore-prefixed (the stub convention).
/// Falls back to the first entry in the group.
fn pick_canonical_entry(indices: &[usize], functions: &[FunctionDef]) -> usize {
    for &idx in indices {
        let func = &functions[idx];
        let all_underscore = !func.params.is_empty() && func.params.iter().all(|p| p.name.starts_with('_'));
        if !all_underscore {
            return idx;
        }
    }
    indices[0]
}

#[cfg(test)]
mod tests;
