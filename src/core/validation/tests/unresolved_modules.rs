use super::*;
use crate::core::ir::UnresolvedModuleDeclaration;

#[test]
fn an_unresolved_module_reports_even_when_nothing_sanitized() {
    // Standalone defect (#464): a `mod x;` declaration the extractor could not resolve to a
    // source file must be reported on its own, independent of whether anything downstream was
    // sanitized as a result.
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        unresolved_modules: vec![UnresolvedModuleDeclaration {
            module_path: "credentials".to_string(),
            declared_in: "src/config.rs".into(),
            candidates: vec!["src/config/credentials.rs".into(), "src/credentials.rs".into()],
            declared_sources: vec!["src/lib.rs".into()],
        }],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    assert!(
        !report.has_errors(),
        "an unresolved module alone is a warning, not a fatal error: {report:?}"
    );
    assert_eq!(
        report.diagnostics.len(),
        1,
        "exactly one diagnostic for one unresolved module and nothing sanitized: {report:?}"
    );
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
    assert_eq!(diagnostic.code, ValidationCode::UnresolvedModuleDeclaration);
    assert!(diagnostic.reason.contains("credentials"), "{}", diagnostic.reason);
}

/// End-to-end regression for #464: a real on-disk crate with an unresolvable `mod` declaration,
/// extracted through the real `extractor::extract`, must carry both the standalone
/// unresolved-module warning and (once something is sanitized) the non-causal note pointing at
/// it -- in the same validation report. Fails against pre-#464 behaviour, which has neither.
#[test]
fn real_extraction_reports_both_unresolved_module_and_sanitized_note() {
    let tmp = std::env::temp_dir().join("alef_test_validation_unresolved_and_sanitized_464");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src/lib.rs"), "pub mod credentials;\n").unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let mut api = crate::extract::extractor::extract(&sources, "sample_lib", "0.1.0", None)
        .expect("extraction of the fixture crate must succeed");

    assert_eq!(
        api.unresolved_modules.len(),
        1,
        "the fixture's `mod credentials;` has no backing file anywhere and must be recorded: {:?}",
        api.unresolved_modules
    );

    // The lossy-placeholder rewrite itself is performed by the separate
    // `cli::pipeline::extract::sanitizer` stage, out of this test's reach; setting `sanitized`
    // directly reproduces its output for a field whose declared type this crate never resolved --
    // exactly the shape issue #464 reports.
    api.types.push(TypeDef {
        name: "Config".to_string(),
        rust_path: "sample_lib::Config".to_string(),
        fields: vec![FieldDef {
            sanitized: true,
            original_type: Some("Credentials".to_string()),
            ..field_def("credentials", TypeRef::String)
        }],
        ..TypeDef::default()
    });

    let report = validate_api_surface(&api);
    let rendered = report
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered.contains("unresolved_module_declaration") && rendered.contains("credentials"),
        "report must carry the standalone unresolved-module line: {rendered}"
    );
    assert!(
        rendered.contains("cannot tell"),
        "report must carry the non-causal sanitized-item note alongside it: {rendered}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
