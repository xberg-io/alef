//! Workspace cargo-sort, cargo fmt and mode-snapshot coverage for the format pipeline, split from tests.rs.

use super::tests::*;
use super::*;
use crate::core::config::{Language, NewAlefConfig};

/// Writes a two-crate cargo workspace at `base` with deliberately unsorted
/// `[dependencies]` in both members. `second_crate_suffix` lets callers simulate a
/// crate that historically had no per-language cargo-sort residual (e.g. a `-py`
/// or `-node` crate) to prove the workspace-wide sort covers it too.
fn write_unsorted_workspace(base: &Path, second_crate_suffix: &str) {
    std::fs::write(
        base.join("Cargo.toml"),
        format!("[workspace]\nmembers = [\"crates/pkg-ffi\", \"crates/pkg{second_crate_suffix}\"]\nresolver = \"2\"\n"),
    )
    .unwrap();
    for member in ["pkg-ffi", &format!("pkg{second_crate_suffix}")] {
        let crate_dir = base.join("crates").join(member);
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{member}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\nserde = \"1\"\nanyhow = \"1\"\n"
            ),
        )
        .unwrap();
        std::fs::write(crate_dir.join("src/lib.rs"), "pub fn noop() {}\n").unwrap();
    }
}

#[test]
fn run_workspace_cargo_sort_sorts_every_member_regardless_of_language() {
    // Spawns a real `cargo sort`; see `test_support::REAL_CARGO_LOCK`'s doc for why this must
    // be serialized against every other test in the crate that also spawns one. ~keep
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    // "-py" simulates a crate suffix (python/pyo3) that has no entry at all in
    // `language_residuals` — the coverage gap this workspace-wide pass replaces.
    write_unsorted_workspace(base, "-py");

    run_workspace_cargo_sort(base, &mut FormatPass::new(&is_tool_available));

    let py_toml = std::fs::read_to_string(base.join("crates/pkg-py/Cargo.toml")).unwrap();
    if is_tool_available("cargo-sort") {
        let anyhow_pos = py_toml.find("anyhow").expect("anyhow present");
        let serde_pos = py_toml.find("serde").expect("serde present");
        assert!(
            anyhow_pos < serde_pos,
            "cargo sort -n -w must sort deps alphabetically even for a crate with no \
             per-language residual, got: {py_toml}"
        );
        let check = std::process::Command::new("cargo")
            .args(["sort", "--check", "-w"])
            .current_dir(base)
            .output()
            .expect("cargo sort --check");
        assert!(
            check.status.success(),
            "workspace must be reported sorted after run_workspace_cargo_sort: {}",
            String::from_utf8_lossy(&check.stderr)
        );
    } else {
        assert!(
            py_toml.contains("serde = \"1\"\nanyhow = \"1\""),
            "without cargo-sort, untouched"
        );
    }
}

#[test]
fn run_workspace_cargo_sort_is_noop_without_root_cargo_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    std::fs::write(base.join("marker.txt"), "untouched").unwrap();

    run_workspace_cargo_sort(base, &mut FormatPass::new(&is_tool_available));

    assert!(
        !base.join("Cargo.toml").exists(),
        "must not create a Cargo.toml when there was none"
    );
}

#[test]
fn run_cargo_fmt_formats_workspace_rust_files_when_available() {
    // Spawns a real `cargo fmt`; see `test_support::REAL_CARGO_LOCK`'s doc. ~keep
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    write_unsorted_workspace(base, "-node");
    let lib_path = base.join("crates/pkg-ffi/src/lib.rs");
    std::fs::write(&lib_path, "pub fn noop( ) {\nlet x=1;\nx;\n}\n").unwrap();

    run_cargo_fmt(base, &mut FormatPass::new(&is_tool_available));

    let formatted = std::fs::read_to_string(&lib_path).unwrap();
    if is_tool_available("cargo") && is_tool_available("rustfmt") {
        assert_eq!(
            formatted, "pub fn noop() {\n    let x = 1;\n    x;\n}\n",
            "cargo fmt --all must reformat every workspace member's Rust source"
        );
    } else {
        assert_eq!(
            formatted, "pub fn noop( ) {\nlet x=1;\nx;\n}\n",
            "without cargo/rustfmt, untouched"
        );
    }
}

