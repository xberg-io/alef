use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;

const SNAPSHOT_FORBIDDEN_MARKERS: &[&str] = &[
    "kreuzberg",
    "kreuzberglib",
    "literllmclient",
    "liter-llm",
    "spikard",
    "kreuzcrawl",
    "html-to-markdown",
];

#[test]
fn no_project_name_special_casing_in_enforced_files() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let hook = workspace_root.join("hooks/check_project_mentions.py");
    let files = enforced_files(&workspace_root);

    let output = Command::new("python3")
        .arg(hook)
        .args(&files)
        .output()
        .expect("project mention hook must run");

    assert!(
        output.status.success(),
        "project mention hook failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn no_downstream_project_names_in_snapshot_filenames() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let snapshot_root = workspace_root.join("tests/snapshots");
    let leaks: Vec<String> = WalkDir::new(&snapshot_root)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|entry_path| entry_path.is_file())
        .filter_map(|entry_path| {
            let display_path = entry_path
                .strip_prefix(&snapshot_root)
                .unwrap_or(&entry_path)
                .display()
                .to_string();
            let normalized = display_path.to_lowercase().replace(['_', '/'], "-");
            SNAPSHOT_FORBIDDEN_MARKERS
                .iter()
                .any(|marker| normalized.contains(marker))
                .then_some(display_path)
        })
        .collect();

    assert!(
        leaks.is_empty(),
        "snapshot filenames must use neutral fixture names:\n{}",
        leaks.join("\n")
    );
}

#[test]
fn no_downstream_project_names_in_snapshot_content() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let snapshot_root = workspace_root.join("tests/snapshots");
    let mut leaks = Vec::new();
    for entry_path in WalkDir::new(snapshot_root)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|entry_path| entry_path.is_file())
    {
        let Ok(content) = std::fs::read_to_string(&entry_path) else {
            continue;
        };
        let normalized = content.to_lowercase().replace('_', "-");
        for marker in SNAPSHOT_FORBIDDEN_MARKERS {
            if normalized.contains(marker) {
                leaks.push(format!("{} contains {marker}", entry_path.display()));
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "snapshot content must use neutral fixture names:\n{}",
        leaks.join("\n")
    );
}

fn enforced_files(workspace_root: &Path) -> Vec<PathBuf> {
    [
        "src",
        "tests",
        "alef.toml",
        "Cargo.toml",
        ".pre-commit-config.yaml",
        ".pre-commit-hooks.yaml",
        "hooks",
    ]
    .into_iter()
    .flat_map(|relative| {
        let path = workspace_root.join(relative);
        if path.is_file() {
            vec![path]
        } else {
            WalkDir::new(path)
                .into_iter()
                .filter_map(Result::ok)
                .map(walkdir::DirEntry::into_path)
                .filter(|entry_path| entry_path.is_file())
                .collect()
        }
    })
    .collect()
}
