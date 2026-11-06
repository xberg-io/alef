//! Unit coverage for `alef component` command handling.

use super::build::{
    built_library_path, component_build_command, component_target_dir, ensure_native_go_target, go_target,
    producer_manifest_path,
};
use super::*;
use crate::component::artifact::{ComponentDeliveryMode, ComponentIdentity, ComponentLockEntry};
use crate::component::plan::ComponentPlan;
use crate::core::config::{ComponentConfig, ComponentProvidesConfig};

fn provides(contract: &str, implementation: &str) -> Vec<ComponentProvidesConfig> {
    vec![ComponentProvidesConfig {
        contract: contract.into(),
        implementation: implementation.into(),
    }]
}

#[test]
fn target_filter_must_be_declared_by_profile() {
    let config = ResolvedCrateConfig::default();
    let profile = ComponentConfig {
        name: "fast".into(),
        provides: provides("engine", "demo::Fast"),
        features: vec!["fast".into()],
        default_features: false,
        targets: Some(vec!["aarch64-apple-darwin".into()]),
        bundled_on: Vec::new(),
        source_manifest: None,
        producer_dir: None,
        extra_dependencies: Default::default(),
        producer_language: Default::default(),
        executable: None,
    };
    let requested = vec!["x86_64-unknown-linux-gnu".to_string()];
    assert!(select_targets(&config, &profile, &requested).is_err());
}

