use alef_core::config::output::StringOrVec;
use alef_core::config::{AlefConfig, Language};
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::registry;

use super::helpers::{check_precondition, run_before, run_command, run_command_captured};

/// Which lint phases to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LintPhase {
    Format,
    Check,
    Typecheck,
}

/// Filter languages through precondition and before hooks for lint/fmt.
///
/// Returns the subset of languages whose precondition passed (or was absent).
/// Runs before hooks for each passing language. Fails if any before hook fails.
fn prepare_lint_languages(config: &AlefConfig, languages: &[Language]) -> anyhow::Result<Vec<Language>> {
    let mut ready = Vec::with_capacity(languages.len());
    for &lang in languages {
        let lang_lint = config.lint_config_for_language(lang);
        if !check_precondition(lang, lang_lint.precondition.as_deref()) {
            continue;
        }
        run_before(lang, lang_lint.before.as_ref())?;
        ready.push(lang);
    }
    Ok(ready)
}

/// Run a single lint phase across all languages in parallel.
fn run_phase(config: &AlefConfig, languages: &[Language], phase: LintPhase) -> anyhow::Result<()> {
    let tasks: Vec<(&Language, String)> = languages
        .iter()
        .filter_map(|lang| {
            let lang_lint = config.lint_config_for_language(*lang);
            let cmds = match phase {
                LintPhase::Format => lang_lint.format,
                LintPhase::Check => lang_lint.check,
                LintPhase::Typecheck => lang_lint.typecheck,
            };
            cmds.map(|cmd_list| {
                cmd_list
                    .commands()
                    .into_iter()
                    .map(|c| (lang, c.to_string()))
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();

    let results: Vec<anyhow::Result<(String, String, String)>> = tasks
        .par_iter()
        .map(|(_, cmd)| {
            let (stdout, stderr) = run_command_captured(cmd)?;
            Ok((cmd.clone(), stdout, stderr))
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for result in results {
        match result {
            Ok((cmd, stdout, stderr)) => {
                if !stdout.is_empty() {
                    info!("[{cmd}] stdout:\n{stdout}");
                }
                if !stderr.is_empty() {
                    info!("[{cmd}] stderr:\n{stderr}");
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Run all configured lint/format commands on generated output.
///
/// Executes in two waves for correctness:
/// 1. Format (all languages in parallel) — normalizes code first
/// 2. Check + Typecheck (all languages in parallel) — validates normalized code
pub fn lint(config: &AlefConfig, languages: &[Language]) -> anyhow::Result<()> {
    let ready = prepare_lint_languages(config, languages)?;
    // Wave 1: format all languages in parallel
    run_phase(config, &ready, LintPhase::Format)?;
    // Wave 2: check + typecheck all languages in parallel
    run_phase(config, &ready, LintPhase::Check)?;
    run_phase(config, &ready, LintPhase::Typecheck)?;
    Ok(())
}

/// Run only format commands on generated output.
pub fn fmt(config: &AlefConfig, languages: &[Language]) -> anyhow::Result<()> {
    let ready = prepare_lint_languages(config, languages)?;
    run_phase(config, &ready, LintPhase::Format)
}

/// Run format commands as part of post-generation, never propagating failure.
///
/// Formatting after `alef generate` is best-effort: formatters are *expected*
/// to modify generated files (that's their purpose), and a missing formatter,
/// a config issue, or a non-zero exit must not abort the generate run. Each
/// per-language failure is logged as a warning and processing continues with
/// the next language.
pub fn fmt_post_generate(config: &AlefConfig, languages: &[Language]) {
    for &lang in languages {
        let lang_lint = config.lint_config_for_language(lang);
        if !check_precondition(lang, lang_lint.precondition.as_deref()) {
            // `check_precondition` already warns and returns false on miss.
            continue;
        }
        if let Err(e) = run_before(lang, lang_lint.before.as_ref()) {
            warn!("[{lang}] post-generation `before` hook failed: {e:#}");
            continue;
        }
        let Some(cmd_list) = lang_lint.format else {
            continue;
        };
        for cmd in cmd_list.commands() {
            match run_command_captured(cmd) {
                Ok((stdout, stderr)) => {
                    if !stdout.is_empty() {
                        info!("[{cmd}] stdout:\n{stdout}");
                    }
                    if !stderr.is_empty() {
                        info!("[{cmd}] stderr:\n{stderr}");
                    }
                }
                Err(e) => {
                    warn!("[{lang}] post-generation format command failed (continuing): {e:#}");
                }
            }
        }
    }
}

/// Deduplicate command strings across language plans, preserving within-language order.
///
/// The first language claiming a command owns it; subsequent languages with the same
/// command string get it removed from their list. This prevents races when multiple
/// languages (e.g. Node and Wasm) share workspace-wide commands like `pnpm up --latest -r -w`.
fn dedupe_plans(plans: Vec<(Language, Vec<String>)>) -> Vec<(Language, Vec<String>)> {
    let mut seen = std::collections::HashSet::<String>::new();
    plans
        .into_iter()
        .map(|(lang, cmds)| {
            let unique: Vec<String> = cmds.into_iter().filter(|c| seen.insert(c.clone())).collect();
            (lang, unique)
        })
        .collect()
}

/// Update dependencies for each language.
///
/// When `latest` is true, runs the aggressive `upgrade` commands (including
/// incompatible/major version bumps). Otherwise runs the safe `update` commands.
///
/// Executes in two phases:
/// 1. Sequential: check preconditions and collect command lists; deduplicate across languages.
/// 2. Parallel: run each language's deduped command list (within-language order preserved).
pub fn update(config: &AlefConfig, languages: &[Language], latest: bool) -> anyhow::Result<()> {
    // Phase 1 (sequential): check preconditions, run before hooks, collect command lists.
    let mut plans: Vec<(Language, Vec<String>)> = Vec::new();
    for &lang in languages {
        let update_cfg = config.update_config_for_language(lang);
        if !check_precondition(lang, update_cfg.precondition.as_deref()) {
            continue;
        }
        run_before(lang, update_cfg.before.as_ref())?;
        let cmds = if latest {
            update_cfg.upgrade.as_ref()
        } else {
            update_cfg.update.as_ref()
        };
        let cmd_strings: Vec<String> = cmds
            .map(|cmd_list| cmd_list.commands().into_iter().map(|c| c.to_string()).collect())
            .unwrap_or_default();
        plans.push((lang, cmd_strings));
    }

    // Deduplicate shared commands (e.g. pnpm workspace commands shared by Node and Wasm).
    let plans = dedupe_plans(plans);

    type LangOutputs = Vec<(String, String, String)>;

    // Phase 2 (parallel): run each language's deduped command list.
    let results: Vec<(Language, anyhow::Result<LangOutputs>)> = plans
        .par_iter()
        .map(|(lang, cmds)| {
            let mut outputs = Vec::new();
            for cmd in cmds {
                match run_command_captured(cmd) {
                    Ok((stdout, stderr)) => outputs.push((cmd.clone(), stdout, stderr)),
                    Err(e) => return (*lang, Err(e)),
                }
            }
            (*lang, Ok(outputs))
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (_lang, result) in results {
        match result {
            Ok(outputs) => {
                for (cmd, stdout, stderr) in outputs {
                    if !stdout.is_empty() {
                        info!("[{cmd}] stdout:\n{stdout}");
                    }
                    if !stderr.is_empty() {
                        info!("[{cmd}] stderr:\n{stderr}");
                    }
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Run configured test commands for each language.
///
/// When `coverage` is true, runs coverage commands instead of regular test commands.
/// When `e2e` is true, also runs e2e test commands.
pub fn test(config: &AlefConfig, languages: &[Language], e2e: bool, coverage: bool) -> anyhow::Result<()> {
    let results: Vec<anyhow::Result<Vec<(String, String, String)>>> = languages
        .par_iter()
        .map(|lang| {
            let lang_test = config.test_config_for_language(*lang);
            if !check_precondition(*lang, lang_test.precondition.as_deref()) {
                return Ok(Vec::new());
            }
            run_before(*lang, lang_test.before.as_ref())?;
            let mut outputs = Vec::new();

            // Use coverage commands when --coverage flag is set, fall back to regular test
            let test_cmds = if coverage {
                lang_test.coverage.as_ref().or(lang_test.command.as_ref())
            } else {
                lang_test.command.as_ref()
            };

            if let Some(cmd_list) = test_cmds {
                for cmd in cmd_list.commands() {
                    let (stdout, stderr) = run_command_captured(cmd)?;
                    outputs.push((cmd.to_string(), stdout, stderr));
                }
            }
            if e2e {
                if let Some(e2e_cmd_list) = &lang_test.e2e {
                    for cmd in e2e_cmd_list.commands() {
                        let (stdout, stderr) = run_command_captured(cmd)?;
                        outputs.push((cmd.to_string(), stdout, stderr));
                    }
                }
            }

            Ok(outputs)
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for result in results {
        match result {
            Ok(outputs) => {
                for (cmd, stdout, stderr) in outputs {
                    if !stdout.is_empty() {
                        info!("[{cmd}] stdout:\n{stdout}");
                    }
                    if !stderr.is_empty() {
                        info!("[{cmd}] stderr:\n{stderr}");
                    }
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Install dependencies for each language.
///
/// If `timeout_override` is Some, all languages use that timeout; otherwise each
/// language uses its configured `timeout_seconds` (defaulting to 600 seconds).
#[allow(clippy::type_complexity)]
pub fn setup(config: &AlefConfig, languages: &[Language], timeout_override: Option<u64>) -> anyhow::Result<()> {
    let results: Vec<(Language, anyhow::Result<Vec<(String, String, String)>>)> = languages
        .par_iter()
        .map(|lang| {
            let setup_cfg = config.setup_config_for_language(*lang);
            let timeout_secs = timeout_override.unwrap_or(setup_cfg.timeout_seconds);

            let result: anyhow::Result<Vec<(String, String, String)>> =
                if !check_precondition(*lang, setup_cfg.precondition.as_deref()) {
                    Ok(Vec::new())
                } else {
                    (|| {
                        run_before_with_timeout(*lang, setup_cfg.before.as_ref(), timeout_secs)?;
                        let mut outputs = Vec::new();
                        if let Some(cmd_list) = &setup_cfg.install {
                            for cmd in cmd_list.commands() {
                                let (stdout, stderr) =
                                    super::helpers::run_command_captured_with_timeout(cmd, Some(timeout_secs))
                                        .with_context(|| format!("setup for {lang} timed out after {timeout_secs}s"))?;
                                outputs.push((cmd.to_string(), stdout, stderr));
                            }
                        }
                        Ok(outputs)
                    })()
                };
            (*lang, result)
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (lang, result) in results {
        match result {
            Ok(outputs) => {
                for (cmd, stdout, stderr) in outputs {
                    if !stdout.is_empty() {
                        info!("[{cmd}] stdout:\n{stdout}");
                    }
                    if !stderr.is_empty() {
                        info!("[{cmd}] stderr:\n{stderr}");
                    }
                }
            }
            Err(e) => {
                warn!("Setup failed for {lang}: {e}");
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Run before-hook commands with timeout consideration. Returns `Ok(())` on success.
fn run_before_with_timeout(lang: Language, before: Option<&StringOrVec>, timeout_secs: u64) -> anyhow::Result<()> {
    let Some(cmds) = before else {
        return Ok(());
    };
    for cmd in cmds.commands() {
        info!("Running before hook for {lang}: {cmd}");
        let (stdout, stderr) = super::helpers::run_command_captured_with_timeout(cmd, Some(timeout_secs))
            .with_context(|| format!("before hook timed out for {lang} after {timeout_secs}s: {cmd}"))?;
        if !stdout.is_empty() {
            info!("[{lang} before] {stdout}");
        }
        if !stderr.is_empty() {
            info!("[{lang} before] {stderr}");
        }
    }
    Ok(())
}

/// Clean build artifacts for each language.
pub fn clean(config: &AlefConfig, languages: &[Language]) -> anyhow::Result<()> {
    let results: Vec<anyhow::Result<Vec<(String, String, String)>>> = languages
        .par_iter()
        .map(|lang| {
            let clean_cfg = config.clean_config_for_language(*lang);
            if !check_precondition(*lang, clean_cfg.precondition.as_deref()) {
                return Ok(Vec::new());
            }
            run_before(*lang, clean_cfg.before.as_ref())?;
            let mut outputs = Vec::new();
            if let Some(cmd_list) = &clean_cfg.clean {
                for cmd in cmd_list.commands() {
                    let (stdout, stderr) = run_command_captured(cmd)?;
                    outputs.push((cmd.to_string(), stdout, stderr));
                }
            }
            Ok(outputs)
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for result in results {
        match result {
            Ok(outputs) => {
                for (cmd, stdout, stderr) in outputs {
                    if !stdout.is_empty() {
                        info!("[{cmd}] stdout:\n{stdout}");
                    }
                    if !stderr.is_empty() {
                        info!("[{cmd}] stderr:\n{stderr}");
                    }
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

/// Build language bindings using configured build commands.
///
/// Uses configurable per-language build commands from `[build_commands.<lang>]`
/// in alef.toml, falling back to sensible defaults. Resolves build order
/// (FFI-dependent languages build after FFI).
pub fn build(config: &AlefConfig, languages: &[Language], release: bool) -> anyhow::Result<()> {
    let crate_name = &config.crate_config.name;
    let base_dir = std::env::current_dir()?;

    // Split into FFI-independent and FFI-dependent languages
    let mut independent = Vec::new();
    let mut ffi_dependent = Vec::new();
    let mut need_ffi = false;

    // Rust is handled via configurable build commands, not the registry
    let mut rust_langs: Vec<Language> = Vec::new();

    for &lang in languages {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        if !check_precondition(lang, build_cmd_cfg.precondition.as_deref()) {
            continue;
        }
        if lang == Language::Rust {
            rust_langs.push(lang);
            continue;
        }
        let backend = registry::get_backend(lang);
        if let Some(bc) = backend.build_config() {
            if bc.depends_on_ffi() {
                ffi_dependent.push((lang, bc));
                need_ffi = true;
            } else {
                independent.push((lang, bc));
            }
        } else {
            info!("No build config for {lang}, skipping");
        }
    }

    // Build Rust first (other bindings may depend on it)
    for &lang in &rust_langs {
        let build_cmd_cfg = config.build_command_config_for_language(lang);
        run_before(lang, build_cmd_cfg.before.as_ref())?;
        let cmds = if release {
            build_cmd_cfg.build_release.as_ref()
        } else {
            build_cmd_cfg.build.as_ref()
        };
        if let Some(cmd_list) = cmds {
            for cmd in cmd_list.commands() {
                info!("Building {lang}: {cmd}");
                run_command(cmd).with_context(|| format!("failed to build {lang}"))?;
            }
        }
    }

    // Build FFI first if needed by dependent languages
    if need_ffi
        && !independent
            .iter()
            .any(|(_, bc)| bc.tool == "cargo" && bc.crate_suffix == "-ffi")
    {
        // Resolve FFI crate name from output path
        let ffi_crate = output_path_for(Language::Ffi, config)
            .map(resolve_crate_dir)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| {
                // Fallback: construct from crate name
                Box::leak(format!("{crate_name}-ffi").into_boxed_str())
            });
        info!("Building FFI crate: {ffi_crate}");
        let mut cmd = format!("cargo build -p {ffi_crate}");
        if release {
            cmd.push_str(" --release");
        }
        run_command(&cmd).context("failed to build FFI crate")?;
    }

    // Run before hooks for independent languages (sequentially — they may have side effects)
    for (lang, _) in &independent {
        let build_cmd_cfg = config.build_command_config_for_language(*lang);
        run_before(*lang, build_cmd_cfg.before.as_ref())?;
    }

    // Build independent languages in parallel
    let build_results: Vec<anyhow::Result<(String, String)>> = independent
        .par_iter()
        .map(|(lang, bc)| {
            // Check for explicit build_commands override before using backend default
            let build_cmd_cfg = config.build_command_config_for_language(*lang);
            let override_cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = override_cmds {
                // Use the user-provided build_commands override if the override differs from defaults
                if config
                    .build_commands
                    .as_ref()
                    .and_then(|m| m.get(&lang.to_string()))
                    .is_some()
                {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured(cmd)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
            }
            info!("Building {lang} ({})...", bc.tool);
            let build_cmd = build_command_for(*lang, bc, config, release);
            run_command_captured(&build_cmd).with_context(|| format!("failed to build language bindings for {lang}"))
        })
        .collect();

    for ((lang, bc), result) in independent.iter().zip(build_results) {
        let (stdout, stderr) = result?;
        if !stdout.is_empty() {
            info!("[{lang} build] {stdout}");
        }
        if !stderr.is_empty() {
            debug!("[{lang} build] {stderr}");
        }
        run_post_build(*lang, bc, config, &base_dir)
            .with_context(|| format!("failed to run post-build steps for {lang}"))?;
    }

    // Run before hooks for FFI-dependent languages
    for (lang, _) in &ffi_dependent {
        let build_cmd_cfg = config.build_command_config_for_language(*lang);
        run_before(*lang, build_cmd_cfg.before.as_ref())?;
    }

    // Build FFI-dependent languages in parallel
    let build_results: Vec<anyhow::Result<(String, String)>> = ffi_dependent
        .par_iter()
        .map(|(lang, bc)| {
            // Check for explicit build_commands override before using backend default
            let build_cmd_cfg = config.build_command_config_for_language(*lang);
            let override_cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = override_cmds {
                if config
                    .build_commands
                    .as_ref()
                    .and_then(|m| m.get(&lang.to_string()))
                    .is_some()
                {
                    let mut combined_output = (String::new(), String::new());
                    for cmd in cmd_list.commands() {
                        info!("Building {lang}: {cmd}");
                        let (stdout, stderr) = run_command_captured(cmd)
                            .with_context(|| format!("failed to build language bindings for {lang}"))?;
                        combined_output.0.push_str(&stdout);
                        combined_output.1.push_str(&stderr);
                    }
                    return Ok(combined_output);
                }
            }
            info!("Building {lang} ({})...", bc.tool);
            let build_cmd = build_command_for(*lang, bc, config, release);
            run_command_captured(&build_cmd).with_context(|| format!("failed to build language bindings for {lang}"))
        })
        .collect();

    for ((lang, bc), result) in ffi_dependent.iter().zip(build_results) {
        let (stdout, stderr) = result?;
        if !stdout.is_empty() {
            info!("[{lang} build] {stdout}");
        }
        if !stderr.is_empty() {
            debug!("[{lang} build] {stderr}");
        }
        run_post_build(*lang, bc, config, &base_dir)
            .with_context(|| format!("failed to run post-build steps for {lang}"))?;
    }

    Ok(())
}

/// Resolve the crate directory from the output config path.
/// Output paths like `crates/html-to-markdown-node/src/` → `crates/html-to-markdown-node`.
fn resolve_crate_dir(output_path: &Path) -> &Path {
    // If path ends in src/ or src, go up one level
    if output_path.file_name().is_some_and(|n| n == "src") {
        output_path.parent().unwrap_or(output_path)
    } else {
        output_path
    }
}

/// Get the output path for a language from config.
fn output_path_for(lang: Language, config: &AlefConfig) -> Option<&Path> {
    match lang {
        Language::Python => config.output.python.as_deref(),
        Language::Node => config.output.node.as_deref(),
        Language::Ruby => config.output.ruby.as_deref(),
        Language::Php => config.output.php.as_deref(),
        Language::Ffi => config.output.ffi.as_deref(),
        Language::Go => config.output.go.as_deref(),
        Language::Java => config.output.java.as_deref(),
        Language::Csharp => config.output.csharp.as_deref(),
        Language::Wasm => config.output.wasm.as_deref(),
        Language::Elixir => config.output.elixir.as_deref(),
        Language::R => config.output.r.as_deref(),
        // Rust is the core language — no separate output path
        Language::Rust => None,
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => None,
    }
}

/// Generate the shell command to build a specific language.
fn build_command_for(
    lang: Language,
    bc: &alef_core::backend::BuildConfig,
    config: &AlefConfig,
    release: bool,
) -> String {
    let release_flag = if release { " --release" } else { "" };

    // Resolve the crate directory from the output path
    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .and_then(|p| p.to_str())
        .unwrap_or("");

    match bc.tool {
        "maturin" => {
            format!("maturin develop --manifest-path {crate_dir}/Cargo.toml{release_flag}")
        }
        "napi" => {
            // NAPI outputs .node + .d.ts to the crate directory
            format!("napi build --platform --manifest-path {crate_dir}/Cargo.toml -o {crate_dir}{release_flag}")
        }
        "wasm-pack" => {
            let profile = if release { "--release" } else { "--dev" };
            format!("wasm-pack build {crate_dir} {profile} --target bundler")
        }
        "cargo" => {
            // Check for a standalone crate directory (e.g., Ruby's native/ subdir)
            // that is excluded from the workspace and must be built via --manifest-path.
            let native_dir = Path::new(crate_dir).join("native");
            let native_manifest = native_dir.join("Cargo.toml");
            if native_manifest.exists() {
                let dir = native_dir.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else {
                // Extract crate name from directory name for -p flag
                let crate_name = Path::new(crate_dir)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(crate_dir);
                format!("cargo build -p {crate_name}{release_flag}")
            }
        }
        "mix" => "mix compile".to_string(),
        "mvn" => {
            let dir = config
                .output
                .java
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/java");
            format!("cd {dir} && mvn package -DskipTests -q")
        }
        "dotnet" => {
            let dir = config
                .output
                .csharp
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/csharp");
            // Find the directory containing the .csproj (may be in a subdirectory)
            let build_dir = {
                let dir_path = std::path::Path::new(dir);
                // Check if .csproj exists directly in dir
                let has_direct = dir_path
                    .read_dir()
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().extension().is_some_and(|ext| ext == "csproj"))
                    })
                    .unwrap_or(false);
                if has_direct {
                    dir.to_string()
                } else {
                    // Search one level of subdirectories
                    dir_path
                        .read_dir()
                        .ok()
                        .and_then(|entries| {
                            entries
                                .filter_map(|e| e.ok())
                                .find(|e| {
                                    e.path().is_dir()
                                        && e.path().read_dir().ok().is_some_and(|sub| {
                                            sub.filter_map(|s| s.ok())
                                                .any(|s| s.path().extension().is_some_and(|ext| ext == "csproj"))
                                        })
                                })
                                .map(|e| e.path().to_string_lossy().to_string())
                        })
                        .unwrap_or_else(|| dir.to_string())
                }
            };
            let dotnet_config = if release { "Release" } else { "Debug" };
            format!("cd {build_dir} && dotnet build --configuration {dotnet_config} -q")
        }
        "go" => {
            let dir = config
                .output
                .go
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/go");
            format!("cd {dir} && go build ./...")
        }
        other => format!("echo 'Unknown build tool: {other}'"),
    }
}

/// Run post-build processing steps (e.g., patching .d.ts files).
fn run_post_build(
    lang: Language,
    bc: &alef_core::backend::BuildConfig,
    config: &AlefConfig,
    base_dir: &Path,
) -> anyhow::Result<()> {
    use alef_core::backend::PostBuildStep;

    // Resolve the crate directory from the output path
    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .unwrap_or(Path::new(""));

    for step in &bc.post_build {
        match step {
            PostBuildStep::PatchFile { path, find, replace } => {
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-build patch target {}", file_path.display()))?;
                    let patched = content.replace(find, replace);
                    if patched != content {
                        std::fs::write(&file_path, &patched)
                            .with_context(|| format!("failed to write patched file {}", file_path.display()))?;
                        info!("Patched {}: replaced '{}' → '{}'", file_path.display(), find, replace);
                    }
                } else {
                    debug!("Post-build patch target not found: {}", file_path.display());
                }
            }
            PostBuildStep::RunCommand { cmd, args } => {
                // TODO: Wire up post-build command execution for Dart flutter_rust_bridge codegen.
                // For now, this is unimplemented() to allow Phase 0 to compile.
                debug!("Post-build command {} not yet executed: {:?}", cmd, args);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    #[test]
    fn dedupe_plans_removes_duplicate_commands_across_languages() {
        let plans = vec![
            (
                Language::Node,
                vec![
                    "corepack use pnpm@latest".to_string(),
                    "pnpm up --latest -r -w".to_string(),
                ],
            ),
            (
                Language::Wasm,
                vec![
                    "corepack use pnpm@latest".to_string(),
                    "pnpm up --latest -r -w".to_string(),
                ],
            ),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["corepack use pnpm@latest", "pnpm up --latest -r -w"]);
        assert!(result[1].1.is_empty(), "Wasm should have no commands after dedupe");
    }

    #[test]
    fn dedupe_plans_preserves_within_language_order() {
        let plans = vec![
            (
                Language::Rust,
                vec!["cargo upgrade --incompatible".to_string(), "cargo update".to_string()],
            ),
            (
                Language::Node,
                vec!["cargo update".to_string()], // hypothetical duplicate
            ),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["cargo upgrade --incompatible", "cargo update"]);
        assert!(result[1].1.is_empty());
    }

    #[test]
    fn dedupe_plans_unique_commands_unchanged() {
        let plans = vec![
            (Language::Python, vec!["uv sync --upgrade".to_string()]),
            (Language::Ruby, vec!["bundle update --all".to_string()]),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["uv sync --upgrade"]);
        assert_eq!(result[1].1, vec!["bundle update --all"]);
    }
}

#[cfg(all(test, unix))]
mod fmt_post_generate_tests {
    // Tests in this module rely on `sh -c` and the POSIX `true`/`false`
    // builtins. They are skipped on Windows where `sh` is not on PATH and
    // a missing-program error is indistinguishable from a precondition
    // miss (both cause `check_precondition` to return false), which would
    // make every test trivially pass for the wrong reason.
    use super::*;
    use alef_core::config::output::{LintConfig, StringOrVec};
    use std::collections::HashMap;

    fn config_with_lint(lang: Language, cfg: LintConfig) -> AlefConfig {
        let mut c: AlefConfig = toml::from_str(
            r#"
languages = ["python"]
[crate]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let mut map = HashMap::new();
        map.insert(lang.to_string(), cfg);
        c.lint = Some(map);
        c
    }

    #[test]
    fn fmt_post_generate_swallows_failing_format_command() {
        // A format command that exits non-zero must not panic or propagate;
        // fmt_post_generate has no return value, so reaching the end is the
        // contract.
        let lint = LintConfig {
            precondition: Some("true".to_string()),
            before: None,
            format: Some(StringOrVec::Single("false".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        fmt_post_generate(&cfg, &[Language::Python]);
    }

    #[test]
    fn fmt_post_generate_swallows_failing_before_hook() {
        let lint = LintConfig {
            precondition: Some("true".to_string()),
            before: Some(StringOrVec::Single("false".to_string())),
            format: Some(StringOrVec::Single("true".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        fmt_post_generate(&cfg, &[Language::Python]);
    }

    #[test]
    fn fmt_post_generate_skips_when_precondition_fails() {
        let lint = LintConfig {
            precondition: Some("false".to_string()),
            before: None,
            format: Some(StringOrVec::Single("false".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        // Precondition is `false` → format step never runs, so the (otherwise
        // failing) `false` command isn't invoked. No panic, no propagation.
        fmt_post_generate(&cfg, &[Language::Python]);
    }
}
