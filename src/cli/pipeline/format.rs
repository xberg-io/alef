use crate::core::config::{FormatConfig, Language, ResolvedCrateConfig};
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
/// Formatting is delegated to the `poly` (polylint) CLI as a system dependency:
/// a single `poly fmt --fix` pass over the generated repo formats every language
/// poly supports (Python, JS/TS/JSON, PHP, Ruby, Rust, Go, Markdown, TOML, YAML,
/// CSS, Java, Kotlin, R, Swift, Dart, Gleam, Zig, Shell), collapsing what used to
/// be ~19 per-language formatter shell-outs into one tool. A small set of residual
/// native passes runs afterwards for the project-wide tools poly cannot wrap
/// (`cargo sort`, `mix format`, `dotnet format`).
///
/// Best-effort: a missing `poly` binary, a poly error, or a missing residual tool
/// is logged as a warning and never aborts the generate command.
pub fn format_generated(
    files: &[(Language, Vec<crate::core::backend::GeneratedFile>)],
    config: &ResolvedCrateConfig,
    base_dir: &Path,
    only_languages: Option<&HashSet<Language>>,
) {
    // Deduplicated languages present in this batch, in first-seen order.
    let mut seen = HashSet::new();
    let present: Vec<Language> = files
        .iter()
        .map(|(lang, _)| *lang)
        .filter(|lang| seen.insert(*lang))
        .collect();

    // Languages that should be formatted by poly's default pass. Custom overrides
    // run immediately (they bypass the only_languages filter — an explicit
    // declaration that the formatter must run whenever the language's files are
    // present, so the embedded `alef:hash:` is computed over formatted content).
    let mut poly_langs: Vec<Language> = Vec::new();
    for &lang in &present {
        let lang_str = lang.to_string().to_lowercase();
        let fmt_cfg = effective_format_cfg(config, lang);

        // `[workspace.format] enabled = false` (or a per-language override) skips
        // formatting for that language entirely — including the poly pass.
        if !fmt_cfg.enabled {
            debug!("  [{lang_str}] formatting disabled, skipping");
            continue;
        }

        // Custom command replaces poly for this language and always runs.
        if let Some(custom) = &fmt_cfg.command {
            if let Err(e) = run_custom_formatter(custom, base_dir) {
                warn!("[{lang_str}] custom formatter failed: {e}");
            }
            continue;
        }

        // Default (poly) formatters respect the only_languages filter so warming
        // the cache (no file writes) avoids unnecessary formatting work.
        if let Some(filter) = only_languages
            && !filter.contains(&lang)
        {
            continue;
        }
        poly_langs.push(lang);
    }

    if poly_langs.is_empty() {
        return;
    }

    // Single in-process poly pass. When only_languages is None (full regen) format
    // the whole repo once; when Some (partial regen) scope to just the changed
    // languages' package directories so unchanged languages are not reformatted.
    let paths = poly_paths(config, base_dir, only_languages, &poly_langs);
    poly_format(&paths, base_dir);

    // Residual native passes poly cannot perform.
    for &lang in &poly_langs {
        let lang_str = lang.to_string().to_lowercase();
        for step in language_residuals(config, lang, base_dir) {
            run_residual(&step, &lang_str);
        }
    }
}

/// Resolve the effective format config for a language: a per-language
/// `[workspace.format_overrides.<lang>]` shadows the `[workspace.format]` default.
fn effective_format_cfg(config: &ResolvedCrateConfig, lang: Language) -> &FormatConfig {
    let lang_str = lang.to_string().to_lowercase();
    config.format_overrides.get(&lang_str).unwrap_or(&config.format)
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
/// `cargo sort` for binding crates whose `Cargo.toml` is excluded from the poly
/// pass — a dependency-ordering tool (not a formatter) that ships with cargo and
/// is always present in alef's build environment. Everything else, including
/// Elixir and C#, is formatted by poly's deterministic pure-Rust tier-2 tier
/// (no `mix format` / `dotnet format` system-toolchain dependency).
fn language_residuals(config: &ResolvedCrateConfig, lang: Language, base_dir: &Path) -> Vec<ResidualStep> {
    match lang {
        // The wasm binding crate is often excluded from the root workspace, so
        // `cargo sort -w` never reaches it. Sort its Cargo.toml directly so it is
        // already canonical when its hash is finalised.
        Language::Wasm => {
            let crate_dir = config
                .output_for("wasm")
                .map(resolve_crate_dir)
                .unwrap_or_else(|| Path::new("crates").join(format!("{}-wasm", config.name)));
            // Cargo accepts `/` on every platform; emit POSIX paths for cross-OS parity.
            let crate_dir_str = crate_dir.to_string_lossy().into_owned().replace('\\', "/");
            vec![cargo_sort(vec![crate_dir_str], base_dir.to_path_buf())]
        }
        // Workspace-wide cargo sort normalises every in-workspace binding crate's
        // Cargo.toml (FFI, PyO3, NAPI-RS, Magnus, ext-php-rs, Rustler, wasm-bindgen).
        Language::Ffi => vec![cargo_sort(vec!["-w".to_owned()], base_dir.to_path_buf())],
        // Ruby's native crate lives outside the consumer workspace.
        Language::Ruby => {
            let gem_name = config.ruby_gem_name();
            let native_subdir = format!("ext/{gem_name}/native");
            vec![cargo_sort(vec![native_subdir], base_dir.join("packages/ruby"))]
        }
        // Elixir: cargo sort for the workspace-excluded NIF crate. The `.ex`/
        // `.exs` sources are formatted by poly's tier-2 tier (no `mix format`).
        Language::Elixir => {
            let app_name = config.elixir_app_name();
            let native_subdir = format!("native/{app_name}_nif");
            vec![cargo_sort(vec![native_subdir], base_dir.join("packages/elixir"))]
        }
        // The extendr R crate is workspace-excluded.
        Language::R => vec![cargo_sort(
            vec!["packages/r/src/rust".to_owned()],
            base_dir.to_path_buf(),
        )],
        // C# is formatted by poly's tier-2 tier — no `dotnet format` residual.
        _ => vec![],
    }
}

/// Construct a `cargo sort` residual step.
fn cargo_sort(mut sort_args: Vec<String>, work_dir: PathBuf) -> ResidualStep {
    let mut args = vec!["sort".to_owned()];
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

/// Run a custom formatter command (shell-style string) in a directory.
fn run_custom_formatter(cmd: &str, work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("sh").arg("-c").arg(cmd).current_dir(work_dir).output()?;

    if !output.status.success() {
        debug!("custom formatter output: {}", format_command_output(&output));
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
