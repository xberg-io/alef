use serde_json::Value;

fn parse(source: &str) -> Value {
    serde_saphyr::from_str(source).expect("CI workflow must be valid YAML")
}

fn workflow() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml");
    parse(&std::fs::read_to_string(path).expect("read the actual CI workflow"))
}

fn truthy(value: &str) -> bool {
    !value.is_empty() && value != "false"
}

fn expression(source: &str, context: &Value) -> String {
    let source = source.trim();
    if let Some((left, right)) = source.split_once("||") {
        let value = expression(left, context);
        return if truthy(&value) {
            value
        } else {
            expression(right, context)
        };
    }
    if let Some((left, right)) = source.split_once("&&") {
        let value = expression(left, context);
        return if truthy(&value) {
            expression(right, context)
        } else {
            value
        };
    }
    if let Some((left, right)) = source.split_once("==") {
        return (expression(left, context) == expression(right, context)).to_string();
    }
    if let Some(key) = source.strip_prefix("github.") {
        return context[key].as_str().expect("known GitHub context field").to_owned();
    }
    if source.starts_with('\'') && source.ends_with('\'') {
        return source[1..source.len() - 1].to_owned();
    }
    assert!(matches!(source, "true" | "false"), "unsupported expression: {source}");
    source.to_owned()
}

fn render(source: &Value, context: &Value) -> String {
    if let Some(boolean) = source.as_bool() {
        return boolean.to_string();
    }
    let mut source = source.as_str().expect("concurrency expression");
    let mut result = String::new();
    while let Some((prefix, remainder)) = source.split_once("${{") {
        result.push_str(prefix);
        let (body, suffix) = remainder.split_once("}}").expect("closed expression");
        result.push_str(&expression(body, context));
        source = suffix;
    }
    result.push_str(source);
    result
}

fn context(event: &str, reference: &str, sha: &str) -> Value {
    serde_json::json!({"workflow": "CI", "event_name": event, "ref": reference, "sha": sha})
}

fn assert_policy(workflow: &Value) {
    let concurrency = &workflow["concurrency"];
    for event in ["push", "workflow_dispatch"] {
        let first = context(event, "refs/heads/main", "aaaa");
        let second = context(event, "refs/heads/main", "bbbb");
        assert_ne!(
            render(&concurrency["group"], &first),
            render(&concurrency["group"], &second),
            "main commits must have independent concurrency groups"
        );
        assert_eq!(render(&concurrency["cancel-in-progress"], &first), "false");
    }
    let first = context("pull_request", "refs/pull/42/merge", "aaaa");
    let second = context("pull_request", "refs/pull/42/merge", "bbbb");
    let other = context("pull_request", "refs/pull/43/merge", "cccc");
    assert_eq!(
        render(&concurrency["group"], &first),
        render(&concurrency["group"], &second)
    );
    assert_ne!(
        render(&concurrency["group"], &first),
        render(&concurrency["group"], &other)
    );
    assert_eq!(render(&concurrency["cancel-in-progress"], &first), "true");
}

#[test]
fn main_commits_should_run_independently_while_pull_request_updates_share_a_group() {
    assert_policy(&workflow());
}

#[test]
#[should_panic(expected = "main commits must have independent concurrency groups")]
fn should_reject_ref_groups_that_replace_pending_main_commits() {
    let mut workflow = workflow();
    workflow["concurrency"]["group"] = "${{ github.workflow }}-${{ github.ref }}".into();
    assert_policy(&workflow);
}

#[test]
#[should_panic]
fn should_reject_always_cancelling_main_ci() {
    let mut workflow = workflow();
    workflow["concurrency"]["cancel-in-progress"] = Value::Bool(true);
    assert_policy(&workflow);
}

#[test]
#[should_panic]
fn should_reject_disabling_cancellation_for_superseded_pull_requests() {
    let mut workflow = workflow();
    workflow["concurrency"]["cancel-in-progress"] = Value::Bool(false);
    assert_policy(&workflow);
}