#[test]
fn run_cargo_fmt_is_noop_without_root_cargo_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let file_path = base.join("orphan.rs");
    std::fs::write(&file_path, "fn noop( ) {}\n").unwrap();

    run_cargo_fmt(base, &mut FormatPass::new(&is_tool_available));

    assert_eq!(
        std::fs::read_to_string(&file_path).unwrap(),
        "fn noop( ) {}\n",
        "a directory with no root Cargo.toml is not a cargo workspace; must be left untouched"
    );
}

/// Writes a minimal, dependency-free mix project at `elixir_dir` (`mix.exs`,
/// `.formatter.exs`, and `lib/sample.ex` with `content`). Deliberately has no
/// `deps` (no `import_deps`), so `mix deps.get`/`mix format` never touch the
/// network -- these tests must stay hermetic per test-independence, unlike the
/// scaffolded `.formatter.exs`'s real `import_deps: [:rustler]`.
fn write_minimal_mix_project(elixir_dir: &Path, content: &str) {
    std::fs::create_dir_all(elixir_dir.join("lib")).unwrap();
    std::fs::write(
        elixir_dir.join("mix.exs"),
        "defmodule Sample.MixProject do\n  use Mix.Project\n\n  def project do\n    [app: :sample, \
         version: \"0.1.0\", elixir: \"~> 1.14\"]\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(
        elixir_dir.join(".formatter.exs"),
        "[inputs: [\"mix.exs\", \"lib/**/*.{ex,exs}\"]]\n",
    )
    .unwrap();
    std::fs::write(elixir_dir.join("lib/sample.ex"), content).unwrap();
}

#[test]
fn run_elixir_mix_format_is_noop_without_packages_elixir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();

    run_elixir_mix_format(base, &mut FormatPass::new(&is_tool_available));

    assert!(
        !base.join("packages/elixir").exists(),
        "must not create a packages/elixir when there was none"
    );
}

#[test]
fn run_elixir_mix_format_reformats_the_generated_package_when_mix_installed() {
    if !is_tool_available("mix") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let elixir_dir = base.join("packages/elixir");
    write_minimal_mix_project(&elixir_dir, "defmodule Sample do\n  def noop, do:    :ok\nend\n");

    run_elixir_mix_format(base, &mut FormatPass::new(&is_tool_available));

    assert_eq!(
        std::fs::read_to_string(elixir_dir.join("lib/sample.ex")).unwrap(),
        "defmodule Sample do\n  def noop, do: :ok\nend\n",
        "mix format must collapse the misaligned `do:` spacing"
    );
}

#[test]
fn poly_format_never_rewrites_elixir_source() {
    if !is_tool_available("poly") || !is_tool_available("mix") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let elixir_dir = base.join("packages/elixir");
    let correctly_formatted = "defmodule Sample do\n  def clean(map) do\n    map\n    |> Enum.reject(fn {_k, v} -> v == nil end)\n    \
         |> Map.new()\n  end\nend\n";
    write_minimal_mix_project(&elixir_dir, correctly_formatted);
    // A sibling non-Elixir file in the same package dir proves the exclude is
    // scoped to `.ex`/`.exs`, not the whole `packages/elixir` directory (the
    // native NIF crate's Rust sources still need poly's rustfmt engine).
    std::fs::write(elixir_dir.join("unrelated.py"), "x=1").unwrap();

    poly_format(&[base.to_path_buf()], base);

    assert_eq!(
        std::fs::read_to_string(elixir_dir.join("lib/sample.ex")).unwrap(),
        correctly_formatted,
        "poly must never rewrite .ex files -- mix format is their sole authority"
    );
    assert_eq!(
        std::fs::read_to_string(elixir_dir.join("unrelated.py")).unwrap(),
        "x = 1\n",
        "excluding .ex/.exs must not stop poly from formatting other files in the same dir"
    );
}