#[test]
fn component_extraction_uses_feature_union_without_changing_binding_features() {
    let config = ResolvedCrateConfig {
        features: vec!["base".into()],
        components: vec![
            ComponentConfig {
                name: "fast".into(),
                provides: provides("engine", "demo::Fast"),
                features: vec!["simd".into(), "base".into()],
                default_features: false,
                targets: Some(vec!["aarch64-apple-darwin".into()]),
                bundled_on: Vec::new(),
                source_manifest: None,
                producer_dir: None,
                extra_dependencies: Default::default(),
                producer_language: Default::default(),
                executable: None,
            },
            ComponentConfig {
                name: "gpu".into(),
                provides: provides("engine", "demo::Gpu"),
                features: vec!["cuda".into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                bundled_on: Vec::new(),
                source_manifest: None,
                producer_dir: None,
                extra_dependencies: Default::default(),
                producer_language: Default::default(),
                executable: None,
            },
        ],
        ..ResolvedCrateConfig::default()
    };

    let extraction = component_extraction_config(&config);
    assert_eq!(extraction.features, vec!["base", "cuda", "simd"]);
    assert_eq!(config.features, vec!["base"]);
}

#[test]
fn default_python_lock_path_does_not_select_workspace_manifest() {
    let config = ResolvedCrateConfig {
        name: "demo-core".into(),
        output_paths: HashMap::from([("python".into(), PathBuf::from("packages/python/demo_core"))]),
        ..ResolvedCrateConfig::default()
    };
    assert_eq!(
        python_component_lock_path(&config),
        PathBuf::from("crates/demo-core-py/components.lock.json")
    );
}

#[test]
fn component_lock_paths_cover_each_native_binding_once() {
    let config = ResolvedCrateConfig {
        name: "demo-core".into(),
        languages: vec![
            Language::Python,
            Language::Node,
            Language::Ruby,
            Language::Php,
            Language::Elixir,
            Language::R,
            Language::Ffi,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Kotlin,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
            Language::Jni,
            Language::Wasm,
            Language::KotlinAndroid,
        ],
        ..ResolvedCrateConfig::default()
    };

    assert_eq!(
        binding_component_lock_paths(&config),
        BTreeSet::from([
            PathBuf::from("crates/demo-core-ffi/components.lock.json"),
            PathBuf::from("crates/demo-core-node/components.lock.json"),
            PathBuf::from("crates/demo-core-php/components.lock.json"),
            PathBuf::from("crates/demo-core-py/components.lock.json"),
            PathBuf::from("packages/elixir/components.lock.json"),
            PathBuf::from("packages/dart/rust/components.lock.json"),
            PathBuf::from("packages/r/components.lock.json"),
            PathBuf::from("packages/ruby/components.lock.json"),
            PathBuf::from("packages/swift/rust/components.lock.json"),
        ])
    );
}

#[test]
fn staged_binding_lock_is_scoped_to_one_crate_and_its_keys() {
    let entry = |crate_name: &str, component: &str, key_id: &str| ComponentLockEntry {
        identity: ComponentIdentity {
            crate_name: crate_name.into(),
            component: component.into(),
            version: "1.0.0".into(),
            target: "aarch64-apple-darwin".into(),
            feature_hash: "01".repeat(32),
            contract_hash: "02".repeat(32),
        },
        provides: Vec::new(),
        mode: ComponentDeliveryMode::Download,
        url: "https://example.invalid/component.tar.gz".into(),
        sha256: "03".repeat(32),
        size: 10,
        manifest_sha256: "04".repeat(32),
        key_id: key_id.into(),
        input_hash: Some("ab".repeat(32)),
    };
    let lock = ComponentLock {
        schema_version: 1,
        public_keys: BTreeMap::from([
            ("alpha-key".into(), "alpha-public-key".into()),
            ("beta-key".into(), "beta-public-key".into()),
        ]),
        artifacts: vec![
            entry("alpha-core", "fast", "alpha-key"),
            entry("beta-core", "fast", "beta-key"),
        ],
    };

    let scoped = binding_lock_for_crate(&lock, "alpha-core");
    assert_eq!(scoped.artifacts.len(), 1);
    assert_eq!(lock.artifacts[0].input_hash.as_deref(), Some("ab".repeat(32).as_str()));
    assert!(
        scoped.artifacts[0].input_hash.is_none(),
        "staged locks carry no input_hash"
    );
    let staged = String::from_utf8(canonical_json(&scoped).unwrap()).unwrap();
    assert!(!staged.contains("input_hash"), "{staged}");
    assert_eq!(scoped.artifacts[0].identity.crate_name, "alpha-core");
    assert_eq!(
        scoped.public_keys,
        BTreeMap::from([("alpha-key".into(), "alpha-public-key".into())])
    );
}

fn one_component_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "demo-core".into(),
        languages: vec![crate::core::config::Language::Ffi],
        components: vec![ComponentConfig {
            name: "fast".into(),
            provides: provides("engine", "demo::Fast"),
            features: vec!["fast".into()],
            default_features: false,
            targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
            bundled_on: Vec::new(),
            source_manifest: None,
            producer_dir: None,
            extra_dependencies: Default::default(),
            producer_language: Default::default(),
            executable: None,
        }],
        ..ResolvedCrateConfig::default()
    }
}

#[test]
fn bootstrap_writes_a_schema_valid_empty_lock_when_none_exists() {
    let base_dir = tempfile::tempdir().unwrap();
    let config = one_component_config();

    bootstrap_missing_component_locks(&config, base_dir.path()).unwrap();

    let path = base_dir.path().join(ffi_component_lock_path(&config));
    let written = fs::read_to_string(&path).unwrap();
    assert_eq!(written, "{\"schema_version\":2,\"public_keys\":{},\"artifacts\":[]}\n");
    // Must parse back into the exact type `ComponentManager::from_lock` consumes, not just
    // be well-formed JSON.
    let parsed: ComponentLock = serde_json::from_str(&written).unwrap();
    assert!(parsed.artifacts.is_empty());
}

#[test]
fn bootstrap_never_overwrites_an_existing_lock() {
    let base_dir = tempfile::tempdir().unwrap();
    let config = one_component_config();
    let path = base_dir.path().join(ffi_component_lock_path(&config));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not a lock, but must survive untouched").unwrap();

    bootstrap_missing_component_locks(&config, base_dir.path()).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "not a lock, but must survive untouched"
    );
}

