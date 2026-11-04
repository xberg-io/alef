use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::*;

const RUSTC: &str = "rustc 1.99.0 (abc 2026-01-01)\nbinary: rustc\ncommit-hash: abc\ncommit-date: 2026-01-01\nhost: x86_64-unknown-linux-gnu\nrelease: 1.99.0\nLLVM version: 21.1.0";

fn input<'a>(features: &'a [String], contracts: &'a [(String, String)]) -> FingerprintInput<'a> {
    FingerprintInput {
        crate_name: "demo",
        component: "fast",
        target: "x86_64-unknown-linux-gnu",
        release: true,
        features,
        default_features: false,
        contracts,
        distribution: None,
        alef_version: "0.0.1",
        rustc: RUSTC,
        env: &[],
        extras: &[],
    }
}

fn contracts() -> Vec<(String, String)> {
    vec![("engine".into(), "aa".repeat(32))]
}

fn producer() -> ProducerInputs {
    ProducerInputs {
        packages: vec!["serde@1.0.0#registry+x#abc features=std".into()],
        manifest_tables: vec![("profile".into(), "{}".into())],
        config_files: Vec::new(),
    }
}

#[test]
fn fingerprint_is_stable_hex_and_ignores_feature_and_extra_order() {
    let contracts = contracts();
    let features = vec!["b".to_string(), "a".to_string()];
    let reordered = vec!["a".to_string(), "b".to_string()];
    let first = compute(&input(&features, &contracts), &producer());
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(first, compute(&input(&features, &contracts), &producer()));
    assert_eq!(first, compute(&input(&reordered, &contracts), &producer()));

    let extras_ab = vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())];
    let extras_ba = vec![extras_ab[1].clone(), extras_ab[0].clone()];
    let mut one = input(&features, &contracts);
    one.extras = &extras_ab;
    let mut two = input(&features, &contracts);
    two.extras = &extras_ba;
    assert_eq!(compute(&one, &producer()), compute(&two, &producer()));
}

#[test]
fn fingerprint_changes_with_every_input() {
    let contracts = contracts();
    let features = vec!["a".to_string()];
    let base = compute(&input(&features, &contracts), &producer());
    macro_rules! differs {
        ($producer:expr, |$i:ident| $body:expr) => {{
            let mut $i = input(&features, &contracts);
            $body;
            assert_ne!(compute(&$i, &$producer), base);
        }};
    }
    let env = vec![("RUSTFLAGS".to_string(), "-C strip=symbols".to_string())];
    let extra = vec![("image".to_string(), "x".to_string())];
    let other_contracts = vec![("engine".to_string(), "bb".repeat(32))];
    let other_features = vec!["z".to_string()];
    let distribution = ComponentDistributionConfig {
        url_template: "https://example.invalid/{artifact}".into(),
        ..Default::default()
    };
    let newer_rustc = RUSTC.replace("release: 1.99.0", "release: 1.99.1");
    differs!(producer(), |i| i.rustc = &newer_rustc);
    let newer_llvm = RUSTC.replace("LLVM version: 21.1.0", "LLVM version: 22.0.0");
    differs!(producer(), |i| i.rustc = &newer_llvm);
    differs!(producer(), |i| i.env = &env);
    differs!(producer(), |i| i.extras = &extra);
    differs!(producer(), |i| i.target = "aarch64-apple-darwin");
    differs!(producer(), |i| i.release = false);
    differs!(producer(), |i| i.default_features = true);
    differs!(producer(), |i| i.alef_version = "0.0.2");
    differs!(producer(), |i| i.contracts = &other_contracts);
    differs!(producer(), |i| i.features = &other_features);
    differs!(producer(), |i| i.distribution = Some(&distribution));

    let mut dependency_bump = producer();
    dependency_bump.packages = vec!["serde@1.0.1#registry+x#abc features=std".into()];
    differs!(dependency_bump, |i| i.crate_name = "demo");
    let mut changed_checksum = producer();
    changed_checksum.packages = vec!["serde@1.0.0#registry+x#def features=std".into()];
    differs!(changed_checksum, |i| i.crate_name = "demo");
    let mut profile = producer();
    profile.manifest_tables = vec![("profile".into(), "{\"release\":{\"lto\":true}}".into())];
    differs!(profile, |i| i.crate_name = "demo");
    let mut config = producer();
    config.config_files = vec![("cwd:0:rust-toolchain.toml".into(), "00".repeat(32))];
    differs!(config, |i| i.crate_name = "demo");
}

