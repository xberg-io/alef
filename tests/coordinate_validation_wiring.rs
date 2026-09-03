//! The wiring test for coordinate validation: a hostile coordinate must be rejected by the real
//! production entry point, not merely by a validator someone remembered to call.
//!
//! An earlier version of this work shipped a fully correct grammar with zero production callers,
//! so an invalid coordinate flowed straight through resolution into generated manifests. These
//! tests exist to make that failure mode impossible to reintroduce silently: they drive the
//! actual `alef` binary over an actual `alef.toml`, and they assert on the artifacts on disk.
//!
//! Against the shipped 0.79.2 binary every hostile case below exited 0 and generated files:
//! `<groupId>dev"; System.exit(1); //</groupId>` landed verbatim in `pom.xml`, and a namespace of
//! `My.$(Evil)` produced a `.csproj` containing `<RootNamespace>My.$(Evil)</RootNamespace>`,
//! where `$(Evil)` is live MSBuild property expansion.

use std::path::Path;
use std::process::Command;

use alef::core::config::NewAlefConfig;

fn write_project(dir: &Path, java_package: &str, csharp_namespace: &str) {
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(dir.join("src/lib.rs"), "pub fn add(a: i64, b: i64) -> i64 { a + b }\n").expect("write lib.rs");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(dir.join("alef.toml"), config_toml(java_package, csharp_namespace)).expect("write alef.toml");
}

fn config_toml(java_package: &str, csharp_namespace: &str) -> String {
    format!(
        r#"[workspace]
languages = ["java", "csharp"]

[workspace.package_metadata]
repository = "https://example.com/sample-core"
authors = ["Sample Author <sample@example.com>"]
license = "MIT"
description = "Sample core library"

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.java]
package = {java_package:?}

[crates.csharp]
namespace = {csharp_namespace:?}
"#
    )
}

/// `(name, java package, csharp namespace)` — each is rejected by javac or dotnet, and each was
/// accepted end-to-end by alef before coordinate validation was wired into resolution.
const HOSTILE: &[(&str, &str, &str)] = &[
    ("java keyword segment", "dev.class", "Dev.Sample"),
    ("java path traversal", "../../etc", "Dev.Sample"),
    ("java source injection", "dev\"; System.exit(1); //", "Dev.Sample"),
    ("java empty segment", "dev..sample", "Dev.Sample"),
    ("csharp msbuild injection", "dev.sample", "My.$(Evil)"),
    ("csharp keyword segment", "dev.sample", "My.class"),
    ("csharp xml break-out", "dev.sample", "My\"><Evil/>"),
    ("csharp digit start", "dev.sample", "My.1Lib"),
];

const VALID: (&str, &str) = ("dev.example.samplecore", "Dev.Example.SampleCore");

/// ~keep Two independent, both-correct guards can refuse a hostile coordinate, and which one
/// answers depends on the characters in the value. `validate_language_specific_path_fields`
/// (`core::config::output`) runs first and rejects any raw path separator with "path separators
/// are not allowed"; `validate_package_coordinates` runs later and rejects on coordinate grammar
/// with "not a valid coordinate". Reordering them is not an option -- `path_safety_tests.rs` locks
/// in the first guard's precedence and wording. Asserting only the second guard's phrasing made
/// the most realistic exploit strings (the ones carrying `//` or `/>`, i.e. exactly the shapes
/// that make a source or XML injection work) look like test failures, and the tempting "fix" is to
/// strip those characters from the fixtures -- which silently deletes the coverage the fixtures
/// exist for. Accept either diagnostic instead: these tests assert that a hostile value is refused
/// before anything is written, not which guard refuses it.
fn is_hostile_coordinate_rejection(message: &str) -> bool {
    message.contains("not a valid coordinate") || message.contains("path separators are not allowed")
}

/// ~keep The same two guards label the offending field differently: the coordinate guard emits the
/// bracketed `[crates.java].package`, the path-separator guard the bare `java.package`. Accept
/// either spelling of the same field so the assertion still pins WHICH field was blamed.
fn names_offending_field(message: &str, bracketed_field: &str) -> bool {
    let bare = bracketed_field.replace("[crates.", "").replace(']', "");
    message.contains(bracketed_field) || message.contains(&bare)
}

#[test]
fn hostile_coordinates_are_rejected_by_the_real_cli_before_anything_is_written() {
    for &(name, java_package, csharp_namespace) in HOSTILE {
        let dir = tempfile::tempdir().expect("create temp workspace");
        write_project(dir.path(), java_package, csharp_namespace);

        let output = Command::new(env!("CARGO_BIN_EXE_alef"))
            .arg("--config")
            .arg(dir.path().join("alef.toml"))
            .arg("scaffold")
            .current_dir(dir.path())
            .output()
            .expect("run the alef binary");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "`{name}` must fail `alef scaffold`; it exited {:?}\nstderr:\n{stderr}",
            output.status.code()
        );
        assert!(
            is_hostile_coordinate_rejection(&stderr),
            "`{name}` must fail with a hostile-coordinate diagnostic, not an unrelated error\nstderr:\n{stderr}"
        );
        assert!(
            !dir.path().join("packages").exists(),
            "`{name}` must be rejected before any package file is written"
        );
    }
}