#[test]
fn bootstrap_is_a_noop_for_a_crate_with_no_configured_components() {
    let base_dir = tempfile::tempdir().unwrap();
    let config = ResolvedCrateConfig {
        name: "demo-core".into(),
        languages: vec![crate::core::config::Language::Ffi],
        ..ResolvedCrateConfig::default()
    };

    bootstrap_missing_component_locks(&config, base_dir.path()).unwrap();

    assert!(fs::read_dir(base_dir.path()).unwrap().next().is_none());
}

#[test]
fn signing_key_source_maps_flags_and_dash_to_stdin() {
    assert_eq!(signing_key_source(None, None), None);
    assert_eq!(
        signing_key_source(Some(PathBuf::from("key.pem")), None),
        Some(SigningKeySource::File(PathBuf::from("key.pem")))
    );
    assert_eq!(
        signing_key_source(Some(PathBuf::from("-")), None),
        Some(SigningKeySource::Stdin)
    );
    assert_eq!(
        signing_key_source(None, Some("RELEASE_KEY".into())),
        Some(SigningKeySource::Env("RELEASE_KEY".into()))
    );
}

#[test]
fn shared_target_dir_replaces_the_per_component_directory() {
    let config = ResolvedCrateConfig {
        name: "demo-core".into(),
        ..ResolvedCrateConfig::default()
    };
    assert_eq!(
        component_target_dir(&config, "fast", None),
        PathBuf::from(".alef/components/build/demo-core/fast")
    );
    let shared = Path::new("target/components");
    assert_eq!(component_target_dir(&config, "fast", Some(shared)), shared);
    assert_eq!(component_target_dir(&config, "slow", Some(shared)), shared);
    let library = built_library_path(&config, "fast", "x86_64-unknown-linux-gnu", false, Some(shared));
    assert!(
        library.starts_with("target/components/x86_64-unknown-linux-gnu/release"),
        "{library:?}"
    );
}

#[test]
fn components_only_crate_has_no_binding_lock_paths() {
    let config = ResolvedCrateConfig {
        name: "demo-core".into(),
        languages: Vec::new(),
        ..ResolvedCrateConfig::default()
    };
    assert!(binding_component_lock_paths(&config).is_empty());
}

#[test]
fn bootstrap_writes_nothing_for_a_components_only_crate() {
    let base_dir = tempfile::tempdir().unwrap();
    let mut config = one_component_config();
    config.languages = Vec::new();

    bootstrap_missing_component_locks(&config, base_dir.path()).unwrap();

    assert!(fs::read_dir(base_dir.path()).unwrap().next().is_none());
}

#[test]
fn rust_component_command_preserves_cargo_arguments() {
    let config = one_component_config();
    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("Cargo.toml");
    fs::write(&manifest, "[package]\n").unwrap();
    for debug in [false, true] {
        let command = component_build_command(
            &config,
            &config.components[0],
            "x86_64-unknown-linux-gnu",
            debug,
            Some(Path::new("shared")),
            &manifest,
        )
        .unwrap();
        assert_eq!(command.get_program(), "cargo");
        let mut expected = vec![
            "build".into(),
            "--manifest-path".into(),
            manifest.clone().into_os_string(),
            "--target".into(),
            "x86_64-unknown-linux-gnu".into(),
            "--target-dir".into(),
            "shared".into(),
        ];
        if !debug {
            expected.push("--release".into());
        }
        assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
        assert_eq!(command.get_current_dir(), None);
        assert_eq!(command.get_envs().count(), 0);
    }
}

