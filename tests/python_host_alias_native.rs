use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::{Command, Output};

#[path = "support/python_native.rs"]
mod python_native;

const NATIVE_CASES: [&str; 3] = [
    "legacy_host_errors_and_explicit_overrides_reach_checked_callbacks",
    "unrelated_rust_default_retains_host_alias_errors_and_overrides",
    "checked_and_delegated_callbacks_preserve_conversion_failures",
];

#[test]
fn should_retain_host_errors_through_configured_and_delegated_callbacks() {
    let (output, _) = run_native(true);
    assert_success(&output);
    assert_native_census(&output, "ok");
    assert!(String::from_utf8_lossy(&output.stdout).contains("3 passed; 0 failed"));
}

#[test]
fn should_detect_swallowed_errors_when_the_alias_is_disabled() {
    let (output, cases) = run_native(false);
    assert!(!output.status.success(), "legacy bridge unexpectedly preserved errors");
    assert_native_census(&output, "FAILED");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 passed; 3 failed"), "{stdout}");
    assert_eq!(cases.len(), NATIVE_CASES.len());
    for (name, case) in NATIVE_CASES.into_iter().zip(cases) {
        let stdout = String::from_utf8_lossy(&case.stdout);
        assert_eq!(
            legacy_failure(name, case.status.code(), &stdout),
            Ok(()),
            "{name}: {stdout}"
        );
    }
}

#[test]
fn should_reject_each_unrelated_failure_among_intended_legacy_failures() {
    for wrong in 0..NATIVE_CASES.len() {
        let actual: Vec<_> = NATIVE_CASES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let cause = if index == wrong {
                    "unrelated panic"
                } else {
                    legacy_cause(name)
                };
                let stdout = format!(
                    "running 1 test\ntest host_alias_runtime::{name} ... FAILED\n{cause}\n\
                     test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2 filtered out;"
                );
                legacy_failure(name, Some(101), &stdout)
            })
            .collect();
        let mut expected = vec![Ok(()); NATIVE_CASES.len()];
        expected[wrong] = Err("wrong failure cause");
        assert_eq!(
            actual, expected,
            "wrong cause in {} must be rejected",
            NATIVE_CASES[wrong]
        );
    }
}

fn legacy_cause(name: &str) -> &'static str {
    if name == NATIVE_CASES[1] {
        "the default must retain the fallible host path: 0"
    } else {
        "called `Result::unwrap_err()` on an `Ok` value: 0"
    }
}

fn legacy_failure(name: &str, code: Option<i32>, stdout: &str) -> Result<(), &'static str> {
    if code != Some(101) {
        return Err("unexpected native exit");
    }
    let row = format!("test host_alias_runtime::{name} ... FAILED");
    if !stdout.contains("running 1 test\n")
        || stdout.matches(&row).count() != 1
        || !stdout.contains("test result: FAILED. 0 passed; 1 failed; 0 ignored;")
    {
        return Err("unexpected native case census");
    }
    if !stdout.contains(legacy_cause(name)) {
        return Err("wrong failure cause");
    }
    Ok(())
}

fn run_native(aliased: bool) -> (Output, Vec<Output>) {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    write(
        root,
        "sample/Cargo.toml",
        "[package]\nname='sample'\nversion='0.1.0'\nedition='2024'\n",
    );
    write(
        root,
        "sample/src/lib.rs",
        include_str!("fixtures/python_host_aliases/core.rs"),
    );
    let source = root.join("sample/src/lib.rs");
    let api = alef::extract::extractor::extract(&[&source], "sample", "0.1.0", None).unwrap();
    assert_eq!(api.types.iter().filter(|typ| typ.is_trait).count(), 1);
    let configured =
        Pyo3Backend::with_host_aliases(&toml::from_str("[traits.Counter]\nchecked_count='count'\n").unwrap()).unwrap();
    let config = toml::from_str::<NewAlefConfig>(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample'\n\
         sources=[]\ncore_import='sample'\nerror_type='Error'\n\
         error_constructor='sample::Error::from({msg})'\n\
         [crates.python]\nmodule_name='_sample'\n\
         [[crates.trait_bridges]]\ntrait_name='Counter'\n",
    )
    .unwrap()
    .resolve()
    .unwrap()
    .remove(0);
    let backend: &dyn Backend = if aliased { &configured } else { &Pyo3Backend };
    let files = backend.generate_bindings(&api, &config).unwrap();
    let bindings: Vec<_> = files.iter().filter(|file| file.path.ends_with("lib.rs")).collect();
    assert_eq!(bindings.len(), 1, "must compile exactly one actual generated bridge");
    write_binding(root, &bindings[0].content);
    let mut lock = Command::new("cargo");
    let locked = lock
        .args(["generate-lockfile", "--offline"])
        .current_dir(root.join("binding"))
        .output()
        .unwrap();
    assert_success(&locked);
    let mut command = Command::new("cargo");
    python_native::configure_python(&mut command);
    let output = command
        .args([
            "test",
            "--lib",
            "--locked",
            "--offline",
            "--message-format=json",
            "-j1",
            "--",
            "--test-threads=1",
        ])
        .current_dir(root.join("binding"))
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .output()
        .unwrap();
    capture_native_evidence(root, aliased, &output);
    let cases = if aliased { vec![] } else { run_legacy_cases(&output) };
    (output, cases)
}

