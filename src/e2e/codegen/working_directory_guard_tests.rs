//! Regression coverage for one decision every e2e backend that emits a build-tool-level (or
//! process-level) working-directory setting must make the same way: guard it on the
//! test_documents directory's existence.
//!
//! Unguarded, a forked test worker (Gradle) or Surefire's forked JVM (Maven) fails to fork
//! at all when the directory does not exist -- Gradle reports a misleading "Gradle Test
//! Executor N ... not in started or detached state" with the real fork `IOException` masked
//! and no assertion text at all. A fresh checkout of a consumer repo whose `test_documents/`
//! fixture directory has zero tracked files hits this on every run.
//!
//! Five backends implement this decision independently: Kotlin and Kotlin Android both emit
//! Gradle Kotlin DSL `workingDir =`; Java emits a Maven Surefire `<workingDirectory>`; C#
//! emits a runtime `Directory.SetCurrentDirectory` chdir inside the generated test binary's
//! module initializer instead of a build-file setting; Zig's `build.zig` calls
//! `RunStep.setCwd` at build-configure time. Gradle DSL, Maven XML, "chdir at runtime in the
//! emitted binary", and "RunStep.setCwd in build.zig" are different enough mechanisms that a
//! single shared Rust/template helper spanning all of them does not fit -- see CLAUDE.md's
//! `avoid-duplication` rule (shared code must have one reason to change; these do not share
//! one). Kotlin and Kotlin Android *do* share a mechanism (both are Gradle Kotlin DSL) and
//! share the `gradle/guarded_working_dir.kt.jinja` template for it.
//!
//! What still needs a guardrail is the *decision*, independent of syntax: this test pins it
//! for every backend that emits one, through each backend's own public
//! [`super::E2eCodegen::generate`] entry point (rather than each backend's own
//! privately-scoped renderer, which a peer module cannot reach across a private `mod`
//! boundary) -- so a regression in an existing backend, or a new backend that reintroduces an
//! unguarded working-directory setting, is caught here even before it is added to this file's
//! own suite.

use super::E2eCodegen;
use super::csharp::CSharpCodegen;
use super::dart::DartE2eCodegen;
use super::go::GoCodegen;
use super::java::JavaCodegen;
use super::kotlin::KotlinE2eCodegen;
use super::kotlin_android::KotlinAndroidE2eCodegen;
use super::php::PhpCodegen;
use super::python::PythonE2eCodegen;
use super::ruby::RubyCodegen;
use super::swift::SwiftE2eCodegen;
use super::typescript::TypeScriptCodegen;
use super::wasm::WasmCodegen;
use super::zig::ZigE2eCodegen;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};

fn generated_file_ending_in<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a GeneratedFile {
    files.iter().find(|f| f.path.ends_with(suffix)).unwrap_or_else(|| {
        panic!(
            "expected a generated file ending in `{suffix}`, got: {:?}",
            files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
        )
    })
}