#[test]
fn fingerprint_records_are_length_prefixed() {
    let contracts = contracts();
    let a = vec![("k".to_string(), "v\nextra:z=1:1".to_string())];
    let b = vec![("k".to_string(), "v".to_string()), ("z".to_string(), "1".to_string())];
    let mut one = input(&[], &contracts);
    one.extras = &a;
    let mut two = input(&[], &contracts);
    two.extras = &b;
    assert_ne!(compute(&one, &producer()), compute(&two, &producer()));
}

#[test]
fn fingerprint_ignores_the_rustc_host_but_not_its_identity() {
    let contracts = contracts();
    let on_mac = RUSTC.replace("host: x86_64-unknown-linux-gnu", "host: aarch64-apple-darwin");
    let mut linux = input(&[], &contracts);
    linux.rustc = RUSTC;
    let mut mac = input(&[], &contracts);
    mac.rustc = &on_mac;
    assert_eq!(compute(&linux, &producer()), compute(&mac, &producer()));
    assert!(!rustc_identity(RUSTC).contains("host:"));
    assert_eq!(
        rustc_identity(RUSTC),
        "commit-hash: abc\nrelease: 1.99.0\nLLVM version: 21.1.0"
    );
    assert_eq!(
        rustc_identity("rustc 1.0.0\nhost: x"),
        "rustc 1.0.0",
        "an unknown format falls back to everything but the host"
    );
}

fn vars(names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .map(|name| ((*name).to_string(), "v".to_string()))
        .collect()
}

fn names(selected: Vec<(String, String)>) -> Vec<String> {
    selected.into_iter().map(|(name, _)| name).collect()
}

const LINUX: &str = "x86_64-unknown-linux-gnu";

#[test]
fn build_env_keeps_only_build_affecting_variables() {
    let selected = names(build_env(
        vars(&[
            "RUSTFLAGS",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_TARGET_X_RUSTFLAGS",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_BUILD_JOBS",
            "HOME",
            "PATH",
            "UNRELATED",
        ]),
        LINUX,
        &[],
    ));
    assert_eq!(
        selected,
        [
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_TARGET_X_RUSTFLAGS",
            "RUSTFLAGS"
        ]
    );
}

#[test]
fn build_env_covers_c_toolchain_sdk_and_wrapper_variables() {
    let all = [
        "CC",
        "CXX",
        "AR",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "CC_x86_64-unknown-linux-gnu",
        "CFLAGS_x86_64_unknown_linux_gnu",
        "TARGET_CXX",
        "HOST_CC",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
        "MACOSX_DEPLOYMENT_TARGET",
        "IPHONEOS_DEPLOYMENT_TARGET",
        "SDKROOT",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTC",
        "CARGO_BUILD_TARGET",
    ];
    let selected = names(build_env(vars(&all), LINUX, &[]));
    assert_eq!(selected.len(), all.len(), "{selected:?}");

    let other_target = names(build_env(
        vars(&[
            "CC_aarch64-apple-darwin",
            "CFLAGS_aarch64_apple_darwin",
            "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER",
        ]),
        LINUX,
        &[],
    ));
    assert!(
        other_target.is_empty(),
        "another target's variables are ignored: {other_target:?}"
    );
}

#[test]
fn build_env_opt_in_names_are_included_when_set() {
    let selected = names(build_env(
        vars(&["OPENSSL_DIR", "HOME"]),
        LINUX,
        &["OPENSSL_DIR".to_string(), "NOT_SET".to_string()],
    ));
    assert_eq!(selected, ["OPENSSL_DIR"]);
}

#[test]
fn extras_parse_sorted_and_reject_malformed_or_duplicate_keys() {
    let parsed = parse_extras(&["b=2".into(), "a=x=y".into()]).unwrap();
    assert_eq!(
        parsed,
        [("a".to_string(), "x=y".to_string()), ("b".to_string(), "2".to_string())]
    );
    assert!(parse_extras(&["novalue".into()]).is_err());
    assert!(parse_extras(&["=v".into()]).is_err());
    assert!(parse_extras(&["a=1".into(), "a=2".into()]).is_err());
}

