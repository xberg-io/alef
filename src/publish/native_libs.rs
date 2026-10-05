//! Build-time facts the Go FFI package needs about linking the static FFI library: the native
//! libraries rustc says it requires, and the macOS deployment target the build ran with.

use super::platform::{Os, RustTarget};
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::{debug, warn};

/// File name, inside the Go package's `lib/` and next to the built artifacts, that holds the
/// linker arguments `rustc --print native-static-libs` reported for the static FFI library.
pub const NATIVE_STATIC_LIBS_FILE: &str = "native-static-libs.txt";

const NOTE_MARKER: &str = "native-static-libs:";
const DEPLOYMENT_TARGET_VAR: &str = "MACOSX_DEPLOYMENT_TARGET";

/// The linker arguments from one `note: native-static-libs: ...` line of rustc output.
pub(crate) fn parse_native_static_libs_line(line: &str) -> Option<String> {
    let (_, flags) = line.split_once(NOTE_MARKER)?;
    let flags = flags.trim();
    (!flags.is_empty()).then(|| flags.to_string())
}

/// Where `alef publish build` records the native libraries: beside the release artifacts, in the
/// directory `find_built_artifact` looks in first for this target.
pub(crate) fn record_path(workspace_root: &Path, target: Option<&RustTarget>) -> PathBuf {
    let target_dir = workspace_root.join("target");
    let target_dir = match target {
        Some(target) => target_dir.join(&target.triple),
        None => target_dir,
    };
    target_dir.join("release").join(NATIVE_STATIC_LIBS_FILE)
}

/// The recorded native libraries for `target`, searched where the built artifacts are.
pub(crate) fn find_recorded(workspace_root: &Path, target: &RustTarget) -> Option<PathBuf> {
    [Some(target), None]
        .into_iter()
        .map(|candidate| record_path(workspace_root, candidate))
        .find(|path| path.is_file())
}

/// `MACOSX_DEPLOYMENT_TARGET=<rustc default>` for a macOS target, unless the caller already set
/// the variable. Without it, C dependencies built by the `cc` crate pick a newer minimum than
/// rustc's own objects, and the static library then links with "built for newer macOS" warnings.
pub(crate) fn deployment_target_env(target: Option<&RustTarget>) -> Option<(&'static str, String)> {
    let target = target.cloned().or_else(|| super::platform::host_target().ok())?;
    deployment_target_env_for(
        &target,
        std::env::var_os(DEPLOYMENT_TARGET_VAR),
        rustc_default_deployment_target,
    )
}

fn deployment_target_env_for(
    target: &RustTarget,
    already_set: Option<OsString>,
    rustc_default: impl FnOnce(&RustTarget) -> Option<String>,
) -> Option<(&'static str, String)> {
    if target.os != Os::MacOs || already_set.is_some_and(|value| !value.is_empty()) {
        return None;
    }
    rustc_default(target).map(|version| (DEPLOYMENT_TARGET_VAR, version))
}

fn rustc_default_deployment_target(target: &RustTarget) -> Option<String> {
    let output = Command::new("rustc")
        .args(["--print", "deployment-target", "--target", &target.triple])
        .current_dir(std::env::temp_dir())
        .output()
        .ok()?;
    if !output.status.success() {
        warn!(target = %target.triple, "rustc could not report a default deployment target");
        return None;
    }
    parse_deployment_target(&String::from_utf8_lossy(&output.stdout))
}

fn parse_deployment_target(output: &str) -> Option<String> {
    let value = output
        .lines()
        .find_map(|line| line.strip_prefix(DEPLOYMENT_TARGET_VAR)?.strip_prefix('='))?
        .trim();
    let valid = !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.');
    valid.then(|| value.to_string())
}