fn native_artifact(output: &Output) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| {
            row["reason"] == "compiler-artifact"
                && row["target"]["name"] == "python_host_alias_probe"
                && row["profile"]["test"] == true
                && row["executable"].is_string()
        })
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "must preserve one actual compiled native test executable"
    );
    rows.into_iter().next().unwrap()
}

fn run_legacy_cases(output: &Output) -> Vec<Output> {
    let artifact = native_artifact(output);
    NATIVE_CASES
        .iter()
        .map(|name| {
            let mut command = Command::new(artifact["executable"].as_str().unwrap());
            python_native::configure_python(&mut command);
            let output = command
                .args(["--exact", &format!("host_alias_runtime::{name}"), "--test-threads=1"])
                .output()
                .unwrap();
            if let Some(directory) = std::env::var_os("ALEF_NATIVE_FIXTURE_EVIDENCE") {
                let prefix = Path::new(&directory).join("legacy").join(name);
                std::fs::write(prefix.with_extension("stdout"), &output.stdout).unwrap();
                std::fs::write(prefix.with_extension("stderr"), &output.stderr).unwrap();
                std::fs::write(prefix.with_extension("exit"), format!("{:?}", output.status.code())).unwrap();
            }
            output
        })
        .collect()
}

fn capture_native_evidence(root: &Path, aliased: bool, output: &Output) {
    let Some(directory) = std::env::var_os("ALEF_NATIVE_FIXTURE_EVIDENCE") else {
        return;
    };
    let directory = Path::new(&directory).join(if aliased { "configured" } else { "legacy" });
    std::fs::create_dir(&directory).expect("native evidence destination must be fresh");
    std::fs::write(directory.join("stdout.jsonl"), &output.stdout).unwrap();
    std::fs::write(directory.join("stderr.log"), &output.stderr).unwrap();
    for name in ["src/lib.rs", "src/hosts.py", "Cargo.toml", "Cargo.lock"] {
        let destination = directory.join(name);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(root.join("binding").join(name), destination).unwrap();
    }
    let artifact = native_artifact(output);
    let executable = Path::new(artifact["executable"].as_str().unwrap());
    let copied = directory.join("native-probe");
    std::fs::copy(executable, &copied).unwrap();
    let digest: String = Sha256::digest(std::fs::read(&copied).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let receipt = serde_json::json!({"artifact": artifact, "exit": output.status.code(), "sha256": digest});
    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

fn write_binding(root: &Path, bridge: &str) {
    write(
        root,
        "binding/Cargo.toml",
        "[package]\nname='python-host-alias-probe'\nversion='0.1.0'\nedition='2024'\n\
         [dependencies]\nsample={path='../sample'}\n\
         pyo3={version='0.30',features=['auto-initialize']}\n\
         serde={version='1',features=['derive']}\nserde_json='1'\ntracing='0.1'\n",
    );
    write(
        root,
        "binding/src/lib.rs",
        &format!("{bridge}\n{}", include_str!("fixtures/python_host_aliases/probe.rs")),
    );
    write(
        root,
        "binding/src/hosts.py",
        include_str!("fixtures/python_host_aliases/hosts.py"),
    );
}

fn assert_native_census(output: &Output, outcome: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 3 tests"), "native tests did not run: {stdout}");
    for name in NATIVE_CASES {
        let row = format!("test host_alias_runtime::{name} ... {outcome}");
        assert_eq!(
            stdout.matches(&row).count(),
            1,
            "missing/duplicate native row: {row}\n{stdout}"
        );
    }
    assert!(
        stdout.contains("; 0 ignored;"),
        "unexpected skipped native case: {stdout}"
    );
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
