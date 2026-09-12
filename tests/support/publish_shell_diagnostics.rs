use std::process::{Command, Output};

pub(super) fn bash_command() -> Command {
    // ~keep Windows' bare executable search can launch WSL even when PATH resolves Git Bash.
    let executable = which::which("bash").expect("bash must be installed for publish script tests");
    assert!(
        executable.is_absolute(),
        "resolved bash must be absolute: {executable:?}"
    );
    Command::new(executable)
}

pub(super) fn describe(output: &Output) -> String {
    format!(
        "status={:?}; stdout={:?}; stderr={:?}; which::which(bash)={:?}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        which::which("bash")
    )
}
