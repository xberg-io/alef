use serde_json::{Value, json};

const TOOL: &str = "github:Goldziher/ai-rulez";
const SETUP: &str = "xberg-io/actions/setup-ai-rulez@v1";
const VALIDATE: &str = "xberg-io/actions/validate-ai-rulez@v1";
const HOOK_REVISION: &str = "v5";

// ai-rulez is never pinned (xberg-io org ADR-0015): CI installs the latest release.
fn assert_latest_workflow(workflow: &Value) {
    let steps = workflow["jobs"]["validate"]["steps"]
        .as_array()
        .expect("validation steps");
    let installers: Vec<_> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step["uses"] == SETUP)
        .collect();
    assert_eq!(installers.len(), 1, "exactly one setup-ai-rulez installer is required");
    assert!(
        installers[0].1["with"]["ai-rulez-version"].is_null(),
        "setup-ai-rulez ignores ai-rulez-version; do not pass a version"
    );
    assert!(
        steps.iter().all(|step| !step["with"]["install-args"]
            .as_str()
            .is_some_and(|args| args.split_whitespace().any(|arg| arg == TOOL))),
        "ai-rulez must not be installed through mise"
    );
    let validators: Vec<_> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step["uses"] == VALIDATE)
        .collect();
    assert_eq!(
        validators.len(),
        1,
        "exactly one shared governance validator is required"
    );
    assert!(installers[0].0 < validators[0].0, "install the CLI before validation");
    for event in ["push", "pull_request"] {
        let paths = workflow["on"][event]["paths"]
            .as_array()
            .expect("governance path filters");
        for path in ["mise.toml", "mise.lock"] {
            assert!(
                paths.iter().any(|value| value == path),
                "{event} must validate changes to {path}"
            );
        }
    }
}

fn assert_unpinned_project(config: &toml::Value, lock: &toml::Value, poly: &toml::Value) {
    assert!(config["tools"].get(TOOL).is_none(), "mise.toml must not pin ai-rulez");
    assert!(
        lock.get("tools").and_then(|tools| tools.get(TOOL)).is_none(),
        "mise.lock must not lock ai-rulez"
    );
    let sources = poly["hooks"]["sources"].as_array().expect("poly hook sources");
    let revisions: Vec<_> = sources
        .iter()
        .filter(|source| source["id"].as_str() == Some("ai-rulez"))
        .map(|source| source["revision"].as_str())
        .collect();
    assert_eq!(
        revisions,
        vec![Some(HOOK_REVISION)],
        "the ai-rulez hook source must follow the v5 floating tag"
    );
}

fn latest_workflow() -> Value {
    json!({
        "on": {
            "push": {"paths": ["mise.toml", "mise.lock"]},
            "pull_request": {"paths": ["mise.toml", "mise.lock"]}
        },
        "jobs": {"validate": {"steps": [
            {"uses": SETUP},
            {"uses": VALIDATE}
        ]}}
    })
}

fn hook_source(revision: &str) -> String {
    let git = "https://github.com/Goldziher/ai-rulez.git";
    format!("[[hooks.sources]]\nid = \"ai-rulez\"\ngit = \"{git}\"\nrevision = \"{revision}\"\n")
}

fn unpinned_project() -> (toml::Value, toml::Value, toml::Value) {
    let parse = |source: &str| -> toml::Value { toml::from_str(source).expect("valid fixture TOML") };
    (
        parse("[tools]\nnode = \"26.12.1\"\n"),
        parse("[[tools.node]]\nversion = \"26.12.1\"\n"),
        parse(&hook_source(HOOK_REVISION)),
    )
}

#[test]
fn should_validate_governance_with_the_latest_unpinned_cli() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(".github/workflows/ai-rulez-validation.yml")).unwrap();
    let workflow: Value = serde_saphyr::from_str(&source).expect("valid workflow YAML");
    assert_latest_workflow(&workflow);
    let read_toml = |path: &str| -> toml::Value {
        toml::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).expect("valid project TOML")
    };
    assert_unpinned_project(
        &read_toml("mise.toml"),
        &read_toml("mise.lock"),
        &read_toml("poly.toml"),
    );
}

#[test]
fn should_accept_latest_installation_before_shared_validation() {
    assert_latest_workflow(&latest_workflow());
    let (config, lock, poly) = unpinned_project();
    assert_unpinned_project(&config, &lock, &poly);
}

#[test]
#[should_panic(expected = "setup-ai-rulez ignores ai-rulez-version; do not pass a version")]
fn should_reject_a_literal_ai_rulez_version() {
    let mut workflow = latest_workflow();
    workflow["jobs"]["validate"]["steps"][0] = json!({
        "uses": SETUP,
        "with": {"ai-rulez-version": "5.9.3"}
    });
    assert_latest_workflow(&workflow);
}

#[test]
#[should_panic(expected = "ai-rulez must not be installed through mise")]
fn should_reject_installing_ai_rulez_through_mise() {
    let mut workflow = latest_workflow();
    workflow["jobs"]["validate"]["steps"].as_array_mut().unwrap().insert(
        0,
        json!({"uses": "xberg-io/actions/setup-mise@v1", "with": {"install-args": TOOL}}),
    );
    assert_latest_workflow(&workflow);
}

#[test]
#[should_panic(expected = "mise.toml must not pin ai-rulez")]
fn should_reject_a_mise_pin() {
    let (_, lock, poly) = unpinned_project();
    let config: toml::Value = toml::from_str("[tools]\n\"github:Goldziher/ai-rulez\" = \"5.9.3\"\n").unwrap();
    assert_unpinned_project(&config, &lock, &poly);
}

#[test]
#[should_panic(expected = "the ai-rulez hook source must follow the v5 floating tag")]
fn should_reject_an_exact_hook_revision() {
    let (config, lock, _) = unpinned_project();
    let poly: toml::Value = toml::from_str(&hook_source("v5.9.3")).unwrap();
    assert_unpinned_project(&config, &lock, &poly);
}

#[test]
#[should_panic(expected = "exactly one shared governance validator is required")]
fn should_reject_removing_the_shared_validator() {
    let mut workflow = latest_workflow();
    workflow["jobs"]["validate"]["steps"].as_array_mut().unwrap().pop();
    assert_latest_workflow(&workflow);
}

#[test]
#[should_panic(expected = "pull_request must validate changes to mise.lock")]
fn should_reject_filters_that_miss_lockfile_updates() {
    let mut workflow = latest_workflow();
    workflow["on"]["pull_request"]["paths"] = json!(["mise.toml"]);
    assert_latest_workflow(&workflow);
}
