//! Disambiguate types/enums/errors that share the same identifier across different
//! source modules. When two types collapse to the same binding-side ident
//! (e.g. `crate::SseEvent` and `crate::testing::SseEvent`), the second one is
//! renamed by prepending its PascalCase parent module segment (`TestingSseEvent`).
//!
//! The first-seen variant (sorted deterministically by `rust_path`) keeps its
//! original name. Subsequent collisions walk further up the module tree if needed
//! to find a unique prefix (parent, then grandparent::parent, etc.).
//!
//! All references to the renamed type (field types, param types, return types,
//! enum variant fields, super-traits, excluded-trait-names, excluded-type-paths)
//! are updated consistently so downstream codegen sees a coherent IR.
use crate::core::ir::ApiSurface;
use ahash::{AHashMap, AHashSet};
use heck::ToPascalCase;

/// Apply the disambiguation pass to the surface in place.
pub(crate) fn disambiguate_type_names(surface: &mut ApiSurface) {
    let renames = compute_renames(surface);
    if renames.is_empty() {
        return;
    }
    apply_renames(surface, &renames);
}

/// Build a map of `old_name -> new_name` for every collision.
fn compute_renames(surface: &ApiSurface) -> AHashMap<String, String> {
    // Collect (name, rust_path, kind, binding_excluded) for every nominal definition.
    // Three kinds share the binding-side namespace: types, enums, errors.
    let mut entries: Vec<(String, String, Kind, bool)> = Vec::new();
    for t in &surface.types {
        entries.push((t.name.clone(), t.rust_path.clone(), Kind::Type, t.binding_excluded));
    }
    for e in &surface.enums {
        entries.push((e.name.clone(), e.rust_path.clone(), Kind::Enum, e.binding_excluded));
    }
    for e in &surface.errors {
        entries.push((e.name.clone(), e.rust_path.clone(), Kind::Error, e.binding_excluded));
    }

    // Group by name. Tuple is (rust_path, kind, binding_excluded).
    let mut by_name: AHashMap<String, Vec<(String, Kind, bool)>> = AHashMap::new();
    for (name, path, kind, bx) in entries {
        by_name.entry(name).or_default().push((path, kind, bx));
    }

    // Compute the set of all currently used names so we never produce a fresh collision.
    let mut taken: AHashSet<String> = by_name.keys().cloned().collect();

    let mut renames: AHashMap<String, String> = AHashMap::new();

    // Deterministic iteration: sort group keys.
    let mut group_names: Vec<String> = by_name.keys().cloned().collect();
    group_names.sort();

    for name in group_names {
        let mut paths = by_name.remove(&name).expect("present");
        if paths.len() < 2 {
            continue;
        }
        // Sort by (binding_excluded ASC, rust_path ASC) so non-excluded entries come
        // first: a legitimate (bx=false) type always keeps the original name even when
        // an alef(skip)-annotated duplicate (bx=true) would sort earlier alphabetically.
        paths.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)));

        // Deduplicate by rust_path: if a bx=true and bx=false entry share the same
        // rust_path (e.g. a cfg-gated stub alongside a feature-guarded real definition),
        // the bx=true entry is a shadow/stub of the real one. Remove such shadows from
        // the group before computing renames so they do not count as collisions.
        //
        // After sorting, bx=false entries precede bx=true for the same path, so we
        // can deduplicate by taking the first occurrence of each rust_path.
        let mut seen_paths: AHashSet<String> = AHashSet::new();
        paths.retain(|(path, _kind, _bx)| seen_paths.insert(path.clone()));

        if paths.len() < 2 {
            continue;
        }

        // The first variant keeps its original name. All others get renamed.
        for (path, _kind, _bx) in paths.into_iter().skip(1) {
            let new_name = pick_unique_name(&name, &path, &taken);
            // Key the rename map by `path` so multiple collisions sharing the same
            // original short name don't overwrite each other.
            renames.insert(path, new_name.clone());
            taken.insert(new_name);
        }
    }

    renames
}

#[derive(Copy, Clone, Debug)]
enum Kind {
    Type,
    Enum,
    Error,
}

