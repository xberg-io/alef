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