#[test]
fn go_component_command_uses_c_shared_and_the_packaged_output_path() {
    let mut config = one_component_config();
    config.components[0].producer_language = crate::core::config::ComponentProducerLanguage::Go;
    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("go.mod");
    fs::write(&manifest, "module producer\n").unwrap();
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        platform => panic!("unsupported test host: {platform:?}"),
    };
    for (debug, shared) in [
        (false, None),
        (true, None),
        (false, Some(directory.path())),
        (true, Some(directory.path())),
    ] {
        let command =
            component_build_command(&config, &config.components[0], target, debug, shared, &manifest).unwrap();
        let library = std::path::absolute(built_library_path(&config, "fast", target, debug, shared)).unwrap();
        assert_eq!(command.get_program(), "go");
        assert_eq!(command.get_current_dir(), Some(directory.path()));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![
                std::ffi::OsStr::new("build"),
                std::ffi::OsStr::new("-buildmode=c-shared"),
                std::ffi::OsStr::new("-o"),
                library.as_os_str(),
                std::ffi::OsStr::new("."),
            ]
        );
        let environment = command.get_envs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            environment[std::ffi::OsStr::new("CGO_ENABLED")],
            Some(std::ffi::OsStr::new("1"))
        );
        let (os, arch) = go_target(target).unwrap();
        assert_eq!(
            environment[std::ffi::OsStr::new("GOOS")],
            Some(std::ffi::OsStr::new(os))
        );
        assert_eq!(
            environment[std::ffi::OsStr::new("GOARCH")],
            Some(std::ffi::OsStr::new(arch))
        );
    }
}

#[test]
fn go_component_targets_map_only_supported_native_abis() {
    for (target, expected) in [
        ("x86_64-unknown-linux-gnu", ("linux", "amd64")),
        ("aarch64-unknown-linux-gnu", ("linux", "arm64")),
        ("x86_64-apple-darwin", ("darwin", "amd64")),
        ("aarch64-apple-darwin", ("darwin", "arm64")),
        ("x86_64-pc-windows-gnu", ("windows", "amd64")),
        ("x86_64-pc-windows-msvc", ("windows", "amd64")),
        ("aarch64-pc-windows-msvc", ("windows", "arm64")),
    ] {
        assert_eq!(go_target(target).unwrap(), expected);
    }
    for target in ["wasm32-unknown-unknown", "x86_64-unknown-linux-musl"] {
        assert!(
            go_target(target)
                .unwrap_err()
                .to_string()
                .contains("unsupported Go component target")
        );
    }
    assert!(
        ensure_native_go_target("linux", "amd64", "macos", "x86_64")
            .unwrap_err()
            .to_string()
            .contains("cross-compilation")
    );
    assert!(ensure_native_go_target("linux", "arm64", "linux", "x86_64").is_err());
    ensure_native_go_target("darwin", "arm64", "macos", "aarch64").unwrap();
}

#[test]
fn go_component_build_requires_go_module_before_dry_run() {
    let mut config = one_component_config();
    config.components[0].producer_language = crate::core::config::ComponentProducerLanguage::Go;
    let directory = tempfile::tempdir().unwrap();
    let manifest = directory.path().join("go.mod");
    let error = component_build_command(
        &config,
        &config.components[0],
        "x86_64-unknown-linux-gnu",
        false,
        None,
        &manifest,
    )
    .unwrap_err();
    assert!(error.to_string().contains("go.mod"));
    assert!(error.to_string().contains("alef scaffold"));
}

#[test]
fn producer_manifest_path_defaults_and_follows_producer_dir() {
    let mut config = one_component_config();
    config.name = "demo".into();
    config.sources = vec![std::path::PathBuf::from("crates/demo/src/lib.rs")];
    let profile = config.components[0].clone();
    assert_eq!(
        producer_manifest_path(&config, &profile),
        Path::new("crates/demo-fast-component/Cargo.toml")
    );

    let mut relocated = profile;
    relocated.producer_dir = Some("components/crates/demo-fast-component".into());
    assert_eq!(
        producer_manifest_path(&config, &relocated),
        Path::new("components/crates/demo-fast-component/Cargo.toml")
    );
}

