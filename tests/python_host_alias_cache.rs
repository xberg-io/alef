use std::path::Path;
use std::process::{Command, Output};

fn configure(root: &Path, target: Option<&str>) {
    let aliases = target.map_or_else(String::new, |target| {
        format!("\n[extensions.python_host_aliases.traits.Counter]\nchecked_count = '{target}'\n")
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
    success(run(root, &["generate", "--lang", "python"]));
    let before = binding(root);
    assert!(before.contains("match obj.getattr(\"checked_count\")"));
    success(run(root, &["generate", "--lang", "python"]));
    assert_eq!(binding(root), before);
    success(run(root, &["verify"]));
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
        assert!(report.contains("sample") && report.contains("stale"), "{report}");
        success(run(root, &["generate", "--lang", "python"]));
        let after = binding(root);
        assert_ne!(after, before);
        assert_eq!(after.contains("match obj.getattr(\"checked_count\")"), target.is_some());
    }
}
