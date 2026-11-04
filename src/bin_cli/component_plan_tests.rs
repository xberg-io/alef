use super::*;

#[test]
fn sidecar_sits_next_to_the_library_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("libdemo.so");
    assert_eq!(sidecar_path(&library), dir.path().join("libdemo.so.input-hash"));

    let missing = check_sidecar(&library, Some("abc"), "", "1.0.0").unwrap_err();
    assert!(format!("{missing:#}").contains("component build --plan"), "{missing:#}");
    check_sidecar(&library, None, "", "1.0.0").unwrap();

    std::fs::write(&library, b"original library").unwrap();
    write_sidecar(&library, "abc", "1.0.0").unwrap();
    let original_hash = sha256_file(&library).unwrap();
    check_sidecar(&library, Some("abc"), &original_hash, "1.0.0").unwrap();
    let error = check_sidecar(&library, Some("def"), &original_hash, "1.0.0").unwrap_err();
    assert!(error.to_string().contains("different fingerprint"), "{error}");

    let stale_version = check_sidecar(&library, Some("abc"), &original_hash, "2.0.0").unwrap_err();
    assert!(
        stale_version.to_string().contains("planned release version"),
        "{stale_version}"
    );

    std::fs::write(sidecar_path(&library), format!("abc\n{original_hash}\n")).unwrap();
    let legacy = check_sidecar(&library, Some("abc"), &original_hash, "1.0.0").unwrap_err();
    assert!(legacy.to_string().contains("planned release version"), "{legacy}");
    write_sidecar(&library, "abc", "1.0.0").unwrap();

    std::fs::write(&library, b"replacement library").unwrap();
    let error = check_sidecar(&library, Some("abc"), &sha256_file(&library).unwrap(), "1.0.0").unwrap_err();
    assert!(error.to_string().contains("library bytes"), "{error}");

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
            version: Some("1.0.0".into()),
            source_hash: None,
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

#[test]
fn portable_reuse_hash_changes_when_source_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = root.join("lib.rs");
    let manifest = root.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"test-producer\"\nversion = \"0.1.0\"\n[lib]\npath = \"lib.rs\"\n",
    )
    .unwrap();
    std::fs::write(&source, "pub fn value() -> u8 { 1 }\n").unwrap();
    let lock = std::process::Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(lock.success());

    let before = fingerprint::producer_inputs(&manifest, root).unwrap();
    let mut after_host_change = before.clone();
    after_host_change
        .config_files
        .push(("cargo-home:config".into(), "other".into()));
    assert_eq!(
        portable_producer_hash(&before),
        portable_producer_hash(&after_host_change)
    );

    std::fs::write(&source, "pub fn value() -> u8 { 2 }\n").unwrap();
    let after_source_change = fingerprint::producer_inputs(&manifest, root).unwrap();
    assert_ne!(
        portable_producer_hash(&before),
        portable_producer_hash(&after_source_change)
    );

    std::fs::write(&source, "pub fn value() -> u8 { 1 }\n").unwrap();
    let config_dir = root.join(".cargo");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[build]\nrustflags = [\"-Copt-level=1\"]\n",
    )
    .unwrap();
    let after_config_change = fingerprint::producer_inputs(&manifest, root).unwrap();
    let mut config_only = before.clone();
    config_only.checkout_config_files = after_config_change.checkout_config_files;
    assert_ne!(portable_producer_hash(&before), portable_producer_hash(&config_only));
}
