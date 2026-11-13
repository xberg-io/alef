use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

pub(super) fn census_file() -> Option<PathBuf> {
    static TARGET_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    let target = TARGET_DIR.get_or_init(target_directory).as_ref()?;
    let exe = std::env::current_exe().ok()?;
    let name = exe.file_stem()?.to_str()?;
    Some(target.join("toolchain-census").join(format!("{name}.tsv")))
}

fn target_directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ALEF_TEST_TARGET_DIR") {
        return Some(PathBuf::from(path));
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(PathBuf::from(metadata["target_directory"].as_str()?))
}
