#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

//! Differential test: `javac`, `dotnet`, and `kotlinc` are the authority for the Java, C#, and
//! Kotlin identifier grammars, invoked as subprocesses on every run.
//!
//! The expected accept/reject value for each probe is not written down anywhere in this file. It
//! is whatever the compiler says when it is handed the probe. A table of transcribed booleans
//! would prove only that the implementation equals a constant a human copied out of it once;
//! this asks the tool that actually has to accept alef's generated source.
//!
//! Two guards keep the comparison from passing vacuously:
//!
//! - `javac` truncates diagnostics at 100 by default. A truncated run makes rejected probes look
//!   accepted. `-Xmaxerrs` is passed and the "only showing the first" banner is asserted absent.
//! - The compiler must have both accepted and rejected at least one probe. A toolchain that
//!   failed to compile anything, or accepted everything, fails the run instead of agreeing with
//!   a validator that does the same.

use std::path::Path;
use std::process::Command;

use alef::codegen::coordinates::{
    nuget_ordinal_fold, validate_csharp_namespace, validate_java_package, validate_kotlin_package,
    validate_nuget_package_id,
};

/// Probe characters, chosen to straddle every boundary where the two grammars differ from each
/// other or from `char::is_alphabetic` / `char::is_alphanumeric`. No expected verdict here: the
/// compiler supplies it.
const PROBES: &[(char, &str)] = &[
    ('A', "Lu latin capital"),
    ('a', "Ll latin small"),
    ('\u{1c5}', "Lt titlecase"),
    ('\u{2b0}', "Lm modifier letter"),
    ('\u{6130}', "Lo cjk ideograph"),
    ('\u{fc}', "Ll u-umlaut"),
    ('\u{216b}', "Nl roman numeral twelve"),
    ('_', "Pc low line"),
    ('\u{203f}', "Pc undertie"),
    ('\u{fe4d}', "Pc dashed low line"),
    ('$', "Sc dollar sign"),
    ('\u{20ac}', "Sc euro sign"),
    ('\u{20a3}', "Sc french franc sign"),
    ('0', "Nd digit zero"),
    ('\u{660}', "Nd arabic-indic zero"),
    ('\u{301}', "Mn combining acute"),
    ('\u{93e}', "Mc devanagari vowel sign aa"),
    ('\u{200c}', "Cf zero-width non-joiner"),
    ('\u{b2}', "No superscript two"),
    ('\u{2603}', "So snowman"),
    ('\u{2c2}', "Sk modifier letter left arrowhead"),
    ('\u{20de}', "Me combining enclosing square"),
    ('-', "Pd hyphen-minus"),
    ('+', "Sm plus sign"),
    ('!', "Po exclamation mark"),
    ('\u{10400}', "Lu supplementary deseret"),
    ('\u{10000}', "Lo supplementary linear b"),
    ('\u{104a0}', "Nd supplementary osmanya digit"),
    ('\u{101fd}', "Mn supplementary phaistos"),
];

/// `(segment, description)` for both identifier positions.
fn probe_segments() -> Vec<(String, String)> {
    let mut segments = Vec::new();
    for &(character, description) in PROBES {
        segments.push((format!("{character}z"), format!("{description} as start")));
        segments.push((format!("a{character}z"), format!("{description} as continuation")));
    }
    segments
}

