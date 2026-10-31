//! The publish workflow must gate CLI builds and uploads on the *assets* a release carries,
//! not on the release object existing.
//!
//! v0.62.6 was published by a run that created the release, uploaded two homebrew bottles and
//! then died. The re-run's guard asked "does the release exist?", got `true`, and skipped both
//! `build-cli` and `upload-release-assets` — leaving `cargo binstall alef` and the direct
//! download path broken on the published version.
//!
//! Everything here is extracted from the committed workflow and executed, rather than
//! transcribed, so the test cannot drift away from what CI actually runs.

#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

const WORKFLOW: &str = ".github/workflows/publish.yaml";
const CARGO_MANIFEST: &str = "Cargo.toml";
const TARGETS_FILE: &str = ".github/cli-targets.json";
#[cfg(unix)]
const RESOLVE_STEP_ID: &str = "cli-targets";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workflow() -> serde_json::Value {
    let path = repo_root().join(WORKFLOW);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_saphyr::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn cargo_manifest() -> toml::Value {
    let path = repo_root().join(CARGO_MANIFEST);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn job(workflow: &serde_json::Value, name: &str) -> serde_json::Value {
    workflow["jobs"][name]
        .as_object()
        .unwrap_or_else(|| panic!("{WORKFLOW} has no job `{name}`"))
        .clone()
        .into()
}

fn steps(job: &serde_json::Value) -> Vec<serde_json::Value> {
    job["steps"].as_array().expect("job has steps").clone()
}

fn step_with_id(job: &serde_json::Value, id: &str) -> serde_json::Value {
    steps(job)
        .into_iter()
        .find(|step| step["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("no step with id `{id}`"))
}

#[cfg(unix)]
fn step_named(job: &serde_json::Value, name: &str) -> serde_json::Value {
    steps(job)
        .into_iter()
        .find(|step| step["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no step named `{name}`"))
}

/// Run the workflow's own resolve-matrix script in `workdir` and return its `$GITHUB_OUTPUT`
/// keys. The script text comes from the committed workflow verbatim.
#[cfg(unix)]
fn run_resolve_step(workdir: &Path) -> Vec<(String, String)> {
    let script = step_with_id(&job(&workflow(), "prepare"), RESOLVE_STEP_ID)["run"]
        .as_str()
        .expect("resolve step has a run block")
        .to_string();

    let output_dir = tempfile::tempdir().expect("tempdir");
    let github_output = output_dir.path().join("github_output");
    std::fs::write(&github_output, "").expect("seed GITHUB_OUTPUT");

    let source = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workdir)
        .output()
        .expect("resolve immutable fixture source");
    assert!(source.status.success(), "fixture source must be committed");
    let source_sha = String::from_utf8(source.stdout).expect("source SHA is UTF-8");

    let result = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(workdir)
        .env("GITHUB_OUTPUT", &github_output)
        .env("SOURCE_SHA", source_sha.trim())
        .output()
        .expect("running the extracted resolve step");
    assert!(
        result.status.success(),
        "resolve step failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    std::fs::read_to_string(&github_output)
        .expect("reading GITHUB_OUTPUT")
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line.split_once('=').expect("GITHUB_OUTPUT line is key=value");
            (key.to_string(), value.to_string())
        })
        .collect()
}

#[cfg(unix)]
fn output_value(outputs: &[(String, String)], key: &str) -> String {
    outputs
        .iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("resolve step emitted no `{key}` output"))
        .1
        .clone()
}

/// The archive filename `build-cli` uploads, taken from that job's own shell and expanded for
/// one matrix entry.
#[cfg(unix)]
fn archive_name_for(entry: &serde_json::Value) -> String {
    let archive_step = step_named(&job(&workflow(), "build-cli"), "Create release archive");
    let script = archive_step["run"].as_str().expect("archive step has a run block");
    let assignment = script
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("ARCHIVE_NAME="))
        .expect("archive step assigns ARCHIVE_NAME");
    let mut name = assignment
        .trim_start_matches("ARCHIVE_NAME=")
        .trim_matches('"')
        .to_string();
    for (key, value) in entry.as_object().expect("matrix entry is an object") {
        let placeholder = format!("${{{{ matrix.{key} }}}}");
        name = name.replace(&placeholder, value.as_str().expect("matrix values are strings"));
    }
    assert!(
        !name.contains("${{"),
        "archive name still has unexpanded expressions: {name}"
    );
    name
}