#[test]
fn rust_component_build_uses_the_relocated_producer_manifest() {
    let mut config = one_component_config();
    let directory = tempfile::tempdir().unwrap();
    let producer_dir = directory.path().join("components/crates/demo-fast-component");
    fs::create_dir_all(&producer_dir).unwrap();
    fs::write(producer_dir.join("Cargo.toml"), "[package]\n").unwrap();
    config.components[0].producer_dir = Some(producer_dir.to_string_lossy().into_owned());

    let manifest = producer_manifest_path(&config, &config.components[0]);
    let command = component_build_command(
        &config,
        &config.components[0],
        "x86_64-unknown-linux-gnu",
        false,
        Some(Path::new("shared")),
        &manifest,
    )
    .unwrap();
    let args = command.get_args().collect::<Vec<_>>();
    assert_eq!(args[1], "--manifest-path");
    assert_eq!(args[2], producer_dir.join("Cargo.toml").as_os_str());
}

fn record_for(component: &str) -> (PathBuf, ComponentArtifactRecord) {
    use crate::component::artifact::{ComponentLibrary, ComponentManifest, ComponentProvidedContract};
    let record = ComponentArtifactRecord {
        manifest: ComponentManifest {
            schema_version: 1,
            abi_version: 1,
            identity: ComponentIdentity {
                crate_name: "demo-core".into(),
                component: component.into(),
                version: "1.0.0".into(),
                target: "x86_64-unknown-linux-gnu".into(),
                feature_hash: "a".repeat(64),
                contract_hash: "b".repeat(64),
            },
            provides: vec![ComponentProvidedContract {
                contract: "engine".into(),
                interface_version: 1,
                contract_hash: "b".repeat(64),
                implementation: "demo::Fast".into(),
            }],
            features: vec![],
            default_features: false,
            library: ComponentLibrary {
                file: "libdemo.so".into(),
                sha256: "c".repeat(64),
                size: 1,
            },
        },
        manifest_sha256: "d".repeat(64),
        archive: format!("{component}.tar.gz"),
        archive_sha256: "e".repeat(64),
        archive_size: 2,
        signature: None,
        input_hash: None,
    };
    (PathBuf::from(format!("{component}.record.json")), record)
}

#[test]
fn lock_matrix_check_covers_only_the_requested_components() {
    let mut config = one_component_config();
    let mut slow = config.components[0].clone();
    slow.name = "slow".into();
    slow.features = vec!["slow".into()];
    config.components.push(slow);
    let fast = record_for("fast");
    let records = vec![&fast];

    let error = ensure_configured_matrix_present(&config, &records, &[], &[], &[]).unwrap_err();
    assert!(error.to_string().contains("missing `slow`"), "{error}");
    ensure_configured_matrix_present(&config, &records, &[], &["fast".to_string()], &[]).unwrap();
    let error = ensure_configured_matrix_present(&config, &records, &[], &["slow".to_string()], &[]).unwrap_err();
    assert!(error.to_string().contains("missing `slow`"), "{error}");
    let reused = [("slow".to_string(), "x86_64-unknown-linux-gnu".to_string())];
    ensure_configured_matrix_present(&config, &records, &[], &[], &reused).unwrap();
}

#[test]
fn lock_component_filter_selects_named_profiles() {
    let config = one_component_config();
    let profile = &config.components[0];
    assert!(profile_selected(&[], profile));
    assert!(profile_selected(&["fast".to_string()], profile));
    assert!(!profile_selected(&["slow".to_string()], profile));
    assert!(select_profiles(&config, &["slow".to_string()]).is_err());
}

fn reuse_plan(rows: &[(&str, &str)]) -> ComponentPlan {
    use crate::component::plan::{PlanAction, PlanRow};
    let previous = record_lock_entry();
    ComponentPlan::new(
        false,
        rows.iter()
            .map(|(component, target)| PlanRow {
                crate_name: "demo-core".into(),
                component: (*component).into(),
                target: (*target).into(),
                input_hash: Some("h".into()),
                action: PlanAction::Reuse,
                version: Some("1.0.0".into()),
                source_hash: None,
                host_family: None,
                previous: Some(previous.clone()),
            })
            .collect(),
    )
}