#[test]
fn a_valid_coordinate_still_scaffolds_through_the_same_cli_path() {
    // The opposite control. Without it, a validator that rejected everything would pass the test
    // above, and "nothing is generated any more" would read identically to "hostile input is
    // blocked".
    let dir = tempfile::tempdir().expect("create temp workspace");
    write_project(dir.path(), VALID.0, VALID.1);

    let output = Command::new(env!("CARGO_BIN_EXE_alef"))
        .arg("--config")
        .arg(dir.path().join("alef.toml"))
        .arg("scaffold")
        .current_dir(dir.path())
        .output()
        .expect("run the alef binary");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "the valid fixture must still scaffold\nstderr:\n{stderr}"
    );
    assert!(
        dir.path().join("packages/java/pom.xml").exists(),
        "the valid fixture must still produce pom.xml\nstderr:\n{stderr}"
    );
    let pom = std::fs::read_to_string(dir.path().join("packages/java/pom.xml")).expect("read pom.xml");
    assert!(
        pom.contains("<groupId>dev.example.samplecore</groupId>"),
        "pom.xml:\n{pom}"
    );
}

#[test]
fn resolution_itself_rejects_hostile_coordinates() {
    // Same gate one layer down, at `NewAlefConfig::resolve` — the function `load_config` calls
    // for every alef subcommand. Asserting here as well as through the binary means a refactor
    // that moves the call out of resolution cannot pass by relocating it into one CLI command.
    for &(name, java_package, csharp_namespace) in HOSTILE {
        let config: NewAlefConfig =
            toml::from_str(&config_toml(java_package, csharp_namespace)).expect("fixture parses");
        let error = config
            .resolve()
            .expect_err(&format!("`{name}` must not resolve"))
            .to_string();
        assert!(is_hostile_coordinate_rejection(&error), "`{name}`: {error}");
    }
}

#[test]
fn resolution_accepts_the_valid_coordinate() {
    let config: NewAlefConfig = toml::from_str(&config_toml(VALID.0, VALID.1)).expect("fixture parses");
    let resolved = config.resolve().expect("the valid fixture must resolve");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].java_package(), VALID.0);
    assert_eq!(resolved[0].csharp_namespace(), VALID.1);
}

#[test]
fn coordinate_validation_is_reachable_from_resolution_for_every_wired_language() {
    // Guards the scope of the wiring rather than one language's grammar: if a language's
    // coordinate check is ever dropped from `validate_package_coordinates`, this fails.
    for (language, table, bad) in [
        (
            "java",
            "[crates.java]\npackage = \"dev.class\"",
            "[crates.java].package",
        ),
        (
            "kotlin",
            "[crates.kotlin]\npackage = \"dev.fun\"",
            "[crates.kotlin].package",
        ),
        (
            "kotlin_android",
            "[crates.kotlin_android]\npackage = \"dev.class\"",
            "[crates.kotlin_android].package",
        ),
        (
            "csharp",
            "[crates.csharp]\nnamespace = \"My.class\"",
            "[crates.csharp].namespace",
        ),
        (
            "swift",
            "[crates.swift]\nmodule_name = \"Sample.Core\"",
            "[crates.swift].module_name",
        ),
        (
            "dart",
            "[crates.dart]\npubspec_name = \"Sample-Core\"",
            "[crates.dart].pubspec_name",
        ),
    ] {
        let toml = format!(
            "[workspace]\nlanguages = [{language:?}]\n\n\
             [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n{table}\n"
        );
        let config: NewAlefConfig = toml::from_str(&toml).expect("fixture parses");
        let error = config
            .resolve()
            .expect_err(&format!("{language} coordinate must be validated during resolution"))
            .to_string();
        assert!(error.contains(bad), "{language}: expected `{bad}` in: {error}");
    }
}

fn kotlin_only_toml(java_package: &str) -> String {
    format!(
        "[workspace]\nlanguages = [\"kotlin\"]\n\n\
         [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.java]\npackage = {java_package:?}\n"
    )
}

/// The gap this closes: `backends::kotlin::gen_bindings` (`emit_client_type_file`,
/// `generate_jvm`), `backends::kotlin::gen_mpp::emit_jvm_actual`, and
/// `backends::kotlin::gen_bindings::service_api::generate` all splice `config.java_package()`
/// verbatim into `import {java_package}.{Type}` and `{java_package}.{ClassName}` in emitted
/// `.kt` source -- and every one of them runs on a `languages = ["kotlin"]` build, with `java`
/// and `kotlin_android` both absent. Before this fix, only the `java`/`kotlin_android` branch of
/// `validate_jvm_coordinates` validated `[crates.java].package`, so this exact build shape
/// reached those four call sites with an unvalidated package.
#[test]
fn kotlin_only_build_validates_the_java_package_it_reuses() {
    let hostile = kotlin_only_toml("dev\"; System.exit(1); //");
    let config: NewAlefConfig = toml::from_str(&hostile).expect("fixture parses");
    let error = config
        .resolve()
        .expect_err("a hostile java package must not resolve on a kotlin-only build")
        .to_string();
    assert!(names_offending_field(&error, "[crates.java].package"), "{error}");
}

