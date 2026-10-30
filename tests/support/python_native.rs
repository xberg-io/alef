#[path = "../../src/test_support/python_native.rs"]
mod shared;

pub(crate) use shared::configure_python;

fn tool_path_with_stable_path(tool_name: &str) -> Option<std::path::PathBuf> {
    which::which(tool_name).ok()
}

fn spawn_from_stable_dir(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.current_dir(std::env::temp_dir());
    command
}
