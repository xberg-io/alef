use super::*;

#[test]
fn sidecar_sits_next_to_the_library_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("libdemo.so");
    assert_eq!(sidecar_path(&library), dir.path().join("libdemo.so.input-hash"));

    let missing = check_sidecar(&library, Some("abc")).unwrap_err();
    assert!(format!("{missing:#}").contains("component build --plan"), "{missing:#}");
    check_sidecar(&library, None).unwrap();

    write_sidecar(&library, "abc").unwrap();
    check_sidecar(&library, Some("abc")).unwrap();
    let error = check_sidecar(&library, Some("def")).unwrap_err();
    assert!(error.to_string().contains("different fingerprint"), "{error}");

    clear_sidecar(&library).unwrap();
    assert!(!sidecar_path(&library).exists());
    clear_sidecar(&library).unwrap();
}

#[test]
fn fingerprinter_rejects_malformed_env_names() {
    assert!(Fingerprinter::new(&[], &["OPENSSL_DIR".to_string()]).is_ok());
    assert!(Fingerprinter::new(&[], &[String::new()]).is_err());
    assert!(Fingerprinter::new(&[], &["A=B".to_string()]).is_err());
}

#[test]
fn a_plan_without_explicit_selection_drives_profiles_and_targets() {
    use crate::component::plan::{ComponentPlan, PlanAction, PlanRow};
    let config = crate::core::config::ResolvedCrateConfig {
        name: "demo-core".into(),
        components: ["fast", "slow"]
            .into_iter()
            .map(|name| crate::core::config::ComponentConfig {
                name: name.into(),
                provides: Vec::new(),
                features: vec![name.into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into(), "aarch64-apple-darwin".into()]),
                bundled_on: Vec::new(),
                source_manifest: None,
                producer_dir: None,
                extra_dependencies: Default::default(),
                producer_language: Default::default(),
            })
            .collect(),
        ..Default::default()
    };
    let plan = ComponentPlan {
        schema_version: 1,
        debug: false,
        rows: vec![PlanRow {
            crate_name: "demo-core".into(),
            component: "slow".into(),
            target: "aarch64-apple-darwin".into(),
            input_hash: Some("a".repeat(64)),
            action: PlanAction::Build,
            host_family: None,
            previous: None,
        }],
    };
    let profiles = plan_profiles(&config, &[], Some(&plan)).unwrap();
    assert_eq!(profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["slow"]);
    assert_eq!(
        plan_targets(&config, profiles[0], &[], Some(&plan)).unwrap(),
        ["aarch64-apple-darwin"]
    );
    // Explicit selection still wins, and without a plan everything configured is selected.
    assert_eq!(
        plan_profiles(&config, &["fast".to_string()], Some(&plan))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(plan_profiles(&config, &[], None).unwrap().len(), 2);
    assert_eq!(plan_targets(&config, profiles[0], &[], None).unwrap().len(), 2);
}

#[test]
fn fingerprinter_feeds_the_probed_toolchain_for_the_target_host_only() {
    let probe = |program: &str, args: &[&str]| match (program, args.last().copied()) {
        ("xcrun", Some("--show-sdk-version")) => Some("15.0".to_string()),
        ("xcrun", Some(_)) => Some("other".to_string()),
        _ => None,
    };
    let mut mac = Fingerprinter::new(&[], &[])
        .unwrap()
        .with_probe(HostFamily::Macos, Box::new(probe));
    let records = mac.toolchain_for("aarch64-apple-darwin");
    assert!(
        records.contains(&("apple-sdk:macosx:version".to_string(), "15.0".to_string())),
        "{records:?}"
    );
    assert!(mac.toolchain_for("x86_64-unknown-linux-gnu").is_empty());

    let mut linux = Fingerprinter::new(&[], &[])
        .unwrap()
        .with_probe(HostFamily::Other, Box::new(probe));
    assert!(linux.toolchain_for("aarch64-apple-darwin").is_empty());
}

#[test]
fn a_plan_from_another_os_family_is_rejected_for_native_targets_only() {
    let mac = "aarch64-apple-darwin";
    let error = check_plan_host(Some(HostFamily::Other), HostFamily::Macos, mac).unwrap_err();
    assert_eq!(
        error.to_string(),
        "plan for aarch64-apple-darwin was made on a other host but this build runs on macos; plan Apple and MSVC targets on a matching runner"
    );
    assert!(check_plan_host(Some(HostFamily::Other), HostFamily::Windows, "x86_64-pc-windows-msvc").is_err());
    check_plan_host(Some(HostFamily::Macos), HostFamily::Macos, mac).unwrap();
    check_plan_host(None, HostFamily::Macos, mac).unwrap();
    check_plan_host(Some(HostFamily::Other), HostFamily::Macos, "x86_64-unknown-linux-gnu").unwrap();
}

#[test]
fn lock_input_check_propagates_access_errors_and_tolerates_absence() {
    let dir = tempfile::tempdir().unwrap();
    assert!(lock_input_records(&dir.path().join("absent"), true).unwrap().is_empty());
    assert!(lock_input_records(&dir.path().join("absent"), false).is_err());
}
