use alef_backend_go::gen_visitor::gen_visitor_file;

/// The VTable C struct name is `{PREFIX_UPPER}{PrefixPascal}{TraitName}VTable`.
///
/// For `ffi_prefix = "htm"`, `vtable_trait_name = "HtmlVisitor"`:
/// Rust struct `HtmHtmlVisitorVTable` → cbindgen emits `HTMHtmHtmlVisitorVTable`.
///
/// The `NodeContext` C type is simply `{PREFIX_UPPER}NodeContext` because the Rust struct
/// is named `NodeContext` (no FFI prefix in the name), so cbindgen emits `HTMNodeContext`.
#[test]
fn test_visitor_vtable_struct_name_includes_prefix_and_trait() {
    let output = gen_visitor_file("mypkg", "htm", "my_lib.h", "crates/my-lib-ffi", "../../", "HtmlVisitor");
    assert!(
        output.contains("HTMHtmHtmlVisitorVTable"),
        "expected cbindgen-prefixed VTable struct 'HTMHtmHtmlVisitorVTable' in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    // Old double-prefixed name must not appear.
    assert!(
        !output.contains("HTMHtmVisitorCallbacks"),
        "output must not contain the stale name 'HTMHtmVisitorCallbacks'"
    );
}

#[test]
fn test_node_context_type_has_no_double_prefix() {
    // NodeContext is defined in Rust as `NodeContext` (no FFI prefix).
    // cbindgen emits `HTMNodeContext`, NOT `HTMHtmNodeContext`.
    let output = gen_visitor_file("mypkg", "htm", "my_lib.h", "crates/my-lib-ffi", "../../", "HtmlVisitor");
    assert!(
        output.contains("HTMNodeContext"),
        "expected 'HTMNodeContext' (no double prefix) in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(
        !output.contains("HTMHtmNodeContext"),
        "output must not contain the double-prefixed name 'HTMHtmNodeContext'"
    );
}

#[test]
fn test_visitor_vtable_struct_name_generalises_to_other_prefixes() {
    let output = gen_visitor_file("mypkg", "abc", "my_lib.h", "crates/my-lib-ffi", "../../", "OcrBackend");
    assert!(
        output.contains("ABCAbcOcrBackendVTable"),
        "expected 'ABCAbcOcrBackendVTable' for ffi_prefix='abc', trait='OcrBackend', got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(
        !output.contains("ABCAbcVisitorCallbacks"),
        "output must not contain the stale name 'ABCAbcVisitorCallbacks'"
    );
}
