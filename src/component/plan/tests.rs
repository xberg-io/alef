use std::collections::BTreeMap;

use super::*;
use crate::component::artifact::ComponentIdentity;

fn entry(component: &str, target: &str, version: &str, input_hash: Option<&str>, key_id: &str) -> ComponentLockEntry {
    ComponentLockEntry {
        identity: ComponentIdentity {
            crate_name: "demo".into(),
            component: component.into(),
            version: version.into(),
            target: target.into(),
            feature_hash: "f".repeat(64),
            contract_hash: "c".repeat(64),
        },
        provides: Vec::new(),
        mode: ComponentDeliveryMode::Download,
        url: format!("https://example.invalid/{version}/{component}.tar.gz"),
        sha256: "1".repeat(64),
        size: 5,
        manifest_sha256: "2".repeat(64),
        key_id: key_id.into(),
        input_hash: input_hash.map(str::to_string),
    }
}

fn candidate(component: &str, hash: &str) -> PlanCandidate {
    PlanCandidate {
        crate_name: "demo".into(),
        component: component.into(),
        target: "x86_64-unknown-linux-gnu".into(),
        feature_hash: "f".repeat(64),
        contract_hash: "c".repeat(64),
        input_hash: Some(hash.into()),
    }
}

const TARGET: &str = "x86_64-unknown-linux-gnu";

#[test]
fn equal_input_hash_and_identity_apart_from_version_reuses_the_previous_entry() {
    let previous = vec![entry("fast", TARGET, "1.0.0", Some("h1"), "k")];
    let row = decide(&candidate("fast", "h1"), &previous).unwrap();
    assert_eq!(row.action, PlanAction::Reuse);
    assert_eq!(row.previous.as_ref(), Some(&previous[0]));
}

#[test]
fn a_changed_hash_or_identity_or_missing_hash_builds() {
    let previous = vec![entry("fast", TARGET, "1.0.0", Some("h1"), "k")];
    assert_eq!(
        decide(&candidate("fast", "h2"), &previous).unwrap().action,
        PlanAction::Build
    );
    assert_eq!(
        decide(&candidate("slow", "h1"), &previous).unwrap().action,
        PlanAction::Build
    );

    let mut other_features = candidate("fast", "h1");
    other_features.feature_hash = "e".repeat(64);
    assert_eq!(decide(&other_features, &previous).unwrap().action, PlanAction::Build);
    let mut other_contract = candidate("fast", "h1");
    other_contract.contract_hash = "d".repeat(64);
    assert_eq!(decide(&other_contract, &previous).unwrap().action, PlanAction::Build);
    let mut other_target = candidate("fast", "h1");
    other_target.target = "aarch64-apple-darwin".into();
    assert_eq!(decide(&other_target, &previous).unwrap().action, PlanAction::Build);

    let legacy = vec![entry("fast", TARGET, "1.0.0", None, "k")];
    let row = decide(&candidate("fast", "h1"), &legacy).unwrap();
    assert_eq!(row.action, PlanAction::Build);
    assert!(row.previous.is_none());
    assert_eq!(decide(&candidate("fast", "h1"), &[]).unwrap().action, PlanAction::Build);
}

#[test]
fn plan_rows_are_sorted_and_round_trip_through_the_file() {
    let previous = vec![entry("fast", TARGET, "1.0.0", Some("h1"), "k")];
    let plan = ComponentPlan::new(
        false,
        vec![
            decide(&candidate("slow", "h9"), &previous).unwrap(),
            decide(&candidate("fast", "h1"), &previous).unwrap(),
        ],
    );
    assert_eq!(plan.rows[0].component, "fast");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
    assert_eq!(ComponentPlan::read(&path).unwrap(), plan);
    assert!(plan.build_row("demo", "slow", TARGET).unwrap().is_some());
    assert!(
        plan.build_row("demo", "fast", TARGET).unwrap().is_none(),
        "a reuse row is not a build row"
    );
    let error = plan.build_row("demo", "absent", TARGET).unwrap_err();
    assert!(error.to_string().contains("not in the plan"), "{error}");

    let json = serde_json::to_value(&plan).unwrap();
    assert_eq!(json["rows"][0]["action"], "reuse");
    assert_eq!(json["rows"][1]["action"], "build");
    assert!(json["rows"][1].get("previous").is_none());
}

#[test]
fn a_reuse_row_without_its_previous_entry_is_rejected_on_read() {
    let mut plan = ComponentPlan::new(false, vec![decide(&candidate("fast", "h1"), &[]).unwrap()]);
    plan.rows[0].action = PlanAction::Reuse;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let error = ComponentPlan::read(&path).unwrap_err();
    assert!(error.to_string().contains("previous"), "{error}");
}