fn nested_bytes_fixture() -> (E2eConfig, FixtureGroup, TypeDef) {
    let config = E2eConfig {
        call: CallConfig {
            function: "process".into(),
            module: "sample_package".into(),
            args: vec![ArgMapping {
                name: "request".into(),
                field: "input".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: true,
                element_type: Some("SampleRequest".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let group = FixtureGroup {
        category: "documents".into(),
        fixtures: vec![Fixture {
            id: "process_document".into(),
            description: "Process a local document".into(),
            input: serde_json::json!({"content": "documents/sample.bin"}),
            ..Default::default()
        }],
    };
    let request = TypeDef {
        name: "SampleRequest".into(),
        fields: vec![FieldDef {
            name: "content".into(),
            ty: TypeRef::Bytes,
            ..Default::default()
        }],
        ..Default::default()
    };
    (config, group, request)
}

#[test]
fn node_nested_bytes_fixture_emits_test_document_setup() {
    let (e2e_config, group, request) = nested_bytes_fixture();
    let files = TypeScriptCodegen
        .generate(
            &[group],
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[request],
            &[],
            &[],
            &[],
        )
        .expect("Node e2e generation succeeds");

    generated_file_ending_in(&files, "setup.ts");
}

#[test]
fn python_nested_bytes_fixture_emits_test_document_chdir() {
    let (e2e_config, group, request) = nested_bytes_fixture();
    let files = PythonE2eCodegen
        .generate(
            &[group],
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[request],
            &[],
            &[],
            &[],
        )
        .expect("Python e2e generation succeeds");
    let conftest = generated_file_ending_in(&files, "conftest.py");

    assert!(
        conftest.content.contains("os.chdir(_TEST_DOCUMENTS)"),
        "nested bytes file reads require the generated conftest to enter test_documents, got:\n{}",
        conftest.content
    );
}

fn assert_nested_bytes_fixture_emits_setup(generator: &dyn E2eCodegen, expected: &str) {
    let (e2e_config, group, request) = nested_bytes_fixture();
    let files = generator
        .generate(
            &[group],
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[request],
            &[],
            &[],
            &[],
        )
        .unwrap_or_else(|error| panic!("nested bytes fixture generation failed: {error}"));
    assert!(
        files.iter().any(|file| file.content.contains(expected)),
        "expected nested bytes setup containing `{expected}`, got files: {:?}",
        files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn remaining_backends_detect_nested_bytes_fixture_paths() {
    for (generator, expected) in [
        (&DartE2eCodegen as &dyn E2eCodegen, "Directory.current = _dir"),
        (&GoCodegen, "os.Chdir(testDocumentsDir)"),
        (&PhpCodegen, "chdir($_test_documents)"),
        (&RubyCodegen, "Dir.chdir(_test_documents)"),
        (&SwiftE2eCodegen, "FileManager.default.changeCurrentDirectoryPath"),
        (&WasmCodegen, "process.chdir(testDocumentsDir)"),
        (&ZigE2eCodegen, ".setCwd(b.path("),
    ] {
        assert_nested_bytes_fixture_emits_setup(generator, expected);
    }
}

#[test]
fn kotlin_build_gradle_guards_working_dir_on_existence() {
    let files = KotlinE2eCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("kotlin e2e generation succeeds on an empty fixture set");
    let build_gradle = generated_file_ending_in(&files, "build.gradle.kts");
    assert!(
        build_gradle.content.contains(".isDirectory"),
        "plain-Kotlin build.gradle.kts must guard workingDir on directory existence, got:\n{}",
        build_gradle.content
    );
}

#[test]
fn kotlin_android_build_gradle_guards_working_dir_on_existence() {
    let files = KotlinAndroidE2eCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("kotlin_android e2e generation succeeds on an empty fixture set");
    let build_gradle = generated_file_ending_in(&files, "build.gradle.kts");
    assert!(
        build_gradle.content.contains(".isDirectory"),
        "kotlin_android build.gradle.kts must guard workingDir on directory existence, got:\n{}",
        build_gradle.content
    );
}

#[test]
fn java_pom_xml_guards_working_directory_on_existence() {
    let files = JavaCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("java e2e generation succeeds on an empty fixture set");
    let pom_xml = generated_file_ending_in(&files, "pom.xml");
    let (unconditional, guarded) = pom_xml.content.split_once("<profiles>").unwrap_or(("", ""));
    assert!(
        !unconditional.contains("<workingDirectory>"),
        "the unconditionally-active surefire plugin config must not set workingDirectory \
         unguarded, got:\n{}",
        pom_xml.content
    );
    assert!(
        guarded.contains("<activation>") && guarded.contains("<exists>"),
        "workingDirectory must be set only inside a file-existence-activated profile, got:\n{}",
        pom_xml.content
    );
}

#[test]
fn zig_build_zig_guards_set_cwd_on_working_directory_existence() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "detect_mime_type_from_bytes".to_string(),
            args: vec![ArgMapping {
                name: "content".to_string(),
                field: "input.data".to_string(),
                arg_type: "file_path".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let fixture = Fixture {
        id: "mime_detect_from_path".to_string(),
        description: "Detect MIME type from a file path".to_string(),
        input: serde_json::json!({"data": "pdf/fake_memo.pdf"}),
        ..Fixture::default()
    };
    let groups = [FixtureGroup {
        category: "mime".to_string(),
        fixtures: vec![fixture],
    }];

    let files = ZigE2eCodegen
        .generate(
            &groups,
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("zig e2e generation succeeds for a file-fixture-bearing fixture set");
    let build_zig = generated_file_ending_in(&files, "build.zig");
    assert!(
        build_zig.content.contains("openDir("),
        "build.zig must guard RunStep.setCwd on the test_documents directory's existence, got:\n{}",
        build_zig.content
    );
}

/// THE E4 REGRESSION: `remaining_backends_detect_nested_bytes_fixture_paths` above only proves
/// the Dart `setUpAll` body CONTAINS `Directory.current = _dir` somewhere -- it never checks
/// ORDER, so it passed even while `test_file.rs` emitted the chdir BEFORE
/// `render_dart_sut_spawn`'s `Directory.current.uri.resolve('../rust/Cargo.toml')` /
/// `resolve('app_harness.dart')` calls resolved their paths against the ALREADY-CHANGED cwd
/// (`test_documents/`, not `e2e/dart/`) -- `Bad state: mock-server build failed: error:
/// manifest path .../rust/Cargo.toml does not exist` in CI, six suites failing in `setUpAll`
/// with zero fixture assertions run. A fixture SET must combine a file-input fixture (drives
/// `needs_chdir`) with a mock-url fixture (drives `needs_sut_spawn`) in the SAME group --
/// a real downstream crate's actual `contract_test.dart`/`format_specific_test.dart`/
/// `url_test.dart` shape -- neither flag alone reaches the interaction. ~keep
#[test]
fn dart_chdir_and_sut_spawn_combined_setup_resolves_sut_paths_before_the_chdir() {
    let (e2e_config, mut group, request) = nested_bytes_fixture();
    group.fixtures.push(Fixture {
        id: "mock_url_contract".into(),
        description: "Fetch a document from the mock server".into(),
        input: serde_json::json!({"uri": "$mock_url/markdown/comprehensive.md"}),
        ..Default::default()
    });
    let files = DartE2eCodegen
        .generate(
            &[group],
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[request],
            &[],
            &[],
            &[],
        )
        .expect("Dart e2e generation succeeds for a combined chdir + sut-spawn fixture set");
    let test_file = generated_file_ending_in(&files, "documents_test.dart");

    let chdir_pos = test_file
        .content
        .find("Directory.current = _dir")
        .expect("expected the file-input fixture to still emit the test-documents chdir");
    let sut_spawn_pos = test_file
        .content
        .find("Directory.current.uri.resolve('app_harness.dart')")
        .expect("expected the mock-url fixture to still emit the SUT app-harness spawn");

    assert!(
        sut_spawn_pos < chdir_pos,
        "the SUT spawn's own `Directory.current`-relative path resolution must run BEFORE the \
         test-documents chdir reassigns `Directory.current`, or it resolves 'app_harness.dart' \
         and '../rust/Cargo.toml' against the wrong base; got:\n{}",
        test_file.content
    );
}

#[test]
fn csharp_test_setup_guards_working_directory_change_on_existence() {
    let files = CSharpCodegen
        .generate(
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("csharp e2e generation succeeds on an empty fixture set");
    let test_setup = generated_file_ending_in(&files, "TestSetup.cs");
    assert!(
        test_setup.content.contains("Directory.Exists"),
        "TestSetup.cs must guard Directory.SetCurrentDirectory on Directory.Exists, got:\n{}",
        test_setup.content
    );
}