#[test]
fn build_matrix_and_asset_guard_read_the_same_target_list() {
    let workflow = workflow();
    let build_matrix = workflow["jobs"]["build-cli"]["strategy"]["matrix"]["include"]
        .as_str()
        .expect("build-cli matrix include must be an expression, not an inline list");
    assert_eq!(
        build_matrix, "${{ fromJSON(needs.prepare.outputs.cli_matrix) }}",
        "build-cli must take its matrix from prepare, or the guard and the build can disagree"
    );

    let guard_step = step_with_id(&job(&workflow, "check-github-release"), "check");
    assert_eq!(
        workflow["jobs"]["check-github-release"]["env"]["RELEASE_SPEC"].as_str(),
        Some("${{ needs.prepare.outputs.release_spec }}"),
        "inspection must receive the exact source-derived archive specification"
    );
    let script = guard_step["run"].as_str().expect("inspection command");
    assert!(script.contains("github-release-state.py inspect"));
    assert!(
        guard_step["uses"].is_null(),
        "never fall back to a prefix-only registry check"
    );
}

#[test]
fn build_and_upload_stay_gated_on_the_asset_guard() {
    let workflow = workflow();
    for name in ["build-cli", "upload-release-assets"] {
        let condition = workflow["jobs"][name]["if"].as_str().expect("job has an if condition");
        assert!(
            condition.contains("needs.check-github-release.result == 'success'"),
            "{name} no longer consults the release guard"
        );
    }
    let build = workflow["jobs"]["build-cli"]["if"].as_str().unwrap();
    assert!(build.contains("needs.check-github-release.outputs.exists != 'true'"));
    assert!(build.contains("needs.prepare.outputs.dry_run == 'true'"));
    let upload = workflow["jobs"]["upload-release-assets"]["if"].as_str().unwrap();
    assert!(upload.contains("needs.build-cli.result == 'success'"));
}

#[cfg(unix)]
#[test]
fn guard_demands_exactly_the_archives_build_cli_uploads() {
    let outputs = run_resolve_step(&repo_root());
    let matrix: serde_json::Value =
        serde_json::from_str(&output_value(&outputs, "matrix")).expect("matrix output is JSON");
    let entries = matrix.as_array().expect("matrix output is an array");
    assert!(!entries.is_empty(), "the CLI target list is empty");

    let demanded: BTreeSet<String> = output_value(&outputs, "assets")
        .split(',')
        .map(str::to_string)
        .collect();
    let uploaded: BTreeSet<String> = entries.iter().map(archive_name_for).collect();

    assert_eq!(
        demanded, uploaded,
        "the asset set the guard demands must be exactly the set build-cli uploads"
    );
    assert_eq!(demanded.len(), entries.len(), "one archive per CLI target");
}

#[cfg(unix)]
#[test]
fn resolve_step_follows_the_target_list_rather_than_a_baked_in_set() {
    let sandbox = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(sandbox.path().join(".github")).expect("create .github");
    std::fs::write(
        sandbox.path().join(TARGETS_FILE),
        r#"[
          {"label": "a", "runner": "r-a", "target": "sample-target-a", "archive_ext": "tar.gz"},
          {"label": "b", "runner": "r-b", "target": "sample-target-b", "archive_ext": "zip"}
        ]"#,
    )
    .expect("write synthetic target list");

    for arguments in [
        vec!["init", "-q"],
        vec!["add", TARGETS_FILE],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "Fixture",
        ],
    ] {
        let committed = Command::new("git")
            .args(arguments)
            .current_dir(sandbox.path())
            .output()
            .expect("commit synthetic immutable target source");
        assert!(
            committed.status.success(),
            "{}",
            String::from_utf8_lossy(&committed.stderr)
        );
    }
    std::fs::write(sandbox.path().join(TARGETS_FILE), "[]").expect("dirty controller list must not override source");

    let outputs = run_resolve_step(sandbox.path());
    assert_eq!(
        output_value(&outputs, "assets"),
        "alef-sample-target-a.tar.gz,alef-sample-target-b.zip",
        "the demanded asset set must be derived from the target list, not hard-coded"
    );
}

#[cfg(unix)]
#[test]
fn release_publishes_the_intel_macos_archive_expected_by_install_alef() {
    let targets: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo_root().join(TARGETS_FILE)).expect("read CLI targets"))
            .expect("CLI targets are valid JSON");
    let matching: Vec<&serde_json::Value> = targets
        .as_array()
        .expect("CLI targets are an array")
        .iter()
        .filter(|target| target["target"].as_str() == Some("x86_64-apple-darwin"))
        .collect();

    assert_eq!(
        matching.len(),
        1,
        concat!(
            "install-alef resolves Intel macOS to x86_64-apple-darwin, ",
            "so the release must build exactly one matching target"
        )
    );
    let target = matching[0];
    assert_eq!(target["label"].as_str(), Some("macos-x86_64"));
    assert_eq!(target["runner"].as_str(), Some("macos-26-intel"));
    assert_eq!(target["archive_ext"].as_str(), Some("tar.gz"));
    assert_eq!(
        archive_name_for(target),
        "alef-x86_64-apple-darwin.tar.gz",
        "archive naming must match install-alef's release download URL"
    );
}