#[test]
fn manifest_tables_ignore_key_order() {
    let render = |text: &str| {
        let mut out = String::new();
        render_toml(&text.parse::<toml::Table>().unwrap().into(), &mut out);
        out
    };
    assert_eq!(
        render("a = 1\nb = [1, 2]\n[c]\nx = 'y'"),
        render("b = [1, 2]\na = 1\n[c]\nx = 'y'")
    );
    assert_ne!(render("a = 1"), render("a = 2"));
}

fn metadata(root_dir: &Path, serde_version: &str, dev_version: &str, unrelated_version: &str) -> Value {
    let package = |id: &str, name: &str, version: &str, source: Value, dir: &Path| {
        json!({
            "id": id, "name": name, "version": version, "source": source,
            "manifest_path": dir.join("Cargo.toml"),
        })
    };
    let registry = json!("registry+https://github.com/rust-lang/crates.io-index");
    json!({
        "workspace_root": root_dir,
        "packages": [
            package("producer", "producer", "0.1.0", Value::Null, root_dir),
            package("serde", "serde", serde_version, registry.clone(), root_dir),
            package("tempfile", "tempfile", dev_version, registry.clone(), root_dir),
            package("unrelated", "unrelated", unrelated_version, registry, root_dir),
        ],
        "resolve": {"nodes": [
            {"id": "producer", "features": ["default"], "deps": [
                {"pkg": "serde", "dep_kinds": [{"kind": null, "target": null}]},
                {"pkg": "tempfile", "dep_kinds": [{"kind": "dev", "target": null}]},
            ]},
            {"id": "serde", "features": ["std"], "deps": []},
            {"id": "tempfile", "features": [], "deps": []},
            {"id": "unrelated", "features": [], "deps": [{"pkg": "serde", "dep_kinds": [{"kind": null}]}]},
        ]},
    })
}

#[test]
fn closure_follows_normal_dependencies_only_and_tracks_versions() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"producer\"\n").unwrap();
    let manifest = dir.path().join("Cargo.toml");
    let checksums = BTreeMap::from([(
        (
            "serde".to_string(),
            "1.0.0".to_string(),
            "registry+https://github.com/rust-lang/crates.io-index".to_string(),
        ),
        "abc".to_string(),
    )]);
    let records = |serde: &str, dev: &str, unrelated: &str| {
        closure_records(&metadata(dir.path(), serde, dev, unrelated), &manifest, &checksums).unwrap()
    };
    let base = records("1.0.0", "3.0.0", "1.0.0");
    assert_eq!(base.len(), 2, "{base:?}");
    assert!(
        base.iter()
            .any(|record| record.starts_with("serde@1.0.0#registry+") && record.contains("#abc "))
    );
    assert!(
        base.iter()
            .all(|record| !record.contains("tempfile") && !record.contains("unrelated"))
    );
    assert_eq!(
        base,
        records("1.0.0", "4.0.0", "2.0.0"),
        "dev and unreachable packages are not hashed"
    );
    assert_ne!(
        base,
        records("1.0.1", "3.0.0", "1.0.0"),
        "a dependency version bump changes the closure"
    );
}

fn cargo(dir: &Path, args: &[&str]) {
    let status = Command::new("cargo").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "cargo {args:?} failed");
}

fn write_workspace(root: &Path) {
    for (name, deps) in [
        ("producer", "lib-a = { path = \"../lib-a\" }\n"),
        ("lib-a", ""),
        ("other", ""),
    ] {
        let dir = root.join(name);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"),
        )
        .unwrap();
        fs::write(dir.join("src/lib.rs"), "pub fn value() -> u32 { 1 }\n").unwrap();
    }
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"producer\", \"lib-a\", \"other\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    cargo(root, &["generate-lockfile", "--offline"]);
}

fn host_target() -> String {
    rustc_version()
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .unwrap()
}

fn real_fingerprint(root: &Path) -> String {
    let target = host_target();
    let producer = producer_inputs(&root.join("producer/Cargo.toml"), root).unwrap();
    let contracts = contracts();
    let mut fingerprint_input = input(&[], &contracts);
    fingerprint_input.target = &target;
    compute(&fingerprint_input, &producer)
}

