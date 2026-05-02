use alef_backend_go::gen_visitor::gen_visitor_file;

/// Smoke test: gen_visitor_file produces output with the expected prefix structure.
/// The exact C struct name depends on `vtable_trait_name` and `ffi_prefix`.
#[test]
fn test_visitor_file_emits_prefixed_struct() {
    let output = gen_visitor_file(
        "mypkg",
        "htm",
        "my_lib.h",
        "crates/my-lib-ffi",
        "../../",
        "HtmlVisitor",
        "visitor",
    );
    // The cbindgen-derived C type embeds `{PREFIX}{PascalPrefix}{TraitName}VTable`.
    assert!(
        output.contains("VTable"),
        "expected VTable in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(output.contains("HTM"), "expected upper-case prefix HTM in output");
}
