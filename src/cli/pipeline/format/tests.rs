use super::*;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};

fn make_config(crate_name: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid config");
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn formatter_error_includes_stdout_and_stderr() {
    let err = run_formatter(
        "sh",
        &["-c", "printf 'stdout text'; printf 'stderr text' >&2; exit 7"],
        Path::new("."),
    )
    .expect_err("formatter should fail");
    let msg = err.to_string();
    assert!(msg.contains("stdout text"), "missing stdout in error: {msg}");
    assert!(msg.contains("stderr text"), "missing stderr in error: {msg}");
}

// ---------------------------------------------------------------------------
// Residual native passes (the project-wide tools poly cannot wrap).
// ---------------------------------------------------------------------------

#[test]
fn wasm_residual_is_cargo_sort_on_the_crate_dir() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Wasm, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "wasm residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(
        steps[0].args,
        vec!["sort", "crates/sample-model-wasm"],
        "cargo sort arg must be the wasm crate directory"
    );
    assert_eq!(
        steps[0].work_dir,
        Path::new("/repo"),
        "wasm cargo sort runs at repo root"
    );
}

#[test]
fn wasm_residual_uses_configured_output_path() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "sample-language-pack"
sources = ["crates/sample-pack-core/src/lib.rs"]
[crates.output]
wasm = "crates/sample-pack-core-wasm/src/"
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let steps = language_residuals(&config, Language::Wasm, Path::new("/repo"));
    assert_eq!(
        steps[0].args,
        vec!["sort", "crates/sample-pack-core-wasm"],
        "cargo sort arg must match the crate dir derived from the configured output path"
    );
}

#[test]
fn ffi_residual_is_cargo_sort_workspace_wide() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Ffi, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "FFI residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(
        steps[0].args,
        vec!["sort", "-w"],
        "FFI cargo sort must be workspace-wide so every in-workspace binding crate is normalised"
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo"));
}

#[test]
fn ruby_residual_sorts_the_native_crate() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Ruby, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "Ruby residual must be a single cargo sort step");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(steps[0].args[0], "sort");
    assert!(
        steps[0].args[1].starts_with("ext/") && steps[0].args[1].ends_with("/native"),
        "cargo sort arg must target ext/<gem>/native, got: {:?}",
        steps[0].args
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo/packages/ruby"));
}

#[test]
fn elixir_residual_is_cargo_sort_only() {
    // `.ex`/`.exs` are formatted by poly's tier-2 tier (no `mix format`); the
    // only residual is cargo sort for the workspace-excluded NIF crate.
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::Elixir, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "Elixir residual must be cargo sort only");
    assert_eq!(steps[0].command, "cargo");
    assert_eq!(steps[0].args[0], "sort");
    assert!(
        steps[0].args[1].starts_with("native/") && steps[0].args[1].ends_with("_nif"),
        "cargo sort arg must target native/<app>_nif, got: {:?}",
        steps[0].args
    );
    assert_eq!(steps[0].work_dir, Path::new("/repo/packages/elixir"));
}

#[test]
fn r_residual_sorts_the_extendr_crate() {
    let config = make_config("sample-model");
    let steps = language_residuals(&config, Language::R, Path::new("/repo"));
    assert_eq!(steps.len(), 1, "R residual must be a single cargo sort step");
    assert_eq!(steps[0].args, vec!["sort", "packages/r/src/rust"]);
    assert_eq!(steps[0].work_dir, Path::new("/repo"));
}

#[test]
fn csharp_has_no_residual() {
    // C# is formatted by poly's tier-2 tier — no `dotnet format` residual.
    let config = make_config("sample-model");
    assert!(language_residuals(&config, Language::Csharp, Path::new("/repo")).is_empty());
}

#[test]
fn languages_without_residuals_have_none() {
    let config = make_config("sample-model");
    for lang in [
        Language::Python,
        Language::Node,
        Language::Go,
        Language::Java,
        Language::Kotlin,
        Language::Csharp,
    ] {
        assert!(
            language_residuals(&config, lang, Path::new("/repo")).is_empty(),
            "{lang:?} must have no residual native pass (poly formats it)"
        );
    }
}

// ---------------------------------------------------------------------------
// poly_paths scoping.
// ---------------------------------------------------------------------------