#[test]
fn real_workspace_fingerprint_tracks_path_sources_not_unrelated_members_or_checkout_location() {
    let first = tempfile::tempdir().unwrap();
    write_workspace(first.path());
    let base = real_fingerprint(first.path());
    assert_eq!(base, real_fingerprint(first.path()), "stable across runs");

    let second = tempfile::tempdir().unwrap();
    write_workspace(second.path());
    assert_eq!(base, real_fingerprint(second.path()), "stable across checkouts");

    fs::write(first.path().join("other/src/lib.rs"), "pub fn value() -> u32 { 2 }\n").unwrap();
    assert_eq!(
        base,
        real_fingerprint(first.path()),
        "an unrelated member does not matter"
    );

    fs::write(first.path().join("lib-a/src/lib.rs"), "pub fn value() -> u32 { 3 }\n").unwrap();
    assert_ne!(
        base,
        real_fingerprint(first.path()),
        "a path dependency's source file matters"
    );
}

#[test]
fn tree_digest_uses_git_blobs_and_sees_uncommitted_edits() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    fs::write(dir.path().join("a.rs"), "one\n").unwrap();
    git(&["add", "a.rs"]);
    let staged = tree_digest(dir.path()).unwrap();
    git(&["commit", "-q", "-m", "init"]);
    assert_eq!(
        staged,
        tree_digest(dir.path()).unwrap(),
        "committing does not change it"
    );

    fs::write(dir.path().join("untracked.rs"), "x\n").unwrap();
    let with_untracked = tree_digest(dir.path()).unwrap();
    assert_ne!(staged, with_untracked, "an untracked, unignored file is seen");
    fs::write(dir.path().join(".gitignore"), "ignored.rs\n").unwrap();
    fs::write(dir.path().join("ignored.rs"), "x\n").unwrap();
    let with_ignore_file = tree_digest(dir.path()).unwrap();
    fs::write(dir.path().join("ignored.rs"), "y\n").unwrap();
    assert_eq!(
        with_ignore_file,
        tree_digest(dir.path()).unwrap(),
        "ignored files are not"
    );
    fs::remove_file(dir.path().join("untracked.rs")).unwrap();
    fs::remove_file(dir.path().join(".gitignore")).unwrap();
    fs::remove_file(dir.path().join("ignored.rs")).unwrap();

    fs::write(dir.path().join("a.rs"), "two\n").unwrap();
    assert_ne!(staged, tree_digest(dir.path()).unwrap(), "an unstaged edit is seen");
}

#[test]
fn tree_digest_walk_skips_target_and_hashes_content() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("target")).unwrap();
    fs::write(dir.path().join("a.rs"), "one\n").unwrap();
    let base = tree_digest(dir.path()).unwrap();
    fs::write(dir.path().join("target/out.o"), "junk").unwrap();
    assert_eq!(base, tree_digest(dir.path()).unwrap());
    fs::write(dir.path().join("a.rs"), "two\n").unwrap();
    assert_ne!(base, tree_digest(dir.path()).unwrap());
}

#[test]
fn tree_digest_ignores_the_package_version_but_not_other_manifest_changes() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = |version: &str, dependency: &str| {
        format!("[package]\nname = \"p\"\nversion = \"{version}\"\n\n[dependencies]\nserde = \"{dependency}\"\n")
    };
    fs::write(dir.path().join("lib.rs"), "fn f() {}\n").unwrap();
    fs::write(dir.path().join("Cargo.toml"), manifest("1.0.0", "1")).unwrap();
    let base = tree_digest(dir.path()).unwrap();
    fs::write(dir.path().join("Cargo.toml"), manifest("1.1.0", "1")).unwrap();
    assert_eq!(
        base,
        tree_digest(dir.path()).unwrap(),
        "a version bump alone keeps the digest"
    );
    fs::write(dir.path().join("Cargo.toml"), manifest("1.1.0", "2")).unwrap();
    assert_ne!(base, tree_digest(dir.path()).unwrap(), "a dependency change alters it");
}

#[cfg(unix)]
#[test]
fn tree_digest_hashes_the_content_a_symlink_points_at_in_walk_mode() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("real.txt"), "one\n").unwrap();
    std::os::unix::fs::symlink("real.txt", dir.path().join("link.txt")).unwrap();
    let base = tree_digest(dir.path()).unwrap();
    fs::write(dir.path().join("real.txt"), "two\n").unwrap();
    let edited = tree_digest(dir.path()).unwrap();
    assert_ne!(base, edited);
    assert_eq!(edited, tree_digest(dir.path()).unwrap(), "stable for equal content");
}

