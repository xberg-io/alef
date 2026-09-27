/// Regression coverage for alef issue #464: `extract_module`'s `!found` branch (see
/// `reexports::extract_module`) previously returned `Ok(())` with no record at all when neither
/// candidate file existed for a `mod x;` declaration. That is a silent defect independent of
/// whatever it causes downstream -- this proves the cheap hook now records it, and names both
/// paths the extractor actually checked.
#[test]
fn a_mod_declaration_with_no_backing_file_is_recorded_not_silently_skipped() {
    let tmp = std::env::temp_dir().join("alef_test_unresolved_module_no_backing_file_464");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod missing;\n").unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "fixture_crate", "0.1.0", None).unwrap();

    assert_eq!(
        surface.unresolved_modules.len(),
        1,
        "a `mod` declaration whose file exists nowhere the extractor looked must be recorded; got {:?}",
        surface.unresolved_modules
    );
    let unresolved = &surface.unresolved_modules[0];
    assert_eq!(unresolved.module_path, "missing");
    assert_eq!(unresolved.declared_in, lib_rs);

    let expected_sibling = tmp.join("src/missing.rs");
    let expected_subdir_mod = tmp.join("src/missing/mod.rs");
    assert!(
        unresolved.candidates.contains(&expected_sibling),
        "candidates must include the sibling-file path actually checked; got {:?}",
        unresolved.candidates
    );
    assert!(
        unresolved.candidates.contains(&expected_subdir_mod),
        "candidates must include the subdirectory mod.rs path actually checked; got {:?}",
        unresolved.candidates
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// The anti-vacuity control for the test above: the exact same fixture shape, but with the
/// module's file actually present. Without this test, a change that records *every* `mod`
/// declaration (resolved or not) would pass the sibling test above just as well as the correct
/// fix.
#[test]
fn a_resolvable_mod_declaration_records_nothing() {
    let tmp = std::env::temp_dir().join("alef_test_unresolved_module_resolvable_464");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(tmp.join("src/lib.rs"), "pub mod present;\n").unwrap();
    std::fs::write(
        tmp.join("src/present.rs"),
        r#"
pub struct Something {
    pub value: u32,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "fixture_crate", "0.1.0", None).unwrap();

    assert!(
        surface.unresolved_modules.is_empty(),
        "a `mod` declaration whose file exists must never be recorded as unresolved; got {:?}",
        surface.unresolved_modules
    );
    assert_eq!(
        surface.types.len(),
        1,
        "the module's contents must still be extracted normally"
    );
    assert_eq!(surface.types[0].name, "Something");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// #464's third acceptance bullet: a `#[path = "..."]` module resolves against a file this
/// search never looks at, so it must never be reported as unresolved -- doing so would name a
/// file the crate never intended as a "missing" candidate.
#[test]
fn a_path_attributed_mod_is_never_recorded_as_unresolved() {
    let tmp = std::env::temp_dir().join("alef_test_unresolved_module_path_attribute_464");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
#[path = "elsewhere.rs"]
pub mod renamed;
"#,
    )
    .unwrap();
    // Deliberately no `src/renamed.rs`, `src/renamed/mod.rs`, nor `src/elsewhere.rs`: the
    // extractor has no `#[path]` handling, so none of these candidates are ever checked, and
    // none of them should be named as a "missing" file.

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "fixture_crate", "0.1.0", None).unwrap();

    assert!(
        surface.unresolved_modules.is_empty(),
        "a #[path] module must never be recorded as an unresolved module declaration; got {:?}",
        surface.unresolved_modules
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