#[test]
fn poly_paths_full_regen_is_repo_root() {
    let config = make_config("sample-model");
    let paths = poly_paths(&config, Path::new("/repo"), None, &[Language::Python]);
    assert_eq!(
        paths,
        vec![PathBuf::from("/repo")],
        "full regen formats the whole repo once"
    );
}

#[test]
fn poly_paths_partial_regen_scopes_to_existing_package_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = make_config("sample-model");
    let py_dir = base.join(config.package_dir(Language::Python));
    std::fs::create_dir_all(&py_dir).unwrap();

    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);
    assert_eq!(
        paths,
        vec![py_dir],
        "partial regen scopes to the changed language's package dir"
    );
}

#[test]
fn poly_paths_partial_regen_drops_nonexistent_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = make_config("sample-model");
    // No package dirs created on disk.
    let only: HashSet<Language> = [Language::Python].into_iter().collect();
    let paths = poly_paths(&config, base, Some(&only), &[Language::Python]);
    assert!(paths.is_empty(), "nonexistent package dirs are dropped");
}

// ---------------------------------------------------------------------------
// enabled / custom-override gating.
// ---------------------------------------------------------------------------

// Custom format_override commands must run even when the language is absent from
// the only_languages filter (files not re-written this run) so the embedded
// `alef:hash:` is computed over formatter-normalized content.
#[test]
fn custom_override_runs_when_lang_absent_from_only_languages_filter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sentinel = dir.path().join("was_run.txt");
    let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");

    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["php"]

[workspace.format_overrides.php]
command = "touch {sentinel_str}"

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid config");
    let config = cfg.resolve().expect("resolve").remove(0);

    let files: Vec<(Language, Vec<GeneratedFile>)> = vec![(Language::Php, vec![])];
    let only_languages: HashSet<Language> = HashSet::new();

    assert!(!sentinel.exists(), "sentinel must not exist before format_generated");
    format_generated(&files, &config, dir.path(), Some(&only_languages));
    assert!(
        sentinel.exists(),
        "custom format_override command must run even when php is absent from only_languages"
    );
}

// `[workspace.format] enabled = false` (resolved onto a per-language override)
// disables formatting for that language entirely — even a custom command is
// skipped, because the enabled gate precedes command dispatch.
#[test]
fn disabled_language_skips_everything_including_custom_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sentinel = dir.path().join("was_run.txt");
    let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");

    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["php"]

[workspace.format_overrides.php]
enabled = false
command = "touch {sentinel_str}"

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#
    ))
    .expect("valid config");
    let config = cfg.resolve().expect("resolve").remove(0);

    let files: Vec<(Language, Vec<GeneratedFile>)> = vec![(Language::Php, vec![])];
    format_generated(&files, &config, dir.path(), None);
    assert!(
        !sentinel.exists(),
        "enabled = false must skip formatting entirely, including the custom command"
    );
}

// Behavioral: when the `poly` CLI is installed, `format_generated` shells out to
// it and a badly-spaced Python file ends up ruff-formatted. When `poly` is absent
// the pass is a best-effort no-op — the file is left untouched and nothing panics.
#[test]
fn poly_pass_formats_generated_python_when_poly_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let py_path = base.join("packages/python/foo.py");
    std::fs::create_dir_all(py_path.parent().unwrap()).unwrap();
    // ruff format normalizes `x=1` -> `x = 1` and guarantees a trailing newline.
    std::fs::write(&py_path, "x=1").unwrap();

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);

    let files: Vec<(Language, Vec<GeneratedFile>)> = vec![(
        Language::Python,
        vec![GeneratedFile {
            path: PathBuf::from("packages/python/foo.py"),
            content: "x=1".to_owned(),
            generated_header: false,
        }],
    )];

    format_generated(&files, &config, base, None);

    let formatted = std::fs::read_to_string(&py_path).unwrap();
    if is_tool_available("poly") {
        assert_eq!(
            formatted, "x = 1\n",
            "with poly installed, `poly fmt --fix` must reformat the generated Python file"
        );
    } else {
        // Best-effort: no poly on PATH means no reformat and no crash.
        assert_eq!(formatted, "x=1", "without poly the file must be left untouched");
    }
}
