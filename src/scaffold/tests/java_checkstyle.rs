use super::*;

#[test]
fn test_java_checkstyle_no_cosmetic_checks() {
    let mut config = test_config();
    config.languages = vec![Language::Java];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let checkstyle = files.iter().find(|f| f.path.ends_with("checkstyle.xml")).unwrap();
    assert!(!checkstyle.content.contains("WhitespaceAfter"));
    assert!(!checkstyle.content.contains("WhitespaceAround"));
    assert!(!checkstyle.content.contains("GenericWhitespace"));
    assert!(!checkstyle.content.contains("EmptyBlock"));
    assert!(!checkstyle.content.contains("NeedBraces"));
    assert!(!checkstyle.content.contains("MagicNumber"));
    assert!(!checkstyle.content.contains("JavadocPackage"));
    assert!(checkstyle.content.contains("EqualsHashCode"));
    assert!(checkstyle.content.contains("UnusedImports"));
    assert!(checkstyle.content.contains("MethodLength"));
    assert!(checkstyle.content.contains("LineLength"));
    assert!(checkstyle.content.contains("\"200\""));
}

#[test]
fn test_scaffold_java_checkstyle_suppressions_use_config_location() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let xml = files.iter().find(|f| f.path.ends_with("checkstyle.xml")).unwrap();
    assert!(
        xml.content.contains(r#"value="checkstyle-suppressions.xml""#),
        "checkstyle suppressions path must be relative to project basedir; content:\n{}",
        xml.content
    );
    let properties = files
        .iter()
        .find(|f| f.path.ends_with("checkstyle.properties"))
        .unwrap();
    assert!(
        properties.content.is_empty(),
        "checkstyle properties must be empty (0 bytes) so end-of-file-fixer leaves it untouched on every regen; a lone trailing newline gets stripped back to empty; content:\n{}",
        properties.content
    );
}

/// alef writes snippet-validation scratch sources under `.alef/snippets/sessions/<hash>/` inside
/// the scaffolded Java project (see `ValidationSession::workspace_directory`). Because
/// `sourceDirectory` is the project basedir, the maven-checkstyle-plugin walks that scratch
/// directory too unless the plugin config excludes it explicitly.
#[test]
fn test_scaffold_java_checkstyle_plugin_excludes_alef_scratch_directory() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files.iter().find(|f| f.path.ends_with("pom.xml")).unwrap();
    let checkstyle_section = pom
        .content
        .split("<artifactId>maven-checkstyle-plugin</artifactId>")
        .nth(1)
        .and_then(|section| section.split("</plugin>").next())
        .expect("pom.xml must configure maven-checkstyle-plugin");
    assert!(
        checkstyle_section.contains("<excludes>") && checkstyle_section.contains(".alef"),
        "checkstyle plugin must exclude the .alef/ snippet-validation scratch directory so \
         generated snippet scratch sources never fail `mvn compile`; block:\n{checkstyle_section}"
    );
}

/// Regression: `<sourcepath>${project.basedir}</sourcepath>` makes javadoc walk the WHOLE
/// project, including `src/test/java/`. Test sources import JUnit/AssertJ, which are
/// test-scoped and therefore absent from the javadoc classpath, so with the `failOnWarnings`
/// this pom also sets, `attach-javadocs` fails outright for any consumer that has Java tests
/// (observed as a `maven-javadoc-plugin:jar (attach-javadocs)` failure over
/// `packages/java/src/test/java/**`). maven-source-plugin already restricts itself the same
/// way for the same reason; javadoc was the one plugin left unrestricted. ~keep
#[test]
fn test_scaffold_java_javadoc_plugin_documents_only_publishable_sources() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files.iter().find(|f| f.path.ends_with("pom.xml")).unwrap();
    let javadoc_section = pom
        .content
        .split("<artifactId>maven-javadoc-plugin</artifactId>")
        .nth(1)
        .and_then(|section| section.split("</plugin>").next())
        .expect("pom.xml must configure maven-javadoc-plugin");
    assert!(
        javadoc_section.contains("<sourceFileIncludes>"),
        "javadoc plugin must restrict which sources it documents, or a basedir sourcepath \
         sweeps in src/test/java and fails the build; block:\n{javadoc_section}"
    );
    let includes = javadoc_section
        .split("<sourceFileIncludes>")
        .nth(1)
        .and_then(|block| block.split("</sourceFileIncludes>").next())
        .expect("the <sourceFileIncludes> block must be well-formed");
    assert!(
        !includes.contains("src/test/java"),
        "javadoc must never be pointed at test sources; includes:\n{includes}"
    );
    assert!(
        includes.contains("<sourceFileInclude>src/main/java/**/*.java</sourceFileInclude>"),
        "the conventional src/main/java overlay must stay documented; includes:\n{includes}"
    );
}