#[cfg(unix)]
#[test]
fn tree_digest_hashes_the_content_a_symlink_points_at_in_git_mode() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    fs::write(dir.path().join("real.txt"), "one\n").unwrap();
    std::os::unix::fs::symlink("real.txt", dir.path().join("link.txt")).unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);
    let base = tree_digest(dir.path()).unwrap();
    fs::remove_file(dir.path().join("real.txt")).unwrap();
    fs::write(dir.path().join("real.txt"), "two\n").unwrap();
    let edited = tree_digest(dir.path()).unwrap();
    assert_ne!(base, edited);

    // Same bytes in the target, link text unchanged: the digest depends on the target content.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("t.txt"), "a\n").unwrap();
    std::os::unix::fs::symlink(outside.path().join("t.txt"), dir.path().join("abs.txt")).unwrap();
    let with_abs = tree_digest(dir.path()).unwrap();
    fs::write(outside.path().join("t.txt"), "b\n").unwrap();
    assert_ne!(
        with_abs,
        tree_digest(dir.path()).unwrap(),
        "an untracked symlink's target counts"
    );
}

#[test]
fn git_ancestor_detection_finds_a_dot_git_above_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    assert!(tree::has_git_ancestor(&nested));
}

#[test]
fn manifest_tables_cover_workspace_inheritance_but_not_versions() {
    let render = |text: &str| render_manifest_tables(&text.parse::<toml::Table>().unwrap());
    let table = |tables: &[(String, String)], name: &str| {
        tables
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .unwrap()
    };
    let base = render(
        r#"[workspace.package]
version = "1.0.0"
edition = "2021"
[workspace.lints.rust]
unsafe_code = "forbid"
[workspace.dependencies]
core-crate = { path = "crates/core", version = "1.0.0" }
serde = { version = "1.0.0", features = ["derive"] }
"#,
    );
    let bumped = render(
        r#"[workspace.package]
version = "1.0.1"
edition = "2021"
[workspace.lints.rust]
unsafe_code = "forbid"
[workspace.dependencies]
core-crate = { path = "crates/core", version = "1.0.1" }
serde = { version = "1.0.0", features = ["derive"] }
"#,
    );
    assert_eq!(base, bumped, "a lock-step version bump changes nothing");
    assert!(table(&base, "workspace.package").contains("edition"));
    assert!(!table(&base, "workspace.package").contains("version"));
    assert!(table(&base, "workspace.lints").contains("unsafe_code"));
    assert!(table(&base, "workspace.dependencies").contains("serde"));

    let edition = render("[workspace.package]\nedition = \"2024\"\n");
    assert_ne!(table(&base, "workspace.package"), table(&edition, "workspace.package"));
    let lints = render("[workspace.lints.rust]\nunsafe_code = \"warn\"\n");
    assert_ne!(table(&base, "workspace.lints"), table(&lints, "workspace.lints"));
    let registry_bump = render("[workspace.dependencies]\nserde = { version = \"1.0.1\", features = [\"derive\"] }\n");
    assert_ne!(
        table(&base, "workspace.dependencies"),
        table(&registry_bump, "workspace.dependencies"),
        "a registry dependency's version counts"
    );
}

#[test]
fn implicit_config_walks_to_the_root_and_labels_without_absolute_paths() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(root.path().join("a/.git")).unwrap();
    fs::create_dir_all(root.path().join(".cargo")).unwrap();
    fs::write(root.path().join(".cargo/config.toml"), "[build]\n").unwrap();
    fs::write(root.path().join("a/rust-toolchain.toml"), "[toolchain]\n").unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("config.toml"), "[net]\n").unwrap();

    let found = implicit_config(&[("cwd", nested.as_path())], Some(home.path())).unwrap();
    let labels = found.iter().map(|(label, _)| label.as_str()).collect::<Vec<_>>();
    for expected in [
        "cargo-home:config.toml",
        "cwd:2:rust-toolchain.toml",
        "cwd:3:.cargo/config.toml",
    ] {
        assert!(labels.contains(&expected), "{expected} missing from {labels:?}");
    }
    assert!(
        found
            .iter()
            .all(|(label, _)| !label.contains(root.path().to_str().unwrap()))
    );

    let none = implicit_config(&[("cwd", nested.as_path())], None).unwrap();
    assert_eq!(none.len() + 1, found.len());
    fs::write(home.path().join("config.toml"), "[net]\nretry = 1\n").unwrap();
    assert_ne!(
        found,
        implicit_config(&[("cwd", nested.as_path())], Some(home.path())).unwrap()
    );
}