fn tool_available(tool: &str) -> bool {
    let version_flag = if tool == "kotlinc" { "-version" } else { "--version" };
    Command::new(tool)
        .arg(version_flag)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// An independent presence check for the skip guard below, deliberately not implemented by
/// calling [`tool_available`] itself: this test exists to prove `tool_available` agrees with a
/// real toolchain probe, so the guard must probe kotlinc a second, separate way rather than
/// trivially agreeing with the function under test. Checks exit status, not merely that the
/// process spawned -- a version-manager shim spawns fine then exits non-zero, so a spawn-only
/// check here would leave the skip unreachable and fail the assertion below on every machine
/// that has the shim but not a real kotlinc. ~keep
fn kotlinc_is_genuinely_installed() -> bool {
    Command::new("kotlinc")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn installed_kotlinc_is_detected_by_the_availability_probe() {
    if !kotlinc_is_genuinely_installed() {
        eprintln!("SKIPPED: kotlinc is not installed; its availability probe was not verified");
        return;
    }
    assert!(
        tool_available("kotlinc"),
        "the installed kotlinc must not be reported unavailable"
    );
}

/// Checks whether `tool` is on `PATH`. When `required` is `true` and it is not, panics naming
/// `env_var_name` instead of letting the caller silently skip -- the exact failure mode this
/// file exists to close: a CI job whose toolchain setup silently regressed, where the gate still
/// exits 0 because every oracle test quietly skipped instead of running.
///
/// `required` is a plain parameter rather than this function reading `env_var_name` from the
/// environment itself, so `required_javac_mode_fails_when_toolchain_is_unavailable` and its
/// dotnet/kotlinc siblings below can prove the panic fires without depending on process-global
/// environment state (`#[test]`s in this binary run in parallel by default).
fn require_tool(tool: &str, env_var_name: &str, required: bool) -> bool {
    let available = tool_available(tool);
    if !available && required {
        panic!("{env_var_name} is set but {tool} is unavailable");
    }
    available
}

/// Assert the compiler's verdicts are informative, then compare them against `validator`.
fn compare(language: &str, rejected: &[bool], validator: impl Fn(&str) -> bool) {
    let segments = probe_segments();
    assert_eq!(rejected.len(), segments.len(), "one verdict per probe");
    let accepted_count = rejected.iter().filter(|is_rejected| !**is_rejected).count();
    assert!(
        accepted_count > 0,
        "{language}: the compiler accepted no probe at all — the run proves nothing"
    );
    assert!(
        accepted_count < segments.len(),
        "{language}: the compiler rejected no probe at all — the run proves nothing"
    );

    let mut mismatches = Vec::new();
    for ((segment, description), &is_rejected) in segments.iter().zip(rejected) {
        let compiler_accepts = !is_rejected;
        if validator(segment) != compiler_accepts {
            mismatches.push(format!(
                "  {description}: {language} compiler {}, alef {}",
                if compiler_accepts { "ACCEPTS" } else { "rejects" },
                if compiler_accepts { "rejects" } else { "ACCEPTS" }
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{language}: alef disagrees with the compiler on {} of {} probes:\n{}",
        mismatches.len(),
        segments.len(),
        mismatches.join("\n")
    );
    eprintln!(
        "{language}: {accepted_count} accepted / {} rejected, all matched",
        segments.len() - accepted_count
    );
}

fn write_probe_sources(dir: &Path, extension: &str, render: impl Fn(&str, &str) -> String) {
    for (index, (segment, _)) in probe_segments().iter().enumerate() {
        let class = format!("P{index:04}");
        std::fs::write(dir.join(format!("{class}.{extension}")), render(&class, segment)).expect("write probe source");
    }
}

#[test]
fn javac_agrees_with_validate_java_package() {
    if !require_tool(
        "javac",
        "ALEF_REQUIRE_JAVAC",
        std::env::var_os("ALEF_REQUIRE_JAVAC").is_some(),
    ) {
        eprintln!("SKIPPED: javac is not installed; the Java grammar was NOT verified on this machine");
        return;
    }
    let dir = tempfile::tempdir().expect("create temp dir");
    let sources = dir.path().join("src");
    std::fs::create_dir_all(&sources).expect("create src");
    write_probe_sources(&sources, "java", |class, segment| {
        format!("package probe.{segment};\npublic class {class} {{}}\n")
    });

    let mut command = Command::new("javac");
    command
        .args(["-encoding", "UTF-8", "-nowarn", "-Xmaxerrs", "100000", "-d"])
        .arg(dir.path().join("out"))
        .current_dir(&sources);
    for (index, _) in probe_segments().iter().enumerate() {
        command.arg(format!("P{index:04}.java"));
    }
    let output = command.output().expect("run javac");
    let diagnostics = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !diagnostics.contains("only showing the first"),
        "javac truncated its diagnostics, which would make rejected probes look accepted:\n{diagnostics}"
    );

    let rejected: Vec<bool> = probe_segments()
        .iter()
        .enumerate()
        .map(|(index, _)| diagnostics.contains(&format!("P{index:04}.java:")))
        .collect();
    compare("java", &rejected, |segment| {
        validate_java_package(&format!("probe.{segment}")).is_ok()
    });
}

#[test]
fn dotnet_agrees_with_validate_csharp_namespace() {
    let required = std::env::var_os("ALEF_REQUIRE_DOTNET").is_some();
    if !require_tool("dotnet", "ALEF_REQUIRE_DOTNET", required) {
        eprintln!("SKIPPED: dotnet is not installed; the C# grammar was NOT verified on this machine");
        return;
    }
    let dir = tempfile::tempdir().expect("create temp dir");
    std::fs::write(
        dir.path().join("probe.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    \
         <TargetFramework>net10.0</TargetFramework>\n    <Nullable>disable</Nullable>\n    \
         <NoWarn>CS0414;CS0169;CS8981</NoWarn>\n  </PropertyGroup>\n</Project>\n",
    )
    .expect("write csproj");
    write_probe_sources(dir.path(), "cs", |class, segment| {
        format!("namespace Probe.{segment} {{ class {class} {{}} }}\n")
    });

    let output = Command::new("dotnet")
        .args(["build", "-c", "Release", "-v", "q", "--nologo"])
        .current_dir(dir.path())
        .env("DOTNET_NOLOGO", "1")
        .output()
        .expect("run dotnet build");
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if diagnostics.contains("NETSDK1045") || diagnostics.contains("was not found") {
        assert!(
            !required,
            "ALEF_REQUIRE_DOTNET is set but the installed dotnet SDK cannot target net10.0:\n{diagnostics}"
        );
        eprintln!("SKIPPED: this dotnet SDK cannot target net10.0; the C# grammar was NOT verified here");
        return;
    }

    let rejected: Vec<bool> = probe_segments()
        .iter()
        .enumerate()
        .map(|(index, _)| diagnostics.contains(&format!("P{index:04}.cs(")))
        .collect();
    compare("csharp", &rejected, |segment| {
        validate_csharp_namespace(&format!("Probe.{segment}")).is_ok()
    });
}

#[test]
fn dotnet_agrees_with_nuget_validation_and_ordinal_collisions() {
    if !require_tool(
        "dotnet",
        "ALEF_REQUIRE_DOTNET",
        std::env::var_os("ALEF_REQUIRE_DOTNET").is_some(),
    ) {
        eprintln!("SKIPPED: dotnet is not installed; NuGet semantics were NOT verified");
        return;
    }
    let directory = tempfile::tempdir().expect("create temp dir");
    std::fs::write(
        directory.path().join("probe.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>",
    )
    .expect("write csproj");
    std::fs::write(
        directory.path().join("Program.cs"),
        r#"using System;
using System.Text.RegularExpressions;
foreach (var value in new[] { "MyLib", "München.Δοκιμή", "𐐀" })
    Console.WriteLine(Regex.IsMatch(value, @"^\w+([_.-]\w+)*$"));
foreach (var pair in new[] { ("MyLib", "mylib"), ("Ü", "ü"), ("ẞ", "ß"), ("ſ", "S"), ("K", "K") })
    Console.WriteLine(StringComparer.OrdinalIgnoreCase.Equals(pair.Item1, pair.Item2));
"#,
    )
    .expect("write program");
    let output = Command::new("dotnet")
        .args(["run", "-c", "Release", "--nologo"])
        .current_dir(directory.path())
        .output()
        .expect("run dotnet");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let verdicts: Vec<bool> = stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.eq_ignore_ascii_case("true") {
                Some(true)
            } else if line.eq_ignore_ascii_case("false") {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    let expected = [
        validate_nuget_package_id("MyLib").is_ok(),
        validate_nuget_package_id("München.Δοκιμή").is_ok(),
        validate_nuget_package_id("𐐀").is_ok(),
        nuget_ordinal_fold("MyLib") == nuget_ordinal_fold("mylib"),
        nuget_ordinal_fold("Ü") == nuget_ordinal_fold("ü"),
        nuget_ordinal_fold("ẞ") == nuget_ordinal_fold("ß"),
        nuget_ordinal_fold("ſ") == nuget_ordinal_fold("S"),
        nuget_ordinal_fold("K") == nuget_ordinal_fold("K"),
    ];
    assert_eq!(
        verdicts.len(),
        expected.len(),
        "expected one .NET verdict per probe; stdout={stdout:?}, stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(verdicts, expected, "Alef diverged from .NET NuGet semantics");
}

#[test]
fn kotlinc_agrees_with_validate_kotlin_package() {
    if !require_tool(
        "kotlinc",
        "ALEF_REQUIRE_KOTLINC",
        std::env::var_os("ALEF_REQUIRE_KOTLINC").is_some(),
    ) {
        eprintln!("SKIPPED: kotlinc is not installed; the Kotlin grammar was NOT verified on this machine");
        return;
    }
    let dir = tempfile::tempdir().expect("create temp dir");
    let sources = dir.path().join("src");
    std::fs::create_dir_all(&sources).expect("create src");
    write_probe_sources(&sources, "kt", |class, segment| {
        format!("package probe.{segment}\nclass {class}\n")
    });

    let mut command = Command::new("kotlinc");
    command.arg("-d").arg(dir.path().join("out")).current_dir(&sources);
    for (index, _) in probe_segments().iter().enumerate() {
        command.arg(format!("P{index:04}.kt"));
    }
    let output = command.output().expect("run kotlinc");
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let rejected: Vec<bool> = probe_segments()
        .iter()
        .enumerate()
        .map(|(index, _)| diagnostics.contains(&format!("P{index:04}.kt:")))
        .collect();
    compare("kotlin", &rejected, |segment| {
        validate_kotlin_package(&format!("probe.{segment}")).is_ok()
    });
}

/// Proves required-toolchain mode cannot turn a missing compiler into a green oracle run.
/// `required_dotnet_mode_fails_when_toolchain_is_unavailable` and
/// `required_kotlinc_mode_fails_when_toolchain_is_unavailable` are the C#/Kotlin siblings.
#[test]
fn required_javac_mode_fails_when_toolchain_is_unavailable() {
    let result = std::panic::catch_unwind(|| require_tool("alef-javac-does-not-exist", "ALEF_REQUIRE_JAVAC", true));
    let panic = result.expect_err("required mode must fail when javac is unavailable");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    assert!(message.contains("ALEF_REQUIRE_JAVAC is set"), "got: {message}");
}

#[test]
fn required_dotnet_mode_fails_when_toolchain_is_unavailable() {
    let result = std::panic::catch_unwind(|| require_tool("alef-dotnet-does-not-exist", "ALEF_REQUIRE_DOTNET", true));
    let panic = result.expect_err("required mode must fail when dotnet is unavailable");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    assert!(message.contains("ALEF_REQUIRE_DOTNET is set"), "got: {message}");
}

#[test]
fn required_kotlinc_mode_fails_when_toolchain_is_unavailable() {
    let result = std::panic::catch_unwind(|| require_tool("alef-kotlinc-does-not-exist", "ALEF_REQUIRE_KOTLINC", true));
    let panic = result.expect_err("required mode must fail when kotlinc is unavailable");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    assert!(message.contains("ALEF_REQUIRE_KOTLINC is set"), "got: {message}");
}

/// Keeps required mode wired to a job that actually installs the compiler. Checking both
/// needles prevents either half from drifting into a vacuous green runtime test, mirroring
/// `e2e::codegen::elixir::streaming_pipe_precedence_tests::
/// ci_installs_and_requires_elixir_for_runtime_regressions`. ~keep
#[test]
fn ci_installs_and_requires_javac_for_runtime_regressions() {
    let workflow = include_str!("../.github/workflows/ci.yml");
    assert!(workflow.contains("uses: actions/setup-java@v6"));
    assert!(workflow.contains("ALEF_REQUIRE_JAVAC: \"1\""));
}

#[test]
fn ci_installs_and_requires_dotnet_for_runtime_regressions() {
    let workflow = include_str!("../.github/workflows/ci.yml");
    assert!(workflow.contains("uses: actions/setup-dotnet@v6"));
    assert!(workflow.contains("ALEF_REQUIRE_DOTNET: \"1\""));
}

#[test]
fn ci_installs_and_requires_kotlinc_for_runtime_regressions() {
    let workflow = include_str!("../.github/workflows/ci.yml");
    assert!(workflow.contains("uses: fwilhe2/setup-kotlin@"));
    assert!(workflow.contains("ALEF_REQUIRE_KOTLINC: \"1\""));
}