fn record_lock_entry() -> ComponentLockEntry {
    ComponentLockEntry {
        identity: ComponentIdentity {
            crate_name: "demo-core".into(),
            component: "fast".into(),
            version: "1.0.0".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            feature_hash: "a".repeat(64),
            contract_hash: "b".repeat(64),
        },
        provides: Vec::new(),
        mode: ComponentDeliveryMode::Download,
        url: "https://example.invalid/a.tar.gz".into(),
        sha256: "c".repeat(64),
        size: 1,
        manifest_sha256: "d".repeat(64),
        key_id: String::new(),
        input_hash: Some("h".into()),
    }
}

#[test]
fn reuse_rows_must_be_configured_and_selected() {
    let config = one_component_config();
    let linux = "x86_64-unknown-linux-gnu";
    plan::check_reuse_rows(&reuse_plan(&[("fast", linux)]), &config, &[], &[]).unwrap();
    plan::check_reuse_rows(
        &reuse_plan(&[("fast", linux)]),
        &config,
        &["fast".into()],
        &[linux.into()],
    )
    .unwrap();

    let unknown_component = plan::check_reuse_rows(&reuse_plan(&[("ghost", linux)]), &config, &[], &[]).unwrap_err();
    assert!(
        unknown_component.to_string().contains("no such component"),
        "{unknown_component}"
    );
    let unknown_target =
        plan::check_reuse_rows(&reuse_plan(&[("fast", "aarch64-apple-darwin")]), &config, &[], &[]).unwrap_err();
    assert!(
        unknown_target.to_string().contains("not configured"),
        "{unknown_target}"
    );
    let unselected_component =
        plan::check_reuse_rows(&reuse_plan(&[("fast", linux)]), &config, &["slow".into()], &[]).unwrap_err();
    assert!(
        unselected_component.to_string().contains("do not select"),
        "{unselected_component}"
    );
    let unselected_target = plan::check_reuse_rows(
        &reuse_plan(&[("fast", linux)]),
        &config,
        &[],
        &["aarch64-apple-darwin".into()],
    )
    .unwrap_err();
    assert!(
        unselected_target.to_string().contains("do not select"),
        "{unselected_target}"
    );

    let mut other_crate = reuse_plan(&[("ghost", linux)]);
    other_crate.rows[0].crate_name = "other".into();
    plan::check_reuse_rows(&other_crate, &config, &[], &[]).unwrap();
}

#[test]
fn fresh_records_need_a_build_row_in_the_plan() {
    let fast = record_for("fast");
    let records = vec![&fast];
    let empty = ComponentPlan::new(false, Vec::new());
    let error = plan::check_records_against_plan(&empty, "demo-core", &records).unwrap_err();
    assert!(error.to_string().contains("not in the plan"), "{error}");

    let reused = reuse_plan(&[("fast", "x86_64-unknown-linux-gnu")]);
    let error = plan::check_records_against_plan(&reused, "demo-core", &records).unwrap_err();
    assert!(error.to_string().contains("plan reuses"), "{error}");

    let mut build = reuse_plan(&[("fast", "x86_64-unknown-linux-gnu")]);
    build.rows[0].action = crate::component::plan::PlanAction::Build;
    build.rows[0].previous = None;
    let error = plan::check_records_against_plan(&build, "demo-core", &records).unwrap_err();
    assert!(error.to_string().contains("different inputs"), "{error}");
    let mut stamped = record_for("fast");
    stamped.1.input_hash = Some("h".into());
    plan::check_records_against_plan(&build, "demo-core", &[&stamped]).unwrap();
    let mut stale = stamped.clone();
    stale.1.manifest.identity.version = "0.9.0".into();
    let error = plan::check_records_against_plan(&build, "demo-core", &[&stale]).unwrap_err();
    assert!(error.to_string().contains("planned version"), "{error}");

    build.rows[0].input_hash = None;
    stamped.1.input_hash = None;
    let error = plan::check_records_against_plan(&build, "demo-core", &[&stamped]).unwrap_err();
    assert!(error.to_string().contains("requires an input fingerprint"), "{error}");
}

