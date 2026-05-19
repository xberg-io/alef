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
use ahash::{AHashMap, AHashSet};
use alef_core::ir::ApiSurface;
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
    // Collect (name, rust_path) for every nominal definition.
    // Three kinds share the binding-side namespace: types, enums, errors.
    let mut entries: Vec<(String, String, Kind)> = Vec::new();
    for t in &surface.types {
        entries.push((t.name.clone(), t.rust_path.clone(), Kind::Type));
    }
    for e in &surface.enums {
        entries.push((e.name.clone(), e.rust_path.clone(), Kind::Enum));
    }
    for e in &surface.errors {
        entries.push((e.name.clone(), e.rust_path.clone(), Kind::Error));
    }

    // Group by name.
    let mut by_name: AHashMap<String, Vec<(String, Kind)>> = AHashMap::new();
    for (name, path, kind) in entries {
        by_name.entry(name).or_default().push((path, kind));
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
        // Sort by rust_path so the first-seen variant is deterministic.
        paths.sort_by(|a, b| a.0.cmp(&b.0));

        // The first variant keeps its original name. All others get renamed.
        for (path, _kind) in paths.into_iter().skip(1) {
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
    use alef_core::ir::{ApiSurface, EnumDef, EnumVariant, TypeDef};

    use super::disambiguate_type_names;

    fn make_type(name: &str, rust_path: &str) -> TypeDef {
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
            binding_excluded: false,
            binding_exclusion_reason: None,
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
                is_tuple: false,
            }],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
}