#[test]
fn binstall_archive_overrides_match_published_cli_targets() {
    let targets: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo_root().join(TARGETS_FILE)).expect("read CLI targets"))
            .expect("CLI targets are valid JSON");
    let manifest = cargo_manifest();
    let overrides = manifest["package"]["metadata"]["binstall"]["overrides"]
        .as_table()
        .expect("binstall overrides are a TOML table");

    for target in targets.as_array().expect("CLI targets are an array") {
        let triple = target["target"].as_str().expect("CLI target has a target triple");
        let archive_format = target["archive_ext"]
            .as_str()
            .expect("CLI target has an archive extension");
        if archive_format == "zip" {
            assert_eq!(
                overrides[triple]["pkg-fmt"].as_str(),
                Some("zip"),
                "binstall must select ZIP for published target {triple}"
            );
        }
    }

    for triple in overrides.keys() {
        assert!(
            targets
                .as_array()
                .expect("CLI targets are an array")
                .iter()
                .any(|target| target["target"].as_str() == Some(triple)),
            "binstall override {triple} has no matching published CLI target"
        );
    }
}

#[test]
fn draft_flow_and_offline_state_controls_are_part_of_the_native_gate() {
    let workflow = workflow();
    assert!(workflow["on"]["release"].is_null());
    assert_eq!(workflow["on"]["push"]["tags"][0].as_str(), Some("v*"));
    for name in ["prepare-draft", "upload-release-assets", "promote"] {
        assert_eq!(
            workflow["jobs"][name]["permissions"]["contents"].as_str(),
            Some("write")
        );
    }
    for dependency in [
        "publish-crates",
        "upload-release-assets",
        "build-cli",
        "check-github-release",
    ] {
        assert!(
            workflow["jobs"]["promote"]["needs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == dependency)
        );
    }
    let controls = std::process::Command::new("mise")
        .args([
            "exec",
            "--",
            "python",
            "-W",
            "error::ResourceWarning",
            "scripts/tests/test_github_release_state.py",
        ])
        .current_dir(repo_root())
        .output()
        .expect("run offline release-state and actual-workflow controls");
    assert!(
        controls.status.success(),
        "{}",
        String::from_utf8_lossy(&controls.stderr)
    );
    let summary = String::from_utf8_lossy(&controls.stderr);
    assert!(
        offline_state_controls_passed(&summary),
        "complete successful 46-test census: {summary}"
    );
}

fn offline_state_controls_passed(summary: &str) -> bool {
    let census: Vec<_> = summary
        .lines()
        .filter(|line| line.starts_with("Ran ") && line.contains(" tests in "))
        .collect();
    census.len() == 1
        && census[0].starts_with("Ran 46 tests in ")
        && summary.lines().filter(|line| *line == "OK").count() == 1
}

#[test]
fn offline_state_summary_accepts_lf_and_crlf_but_rejects_failed_or_vacuous_controls() {
    let lf = "..............................................\n----------------------------------------------------------------------\nRan 46 tests in 0.071s\n\nOK\n";
    let crlf = lf.replace('\n', "\r\n");
    assert!(offline_state_controls_passed(lf));
    assert!(offline_state_controls_passed(&crlf));
    for summary in [
        "Ran 0 tests in 0.000s\n\nOK\n",
        "Ran 45 tests in 0.001s\n\nOK\n",
        "Ran 146 tests in 0.001s\n\nOK\n",
        "Ran 46 tests in 0.001s\n\nFAILED (failures=1)\n",
        "Ran 46 tests in 0.001s\n",
        "OK\n",
        "",
        "Ran 46 tests in 0.001s\n\nOK (skipped=46)\n",
        "Ran 46 tests in 0.001s\nRan 0 tests in 0.000s\n\nOK\n",
        "Ran 46 tests in 0.001s\n\nOK\nOK\n",
    ] {
        assert!(!offline_state_controls_passed(summary), "{summary:?}");
        assert!(
            !offline_state_controls_passed(&summary.replace('\n', "\r\n")),
            "{summary:?}"
        );
    }
}