#[test]
fn format_generated_partial_regen_leaves_mix_formatted_elixir_untouched_by_poly() {
    if !is_tool_available("poly") || !is_tool_available("mix") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let elixir_dir = base.join(config.package_dir(Language::Elixir));
    let correctly_formatted = "defmodule Sample do\n  def clean(map) do\n    map\n    |> Enum.reject(fn {_k, v} -> \
         v == nil end)\n    |> Map.new()\n  end\nend\n";
    write_minimal_mix_project(&elixir_dir, correctly_formatted);

    let only: HashSet<Language> = [Language::Elixir].into_iter().collect();
    format_generated(&config, base, Some(&only));

    assert_eq!(
        std::fs::read_to_string(elixir_dir.join("lib/sample.ex")).unwrap(),
        correctly_formatted,
        "the full format_generated pipeline (poly excluded + mix residual) must leave \
         already-mix-formatted Elixir source unchanged"
    );
}

#[test]
fn poly_fmt_is_clean_reflects_check_state() {
    if !is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let py_path = base.join("dirty.py");
    std::fs::write(&py_path, "x=1").unwrap();

    assert!(!poly_fmt_is_clean(base), "an unformatted file must report not-clean");

    poly_format(&[base.to_path_buf()], base);

    assert!(
        poly_fmt_is_clean(base),
        "after `poly fmt --fix`, the tree must report clean"
    );
}

#[test]
fn poly_format_accepts_clean_and_reformatted_exit_codes() {
    assert!(poly_format_exit_code_is_success(Some(0)));
    assert!(poly_format_exit_code_is_success(Some(1)));
    assert!(!poly_format_exit_code_is_success(Some(2)));
    assert!(!poly_format_exit_code_is_success(None));
}

#[test]
fn poly_format_rejects_engine_failures_even_when_poly_exits_successfully() {
    assert!(poly_format_output_reports_failure(
        b"WARN format failed: parser rejected generated.py path=generated.py"
    ));
    assert!(!poly_format_output_reports_failure(
        b"WARN generated file was not rewritten without --fix-generated"
    ));
}

/// poly rewrites changed files via atomic rename, which resets the mode to `0644`.
/// `poly_format` must put the executable bit back, or every regen strips it from
/// the generated shebang scripts and poly's own `file-safety` lint rejects the
/// next commit.
#[cfg(unix)]
#[test]
fn poly_format_restores_executable_bit_on_reformatted_shebang_script() {
    use std::os::unix::fs::PermissionsExt as _;

    if !is_tool_available("poly") {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let script = base.join("run_tests.php");
    std::fs::write(&script, "#!/usr/bin/env php\n<?php\n$x   =   1;\necho $x;\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    poly_format(&[base.to_path_buf()], base);

    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o755,
        "poly_format must restore the pre-format mode, got {mode:#o}"
    );
}

/// The snapshot must not walk dependency-cache and build directories: on a
/// repo-root pass they dwarf the tree being formatted and hold no generated
/// scripts.
#[cfg(unix)]
#[test]
fn executable_mode_snapshot_skips_dependency_and_build_directories() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let tracked = base.join("scripts/run.sh");
    let ignored = base.join("node_modules/.bin/tool");
    for path in [&tracked, &ignored] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let snapshot = snapshot_executable_modes(&[base.to_path_buf()]);

    let recorded: Vec<&PathBuf> = snapshot.iter().map(|(path, _)| path).collect();
    assert_eq!(recorded, vec![&tracked], "only the non-skipped script may be recorded");
}

#[test]
fn converge_full_regen_formatting_leaves_workspace_sorted_and_poly_fmt_check_clean() {
    // Spawns real `cargo fmt`/`cargo sort`; see `test_support::REAL_CARGO_LOCK`'s doc. ~keep
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    // "-swift" simulates another crate suffix with no per-language residual today.
    write_unsorted_workspace(base, "-swift");
    std::fs::write(
        base.join("crates/pkg-swift/src/lib.rs"),
        "pub fn noop( ) {\nlet x=1;\nx;\n}\n",
    )
    .unwrap();

    converge_full_regen_formatting(base);

    if is_tool_available("cargo-sort") {
        let swift_toml = std::fs::read_to_string(base.join("crates/pkg-swift/Cargo.toml")).unwrap();
        let anyhow_pos = swift_toml.find("anyhow").expect("anyhow present");
        let serde_pos = swift_toml.find("serde").expect("serde present");
        assert!(
            anyhow_pos < serde_pos,
            "workspace-wide sort must cover every crate on a full regen"
        );
    }
    if is_tool_available("cargo") && is_tool_available("rustfmt") {
        let formatted = std::fs::read_to_string(base.join("crates/pkg-swift/src/lib.rs")).unwrap();
        assert_eq!(formatted, "pub fn noop() {\n    let x = 1;\n    x;\n}\n");
    }
    if is_tool_available("poly") {
        assert!(
            poly_fmt_is_clean(base),
            "the convergence loop must leave `poly fmt --check` clean, satisfying the \
             zero-drift full-regen contract"
        );
    }
}

