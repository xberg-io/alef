//! Guards `.github/workflows/publish.yaml` against the vacuous-green release class.
//!
//! ~keep A release-asset upload whose glob matches zero files is reported by GitHub
//! Actions as a success, so a release can be published carrying no assets at all with
//! no red X anywhere -- the same silent-success shape that an empty CLI target matrix
//! had before 72c7b055a. This lives in its own module rather than in `src/publish/tests.rs`
//! because it asserts over the workflow file rather than over `publish::` code, and the
//! two have no shared fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// `with:` keys that hand a file glob to an asset-upload action.
const ASSET_GLOB_KEYS: [&str; 2] = ["path", "artifacts"];

/// Action name fragments that publish or persist release assets.
const UPLOAD_ACTIONS: [&str; 2] = ["upload-artifact", "publish-github-release"];

fn workflow_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/publish.yaml")
}

/// One `with:` entry that passes a glob to an upload action.
struct GlobUpload {
    job: String,
    step: String,
    key: String,
    glob: String,
    fails_on_empty: bool,
}

/// Collect every glob handed to an upload action, together with whether the step
/// itself already hard-fails on a zero match via `if-no-files-found: error`.
fn collect_glob_uploads(document: &serde_json::Value) -> Vec<GlobUpload> {
    let mut found = Vec::new();
    let Some(jobs) = document.get("jobs").and_then(serde_json::Value::as_object) else {
        panic!("publish.yaml has no `jobs:` mapping");
    };
    for (job_name, job) in jobs {
        let Some(steps) = job.get("steps").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for step in steps {
            let uses = step.get("uses").and_then(serde_json::Value::as_str).unwrap_or_default();
            if !UPLOAD_ACTIONS.iter().any(|action| uses.contains(action)) {
                continue;
            }
            let Some(with) = step.get("with").and_then(serde_json::Value::as_object) else {
                continue;
            };
            let fails_on_empty = with.get("if-no-files-found").and_then(serde_json::Value::as_str) == Some("error");
            let step_name = step
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(uses)
                .to_string();
            for key in ASSET_GLOB_KEYS {
                let Some(value) = with.get(key).and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if !value.contains('*') {
                    continue;
                }
                found.push(GlobUpload {
                    job: job_name.clone(),
                    step: step_name.clone(),
                    key: key.to_string(),
                    glob: value.to_string(),
                    fails_on_empty,
                });
            }
        }
    }
    found
}

/// Every `run:` script body in the workflow, keyed by job name.
fn run_scripts_by_job(document: &serde_json::Value) -> BTreeMap<String, Vec<String>> {
    let mut scripts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Some(jobs) = document.get("jobs").and_then(serde_json::Value::as_object) else {
        return scripts;
    };
    for (job_name, job) in jobs {
        let Some(steps) = job.get("steps").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for step in steps {
            if let Some(run) = step.get("run").and_then(serde_json::Value::as_str) {
                scripts.entry(job_name.clone()).or_default().push(run.to_string());
            }
        }
    }
    scripts
}

/// A glob is guarded when a `run:` step in the same job names that exact glob and
/// exits non-zero, mirroring the exit-before-you-publish shape used for the CLI matrix.
fn has_guard_script(scripts: &BTreeMap<String, Vec<String>>, upload: &GlobUpload) -> bool {
    scripts.get(&upload.job).is_some_and(|bodies| {
        bodies
            .iter()
            .any(|body| body.contains(&upload.glob) && body.contains("exit 1"))
    })
}

#[test]
fn every_release_asset_glob_hard_fails_when_it_matches_nothing() {
    let path = workflow_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let document: serde_json::Value = serde_saphyr::from_str(&raw).expect("parse publish.yaml");

    let uploads = collect_glob_uploads(&document);
    assert!(
        !uploads.is_empty(),
        "found no glob-bearing upload steps in {} -- the scan is looking at the wrong keys, \
         so this test would pass vacuously",
        path.display()
    );

    let scripts = run_scripts_by_job(&document);
    let unguarded: Vec<String> = uploads
        .iter()
        .filter(|upload| !upload.fails_on_empty && !has_guard_script(&scripts, upload))
        .map(|upload| {
            format!(
                "job `{}` step `{}` ({}: {})",
                upload.job, upload.step, upload.key, upload.glob
            )
        })
        .collect();

    assert!(
        unguarded.is_empty(),
        "release-asset glob(s) that silently succeed when they match zero files:\n  {}\n\
         Set `if-no-files-found: error` on the step, or add a `run:` step in the same job that \
         names the glob and `exit 1`s when it matches nothing.",
        unguarded.join("\n  ")
    );
}
