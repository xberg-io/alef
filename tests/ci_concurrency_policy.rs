use serde_json::Value;

const PULL_REQUEST_CANCELLATION: &str = "${{ github.event_name == 'pull_request' }}";
const WORKFLOW_REF_GROUP: &str = "${{ github.workflow }}-${{ github.ref }}";

fn assert_cancellation_policy(workflow: &Value) {
    let concurrency = &workflow["concurrency"];
    assert_eq!(
        concurrency["group"], WORKFLOW_REF_GROUP,
        "CI concurrency must stay scoped to the workflow and ref"
    );
    assert_eq!(
        concurrency["cancel-in-progress"], PULL_REQUEST_CANCELLATION,
        "top-level CI cancellation must be limited to pull requests"
    );
}

fn parse(source: &str) -> Value {
    serde_saphyr::from_str(source).expect("CI workflow must be valid YAML")
}

fn policy_fixture() -> Value {
    parse(&format!(
        "concurrency:\n  group: {WORKFLOW_REF_GROUP}\n  cancel-in-progress: {PULL_REQUEST_CANCELLATION}\n"
    ))
}

#[test]
fn main_ci_should_finish_while_superseded_pull_requests_are_cancelled() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml");
    let source = std::fs::read_to_string(path).expect("read the actual CI workflow");
    assert_cancellation_policy(&parse(&source));
}

#[test]
fn should_accept_the_event_scoped_cancellation_policy() {
    assert_cancellation_policy(&policy_fixture());
}

#[test]
#[should_panic(expected = "top-level CI cancellation must be limited to pull requests")]
fn should_reject_always_cancelling_main_ci() {
    let mut workflow = policy_fixture();
    workflow["concurrency"]["cancel-in-progress"] = Value::Bool(true);
    assert_cancellation_policy(&workflow);
}

#[test]
#[should_panic(expected = "top-level CI cancellation must be limited to pull requests")]
fn should_reject_disabling_cancellation_for_superseded_pull_requests() {
    let mut workflow = policy_fixture();
    workflow["concurrency"]["cancel-in-progress"] = Value::Bool(false);
    assert_cancellation_policy(&workflow);
}
