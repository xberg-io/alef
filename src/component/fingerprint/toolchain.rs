//! The native toolchain identity folded into a fingerprint for Apple and MSVC targets: the SDK and
//! linker a runner image provides, which `rustc -vV` and the build variables do not reveal.

use std::process::Command;

/// Stable value recorded when a probe is unavailable, so a missing tool never fails planning and
/// never differs between two runs that both lack it.
pub const ABSENT: &str = "absent";

/// Runs a probe command and returns its output. Injectable so tests need no real toolchain.
pub trait ToolProbe {
    /// Standard output of `program args`, or standard error when standard output is empty, trimmed.
    /// `None` when the program is missing or exits non-zero.
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

impl<F: Fn(&str, &[&str]) -> Option<String>> ToolProbe for F {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        self(program, args)
    }
}

/// Runs the real command on the build machine.
pub struct SystemProbe;

impl ToolProbe for SystemProbe {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(program).args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            return Some(stdout);
        }
        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// The `xcrun` SDK name for an Apple `target`.
fn apple_sdk(target: &str) -> &'static str {
    if target.contains("apple-ios-macabi") || !target.contains("apple-ios") {
        "macosx"
    } else if target.ends_with("-sim") || target.starts_with("x86_64-") {
        "iphonesimulator"
    } else {
        "iphoneos"
    }
}

fn first_line(output: Option<String>) -> String {
    output
        .and_then(|text| {
            text.lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| ABSENT.to_string())
}

/// `(key, value)` records describing the native toolchain for `target`, sorted by key; empty for
/// targets that are neither `*-apple-*` nor `*-windows-msvc`.
///
/// * Apple: the SDK version and build version `xcrun` reports for the target's SDK, and the first
///   line of `ld -v`.
/// * MSVC: `VCToolsVersion` and `WindowsSDKVersion` from `vars` (set by the developer shell).
///
/// Anything that cannot be probed is recorded as [`ABSENT`].
#[must_use]
pub fn native_toolchain(target: &str, vars: &[(String, String)], probe: &dyn ToolProbe) -> Vec<(String, String)> {
    let mut records = Vec::new();
    if target.contains("-apple-") {
        let sdk = apple_sdk(target);
        let sdk_probe = |flag: &str| first_line(probe.run("xcrun", &["--sdk", sdk, flag]));
        records.push((format!("apple-sdk:{sdk}:version"), sdk_probe("--show-sdk-version")));
        records.push((format!("apple-sdk:{sdk}:build"), sdk_probe("--show-sdk-build-version")));
        records.push(("apple-ld".to_string(), first_line(probe.run("ld", &["-v"]))));
    } else if target.ends_with("-windows-msvc") {
        for name in ["VCToolsVersion", "WindowsSDKVersion"] {
            let value = vars
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| ABSENT.to_string());
            records.push((format!("msvc:{name}"), value));
        }
    }
    records.sort();
    records
}
