use alef::core::config::{Language, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use alef::scaffold::scaffold;
use std::path::Path;
use std::process::Command;

#[path = "support/managed_tool.rs"]
mod managed_tool;

fn emitted_loader() -> (String, String) {
    let api = ApiSurface {
        crate_name: "toolkit".into(),
        version: "0.1.0".into(),
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "toolkit".into(),
        languages: vec![Language::Node],
        ..Default::default()
    };
    let files = scaffold(&api, &config, &[Language::Node]).expect("emit the actual consumer scaffold");
    let content = |path: &str| {
        let matches: Vec<_> = files.iter().filter(|file| file.path == Path::new(path)).collect();
        assert_eq!(matches.len(), 1, "exactly one emitted {path} must be checked");
        matches[0].content.clone()
    };
    (content("crates/toolkit-node/index.js"), content("poly.toml"))
}

fn format_check(seed: &str, config: &str) -> (bool, serde_json::Value) {
    format_check_with_path(seed, config, None)
}

fn format_check_with_path(seed: &str, config: &str, path: Option<&std::ffi::OsStr>) -> (bool, serde_json::Value) {
    let directory = tempfile::tempdir().expect("create isolated consumer formatter fixture");
    std::fs::write(directory.path().join("index.js"), seed).unwrap();
    std::fs::write(directory.path().join("poly.toml"), config).unwrap();
    let mut command = Command::new(managed_tool::executable("poly"));
    command
        .args(["fmt", "--check", "--no-cache", "--format", "json", "index.js"])
        .current_dir(directory.path());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let output = command
        .output()
        .expect("the managed Poly formatter is required; do not skip this regression");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "formatter emitted no JSON report: {error}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(report["summary"]["checked"], 1, "{report}");
    assert_eq!(report["summary"]["skipped"], 0, "{report}");
    assert_eq!(report["summary"]["errored"], 0, "{report}");
    assert_eq!(report["results"].as_array().unwrap().len(), 1, "{report}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("index.js")).unwrap(),
        seed
    );
    (output.status.success(), report)
}

#[test]
fn consumer_formatter_bypasses_path_shims_without_project_configuration() {
    let poisoned = tempfile::Builder::new().prefix("alef poly shim ").tempdir().unwrap();
    std::fs::copy(
        std::env::current_exe().unwrap(),
        poisoned.path().join(format!("poly{}", std::env::consts::EXE_SUFFIX)),
    )
    .unwrap();
    let inherited = std::env::var_os("PATH").expect("test process has PATH");
    let path =
        std::env::join_paths(std::iter::once(poisoned.path().to_path_buf()).chain(std::env::split_paths(&inherited)))
            .unwrap();
    let (seed, config) = emitted_loader();
    let (passed, report) = format_check_with_path(&seed, &config, Some(&path));
    assert!(
        passed,
        "the repository-managed native formatter must bypass the poisoned shim: {report}"
    );
}

#[test]
fn emitted_node_loader_is_already_in_consumer_formatter_canonical_form() {
    let (seed, config) = emitted_loader();
    assert!(seed.contains("const isMusl ="));
    let (passed, report) = format_check(&seed, &config);
    assert!(
        passed,
        "actual emitted Node loader must pass the consumer formatter: {report}"
    );
    assert_eq!(report["results"][0]["changed"], false, "{report}");
}

#[test]
fn consumer_formatter_rejects_valid_javascript_spacing_drift_in_the_emitted_loader() {
    let (seed, config) = emitted_loader();
    let (baseline_passed, baseline_report) = format_check(&seed, &config);
    assert!(
        baseline_passed,
        "the unsabotaged emitted loader must pass first: {baseline_report}"
    );
    let (passed, report) = format_check(&format!("{seed}\nconst formatProbe={{value:1}};\n"), &config);
    assert!(
        !passed,
        "the formatter must reject valid but noncanonical JavaScript: {report}"
    );
    assert_eq!(report["results"][0]["changed"], true, "{report}");
}