/// The opposite control for the test above: a validator that rejected every `[crates.java]`
/// value regardless of build shape would also make the hostile-only test pass.
#[test]
fn kotlin_only_build_accepts_a_valid_java_package() {
    let config: NewAlefConfig = toml::from_str(&kotlin_only_toml("dev.example.samplecore")).expect("fixture parses");
    let resolved = config
        .resolve()
        .expect("a valid java package must still resolve on a kotlin-only build");
    assert_eq!(resolved[0].java_package(), "dev.example.samplecore");
}

/// `(package, namespace, group_id, artifact_id)` -- every coordinate
/// `backends::kotlin_android::naming` derives from `[crates.kotlin_android]`, all four of which
/// reach generated output (see `validate_kotlin_android_coordinates` in `new_config.rs`).
fn kotlin_android_toml(package: &str, namespace: &str, group_id: &str, artifact_id: &str) -> String {
    format!(
        "[workspace]\nlanguages = [\"kotlin_android\"]\n\n\
         [[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.kotlin_android]\npackage = {package:?}\nnamespace = {namespace:?}\n\
         group_id = {group_id:?}\nartifact_id = {artifact_id:?}\n"
    )
}

const KOTLIN_ANDROID_VALID: (&str, &str, &str, &str) = (
    "dev.example.samplecore",
    "dev.example.samplecore.app",
    "dev.example.samplecore",
    "sample-core-android",
);

const KOTLIN_ANDROID_FIELDS: [&str; 4] = [
    "[crates.kotlin_android].package",
    "[crates.kotlin_android].namespace",
    "[crates.kotlin_android].group_id",
    "[crates.kotlin_android].artifact_id",
];

/// `(name, field index into `KOTLIN_ANDROID_FIELDS`/the valid tuple, hostile value)`. Every case
/// swaps exactly one field of [`KOTLIN_ANDROID_VALID`] for a hostile value, so the other three
/// fields are always the accepted defaults -- a validator that rejected everything could not
/// pass this alongside `kotlin_android_valid_coordinates_are_accepted_at_resolution`.
const KOTLIN_ANDROID_HOSTILE: &[(&str, usize, &str)] = &[
    ("package keyword segment", 0, "dev.class"),
    ("package source injection", 0, "dev\"; System.exit(1); //"),
    ("namespace keyword segment", 1, "dev.example.class"),
    ("namespace empty segment", 1, "dev..example"),
    ("group_id path traversal", 2, "../../evil"),
    ("group_id quote injection", 2, "dev\"); System.exit(1); //"),
    ("artifact_id path traversal", 3, "../../evil"),
    ("artifact_id leading dot", 3, ".evil"),
];

fn kotlin_android_toml_with_override(field_index: usize, value: &str) -> String {
    let mut fields = [
        KOTLIN_ANDROID_VALID.0.to_string(),
        KOTLIN_ANDROID_VALID.1.to_string(),
        KOTLIN_ANDROID_VALID.2.to_string(),
        KOTLIN_ANDROID_VALID.3.to_string(),
    ];
    fields[field_index] = value.to_string();
    kotlin_android_toml(&fields[0], &fields[1], &fields[2], &fields[3])
}

#[test]
fn kotlin_android_hostile_coordinates_are_rejected_at_resolution() {
    for &(name, field_index, value) in KOTLIN_ANDROID_HOSTILE {
        let toml = kotlin_android_toml_with_override(field_index, value);
        let config: NewAlefConfig = toml::from_str(&toml).expect("fixture parses");
        let error = config
            .resolve()
            .expect_err(&format!("`{name}` must not resolve"))
            .to_string();
        let expected_field = KOTLIN_ANDROID_FIELDS[field_index];
        assert!(
            names_offending_field(&error, expected_field),
            "`{name}`: expected `{expected_field}` in: {error}"
        );
    }
}

/// The binary control for the hostile matrix above: every one of `KOTLIN_ANDROID_VALID`'s four
/// fields, applied together, must still resolve.
#[test]
fn kotlin_android_valid_coordinates_are_accepted_at_resolution() {
    let toml = kotlin_android_toml(
        KOTLIN_ANDROID_VALID.0,
        KOTLIN_ANDROID_VALID.1,
        KOTLIN_ANDROID_VALID.2,
        KOTLIN_ANDROID_VALID.3,
    );
    let config: NewAlefConfig = toml::from_str(&toml).expect("fixture parses");
    let resolved = config.resolve().expect("the valid kotlin_android fixture must resolve");
    assert_eq!(resolved.len(), 1);
}
