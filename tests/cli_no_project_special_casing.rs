use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;

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
