use super::*;
use crate::core::validation::ValidatedApiSurface;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SOURCE: &str = r#"
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Empty {}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Input {
    Impossible(Empty),
    Text(String),
}
"#;

const EXPECTED_TS: &str = "export type AlefEmptyWire = never;\n\nexport type AlefInput = AlefEmptyWire | string;";

fn extracted_plan() -> AllUntaggedEnumsTsPlan {
    let directory = tempfile::tempdir().expect("source fixture directory");
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, SOURCE).expect("write actual Rust source");
    let api = crate::extract::extractor::extract(&[&source], "test_lib", "0.1.0", None).expect("public extraction");
    assert_eq!(api.enums.len(), 2);
    let empty = api.enums.iter().find(|en| en.name == "Empty").expect("extracted Empty");
    assert_eq!(empty.variants.len(), 0);
    let input = api.enums.iter().find(|en| en.name == "Input").expect("extracted Input");
    assert_eq!(input.variants.len(), 2);
    assert!(input.serde_untagged);
    assert_eq!(input.variants[0].fields[0].ty, TypeRef::Named("Empty".into()));
    ValidatedApiSurface::new(&api, &[]).expect("extracted source passes central validation");
    let plan = build_untagged_enum_ts_plan_for_api(&api, &[], &AHashSet::default(), &AHashSet::default(), "Alef");
    assert_eq!(plan.plans.len(), 1);
    plan
}

#[test]
fn extracted_empty_enum_payload_emits_never_alias() {
    let plan = extracted_plan();
    assert_eq!(plan.ts_body, EXPECTED_TS);
}

#[test]
fn nonempty_aliases_preserve_exact_member_bytes() {
    assert_eq!(render_alias("One", &["\"a\"".into()]), "export type One = \"a\";");
    assert_eq!(
        render_alias("Several", &["string".into(), "number[]".into(), "null".into()]),
        "export type Several = string | number[] | null;"
    );
}

#[test]
#[ignore = "requires explicit managed ALEF_TEST_TSC executable; never installs or skips on missing compiler"]
fn generated_empty_wire_alias_accepts_text_and_rejects_impossible_values() {
    let tsc = managed_tsc();
    let plan = extracted_plan();
    assert_eq!(plan.ts_body, EXPECTED_TS);
    let directory = tempfile::tempdir().expect("TypeScript contract directory");
    let passing = check_typescript(
        &tsc,
        directory.path(),
        "valid",
        &plan.ts_body,
        "const value: AlefInput = 'text';",
    );
    assert!(passing.status.success(), "{}", diagnostics(&passing));
    for (name, declaration, expected_diagnostic) in [
        (
            "empty",
            "const value: AlefEmptyWire = 'impossible';",
            "is not assignable to type 'never'",
        ),
        (
            "input",
            "const value: AlefInput = 42;",
            "Type 'number' is not assignable",
        ),
    ] {
        let rejected = check_typescript(&tsc, directory.path(), name, &plan.ts_body, declaration);
        assert_type_mismatch(name, &rejected, expected_diagnostic);
    }
}

fn assert_type_mismatch(name: &str, output: &Output, expected_diagnostic: &str) {
    let message = diagnostics(output);
    assert!(!output.status.success(), "{name} unexpectedly compiled");
    assert_eq!(message.matches("error TS").count(), 1, "{message}");
    assert!(message.contains("error TS2322:"), "{message}");
    assert!(message.contains(&format!("{name}.ts(")), "{message}");
    assert!(message.contains(expected_diagnostic), "{message}");
}

fn managed_tsc() -> PathBuf {
    let executable = std::env::var_os("ALEF_TEST_TSC").expect("supply the verified managed tsc path explicitly");
    let path = PathBuf::from(executable);
    assert!(
        path.is_absolute() && path.is_file(),
        "managed tsc must be an existing absolute file"
    );
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .expect("invoke managed tsc");
    assert!(output.status.success(), "{}", diagnostics(&output));
    assert!(
        diagnostics(&output).starts_with("Version "),
        "unexpected tsc version output"
    );
    path
}

fn check_typescript(tsc: &Path, directory: &Path, name: &str, emitted: &str, contract: &str) -> Output {
    let path = directory.join(format!("{name}.ts"));
    std::fs::write(&path, format!("{emitted}\n\n{contract}\n")).expect("write actual emitted TypeScript");
    Command::new(tsc)
        .args([
            "--strict",
            "--noEmit",
            "--skipLibCheck",
            "--target",
            "ES2020",
            "--pretty",
            "false",
        ])
        .arg(path)
        .output()
        .expect("compile actual emitted TypeScript")
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