fn previous_lock(entries: Vec<ComponentLockEntry>) -> ComponentLock {
    ComponentLock {
        schema_version: 2,
        public_keys: BTreeMap::from([("release".to_string(), "pub-a".to_string())]),
        artifacts: entries,
    }
}

fn reuse_plan(previous: &ComponentLock) -> ComponentPlan {
    ComponentPlan::new(
        false,
        vec![
            decide(&candidate("fast", "h1"), &previous.artifacts).unwrap(),
            decide(&candidate("slow", "h2"), &previous.artifacts).unwrap(),
        ],
    )
}

#[test]
fn reuse_carries_the_previous_entry_verbatim() {
    let previous = previous_lock(vec![entry("fast", TARGET, "1.0.0", Some("h1"), "release")]);
    let plan = reuse_plan(&previous);
    assert_eq!(plan.rows[1].action, PlanAction::Build);
    let trusted = BTreeMap::from([("release".to_string(), "pub-a".to_string())]);
    let reused = reused_entries(&plan, &previous, "demo", Some(&trusted)).unwrap();
    assert_eq!(reused, previous.artifacts);
    assert!(
        reused_entries(&plan, &previous, "other-crate", Some(&trusted))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reuse_refuses_an_entry_whose_key_is_not_trusted() {
    let previous = previous_lock(vec![entry("fast", TARGET, "1.0.0", Some("h1"), "release")]);
    let plan = reuse_plan(&previous);

    let rotated_away = BTreeMap::from([("next".to_string(), "pub-b".to_string())]);
    let error = reused_entries(&plan, &previous, "demo", Some(&rotated_away)).unwrap_err();
    assert!(error.to_string().contains("not among the trusted"), "{error}");

    let replaced_key = BTreeMap::from([("release".to_string(), "pub-b".to_string())]);
    assert!(reused_entries(&plan, &previous, "demo", Some(&replaced_key)).is_err());

    assert!(
        reused_entries(&plan, &previous, "demo", None).is_err(),
        "a signed entry in an unsigned lock"
    );
}

#[test]
fn reuse_refuses_a_plan_that_disagrees_with_the_previous_lock() {
    let previous = previous_lock(vec![entry("fast", TARGET, "1.0.0", Some("h1"), "release")]);
    let mut plan = reuse_plan(&previous);
    plan.rows[0].previous.as_mut().unwrap().url = "https://evil.invalid/x.tar.gz".into();
    let trusted = BTreeMap::from([("release".to_string(), "pub-a".to_string())]);
    let error = reused_entries(&plan, &previous, "demo", Some(&trusted)).unwrap_err();
    assert!(error.to_string().contains("no identical entry"), "{error}");
}

#[test]
fn unsigned_dev_locks_reuse_unsigned_entries() {
    let previous = ComponentLock {
        public_keys: BTreeMap::new(),
        ..previous_lock(vec![entry("fast", TARGET, "1.0.0", Some("h1"), "")])
    };
    let plan = reuse_plan(&previous);
    assert_eq!(reused_entries(&plan, &previous, "demo", None).unwrap().len(), 1);
}

#[test]
fn several_matching_previous_entries_are_an_error_naming_them() {
    let previous = vec![
        entry("fast", TARGET, "1.0.0", Some("h1"), "k"),
        entry("fast", TARGET, "1.1.0", Some("h1"), "k"),
    ];
    let error = decide(&candidate("fast", "h1"), &previous).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("2 entries"), "{message}");
    assert!(message.contains("1.0.0") && message.contains("1.1.0"), "{message}");
}

#[test]
fn a_row_without_a_fingerprint_always_builds_and_never_matches_an_unhashed_entry() {
    let mut go = candidate("fast", "unused");
    go.input_hash = None;
    let unhashed = vec![entry("fast", TARGET, "1.0.0", None, "k")];
    let row = decide(&go, &unhashed).unwrap();
    assert_eq!(row.action, PlanAction::Build);
    assert!(row.input_hash.is_none());
    assert!(serde_json::to_value(&row).unwrap().get("input_hash").is_none());

    let hashed = vec![entry("fast", TARGET, "1.0.0", Some("h1"), "k")];
    assert_eq!(decide(&go, &hashed).unwrap().action, PlanAction::Build);
}

#[test]
fn a_reuse_row_without_a_fingerprint_is_rejected_on_read() {
    let previous = vec![entry("fast", TARGET, "1.0.0", Some("h1"), "k")];
    let mut plan = ComponentPlan::new(false, vec![decide(&candidate("fast", "h1"), &previous).unwrap()]);
    plan.rows[0].input_hash = None;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let error = ComponentPlan::read(&path).unwrap_err();
    assert!(error.to_string().contains("input_hash"), "{error}");
}
