use std::process::Output;

pub(super) fn describe(output: &Output) -> String {
    format!(
        "status={:?}; stdout={:?}; stderr={:?}; which::which(bash)={:?}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        which::which("bash")
    )
}