#[test]
fn reuse_cannot_be_combined_with_a_url_template() {
    plan::check_reuse_options(None, true).unwrap();
    plan::check_reuse_options(Some("https://x/{artifact}"), false).unwrap();
    let error = plan::check_reuse_options(Some("https://x/{artifact}"), true).unwrap_err();
    assert!(error.to_string().contains("--url-template"), "{error}");
}

fn previous_lock() -> ComponentLock {
    ComponentLock {
        schema_version: 1,
        public_keys: BTreeMap::new(),
        artifacts: vec![record_lock_entry()],
    }
}

#[test]
fn all_reuse_lock_accepts_an_absent_or_empty_input_directory() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("dist/components");
    assert!(plan::lock_input_records(&absent, true).unwrap().is_empty());
    let error = plan::lock_input_records(&absent, false).unwrap_err();
    assert!(error.to_string().contains("does not exist"), "{error}");
    fs::create_dir_all(&absent).unwrap();
    assert!(plan::lock_input_records(&absent, true).unwrap().is_empty());

    let config = one_component_config();
    let reuse = (previous_lock(), reuse_plan(&[("fast", "x86_64-unknown-linux-gnu")]));
    let reused = plan::reuse_for_crate(Some(&reuse), &config, &[], None, &[], &[]).unwrap();
    assert_eq!(reused, vec![record_lock_entry()]);
}

#[test]
fn lock_fails_when_a_build_row_has_no_record() {
    let config = one_component_config();
    let mut build = reuse_plan(&[("fast", "x86_64-unknown-linux-gnu")]);
    build.rows[0].action = crate::component::plan::PlanAction::Build;
    build.rows[0].previous = None;
    let reuse = (previous_lock(), build);
    let error = plan::reuse_for_crate(Some(&reuse), &config, &[], None, &[], &[]).unwrap_err();
    assert!(error.to_string().contains("no record for it"), "{error}");

    let mut stamped = record_for("fast");
    stamped.1.input_hash = Some("h".into());
    plan::reuse_for_crate(Some(&reuse), &config, &[&stamped], None, &[], &[]).unwrap();
    plan::reuse_for_crate(Some(&reuse), &config, &[], None, &["slow".into()], &[]).unwrap();
}

#[test]
fn build_row_checks_honour_filters_and_cover_every_row() {
    use crate::component::plan::{PlanAction, PlanRow};
    let linux = "x86_64-unknown-linux-gnu";
    let mac = "aarch64-apple-darwin";
    let row = |component: &str, target: &str, action: PlanAction| PlanRow {
        crate_name: "demo-core".into(),
        component: component.into(),
        target: target.into(),
        input_hash: None,
        action,
        version: Some("1.0.0".into()),
        source_hash: None,
        host_family: None,
        previous: None,
    };
    let plan = ComponentPlan::new(
        false,
        vec![
            row("fast", linux, PlanAction::Build),
            row("fast", mac, PlanAction::Build),
            row("go", linux, PlanAction::Build),
        ],
    );
    let (go_path, mut go) = record_for("go");
    go.manifest.identity.target = linux.into();
    let go = (go_path, go);
    let fast_linux = record_for("fast");
    let check = |records: &[&(PathBuf, ComponentArtifactRecord)], components: &[String], targets: &[String]| {
        plan::check_build_rows_have_records(&plan, "demo-core", records, components, targets)
    };

    let error = check(&[&fast_linux, &go], &[], &[]).unwrap_err();
    assert!(error.to_string().contains(&format!("fast for {mac}")), "{error}");
    let error = check(&[&fast_linux], &[], &[linux.to_string()]).unwrap_err();
    assert!(error.to_string().contains("go for"), "{error}");
    check(&[&fast_linux], &["fast".to_string()], &[linux.to_string()]).unwrap();
    check(&[&go], &["go".to_string()], &[]).unwrap();
    check(&[&fast_linux, &go], &[], &[linux.to_string()]).unwrap();
    plan::check_build_rows_have_records(&plan, "other-crate", &[], &[], &[]).unwrap();
}
