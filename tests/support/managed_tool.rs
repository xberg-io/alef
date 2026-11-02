//! Resolve repository-selected native tools before a test moves into an isolated fixture.
//! A Mise shim in PATH needs the repository configuration and may add a shell argument boundary.

use std::path::PathBuf;
use std::process::Command;

pub fn executable(tool: &str) -> PathBuf {
    if cfg!(windows)
        && tool == "ruby"
        && let Some(path) = std::env::var_os("ALEF_TEST_RUBY_EXECUTABLE")
    {
        return native_executable(tool, PathBuf::from(path));
    }
    let name = if cfg!(windows) { "mise.exe" } else { "mise" };
    let mise = which::which(name).expect("Mise is required to select the repository-managed test tool");
    let output = Command::new(mise)
        .args(["which", tool])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("MISE_AUTO_INSTALL", "0")
        .env("MISE_TASK_RUN_AUTO_INSTALL", "false")
        .output()
        .expect("resolve the managed tool without installing or selecting a global default");
    assert!(
        output.status.success(),
        "cannot resolve repository-managed {tool}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("managed executable path is UTF-8")
            .trim(),
    );
    native_executable(tool, path)
}

fn native_executable(tool: &str, path: PathBuf) -> PathBuf {
    assert!(
        path.is_absolute() && path.is_file(),
        "managed {tool} must be an absolute file: {path:?}"
    );
    if cfg!(windows) {
        assert!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe")),
            "Windows must invoke the native managed {tool} executable, not a shell shim: {path:?}"
        );
    }
    path
}
