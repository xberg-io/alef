use crate::core::config::{Language, ResolvedCrateConfig};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tracing::{debug, warn};

/// One residual formatter invocation poly cannot perform (project-wide tools that
/// don't fit poly's per-file model): `cargo sort`, `mix format`, `dotnet format`.
struct ResidualStep {
    command: String,
    args: Vec<String>,
    work_dir: PathBuf,
}

/// Run language-native formatters on emitted packages after generation.
///
/// Formatting is always delegated to the `poly` (polylint) CLI — a single
/// `poly fmt --fix` pass formats every language poly supports. A fixed set of
/// residual native passes runs afterwards for the project-wide tools poly cannot
/// wrap (`cargo sort -n`, wasm crate sort, ruby/elixir/R native crate sort).
///
/// Best-effort: a missing `poly` binary, a poly error, or a missing residual tool
/// is logged as a warning and never aborts the generate command.
pub fn format_generated(
    files: &[(Language, Vec<crate::core::backend::GeneratedFile>)],
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
) {
    let mut seen = HashSet::new();
    let poly_langs: Vec<Language> = files
        .iter()
        .map(|(lang, _)| *lang)
        .filter(|lang| seen.insert(*lang) && only_languages.is_none_or(|filter| filter.contains(lang)))
        .collect();

    if poly_langs.is_empty() {
        return;
    }

    let paths = poly_paths(config, base_dir, only_languages, &poly_langs);
    poly_format(&paths, base_dir);

    for &lang in &poly_langs {
        let lang_str = lang.to_string().to_lowercase();
        for step in language_residuals(config, lang, base_dir) {
            run_residual(&step, &lang_str);
        }
    }
}

/// Run `poly fmt --fix <base_dir>`. Best-effort: a missing `poly` binary or
/// non-zero exit is logged as a warning and never propagated.
pub fn poly_fmt(base_dir: &Path) {
    let paths = vec![base_dir.to_path_buf()];
    poly_format(&paths, base_dir);
}

/// Run `poly lint <base_dir>`. Propagates failure — a non-zero exit is an error.
pub fn poly_lint(base_dir: &Path) -> anyhow::Result<()> {
    if !is_tool_available("poly") {
        warn!("poly not found on PATH (skipping lint)");
        return Ok(());
    }
    let path_str = base_dir.to_string_lossy().into_owned();
    let arg_refs: Vec<&str> = vec!["lint", &path_str];
    match run_formatter("poly", &arg_refs, base_dir) {
        Ok(()) => {
            debug!("poly lint ok");
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("poly lint failed: {e}")),
    }
}

/// Return the fixed set of all cargo-sort residual steps that alef always runs
/// after formatting, regardless of which languages the config targets.
///
/// The fixed set covers: workspace-wide (via ffi), wasm, ruby, elixir, R.
/// Dart and swift have no cargo residuals (poly covers them).
fn cargo_sort_residuals(config: &ResolvedCrateConfig, base_dir: &Path) -> Vec<ResidualStep> {
    let mut steps = Vec::new();
    steps.extend(language_residuals(config, Language::Ffi, base_dir));
    steps.extend(language_residuals(config, Language::Wasm, base_dir));
    steps.extend(language_residuals(config, Language::Ruby, base_dir));
    steps.extend(language_residuals(config, Language::Elixir, base_dir));
    steps.extend(language_residuals(config, Language::R, base_dir));
    steps
}

/// Run all cargo-sort residuals (ffi workspace, wasm, ruby, elixir, R). Best-effort.
pub(crate) fn run_cargo_sort_residuals(config: &ResolvedCrateConfig, base_dir: &Path) {
    for step in cargo_sort_residuals(config, base_dir) {
        run_residual(&step, "residual");
    }
}

/// Paths to hand to poly. Full regen → the repo root (one pass). Partial regen →
/// the package directory of each changed language (existing dirs only, deduped).
fn poly_paths(
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
    poly_langs: &[Language],
) -> Vec<PathBuf> {
    match only_languages {
        None => vec![base_dir.to_path_buf()],
        Some(_) => {
            let mut seen = HashSet::new();
            let mut dirs = Vec::new();
            for &lang in poly_langs {
                let dir = base_dir.join(config.package_dir(lang));
                if seen.insert(dir.clone()) && dir.exists() {
                    dirs.push(dir);
                }
            }
            dirs
        }
    }
}

/// Format `paths` by invoking the `poly` CLI (`poly fmt --fix`), rewriting changed
/// files in place. `config_start` is poly's working directory; it walks up from
/// there for `poly.toml`. Best-effort: a missing `poly` binary or a non-zero exit
/// is logged and never propagated (matching the per-language formatter contract).
pub(crate) fn poly_format(paths: &[PathBuf], config_start: &Path) {
    if paths.is_empty() {
        return;
    }
    if !is_tool_available("poly") {
        warn!("poly not found on PATH (skipping post-generation formatting)");
        return;
    }
    let mut args: Vec<String> = vec!["fmt".to_owned(), "--fix".to_owned()];
    args.extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_formatter("poly", &arg_refs, config_start) {
        Ok(()) => debug!("poly fmt over {} path(s) ok", paths.len()),
        Err(e) => warn!("poly fmt failed (non-fatal): {e}"),
    }
}