/// Run `cmd` through `sh -c` like [`super::run_shell_command`], with `env` added. When
/// `record_to` is set, rustc's `native-static-libs` note is picked out of the command's stderr
/// (which is still passed through live) and written there; a record left by an earlier build is
/// removed first so a stale one can never be packaged.
pub(crate) fn run_build(cmd: &str, env: &[(&str, String)], record_to: Option<&Path>) -> Result<()> {
    debug!("  $ {cmd}");
    if let Some(path) = record_to {
        match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(err).with_context(|| format!("removing stale {}", path.display()));
            }
            _ => {}
        }
    }

    let mut command = Command::new("sh");
    command.arg("-c").arg(cmd).envs(env.iter().map(|(k, v)| (*k, v)));
    if record_to.is_some() {
        command.stderr(Stdio::piped());
    }
    let mut child = command.spawn().with_context(|| format!("running: {cmd}"))?;

    let mut flags = None;
    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr);
        let mut passthrough = std::io::stderr();
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line)? > 0 {
            passthrough.write_all(&line)?;
            if let Some(found) = parse_native_static_libs_line(&String::from_utf8_lossy(&line)) {
                flags = Some(found);
            }
            line.clear();
        }
    }
    let status = child.wait().with_context(|| format!("running: {cmd}"))?;
    if !status.success() {
        anyhow::bail!("command failed with exit code {}: {cmd}", status.code().unwrap_or(-1));
    }

    if let Some(path) = record_to {
        match flags {
            Some(flags) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
                }
                std::fs::write(path, format!("{flags}\n")).with_context(|| format!("writing {}", path.display()))?;
                debug!(path = %path.display(), "recorded native-static-libs");
            }
            None => warn!(
                "the build did not report native-static-libs (is `-- --print native-static-libs` in the build \
                 command?); the Go package will not carry {NATIVE_STATIC_LIBS_FILE}"
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(triple: &str) -> RustTarget {
        RustTarget::parse(triple).expect("parse triple")
    }

    #[test]
    fn parses_the_note_line_and_ignores_the_others() {
        let note = "note: native-static-libs: -framework Security -liconv -lSystem -lc -lm";
        assert_eq!(
            parse_native_static_libs_line(note).as_deref(),
            Some("-framework Security -liconv -lSystem -lc -lm")
        );
        assert_eq!(
            parse_native_static_libs_line("note: link against the following native artifacts"),
            None
        );
        assert_eq!(parse_native_static_libs_line("note: native-static-libs:   "), None);
    }

    #[test]
    fn deployment_target_is_set_only_for_macos_when_the_caller_has_not() {
        let rustc = |_: &RustTarget| Some("11.0".to_string());
        let mac = target("aarch64-apple-darwin");
        assert_eq!(
            deployment_target_env_for(&mac, None, rustc),
            Some(("MACOSX_DEPLOYMENT_TARGET", "11.0".to_string()))
        );
        assert_eq!(
            deployment_target_env_for(&mac, Some(OsString::new()), rustc),
            Some(("MACOSX_DEPLOYMENT_TARGET", "11.0".to_string())),
            "an empty variable counts as unset"
        );
        assert_eq!(
            deployment_target_env_for(&mac, Some(OsString::from("14.0")), rustc),
            None
        );
        assert_eq!(
            deployment_target_env_for(&target("x86_64-unknown-linux-gnu"), None, rustc),
            None
        );
        assert_eq!(deployment_target_env_for(&mac, None, |_| None), None);
    }

    #[test]
    fn parses_rustc_deployment_target_output() {
        assert_eq!(
            parse_deployment_target("MACOSX_DEPLOYMENT_TARGET=10.12\n").as_deref(),
            Some("10.12")
        );
        assert_eq!(parse_deployment_target("MACOSX_DEPLOYMENT_TARGET=\n"), None);
        assert_eq!(
            parse_deployment_target("MACOSX_DEPLOYMENT_TARGET=11.0; rm -rf /\n"),
            None
        );
        assert_eq!(parse_deployment_target("error: unknown\n"), None);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn rustc_reports_a_default_for_a_real_macos_target() {
        let version = rustc_default_deployment_target(&target("aarch64-apple-darwin"))
            .expect("rustc prints a deployment target for aarch64-apple-darwin");
        assert!(version.starts_with(|c: char| c.is_ascii_digit()), "got {version}");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn run_build_records_the_note_and_passes_env_through() {
        let tmp = tempfile::tempdir().unwrap();
        let record = tmp.path().join("target/release").join(NATIVE_STATIC_LIBS_FILE);
        let cmd =
            r#"echo "other noise" >&2; echo "note: native-static-libs: -lfoo -lbar" >&2; test "$PROBE_VAR" = set"#;
        run_build(cmd, &[("PROBE_VAR", "set".to_string())], Some(&record)).unwrap();
        assert_eq!(std::fs::read_to_string(&record).unwrap(), "-lfoo -lbar\n");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn run_build_without_a_note_removes_a_stale_record() {
        let tmp = tempfile::tempdir().unwrap();
        let record = tmp.path().join(NATIVE_STATIC_LIBS_FILE);
        std::fs::write(&record, "-lstale\n").unwrap();
        run_build("true", &[], Some(&record)).unwrap();
        assert!(!record.exists(), "a record from an earlier build must not survive");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn run_build_fails_when_the_command_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let record = tmp.path().join(NATIVE_STATIC_LIBS_FILE);
        let err = run_build("echo 'note: native-static-libs: -lfoo' >&2; exit 3", &[], Some(&record)).unwrap_err();
        assert!(err.to_string().contains("exit code 3"), "got {err}");
        assert!(!record.exists(), "a failed build must not leave a record");
    }

    #[test]
    fn record_path_follows_the_artifact_layout() {
        let root = Path::new("/ws");
        let linux = target("x86_64-unknown-linux-gnu");
        assert_eq!(
            record_path(root, Some(&linux)),
            Path::new("/ws/target/x86_64-unknown-linux-gnu/release/native-static-libs.txt")
        );
        assert_eq!(
            record_path(root, None),
            Path::new("/ws/target/release/native-static-libs.txt")
        );
    }
}
