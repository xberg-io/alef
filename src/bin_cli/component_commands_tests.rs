//! Unit coverage for `alef component` command handling.

use super::*;
use crate::component::artifact::{ComponentDeliveryMode, ComponentIdentity, ComponentLockEntry};
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
        extra_dependencies: Default::default(),
        producer_language: Default::default(),
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
                extra_dependencies: Default::default(),
                producer_language: Default::default(),
            },
            ComponentConfig {
                name: "gpu".into(),
                provides: provides("engine", "demo::Gpu"),
                features: vec!["cuda".into()],
                default_features: false,
                targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                bundled_on: Vec::new(),
                extra_dependencies: Default::default(),
                producer_language: Default::default(),
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
            extra_dependencies: Default::default(),
            producer_language: Default::default(),
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
    for debug in [false, true] {
        let command = component_build_command(
            &config,
            &config.components[0],
            target,
            debug,
            Some(directory.path()),
            &manifest,
        )
        .unwrap();
        let library = built_library_path(&config, "fast", target, debug, Some(directory.path()));
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
