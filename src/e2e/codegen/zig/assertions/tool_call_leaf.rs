//! The binding of a `tool_calls[N]...` leaf for a Zig streaming assertion.
//!
//! The accessor for a deep tool-call path parses each collected chunk and copies the string leaf
//! out of the parsed value before the parse is freed (`streaming_assertions::renderers`). The
//! copy belongs to the caller, so the assertion binds it, frees it at the end of its own block,
//! and compares the bound name. A path that finds no leaf reads as the empty string, which
//! `free` accepts because it has zero length. ~keep

/// The name the leaf is bound to inside the assertion block.
pub(super) const LEAF: &str = "_tc_leaf";

/// Whether `field` is a deep tool-call path whose accessor returns a copy the caller must free.
pub(super) fn is_hoisted(field: &str) -> bool {
    field.starts_with("tool_calls[")
}

/// Wrap the assertion lines `out[start..]`, which compare [`LEAF`], in a block that binds
/// `accessor` to it and frees it when the block ends.
pub(super) fn wrap(out: &mut String, start: usize, accessor: &str) {
    let body = out.split_off(start);
    out.push_str("    {\n");
    out.push_str(&format!("        const {LEAF} = {accessor};\n"));
    out.push_str(&format!("        defer std.heap.c_allocator.free({LEAF});\n"));
    for line in body.lines() {
        out.push_str(&format!("    {line}\n"));
    }
    out.push_str("    }\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tool_call_paths_are_hoisted() {
        assert!(is_hoisted("tool_calls[0].function.name"));
        assert!(!is_hoisted("tool_calls"));
        assert!(!is_hoisted("usage.total_tokens"));
    }

    /// The generated Zig test must free the copy the accessor makes. The streaming entry point
    /// binds the leaf, defers the free and compares the bound name, never the raw accessor.
    #[test]
    fn a_tool_call_name_assertion_frees_the_copied_leaf() {
        use crate::e2e::field_access::FieldResolver;
        use crate::e2e::fixture::Assertion;
        use std::collections::{HashMap, HashSet};

        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("tool_calls[0].function.name".to_string()),
            value: Some(serde_json::json!("get_weather")),
            ..Assertion::default()
        };
        let mut out = String::new();
        super::super::render_json_assertion(&mut out, &assertion, "result", &resolver, true);

        assert!(out.contains("const _tc_leaf = @as([]const u8, blk: {"), "got:\n{out}");
        assert!(
            out.contains("defer std.heap.c_allocator.free(_tc_leaf);"),
            "the copy must be freed. Emitted:\n{out}"
        );
        assert!(
            out.contains("try testing.expectEqualStrings(\"get_weather\", _tc_leaf);"),
            "got:\n{out}"
        );
    }

    #[test]
    fn the_wrapped_block_binds_and_frees_the_leaf() {
        let mut out = String::from("    // before\n    try testing.expect(_tc_leaf.len > 0);\n");
        wrap(&mut out, "    // before\n".len(), "ACCESSOR");
        assert_eq!(
            out,
            "    // before\n    {\n        const _tc_leaf = ACCESSOR;\n        defer std.heap.c_allocator.free(_tc_leaf);\n        try testing.expect(_tc_leaf.len > 0);\n    }\n"
        );
    }
}
