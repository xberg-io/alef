use alef::e2e::field_access::FieldResolver;
use std::collections::{HashMap, HashSet};

#[test]
fn root_array_indices_are_executable_accessors() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    for language in ["ruby", "python", "rust"] {
        for index in [0, 1, 12] {
            assert_eq!(
                resolver.accessor(&format!("[{index}].id"), language, "result"),
                format!("result[{index}].id"),
                "{language}"
            );
        }
        assert_eq!(
            resolver.accessor("items[1].id", language, "result"),
            "result.items[1].id"
        );
    }
}

/// #417: the same `ArrayField { name: "" }` guard #411 applied to Ruby/Python/Rust is required
/// for every other backend's `render_accessor`/`render_*_with_optionals` renderer -- each one
/// unconditionally emitted `.` + name (an empty identifier) before the fix, producing a
/// double-dot or an empty-getter call that cannot compile: `result.[0].id`,
/// `result.().get(0).id()`, `result.()[0].id()`, and so on. Expected strings below follow each
/// backend's OWN existing convention for a non-root indexed field (as asserted throughout
/// `src/e2e/field_access/tests.rs`), simply with the root-level empty-name segment collapsed
/// away instead of rendered. ~keep
#[test]
fn root_array_indices_are_executable_accessors_for_remaining_backends() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    let cases: &[(&str, &str, &str)] = &[
        // (language, root-array accessor for "[0].id", non-root accessor for "items[1].id")
        ("typescript", "result[0].id", "result.items[1].id"),
        ("node", "result[0].id", "result.items[1].id"),
        ("wasm", "result[0].id", "result.items[1].id"),
        ("java", "result.get(0).id()", "result.items().get(1).id()"),
        ("kotlin", "result.first().id()", "result.items().get(1).id()"),
        ("kotlin_android", "result.first().id", "result.items.get(1).id"),
        ("csharp", "result[0].Id", "result.Items[1].Id"),
        ("zig", "result[0].id", "result.items[1].id"),
        ("swift", "result[0].id()", "result.items()[1].id()"),
        ("dart", "result[0].id", "result.items[1].id"),
        ("php", "result[0]->id", "result->items[1]->id"),
        ("go", "result[0].ID", "result.Items[1].ID"),
        ("r", "result[[1]]$id", "result$items[[2]]$id"),
    ];

    for (language, root_expected, indexed_expected) in cases {
        assert_eq!(
            resolver.accessor("[0].id", language, "result"),
            *root_expected,
            "{language}: root array accessor"
        );
        assert_eq!(
            resolver.accessor("items[1].id", language, "result"),
            *indexed_expected,
            "{language}: non-root array accessor must be unchanged"
        );
    }
}
