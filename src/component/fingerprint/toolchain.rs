//! The native toolchain identity folded into a fingerprint for Apple and MSVC targets: the SDK and
//! linker a runner image provides, which `rustc -vV` and the build variables do not reveal.
//!
//! Like `rustc -vV`, the identity is only meaningful on the machine that builds: a target's
//! toolchain is probed only when the host OS family matches the target's, so `plan` and `build`
//! agree as long as both run on that family.

use std::process::Command;

/// Stable value recorded when a probe is unavailable, so a missing tool never fails planning and
/// never differs between two runs that both lack it.
pub const ABSENT: &str = "absent";

/// Runs a probe command and returns its output. Injectable so tests need no real toolchain.
pub trait ToolProbe {
    /// Trimmed standard output of `program args`. `None` when the program is missing or exits
    /// non-zero.
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
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// The OS family of the machine running alef.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostFamily {
    Macos,
    Windows,
    Other,
}

impl HostFamily {
    /// The lowercase name used in plans and messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Other => "other",
        }
    }

    /// Whether `target` has a native toolchain identity that only a matching host can probe.
    #[must_use]
    pub fn needs_matching_host(target: &str) -> bool {
        target.contains("-apple-") || target.ends_with("-windows-msvc")
    }

    /// The family alef was compiled for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// The `xcrun` SDK name for an Apple `target`, from its OS component:
/// `darwin` and `ios-macabi` use `macosx`; `ios`, `tvos`, `watchos` and `visionos` use the device
/// SDK, except for the `-sim` variants and the x86_64 targets, which are simulators.
#[must_use]
pub fn apple_sdk(target: &str) -> &'static str {
    let mut parts = target.split('-');
    let arch = parts.next().unwrap_or("");
    let os = parts.nth(1).unwrap_or("");
    let env = parts.next().unwrap_or("");
    let simulator = env == "sim" || env == "simulator" || matches!(arch, "x86_64" | "i386" | "i686");
    let pick = |device, sim| if simulator { sim } else { device };
    if os.starts_with("ios") {
        if env == "macabi" {
            "macosx"
        } else {
            pick("iphoneos", "iphonesimulator")
        }
    } else if os.starts_with("tvos") {
        pick("appletvos", "appletvsimulator")
    } else if os.starts_with("watchos") {
        pick("watchos", "watchsimulator")
    } else if os.starts_with("visionos") {
        pick("xros", "xrsimulator")
    } else {
        "macosx"
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

/// `(key, value)` records describing the native toolchain for `target`, sorted by key; empty unless
/// `host` is the target's own family (see the module docs) and the target is `*-apple-*` or
/// `*-windows-msvc`.
///
/// * Apple: the SDK version and build version `xcrun` reports for the target's SDK, and the first
///   line of `xcrun --sdk <sdk> clang --version` (Apple clang drives the link).
/// * MSVC: the Visual Studio product version and full installation version from `vswhere` under
///   `ProgramFiles(x86)` (matched case-insensitively, default `C:\Program Files (x86)`). The
///   developer-shell variables `VCToolsVersion` and `WindowsSDKVersion` are not used: they exist
///   only inside such a shell, so a plan and a build would disagree.
///
/// Anything that cannot be probed is recorded as [`ABSENT`].
#[must_use]
pub fn native_toolchain(
    target: &str,
    host: HostFamily,
    vars: &[(String, String)],
    probe: &dyn ToolProbe,
) -> Vec<(String, String)> {
    let mut records = Vec::new();
    if target.contains("-apple-") && host == HostFamily::Macos {
        let sdk = apple_sdk(target);
        let sdk_probe = |flag: &str| first_line(probe.run("xcrun", &["--sdk", sdk, flag]));
        records.push((format!("apple-sdk:{sdk}:version"), sdk_probe("--show-sdk-version")));
        records.push((format!("apple-sdk:{sdk}:build"), sdk_probe("--show-sdk-build-version")));
        records.push((
            "apple-clang".to_string(),
            first_line(probe.run("xcrun", &["--sdk", sdk, "clang", "--version"])),
        ));
    } else if target.ends_with("-windows-msvc") && host == HostFamily::Windows {
        let program_files = vars
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("ProgramFiles(x86)"))
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "C:\\Program Files (x86)".to_string());
        let vswhere = format!("{program_files}\\Microsoft Visual Studio\\Installer\\vswhere.exe");
        for (key, property) in [
            ("msvc:vswhere", "catalog_productDisplayVersion"),
            ("msvc:vswhere-installation", "installationVersion"),
        ] {
            let output = probe.run(
                &vswhere,
                &[
                    "-latest",
                    "-products",
                    "*",
                    "-requires",
                    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                    "-property",
                    property,
                ],
            );
            records.push((key.to_string(), first_line(output)));
        }
    }
    records.sort();
    records
}
