use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command, run_command_captured};
use crate::cli::registry;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::template_versions as tv;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, info, warn};

pub fn build(config: &ResolvedCrateConfig, languages: &[Language], release: bool) -> anyhow::Result<()> {
    let crate_name = &config.name;
    let base_dir = std::env::current_dir()?;

    let mut independent = Vec::new();
    let mut ffi_dependent = Vec::new();
    let mut need_ffi = false;

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
        if let Some(bc) = backend.build_config_with_config(config) {
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

    if need_ffi
        && !independent
            .iter()
            .any(|(_, bc)| bc.tool == "cargo" && bc.crate_suffix == "-ffi")
    {
        let ffi_crate = output_path_for(Language::Ffi, config)
            .map(resolve_crate_dir)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| Box::leak(format!("{crate_name}-ffi").into_boxed_str()));
        info!("Building FFI crate: {ffi_crate}");
        let mut cmd = format!("cargo build -p {ffi_crate}");
        if release {
            cmd.push_str(" --release");
        }
        run_command(&cmd).context("failed to build FFI crate")?;
    }

    for (lang, _) in &independent {
        let build_cmd_cfg = config.build_command_config_for_language(*lang);
        run_before(*lang, build_cmd_cfg.before.as_ref())?;
    }

    let build_results: Vec<anyhow::Result<(String, String)>> = independent
        .par_iter()
        .map(|(lang, bc)| {
            let build_cmd_cfg = config.build_command_config_for_language(*lang);
            let override_cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = override_cmds {
                if config.build_commands.contains_key(&lang.to_string()) {
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

    for (lang, _) in &ffi_dependent {
        let build_cmd_cfg = config.build_command_config_for_language(*lang);
        run_before(*lang, build_cmd_cfg.before.as_ref())?;
    }

    let build_results: Vec<anyhow::Result<(String, String)>> = ffi_dependent
        .par_iter()
        .map(|(lang, bc)| {
            let build_cmd_cfg = config.build_command_config_for_language(*lang);
            let override_cmds = if release {
                build_cmd_cfg.build_release.as_ref()
            } else {
                build_cmd_cfg.build.as_ref()
            };
            if let Some(cmd_list) = override_cmds {
                if config.build_commands.contains_key(&lang.to_string()) {
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
/// Output paths like `crates/sample-markdown-node/src/` → `crates/sample-markdown-node`.
fn resolve_crate_dir(output_path: &Path) -> &Path {
    if output_path.file_name().is_some_and(|n| n == "src") {
        output_path.parent().unwrap_or(output_path)
    } else {
        output_path
    }
}

/// Get the output path for a language from config.
fn output_path_for(lang: Language, config: &ResolvedCrateConfig) -> Option<&Path> {
    match lang {
        Language::Python => config.explicit_output.python.as_deref(),
        Language::Node => config.explicit_output.node.as_deref(),
        Language::Ruby => config.explicit_output.ruby.as_deref(),
        Language::Php => config.explicit_output.php.as_deref(),
        Language::Ffi => config.explicit_output.ffi.as_deref(),
        Language::Go => config.explicit_output.go.as_deref(),
        Language::Java => config.explicit_output.java.as_deref(),
        Language::Csharp => config.explicit_output.csharp.as_deref(),
        Language::Wasm => config.explicit_output.wasm.as_deref(),
        Language::Elixir => config.explicit_output.elixir.as_deref(),
        Language::R => config.explicit_output.r.as_deref(),
        Language::Rust | Language::C | Language::Jni => None,
        Language::Kotlin
        | Language::KotlinAndroid
        | Language::Swift
        | Language::Dart
        | Language::Gleam
        | Language::Zig => None,
    }
}

/// Generate the shell command to build a specific language.
fn build_command_for(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    release: bool,
) -> String {
    let release_flag = if release { " --release" } else { "" };

    let crate_dir = output_path_for(lang, config)
        .map(resolve_crate_dir)
        .and_then(|p| p.to_str())
        .unwrap_or("");

    match bc.tool {
        "maturin" => {
            format!("maturin develop --manifest-path {crate_dir}/Cargo.toml{release_flag}")
        }
        "napi" => {
            format!(
                "npx --yes -p @napi-rs/cli@{} napi build --platform --manifest-path {}/Cargo.toml -o {}{}",
                tv::npm::NAPI_RS_CLI_CRATE,
                crate_dir,
                crate_dir,
                release_flag
            )
        }
        "wasm-pack" => {
            let profile = if release { "--release" } else { "--dev" };
            format!("wasm-pack build {crate_dir} {profile} --target web")
        }
        "cargo" => {
            if crate_dir.is_empty() && !bc.crate_suffix.is_empty() {
                return format!("cargo build -p {}{}{}", config.name, bc.crate_suffix, release_flag);
            }
            let native_dir = Path::new(crate_dir).join("native");
            let native_manifest = native_dir.join("Cargo.toml");
            if native_manifest.exists() {
                let dir = native_dir.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else if let Some(standalone) = {
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut found: Option<std::path::PathBuf> = None;
                for _ in 0..3 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest) {
                            if contents.contains("[workspace]") {
                                found = Some(p.clone());
                            }
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                found
            } {
                let dir = standalone.display();
                format!("cd {dir} && cargo build{release_flag}")
            } else {
                let mut p = std::path::PathBuf::from(crate_dir);
                let mut package_name: Option<String> = None;
                let mut package_dir: Option<std::path::PathBuf> = None;
                for _ in 0..4 {
                    let manifest = p.join("Cargo.toml");
                    if manifest.exists() {
                        if let Ok(contents) = std::fs::read_to_string(&manifest) {
                            if contents.contains("[package]") {
                                for line in contents.lines() {
                                    let trimmed = line.trim();
                                    if let Some(rest) = trimmed.strip_prefix("name") {
                                        let rest = rest.trim_start_matches([' ', '=']).trim();
                                        let rest = rest.trim_matches(['"', '\'']);
                                        if !rest.is_empty() {
                                            package_name = Some(rest.to_string());
                                            package_dir = Some(p.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                    if !p.pop() {
                        break;
                    }
                }
                let is_excluded_from_workspace = if let Some(pdir) = &package_dir {
                    let mut q = pdir.clone();
                    let mut excluded = false;
                    while q.pop() {
                        let manifest = q.join("Cargo.toml");
                        if manifest.exists() {
                            if let Ok(contents) = std::fs::read_to_string(&manifest) {
                                if contents.contains("[workspace]") {
                                    let rel = pdir.strip_prefix(&q).unwrap_or(pdir).to_string_lossy().into_owned();
                                    let rel_norm = rel.replace('\\', "/");
                                    excluded = contents.lines().map(|l| l.trim()).any(|l| {
                                        l.contains(&format!("\"{rel_norm}\"")) && {
                                            let needle = format!("\"{rel_norm}\"");
                                            let exclude_section = contents.split("exclude").nth(1).unwrap_or("");
                                            let members_section = contents.split("members").nth(1).unwrap_or("");
                                            let in_exclude = exclude_section.contains(&needle);
                                            let in_members =
                                                members_section.contains(&needle) && !exclude_section.contains(&needle);
                                            in_exclude && !in_members
                                        }
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    excluded
                } else {
                    false
                };
                if is_excluded_from_workspace {
                    if let Some(pdir) = package_dir {
                        let dir = pdir.display();
                        format!("cd {dir} && cargo build{release_flag}")
                    } else {
                        format!("cd {crate_dir} && cargo build{release_flag}")
                    }
                } else {
                    let crate_name = package_name.unwrap_or_else(|| {
                        Path::new(crate_dir)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(crate_dir)
                            .to_string()
                    });
                    format!("cargo build -p {crate_name}{release_flag}")
                }
            }
        }
        "mix" => {
            let dir = config
                .explicit_output
                .elixir
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/elixir");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("mix.exs").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            format!("cd {build_dir} && mix compile")
        }
        "mvn" => {
            let dir = config
                .explicit_output
                .java
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/java");
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                loop {
                    if p.join("pom.xml").exists() {
                        break p.to_string_lossy().into_owned();
                    }
                    if !p.pop() {
                        break dir.to_string();
                    }
                }
            };
            format!("cd {build_dir} && mvn package -DskipTests --batch-mode --no-transfer-progress")
        }
        "dotnet" => {
            let dir = config
                .explicit_output
                .csharp
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("packages/csharp");
            let scan_for_csproj = |start: &std::path::Path| -> Option<String> {
                if start
                    .read_dir()
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().extension().is_some_and(|ext| ext == "csproj"))
                    })
                    .unwrap_or(false)
                {
                    return Some(start.to_string_lossy().to_string());
                }
                start.read_dir().ok().and_then(|entries| {
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
            };
            let build_dir = {
                let mut p = std::path::PathBuf::from(dir);
                let mut found = scan_for_csproj(&p);
                while found.is_none() && p.pop() {
                    found = scan_for_csproj(&p);
                }
                found.unwrap_or_else(|| dir.to_string())
            };
            let dotnet_config = if release { "Release" } else { "Debug" };
            format!("cd {build_dir} && dotnet build --configuration {dotnet_config} --verbosity quiet")
        }
        "go" => {
            let dir = config
                .explicit_output
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
pub fn run_post_build(
    lang: Language,
    bc: &crate::core::backend::BuildConfig,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
) -> anyhow::Result<()> {
    use crate::core::backend::PostBuildStep;

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
                    if content.contains(replace) {
                        debug!("Post-build patch target already patched: {}", file_path.display());
                        continue;
                    }
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
                let work_dir = base_dir.join(crate_dir);
                run_run_command(cmd, args, &work_dir)
                    .with_context(|| format!("post-build RunCommand '{cmd}' failed"))?;
            }
            PostBuildStep::PostProcessFile { path, processor } => {
                use crate::core::backend::PostProcessor;
                let file_path = base_dir.join(crate_dir).join(path);
                if file_path.exists() {
                    let content = std::fs::read_to_string(&file_path)
                        .with_context(|| format!("failed to read post-process target {}", file_path.display()))?;
                    let processed = match processor {
                        PostProcessor::FrbDartSealedVariants => {
                            crate::backends::dart::rewrite_frb_sealed_variants(&content)
                        }
                        PostProcessor::FrbDartExcludeFunctions(excluded) => {
                            let exclude_set: std::collections::HashSet<&str> =
                                excluded.iter().map(|s| s.as_str()).collect();
                            crate::backends::dart::filter_excluded_functions(&content, &exclude_set)
                        }
                        PostProcessor::FrbDartOptionalFieldsWithDefaults => {
                            crate::backends::dart::make_struct_fields_with_defaults_optional(&content)
                        }
                        PostProcessor::FrbDartFixHandlerExecutorCalls => {
                            crate::backends::dart::fix_handler_executor_calls(&content)
                        }
                        PostProcessor::FrbDartInjectTextMethods(type_names) => {
                            crate::backends::dart::inject_display_as_text_methods(&content, type_names)
                        }
                        PostProcessor::DartStripTrailingWhitespace => {
                            crate::backends::dart::strip_trailing_whitespace(&content)
                        }
                    };
                    if processed != content {
                        std::fs::write(&file_path, &processed)
                            .with_context(|| format!("failed to write post-processed file {}", file_path.display()))?;
                        info!("PostProcessed {}: {:?}", file_path.display(), processor);
                    } else {
                        debug!(
                            "PostProcessFile {}: no changes (already rewritten or absent variants)",
                            file_path.display()
                        );
                    }
                } else {
                    debug!("PostProcessFile target not found: {}", file_path.display());
                }
            }
            PostBuildStep::StageDartNatives { lib_stem } => {
                let package_root = base_dir.join("packages/dart");
                crate::publish::dart_native::stage_dart_native_libraries(base_dir, &package_root, lib_stem)
                    .with_context(|| format!("failed to stage Dart native libraries for stem '{lib_stem}'"))?;
                info!("Staged native libraries for Dart package from build output (stem: '{lib_stem}')");
            }
            PostBuildStep::MaterializeSwiftBridge {
                binding_crate_name,
                package_root,
            } => {
                let package_root = base_dir.join(package_root);
                let materialized = crate::backends::swift::gen_bindings::bridge_artifacts::emit_swift_bridge_files(
                    "",
                    binding_crate_name,
                    &package_root,
                )
                .with_context(|| format!("failed to re-materialize swift-bridge files for '{binding_crate_name}'"))?;
                if let Some(files) = materialized {
                    for f in files {
                        if let Some(parent) = f.path.parent() {
                            std::fs::create_dir_all(parent)
                                .with_context(|| format!("failed to create directory {}", parent.display()))?;
                        }
                        std::fs::write(&f.path, &f.content)
                            .with_context(|| format!("failed to write {}", f.path.display()))?;
                    }
                }
                info!("Re-materialized swift-bridge files for '{binding_crate_name}' from fresh build output");
            }
        }
    }

    Ok(())
}

/// Hard upper bound on how long a post-build `RunCommand` may run before alef
/// considers it hung and kills it. Cold-cache `cargo build --release` for the
/// swift binding crate against a polyglot project's full feature set
/// legitimately takes 10-20 minutes; FRB codegen on a warm cache finishes in
/// under a minute. 30 minutes accommodates both without false-positiving
/// slow first-runs on cold CI caches.
const RUN_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

/// Interval between `try_wait()` polls. Short enough to react promptly to a
/// finished child, long enough not to burn CPU in a tight loop.
const RUN_COMMAND_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Execute a `RunCommand` post-build step.
///
/// Spawns `cmd` with `args` in `base_dir`, streaming stdout/stderr through
/// alef's own stdio so interactive subprocess progress is visible. Enforces a
/// `RUN_COMMAND_TIMEOUT` ceiling; on timeout the child is SIGKILL'd and the
/// call returns an error. Returns an error on non-zero exit status.
///
/// Escape hatch: the env var `ALEF_SKIP_COMMANDS` accepts a comma-separated
/// list of `cmd` names to skip without running. Useful in environments where
/// a post-build tool is unavailable, hangs (e.g. `flutter_rust_bridge_codegen`
/// installing Flutter via FVM under CI), or simply isn't desired this run.
/// Each skipped command logs a `warn!` so the omission is visible.
fn run_run_command(cmd: &str, args: &[&str], base_dir: &Path) -> anyhow::Result<()> {
    if let Ok(skip_list) = std::env::var("ALEF_SKIP_COMMANDS") {
        if skip_list.split(',').any(|s| s.trim() == cmd) {
            warn!("[{cmd}] skipped via ALEF_SKIP_COMMANDS env var");
            return Ok(());
        }
    }
    let mut child = match std::process::Command::new(cmd)
        .args(args)
        .current_dir(base_dir)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "[{cmd}] not on PATH — skipping post-build step. Install '{cmd}' to regenerate at build time; falling back to committed generated files."
            );
            return Ok(());
        }
        Err(err) => return Err(anyhow::Error::new(err).context(format!("failed to spawn '{cmd}'"))),
    };

    let started_at = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started_at.elapsed() > RUN_COMMAND_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("'{cmd}' exceeded {}s timeout; killed", RUN_COMMAND_TIMEOUT.as_secs());
                }
                std::thread::sleep(RUN_COMMAND_POLL_INTERVAL);
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context(format!("failed to wait for '{cmd}'")));
            }
        }
    };

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        anyhow::bail!("'{cmd}' exited with status {code}");
    }

    Ok(())
}

#[cfg(test)]
mod build_command_tests {
    use super::*;
    use crate::core::backend::{BuildConfig, BuildDependency};

    #[test]
    fn csharp_build_command_uses_verbosity_flag_not_query_mode() {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let config = alef_cfg.resolve().unwrap().remove(0);
        let build_config = BuildConfig {
            tool: "dotnet",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: Vec::new(),
        };

        let command = build_command_for(Language::Csharp, &build_config, &config, false);

        assert!(
            command.contains("--verbosity quiet"),
            "C# build must use explicit quiet verbosity: {command}"
        );
        assert!(
            !command.contains(" -q"),
            "C# build must not use dotnet query mode shorthand: {command}"
        );
    }
}

#[cfg(all(test, unix))]
mod run_command_tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn restore_skip_env(previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var("ALEF_SKIP_COMMANDS", value),
                None => std::env::remove_var("ALEF_SKIP_COMMANDS"),
            }
        }
    }

    #[test]
    fn run_run_command_succeeds_for_echo() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        unsafe {
            std::env::remove_var("ALEF_SKIP_COMMANDS");
        }
        let dir = std::env::temp_dir();
        let result = run_run_command("echo", &["alef-runcommand-ok"], &dir);
        restore_skip_env(previous);
        assert!(result.is_ok(), "echo should succeed: {result:?}");
    }

    #[test]
    fn run_run_command_fails_for_false() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        unsafe {
            std::env::remove_var("ALEF_SKIP_COMMANDS");
        }
        let dir = std::env::temp_dir();
        let result = run_run_command("false", &[], &dir);
        restore_skip_env(previous);
        assert!(result.is_err(), "false should return Err");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("exited with status"),
            "error should mention exit status: {msg}"
        );
    }

    #[test]
    fn run_run_command_honors_skip_env_var() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let previous = std::env::var("ALEF_SKIP_COMMANDS").ok();
        let dir = std::env::temp_dir();
        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "noop,false , another");
        }
        let skipped = run_run_command("false", &[], &dir);
        assert!(
            skipped.is_ok(),
            "listed command must return Ok without spawning: {skipped:?}"
        );

        unsafe {
            std::env::set_var("ALEF_SKIP_COMMANDS", "something-else");
        }
        let honored = run_run_command("false", &[], &dir);
        restore_skip_env(previous);
        assert!(
            honored.is_err(),
            "unlisted command must still spawn and surface failure"
        );
    }
}