/// Bite test: builds the scaffolded pom/checkstyle config in a real temp Maven project and
/// runs `mvn -o validate` (the phase checkstyle is bound to). A genuine violation in a "real"
/// binding source must still fail the build; the same violation shape planted under
/// `.alef/snippets/sessions/<hash>/` must not. Skips when `mvn` is unavailable.
#[test]
fn test_scaffold_java_checkstyle_ignores_alef_scratch_but_still_catches_real_violations() {
    if !crate::test_support::mvn_is_runnable() {
        return;
    }
    // Spawns real `mvn` against a shared repository directory; see
    // `test_support::REAL_MVN_LOCK`'s doc for why this must be held for the whole test. ~keep
    let _mvn_lock = crate::test_support::RealMvnGuard::acquire();
    let maven_repo_local = format!(
        "-Dmaven.repo.local={}",
        crate::test_support::maven_local_repo_dir().display()
    );

    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);

    let project_dir = tempfile::tempdir().expect("temp project directory");
    for name in [
        "pom.xml",
        "checkstyle.xml",
        "checkstyle.properties",
        "checkstyle-suppressions.xml",
    ] {
        let file = files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap_or_else(|| panic!("scaffold must emit {name}"));
        std::fs::write(project_dir.path().join(name), &file.content).expect("write scaffolded file");
    }

    let group_id = config.java_group_id();
    let source_root = group_id.split('.').next().unwrap_or("dev");
    let package_dir = project_dir.path().join(source_root).join("scratchcheck");
    std::fs::create_dir_all(&package_dir).expect("package directory");

    let long_line = "x".repeat(220);

    // A genuine style violation in a real binding source must still fail `mvn validate`.
    let real_source = package_dir.join("RealSource.java");
    std::fs::write(
        &real_source,
        format!("package scratchcheck;\n\npublic final class RealSource {{\n    // {long_line}\n}}\n"),
    )
    .expect("write real source");

    let real_violation_output = std::process::Command::new("mvn")
        // Not `-o`: a freshly provisioned CI runner has no cached copy of the checkstyle/
        // source plugins this scaffolded `pom.xml` declares, so `-o` fails on plugin
        // resolution before checkstyle ever runs -- passing the `!success()` assertion
        // below for the wrong reason and never even reaching the one it exists to prove.
        // Runners have real network access; only Maven's own offline flag was blocking it.
        // ~keep
        .args(["-q", "validate", &maven_repo_local])
        .current_dir(project_dir.path())
        .output()
        .expect("mvn runs");
    assert!(
        !real_violation_output.status.success(),
        "checkstyle must still fail on a genuine violation in a real binding source; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&real_violation_output.stdout),
        String::from_utf8_lossy(&real_violation_output.stderr),
    );

    std::fs::remove_file(&real_source).expect("remove real violation source");

    // The same violation shape, planted as snippet-validation scratch, must be ignored.
    let scratch_dir = project_dir.path().join(".alef/snippets/sessions/deadbeefcafefeed");
    std::fs::create_dir_all(&scratch_dir).expect("scratch session directory");
    std::fs::write(
        scratch_dir.join("Example.java"),
        format!("public final class _TestVisitor {{\n    // {long_line}\n}}\n"),
    )
    .expect("write scratch source");

    let scratch_output = std::process::Command::new("mvn")
        // Not `-o`: see the comment on the first `mvn` invocation above -- same reason,
        // and here an unresolved plugin would fail this `success()` assertion for a
        // network problem instead of a real checkstyle regression. ~keep
        .args(["-q", "validate", &maven_repo_local])
        .current_dir(project_dir.path())
        .output()
        .expect("mvn runs");
    assert!(
        scratch_output.status.success(),
        "checkstyle must ignore .alef snippet-validation scratch sources; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&scratch_output.stdout),
        String::from_utf8_lossy(&scratch_output.stderr),
    );
}