/// Find a unique name for `original` by walking up the module path of `rust_path`
/// and prepending PascalCase segments until the result is not in `taken`.
fn pick_unique_name(original: &str, rust_path: &str, taken: &AHashSet<String>) -> String {
    // rust_path like `my_crate::a::b::Original`. Segments excluding the final type
    // name and the leading crate name describe the module containment.
    let segments: Vec<&str> = rust_path.split("::").collect();
    if segments.len() <= 2 {
        // No module path to draw from; fall back to a numeric suffix.
        return numeric_suffix(original, taken);
    }

    let module_segments = &segments[1..segments.len() - 1];

    // Try increasingly long prefixes: parent only, then grandparent+parent, etc.
    for take in 1..=module_segments.len() {
        let start = module_segments.len() - take;
        let prefix: String = module_segments[start..]
            .iter()
            .map(|s| s.to_pascal_case())
            .collect::<Vec<_>>()
            .join("");
        let candidate = format!("{prefix}{original}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }

    // Exhausted module segments — fall back to numeric suffix.
    numeric_suffix(original, taken)
}

fn numeric_suffix(original: &str, taken: &AHashSet<String>) -> String {
    let mut n: u32 = 2;
    loop {
        let candidate = format!("{original}{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Apply `renames` (keyed by rust_path) to every name/reference in the surface.
fn apply_renames(surface: &mut ApiSurface, renames: &AHashMap<String, String>) {
    // Build a flat short-name -> new-name map for TypeRef::Named rewrites. This is
    // safe because the first-seen variant is unchanged, so any unqualified short-name
    // reference points to it. Renamed types are only referenced from their own
    // module's code paths; downstream IR currently has no path info to distinguish
    // sibling references, so all `Named("Foo")` resolve to the kept variant.
    //
    // We still rewrite the rename target's own self-references (field of type Self,
    // recursive methods) by tracking the path-keyed map: when we walk a type whose
    // rust_path matches a rename key, we apply the rename to its name only — its
    // internal Named references already point at the kept variant's short name and
    // continue to do so, which is the correct semantics for the unambiguous reference.
    for ty in &mut surface.types {
        if let Some(new_name) = renames.get(&ty.rust_path) {
            ty.name = new_name.clone();
        }
    }
    for en in &mut surface.enums {
        if let Some(new_name) = renames.get(&en.rust_path) {
            en.name = new_name.clone();
        }
    }
    for err in &mut surface.errors {
        if let Some(new_name) = renames.get(&err.rust_path) {
            err.name = new_name.clone();
        }
    }

    // excluded_type_paths is keyed by short name. If a kept-but-renamed entry exists
    // under the old key, rekey it. The values (rust_paths) are unaffected.
    let excluded: Vec<(String, String)> = surface.excluded_type_paths.drain().collect();
    for (name, path) in excluded {
        if let Some(new_name) = renames.get(&path) {
            surface.excluded_type_paths.insert(new_name.clone(), path);
        } else {
            surface.excluded_type_paths.insert(name, path);
        }
    }

    // excluded_trait_names is keyed by short name. Without path info on the set
    // entry we cannot disambiguate which excluded variant a name referred to;
    // leave it alone. Trait collisions of this shape are not currently observed
    // in the wild and would require richer excluded-set tracking to handle.
}

#[cfg(test)]
mod tests {
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, TypeDef};

    use super::disambiguate_type_names;

    fn make_type(name: &str, rust_path: &str) -> TypeDef {
        make_type_with_bx(name, rust_path, false)
    }

    fn make_type_with_bx(name: &str, rust_path: &str, binding_excluded: bool) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: rust_path.to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            doc: String::new(),
            cfg: None,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }
    }

    fn make_enum(name: &str, rust_path: &str) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: rust_path.to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Unit".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
            has_default: false,
        }
    }

    fn empty_surface() -> ApiSurface {
        ApiSurface::default()
    }

    #[test]
    fn pair_collision_renames_second_with_parent_prefix() {
        let mut s = empty_surface();
        s.types.push(make_type("Item", "my_crate::Item"));
        s.types.push(make_type("Item", "my_crate::testing::Item"));
        disambiguate_type_names(&mut s);
        let names: Vec<_> = s.types.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Item"), "first kept original name");
        assert!(names.contains(&"TestingItem"), "second renamed with PascalCase parent");
    }

    #[test]
    fn three_way_collision_uses_each_parent_segment() {
        let mut s = empty_surface();
        s.types.push(make_type("Foo", "my_crate::a::Foo"));
        s.types.push(make_type("Foo", "my_crate::bar::Foo"));
        s.types.push(make_type("Foo", "my_crate::baz::Foo"));
        disambiguate_type_names(&mut s);
        let names: Vec<_> = s.types.iter().map(|t| t.name.clone()).collect();
        // Deterministic sort by rust_path: a < bar < baz, so `a` keeps original.
        assert_eq!(names, vec!["Foo", "BarFoo", "BazFoo"]);
    }

    #[test]
    fn single_occurrence_unchanged() {
        let mut s = empty_surface();
        s.types.push(make_type("Solo", "my_crate::Solo"));
        disambiguate_type_names(&mut s);
        assert_eq!(s.types[0].name, "Solo");
    }

    #[test]
    fn distinct_idents_unchanged() {
        let mut s = empty_surface();
        s.types.push(make_type("Alpha", "my_crate::Alpha"));
        s.types.push(make_type("Beta", "my_crate::Beta"));
        disambiguate_type_names(&mut s);
        assert_eq!(s.types[0].name, "Alpha");
        assert_eq!(s.types[1].name, "Beta");
    }

    #[test]
    fn collision_across_type_and_enum_renames_second() {
        let mut s = empty_surface();
        s.types.push(make_type("Shared", "my_crate::Shared"));
        s.enums.push(make_enum("Shared", "my_crate::other::Shared"));
        disambiguate_type_names(&mut s);
        assert_eq!(s.types[0].name, "Shared");
        assert_eq!(s.enums[0].name, "OtherShared");
    }

    #[test]
    fn cascading_collision_walks_further_up() {
        let mut s = empty_surface();
        // Three Foos: my_crate::Foo, my_crate::ext::Foo, my_crate::other::ext::Foo
        // The second becomes ExtFoo; the third would also be ExtFoo (collision) so it
        // walks up one more segment and becomes OtherExtFoo.
        s.types.push(make_type("Foo", "my_crate::Foo"));
        s.types.push(make_type("Foo", "my_crate::ext::Foo"));
        s.types.push(make_type("Foo", "my_crate::other::ext::Foo"));
        disambiguate_type_names(&mut s);
        let names: Vec<_> = s.types.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, vec!["Foo", "ExtFoo", "OtherExtFoo"]);
    }

    #[test]
    fn snake_case_parent_segment_is_pascal_cased() {
        let mut s = empty_surface();
        s.types.push(make_type("Event", "my_crate::Event"));
        s.types.push(make_type("Event", "my_crate::sse_stream::Event"));
        disambiguate_type_names(&mut s);
        let names: Vec<_> = s.types.iter().map(|t| t.name.clone()).collect();
        assert!(names.contains(&"SseStreamEvent".to_string()));
    }

    #[test]
    fn bx_true_entry_yields_original_name_to_bx_false_entry() {
        // The bx=true path sorts before the bx=false path alphabetically ("my_crate::A..."
        // < "my_crate::B..."), but must not steal the original name. The bx=false entry
        // must always keep the original name; the bx=true entry gets renamed.
        let mut s = empty_surface();
        // bx=true entry — sorts first alphabetically by rust_path ("my_crate::AModule::Preset")
        s.types.push(make_type_with_bx(
            "EmbeddingPreset",
            "my_crate::AModule::EmbeddingPreset",
            true,
        ));
        // bx=false entry — legitimate type, sorts second alphabetically
        s.types.push(make_type_with_bx(
            "EmbeddingPreset",
            "my_crate::BModule::EmbeddingPreset",
            false,
        ));
        disambiguate_type_names(&mut s);
        let names: Vec<_> = s.types.iter().map(|t| t.name.clone()).collect();
        assert!(
            names.contains(&"EmbeddingPreset".to_string()),
            "bx=false entry must keep the original name; got: {names:?}"
        );
        assert!(
            !names.contains(&"EmbeddingPreset2".to_string()),
            "bx=false entry must not receive a numeric suffix; got: {names:?}"
        );
    }

    #[test]
    fn bx_true_shadow_with_same_path_not_counted_as_collision() {
        // A cfg-gated stub (bx=true) sharing the same rust_path as the real type (bx=false)
        // must NOT trigger a rename. The bx=true entry is a shadow of the real one;
        // deduplication by rust_path removes it from the collision group so the legitimate
        // type keeps its original name unchanged.
        let mut s = empty_surface();
        // Real type (bx=false) — feature-guarded in the real codebase
        s.types
            .push(make_type_with_bx("EmbeddingPreset", "my_crate::EmbeddingPreset", false));
        // Stub (bx=true) — same rust_path as the real type, injected by a cfg-gated block
        s.types
            .push(make_type_with_bx("EmbeddingPreset", "my_crate::EmbeddingPreset", true));
        disambiguate_type_names(&mut s);
        // Both entries survive in the surface with the same name (the bx=true will be
        // filtered downstream); crucially, neither should be renamed to EmbeddingPreset2.
        let names: Vec<_> = s.types.iter().map(|t| t.name.clone()).collect();
        assert!(
            names.iter().all(|n| n == "EmbeddingPreset"),
            "same-path shadow must not trigger a rename; got: {names:?}"
        );
    }
}
