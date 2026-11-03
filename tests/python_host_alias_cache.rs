use std::path::Path;
use std::process::{Command, Output};

fn configure(root: &Path, target: Option<&str>) {
    let aliases = target.map_or_else(String::new, |target| {
        format!("\n[extensions.python_host_aliases.crates.sample.traits.Counter]\nchecked_count = '{target}'\n")
    });
    let config = format!(
        "[workspace]\nalef_version='{}'\nlanguages=['python']\n\
         [[crates]]\nname='sample'\ncore_import='sample'\nsources=['src/lib.rs']\n\
         version_from='Cargo.toml'\nerror_type='Error'\nerror_constructor='Error::from({{msg}})'\n\
         [[crates.trait_bridges]]\ntrait_name='Counter'\n\
         [crates.generate]\npublic_api=false\n{aliases}",
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(root.join("alef.toml"), config).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_alef"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn binding(root: &Path) -> String {
    let files: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|entry| {
            let content = std::fs::read_to_string(entry.path()).unwrap();
            content.contains("pub struct PyCounterBridge").then_some(content)
        })
        .collect();
    assert_eq!(files.len(), 1, "must inspect exactly one actual generated bridge");
    files.into_iter().next().unwrap()
}

#[test]
fn alias_changes_and_removal_invalidate_a_real_warm_generation_cache() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='sample'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        include_str!("fixtures/python_host_aliases/core.rs"),
    )
    .unwrap();
    configure(root, Some("count"));
    success(run(
        root,
        &[
            "all",
            "--clean",
            "--strict",
            "--skip-compile",
            "--skip-snippet-validation",
            "--skip-frb",
            "--jobs",
            "1",
        ],
    ));
    let before = binding(root);
    assert!(before.contains("match obj.getattr(\"checked_count\")"));
    success(run(root, &["generate", "--lang", "python"]));
    assert_eq!(binding(root), before);
    let verify_before = rust_snapshot(root);
    let verified = run(root, &["verify"]);
    let coverage = String::from_utf8_lossy(&verified.stdout);
    assert!(coverage.contains("managed surface: 13 path(s)"), "{coverage}");
    assert!(coverage.contains("0 absent"), "{coverage}");
    success(verified);
    assert_eq!(rust_snapshot(root), verify_before, "verify must be read-only");
    for target in [Some("alternate"), None] {
        configure(root, target);
        let stale = run(root, &["verify"]);
        assert!(
            !stale.status.success(),
            "warm verify accepted changed alias configuration"
        );
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&stale.stdout),
            String::from_utf8_lossy(&stale.stderr)
        );
        assert!(
            report.contains("sample") && report.contains("changed since their last successful"),
            "{report}"
        );
        success(run(root, &["generate", "--lang", "python"]));
        let after = binding(root);
        assert_ne!(after, before);
        assert_eq!(after.contains("match obj.getattr(\"checked_count\")"), target.is_some());
    }
}

fn workspace(root: &Path, alias_crate: &str) {
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers=['a','b']\nresolver='3'\n",
    )
    .unwrap();
    for (dir, name) in [("a", "sample-a"), ("b", "sample-b")] {
        std::fs::create_dir_all(root.join(dir).join("src")).unwrap();
        std::fs::write(
            root.join(dir).join("Cargo.toml"),
            format!("[package]\nname='{name}'\nversion='0.1.0'\nedition='2024'\n"),
        )
        .unwrap();
    }
    std::fs::write(
        root.join("a/src/lib.rs"),
        include_str!("fixtures/python_host_aliases/core.rs"),
    )
    .unwrap();
    std::fs::write(
        root.join("b/src/lib.rs"),
        "pub fn identity(value: u32) -> u32 { value }\n",
    )
    .unwrap();
    let config = format!(
        "[workspace]\nalef_version='{}'\nlanguages=['python']\n\
         [[crates]]\nname='sample-a'\ncore_import='sample_a'\nsources=['a/src/lib.rs']\n\
         version_from='a/Cargo.toml'\nerror_type='Error'\nerror_constructor='Error::from({{msg}})'\n\
         [crates.output]\npython='bindings/a'\n\
         [[crates.trait_bridges]]\ntrait_name='Counter'\n\
         [crates.generate]\npublic_api=false\n\
         [[crates]]\nname='sample-b'\ncore_import='sample_b'\nsources=['b/src/lib.rs']\n\
         version_from='b/Cargo.toml'\n[crates.output]\npython='bindings/b'\n\
         [crates.generate]\npublic_api=false\n\
         [extensions.python_host_aliases.crates.{alias_crate}.traits.Counter]\nchecked_count='count'\n",
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(root.join("alef.toml"), config).unwrap();
}

fn rust_snapshot(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "rs"))
        .map(|entry| {
            (
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn crate_scoped_aliases_preserve_unrelated_python_output_across_warm_generation() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    workspace(root, "sample-a");
    success(run(root, &["generate", "--lang", "python"]));
    assert!(binding(root).contains("match obj.getattr(\"checked_count\")"));
    let before = rust_snapshot(root);
    let generated: Vec<_> = before
        .keys()
        .filter(|path| path.starts_with("bindings"))
        .map(std::path::PathBuf::as_path)
        .collect();
    assert_eq!(
        generated,
        [Path::new("bindings/a/lib.rs"), Path::new("bindings/b/lib.rs")]
    );
    let unrelated = &before[Path::new("bindings/b/lib.rs")];
    let unrelated_source = String::from_utf8_lossy(unrelated);
    assert!(unrelated_source.contains("#[pymodule]"));
    assert!(unrelated_source.contains("pub fn identity("));
    assert!(unrelated_source.contains("wrap_pyfunction!(identity,"));
    assert!(!unrelated_source.contains("checked_count"));
    success(run(root, &["generate", "--lang", "python"]));
    assert_eq!(
        rust_snapshot(root),
        before,
        "warm generation must retain both configured Python outputs"
    );
    let mut config: toml::Value = toml::from_str(&std::fs::read_to_string(root.join("alef.toml")).unwrap()).unwrap();
    let target =
        &mut config["extensions"]["python_host_aliases"]["crates"]["sample-a"]["traits"]["Counter"]["checked_count"];
    assert_eq!(target.as_str(), Some("count"));
    *target = toml::Value::String("alternate".to_owned());
    std::fs::write(root.join("alef.toml"), toml::to_string(&config).unwrap()).unwrap();
    success(run(root, &["generate", "--lang", "python"]));
    let changed = rust_snapshot(root);
    assert_eq!(&changed[Path::new("bindings/b/lib.rs")], unrelated);
    assert_ne!(
        changed[Path::new("bindings/a/lib.rs")],
        before[Path::new("bindings/a/lib.rs")]
    );
    assert!(binding(root).contains("obj.getattr(\"alternate\")"));
}

#[test]
fn misspelled_alias_crates_fail_before_binding_writes() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    workspace(root, "sample-typo");
    let before = rust_snapshot(root);
    let failed = run(root, &["generate", "--lang", "python"]);
    assert!(!failed.status.success());
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(report.contains("unknown crate `sample-typo`"), "{report}");
    assert_eq!(
        rust_snapshot(root),
        before,
        "invalid scope must not emit bindings for any crate"
    );
}
