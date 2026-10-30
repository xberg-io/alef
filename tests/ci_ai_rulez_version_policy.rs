use serde_json::{Value, json};

const TOOL: &str = "github:Goldziher/ai-rulez";
const SETUP: &str = "xberg-io/actions/setup-mise@v1";
const VALIDATE: &str = "xberg-io/actions/validate-ai-rulez@v1";

fn assert_pinned_workflow(workflow: &Value) {
    let steps = workflow["jobs"]["validate"]["steps"]
        .as_array()
        .expect("validation steps");
    let installers: Vec<_> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step["uses"] == SETUP)
        .collect();
    assert_eq!(installers.len(), 1, "exactly one pinned mise installer is required");
    assert_eq!(installers[0].1["with"]["install-args"], TOOL);
    assert!(
        steps.iter().all(|step| !step["uses"].as_str().is_some_and(|action| {
            action.starts_with("xberg-io/actions/setup-ai-rulez@") || action.starts_with("Goldziher/ai-rulez@")
        })),
        "a floating installer must not replace the project-pinned CLI"
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
    assert!(
        installers[0].0 < validators[0].0,
        "install the pinned CLI before validation"
    );
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

fn pinned_workflow() -> Value {
    json!({
        "on": {
            "push": {"paths": ["mise.toml", "mise.lock"]},
            "pull_request": {"paths": ["mise.toml", "mise.lock"]}
        },
        "jobs": {"validate": {"steps": [
            {"uses": SETUP, "with": {"install-args": TOOL}},
            {"uses": VALIDATE}
        ]}}
    })
}

#[test]
fn should_use_the_project_locked_cli_for_governance_validation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(".github/workflows/ai-rulez-validation.yml")).unwrap();
    let workflow: Value = serde_saphyr::from_str(&source).expect("valid workflow YAML");
    assert_pinned_workflow(&workflow);
    let read_toml = |path: &str| -> toml::Value {
        toml::from_str(&std::fs::read_to_string(root.join(path)).unwrap()).expect("valid project TOML")
    };
    let config = read_toml("mise.toml");
    let lock = read_toml("mise.lock");
    let governance = read_toml(".ai-rulez/ai-rulez.lock");
    let expected = governance["ai_rulez_version"]
        .as_str()
        .expect("governance generator version");
    assert_eq!(config["tools"][TOOL].as_str(), Some(expected));
    let versions = lock["tools"][TOOL].as_array().expect("locked ai-rulez versions");
    assert_eq!(versions.len(), 1, "one ai-rulez CLI version must be installed");
    assert_eq!(versions[0]["version"].as_str(), Some(expected));
}

#[test]
fn should_accept_pinned_installation_before_shared_validation() {
    assert_pinned_workflow(&pinned_workflow());
}

#[test]
#[should_panic(expected = "exactly one pinned mise installer is required")]
fn should_reject_the_setup_action_that_ignores_its_version_input() {
    let mut workflow = pinned_workflow();
    workflow["jobs"]["validate"]["steps"][0] = json!({
        "uses": "xberg-io/actions/setup-ai-rulez@v1",
        "with": {"ai-rulez-version": "5.9.3"}
    });
    assert_pinned_workflow(&workflow);
}

#[test]
#[should_panic(expected = "a floating installer must not replace the project-pinned CLI")]
fn should_reject_replacing_the_pinned_cli_after_installation() {
    let mut workflow = pinned_workflow();
    workflow["jobs"]["validate"]["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "uses": "Goldziher/ai-rulez@v5", "with": {"version": "latest"}
        }));
    assert_pinned_workflow(&workflow);
}

#[test]
#[should_panic(expected = "exactly one shared governance validator is required")]
fn should_reject_removing_the_shared_validator() {
    let mut workflow = pinned_workflow();
    workflow["jobs"]["validate"]["steps"].as_array_mut().unwrap().pop();
    assert_pinned_workflow(&workflow);
}

#[test]
#[should_panic(expected = "pull_request must validate changes to mise.lock")]
fn should_reject_filters_that_miss_lockfile_updates() {
    let mut workflow = pinned_workflow();
    workflow["on"]["pull_request"]["paths"] = json!(["mise.toml"]);
    assert_pinned_workflow(&workflow);
}
