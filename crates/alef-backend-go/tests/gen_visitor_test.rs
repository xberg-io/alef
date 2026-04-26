use alef_backend_go::gen_visitor::gen_visitor_file;

/// The C struct name cbindgen emits is `{PREFIX}{PascalPrefix}VisitorCallbacks`.
/// For `ffi_prefix = "htm"`: `HTM` + `Htm` + `VisitorCallbacks` = `HTMHtmVisitorCallbacks`.
/// For `ffi_prefix = "abc"`: `ABC` + `Abc` + `VisitorCallbacks` = `ABCAbcVisitorCallbacks`.
/// Previously alef emitted `{PREFIX}VisitorCallbacks` (missing the pascal-prefix segment),
/// causing a compile error when the generated Go file referenced an unknown C type.
#[test]
fn test_visitor_c_struct_name_includes_prefix_pascal_segment() {
    let output = gen_visitor_file("mypkg", "htm", "my_lib.h", "crates/my-lib-ffi", "../../");
    assert!(
        output.contains("HTMHtmVisitorCallbacks"),
        "expected cbindgen-prefixed struct name 'HTMHtmVisitorCallbacks' in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(
        !output.contains("HTMVisitorCallbacks"),
        "output must not contain the unprefixed name 'HTMVisitorCallbacks'"
    );
}

#[test]
fn test_visitor_c_struct_name_generalises_to_other_prefixes() {
    let output = gen_visitor_file("mypkg", "abc", "my_lib.h", "crates/my-lib-ffi", "../../");
    assert!(
        output.contains("ABCAbcVisitorCallbacks"),
        "expected 'ABCAbcVisitorCallbacks' for ffi_prefix='abc', got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(
        !output.contains("ABCVisitorCallbacks"),
        "output must not contain the unprefixed name 'ABCVisitorCallbacks'"
    );
}
