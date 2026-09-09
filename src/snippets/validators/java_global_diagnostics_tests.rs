use super::*;

fn units() -> [JavaBatchUnit; 2] {
    ["sgood", "sbad"].map(|directory| JavaBatchUnit {
        directory: directory.into(),
        file_name: "Snippet.java".into(),
        source: String::new(),
    })
}

#[test]
fn a_global_classfile_warning_is_not_masked_by_another_snippets_error() {
    let output = "/batch/sbad/Snippet.java:1: error: cannot find symbol\nUndefined value;\n^\n/deps/Dependency.class: warning: [classfile] Cannot find annotation method 'value()' in type 'Missing'\n1 error\n1 warning\n";
    let results = JavaValidator::batch_results(&units(), true, false, output);
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result.0, SnippetStatus::Fail);
        assert!(result.1.expect("diagnostic").contains("Dependency.class"));
    }
}

#[test]
fn a_global_error_is_not_masked_by_another_snippets_error() {
    let output = "/batch/sbad/Snippet.java:1: error: cannot find symbol\nUndefined value;\n^\nerror: error reading dependency.jar; zip END header not found\n2 errors\n";
    let results = JavaValidator::batch_results(&units(), true, false, output);
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result.0, SnippetStatus::Fail);
        assert!(result.1.expect("diagnostic").contains("dependency.jar"));
    }
}

#[test]
fn global_warnings_do_not_fail_compile_level_or_hide_the_actual_error() {
    let output = "/deps/Dependency.class: warning: [classfile] Cannot find annotation method 'value()'\n/batch/sbad/Snippet.java:1: error: cannot find symbol\nUndefined value;\n^\n1 error\n1 warning\n";
    let results = JavaValidator::batch_results(&units(), false, false, output);
    assert_eq!(results[0], (SnippetStatus::Pass, None));
    assert_eq!(results[1].0, SnippetStatus::Fail);
    assert!(results[1].1.as_ref().expect("diagnostic").contains("Undefined"));
}

#[test]
#[ignore = "requires javac; executed explicitly in the release verification"]
fn real_javac_dependency_warning_and_broken_snippet_fail_the_entire_batch() {
    let _guard = crate::snippets::validators::jvm_toolchain_test_lock();
    let root = tempfile::tempdir().expect("fixture directory");
    let classes = dependency_without_annotation(root.path());
    let mut sources = Vec::new();
    for (directory, source) in [
        ("sgood", "class Good { Dependency value; }"),
        ("sbad", "class Bad { Undefined value; }"),
    ] {
        let path = root.path().join(directory);
        std::fs::create_dir(&path).expect("source directory");
        let path = path.join("Snippet.java");
        std::fs::write(&path, source).expect("source");
        sources.push(path);
    }
    let output = std::process::Command::new("javac")
        .args(["-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(&classes)
        .args(sources)
        .output()
        .expect("compile snippet batch");
    assert!(!output.status.success());
    let diagnostics = String::from_utf8(output.stderr).expect("UTF-8 diagnostics");
    assert!(diagnostics.contains("Undefined"), "{diagnostics}");
    assert!(diagnostics.contains("Dependency.class: warning:"), "{diagnostics}");
    let results = JavaValidator::batch_results(&units(), true, false, &diagnostics);
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|result| result.0 == SnippetStatus::Fail),
        "{results:?}"
    );
}

fn dependency_without_annotation(root: &std::path::Path) -> std::path::PathBuf {
    let classes = root.join("classes");
    std::fs::create_dir(&classes).expect("classes directory");
    let annotation = root.join("Missing.java");
    let dependency = root.join("Dependency.java");
    std::fs::write(&annotation, "public @interface Missing { String value(); }").expect("annotation");
    std::fs::write(&dependency, "@Missing(\"marker\") public class Dependency {}").expect("dependency");
    let output = std::process::Command::new("javac")
        .arg("-d")
        .arg(&classes)
        .arg(&annotation)
        .arg(&dependency)
        .output()
        .expect("compile fixture dependency");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_file(classes.join("Missing.class")).expect("remove only fixture annotation");
    classes
}

/// A javac run that failed with nothing attributable must fail every snippet carrying the real
/// output — never silently pass the batch. ~keep
#[test]
fn an_unattributable_javac_failure_fails_every_snippet_with_the_real_output() {
    let units = units();

    let results = JavaValidator::batch_results(&units, false, false, "error: invalid flag: --nonsense\n");

    assert_eq!(results.len(), 2);
    for result in &results {
        assert_eq!(result.0, SnippetStatus::Fail);
        assert_eq!(result.1.as_deref(), Some("error: invalid flag: --nonsense"));
    }
}