/// Best-effort wiring of poly's git-hook shims (`poly hooks install`) into the
/// generated repo. This installs the pre-commit + commit-msg stages declared in
/// the scaffolded `poly.toml` `[hooks]` section — polylint, polyfmt, file_safety,
/// the `cargo` builtin (clippy / cargo-sort / machete / deny), and the
/// conventional-commit `commit` hook — so every generated repository lints,
/// formats, and validates on commit without any per-repo manual setup.
///
/// No-op when `poly` is absent from PATH or `base_dir` is not a git repository.
/// Idempotent — `poly hooks install` re-writes the same shims, so it is safe to
/// run on every scaffold pass. Never aborts generation.
pub(crate) fn install_poly_hooks(base_dir: &Path) {
    if !base_dir.join(".git").exists() {
        debug!(
            "not a git repository at {}, skipping poly hooks install",
            base_dir.display()
        );
        return;
    }
    if !is_tool_available("poly") {
        warn!("poly not found on PATH (skipping poly hooks install)");
        return;
    }
    match run_formatter("poly", &["hooks", "install"], base_dir) {
        Ok(()) => debug!("poly hooks install ok"),
        Err(e) => warn!("poly hooks install failed (non-fatal): {e}"),
    }
}

/// Build the residual formatter steps for a language. The only residual is
/// `cargo sort -n` for binding crates whose `Cargo.toml` is excluded from the poly
/// pass — a dependency-ordering tool (not a formatter) that ships with cargo and
/// is always present in alef's build environment. Everything else, including
/// Elixir and C#, is formatted by poly's deterministic pure-Rust tier-2 tier
/// (no `mix format` / `dotnet format` system-toolchain dependency).
fn language_residuals(config: &ResolvedCrateConfig, lang: Language, base_dir: &Path) -> Vec<ResidualStep> {
    match lang {
        Language::Wasm => {
            let crate_dir = config
                .output_for("wasm")
                .map(resolve_crate_dir)
                .unwrap_or_else(|| Path::new("crates").join(format!("{}-wasm", config.name)));
            let crate_dir_str = crate_dir.to_string_lossy().into_owned().replace('\\', "/");
            vec![cargo_sort(vec![crate_dir_str], base_dir.to_path_buf())]
        }
        Language::Ffi => vec![cargo_sort(vec!["-w".to_owned()], base_dir.to_path_buf())],
        Language::Ruby => {
            let gem_name = config.ruby_gem_name();
            let native_subdir = format!("ext/{gem_name}/native");
            vec![cargo_sort(vec![native_subdir], base_dir.join("packages/ruby"))]
        }
        Language::Elixir => {
            let app_name = config.elixir_app_name();
            let native_subdir = format!("native/{app_name}_nif");
            vec![cargo_sort(vec![native_subdir], base_dir.join("packages/elixir"))]
        }
        Language::R => vec![cargo_sort(
            vec!["packages/r/src/rust".to_owned()],
            base_dir.to_path_buf(),
        )],
        _ => vec![],
    }
}

/// Construct a `cargo sort -n` residual step. The `-n` flag preserves single-line
/// array formatting, preventing cargo-sort from expanding dependency arrays that
/// alef emits on one line for readability.
fn cargo_sort(mut sort_args: Vec<String>, work_dir: PathBuf) -> ResidualStep {
    let mut args = vec!["sort".to_owned(), "-n".to_owned()];
    args.append(&mut sort_args);
    ResidualStep {
        command: "cargo".to_owned(),
        args,
        work_dir,
    }
}

/// Run a single residual step, best-effort: a missing work dir or tool is a
/// warning/skip, a non-zero exit is a warning. Never aborts generation.
fn run_residual(step: &ResidualStep, lang_str: &str) {
    if !step.work_dir.exists() {
        debug!(
            "  [{lang_str}] residual work dir does not exist: {}, skipping",
            step.work_dir.display()
        );
        return;
    }
    if !is_tool_available(&step.command) {
        warn!("[{lang_str}] residual formatter not found: {} (skipping)", step.command);
        return;
    }
    let args: Vec<&str> = step.args.iter().map(String::as_str).collect();
    match run_formatter(&step.command, &args, &step.work_dir) {
        Ok(()) => debug!("  [{lang_str}] {} {:?} ok", step.command, args),
        Err(e) => warn!("[{lang_str}] {} {:?} failed: {e}", step.command, args),
    }
}

/// Check if a tool is available on PATH.
fn is_tool_available(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run a formatter command with arguments in a specific directory.
fn run_formatter(command: &str, args: &[&str], work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new(command).args(args).current_dir(work_dir).output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "formatter exited with code {:?}: {}",
            output.status.code(),
            format_command_output(&output)
        ));
    }

    Ok(())
}

fn format_command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = stdout.trim();
    let stderr = stderr.trim();

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout:\n{stdout}\nstderr:\n{stderr}"),
        (false, true) => format!("stdout:\n{stdout}"),
        (true, false) => format!("stderr:\n{stderr}"),
        (true, true) => "<no output>".to_string(),
    }
}

fn resolve_crate_dir(output_path: &Path) -> PathBuf {
    output_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| output_path.to_path_buf())
}

#[cfg(test)]
mod tests;