#[test]
fn format_generated_full_regen_routes_through_convergence_loop() {
    // Spawns real `cargo fmt`/`cargo sort`; see `test_support::REAL_CARGO_LOCK`'s doc. ~keep
    let _cargo_lock = crate::test_support::RealCargoGuard::acquire();
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    write_unsorted_workspace(base, "-py");

    let config = make_config("sample-model");

    // A full regen (`only_languages = None`) must go through
    // `converge_full_regen_formatting`, not the single-pass + per-language-residual
    // branch, so it also sorts the "-py" crate that has no residual entry.
    format_generated(&config, base, None);

    if is_tool_available("cargo-sort") {
        let py_toml = std::fs::read_to_string(base.join("crates/pkg-py/Cargo.toml")).unwrap();
        let anyhow_pos = py_toml.find("anyhow").expect("anyhow present");
        let serde_pos = py_toml.find("serde").expect("serde present");
        assert!(
            anyhow_pos < serde_pos,
            "format_generated(..., None) must workspace-sort crates with no per-language residual"
        );
    }
}

/// `is_tool_available` must resolve a real executable purely by walking `PATH` -- with no
/// `which`/`where` binary anywhere on that PATH. The old implementation shelled out to
/// `Command::new("which")`; on a PATH like this one (no `which` binary present, mirroring
/// every Windows PATH), that spawn itself fails and `unwrap_or(false)` reports the tool
/// absent -- indistinguishable from a tool that is genuinely missing. Asserting only that a
/// known-present tool like `cargo` is found would not catch this: the old code passed that
/// check fine on any platform that happens to ship a `which` binary. This test controls for
/// that by removing `which` from the probe's reach entirely and using a fabricated
/// executable, so a regression to the old shell-out shape fails it regardless of host OS.
#[cfg(unix)]
#[test]
fn is_tool_available_resolves_from_path_alone_without_a_which_binary() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let fake_tool = dir.path().join("alef-probe-fixture-tool");
    std::fs::write(&fake_tool, b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&fake_tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let isolated_path = dir.path().as_os_str().to_os_string();

    assert!(
        is_tool_available_on("alef-probe-fixture-tool", Some(isolated_path.clone())),
        "a real, executable file on PATH must be found even with no `which` binary anywhere \
         on that PATH"
    );
    assert!(
        !is_tool_available_on("alef-probe-fixture-tool-that-does-not-exist", Some(isolated_path)),
        "a tool genuinely absent from PATH must still be reported absent, not found by accident"
    );
}

/// Poly must not format `.cs`.
///
/// Poly delegates C# to an external clang-format whose layout differs across its own versions
/// (18.1.8 and 23.1.0 each reproduce a different committed blob), so letting it reflow generated
/// C# makes the committed bytes depend on which clang-format the machine happens to have. alef's
/// emission is already `dotnet format whitespace`-clean, pinned by
/// `generated_csharp_uses_formatter_stable_layout`, so the exclusion is what keeps that contract
/// authoritative rather than advisory. ~keep
#[test]
fn poly_excludes_csharp_and_elixir_sources() {
    let mut args: Vec<String> = Vec::new();
    super::push_poly_format_excludes(&mut args);

    for glob in ["**/*.cs", "**/*.ex", "**/*.exs"] {
        let excluded = args.windows(2).any(|pair| pair[0] == "--exclude" && pair[1] == glob);
        assert!(excluded, "poly was not told to exclude {glob}: {args:?}");
    }
}
