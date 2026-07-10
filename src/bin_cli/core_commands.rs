use anyhow::Result;
use std::path::PathBuf;
use std::process;

use crate::cli::{cache, dispatch, pipeline, version_pin};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Extract { output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let effective_output = if multi {
                    output
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join(format!("{}.ir.json", resolved_cfg.name))
                } else {
                    output.clone()
                };
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                if let Some(parent) = effective_output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&effective_output, serde_json::to_string_pretty(&api)?)?;
                if multi {
                    eprintln!("[{}] Wrote IR to {}", resolved_cfg.name, effective_output.display());
                } else {
                    println!("Wrote IR to {}", effective_output.display());
                }
            }
            Ok(None)
        }
        Commands::Generate {
            lang,
            clean,
            format: _format,
            skip_frb,
        } => {
            if skip_frb {
                let existing = std::env::var("ALEF_SKIP_COMMANDS").unwrap_or_default();
                let updated = if existing.is_empty() {
                    "flutter_rust_bridge_codegen".to_string()
                } else {
                    format!("{existing},flutter_rust_bridge_codegen")
                };
                // SAFETY: single-threaded CLI dispatch; no concurrent env access here.
                unsafe { std::env::set_var("ALEF_SKIP_COMMANDS", updated) };
            }
            let _ = skip_frb;
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                tracing::warn!("could not update alef.toml version pin: {e}");
            }

            let mut grand_total_written: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    eprintln!(
                        "[{}] Generating bindings for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Generating bindings for: {}", format_languages(&languages));
                }
                let api = pipeline::extract(resolved_cfg, config_path, clean)?;
                let files = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let mut current_gen_paths = std::collections::HashSet::new();
                let mut changed_languages: std::collections::HashSet<crate::core::config::Language> =
                    std::collections::HashSet::new();

                let mut total_written: usize = 0;
                let mut any_written = false;
                for (lang, lang_files) in &files {
                    let lang_str = lang.to_string();
                    for file in lang_files {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }

                    let hashes: Vec<(String, String)> = lang_files
                        .iter()
                        .map(|f| {
                            let normalized = pipeline::normalize_content(&f.path, &f.content);
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&normalized),
                            )
                        })
                        .collect();

                    let cache_key = format!("{}.{lang_str}", resolved_cfg.name);
                    let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                    let cache_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                    if cache_match && !clean && generated_files_match_disk(lang_files, &base_dir) {
                        eprintln!("  [{lang_str}] up to date (skipping)");
                        continue;
                    }

                    let single = vec![(*lang, lang_files.clone())];
                    let written = pipeline::write_files(&single, &base_dir)?;
                    total_written += written;
                    any_written = true;
                    changed_languages.insert(*lang);
                    let _ = cache::write_generation_hashes(&cache_key, &hashes);
                }

                if !api.services.is_empty() {
                    let svc_files = pipeline::generate_service_api(&api, resolved_cfg, &languages)?;
                    if !svc_files.is_empty() {
                        for (_, files) in &svc_files {
                            for file in files {
                                current_gen_paths.insert(base_dir.join(&file.path));
                            }
                        }
                        let svc_count = pipeline::write_files(&svc_files, &base_dir)?;
                        eprintln!("Generated {svc_count} service API files");
                        any_written = true;
                        for (lang, _) in &svc_files {
                            changed_languages.insert(*lang);
                        }
                    }
                }

                if resolved_cfg.generate.public_api {
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages, config_path)?;
                    if !public_api_files.is_empty() {
                        let api_hashes: Vec<(String, String)> = public_api_files
                            .iter()
                            .flat_map(|(_, fs)| {
                                fs.iter().map(|f| {
                                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                                    (
                                        base_dir.join(&f.path).display().to_string(),
                                        cache::hash_content(&normalized),
                                    )
                                })
                            })
                            .collect();
                        let api_cache_key = format!("{}.public_api", resolved_cfg.name);
                        let stored_api = cache::read_generation_hashes(&api_cache_key).unwrap_or_default();
                        let api_match =
                            !api_hashes.is_empty() && api_hashes.iter().all(|(p, h)| stored_api.get(p) == Some(h));

                        for (_, files) in &public_api_files {
                            for file in files {
                                current_gen_paths.insert(base_dir.join(&file.path));
                            }
                        }

                        if !api_match || clean {
                            let api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                            eprintln!("Generated {api_count} public API files");
                            any_written = true;
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                            for (lang, _) in &public_api_files {
                                changed_languages.insert(*lang);
                            }
                        } else {
                            eprintln!("  [public_api] up to date (skipping)");
                        }
                    }
                }

                let stub_files = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                if !stub_files.is_empty() {
                    let stub_hashes: Vec<(String, String)> = stub_files
                        .iter()
                        .flat_map(|(_, fs)| {
                            fs.iter().map(|f| {
                                (
                                    base_dir.join(&f.path).display().to_string(),
                                    cache::hash_content(&f.content),
                                )
                            })
                        })
                        .collect();

                    let stubs_cache_key = format!("{}.stubs", resolved_cfg.name);
                    let stored_stubs = cache::read_generation_hashes(&stubs_cache_key).unwrap_or_default();
                    let stubs_match =
                        !stub_hashes.is_empty() && stub_hashes.iter().all(|(p, h)| stored_stubs.get(p) == Some(h));

                    for (_, files) in &stub_files {
                        for file in files {
                            current_gen_paths.insert(base_dir.join(&file.path));
                        }
                    }

                    if !stubs_match || clean {
                        let stub_count = pipeline::write_files(&stub_files, &base_dir)?;
                        eprintln!("Generated {stub_count} type stub files");
                        any_written = true;
                        let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);

                        for (lang, _) in &stub_files {
                            changed_languages.insert(*lang);
                        }
                    } else {
                        eprintln!("  [stubs] up to date (skipping)");
                    }
                }

                match pipeline::scaffold(&api, resolved_cfg, &languages, config_path) {
                    Ok(scaffold_files) => {
                        for file in &scaffold_files {
                            current_gen_paths.insert(base_dir.join(&file.path));
                        }
                    }
                    Err(err) => {
                        eprintln!("warning: failed to enumerate scaffold paths for cleanup safety: {err}");
                    }
                }

                if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                    if removed > 0 {
                        eprintln!("Removed {removed} stale alef-generated file(s)");
                    }
                }

                {
                    let mut sweep_roots: std::collections::HashSet<std::path::PathBuf> =
                        std::collections::HashSet::new();
                    for &lang in &languages {
                        let pkg = base_dir.join(resolved_cfg.package_dir(lang));
                        sweep_roots.insert(pkg);
                        if let Some(out) = resolved_cfg.output_for(&lang.to_string()) {
                            sweep_roots.insert(base_dir.join(out));
                        }
                    }
                    sweep_roots.insert(base_dir.join("packages/wasm"));
                    sweep_roots.insert(base_dir.join("packages/typescript"));
                    let roots: Vec<std::path::PathBuf> = sweep_roots.into_iter().filter(|d| d.exists()).collect();
                    if let Ok(removed) = pipeline::sweep_orphans(&roots, &current_gen_paths) {
                        if removed > 0 {
                            eprintln!("Removed {removed} stale alef-generated file(s)");
                        }
                    }
                }

                if any_written && !changed_languages.is_empty() {
                    eprintln!("Formatting generated files...");
                    let mut files_to_format = files.clone();
                    files_to_format.extend(stub_files.clone());
                    pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, Some(&changed_languages));
                }

                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                if let Err(e) = pipeline::sync_versions(resolved_cfg, config_path, None, true, true, None) {
                    tracing::warn!("version sync failed: {e}");
                }

                if resolved_cfg.e2e.is_some() {
                    tracing::warn!("[e2e] block detected — run 'alef e2e generate' to regenerate e2e test suites");
                }

                grand_total_written += total_written;
            }
            println!("Generated {grand_total_written} files");
            Ok(None)
        }
        Commands::Stubs { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    eprintln!(
                        "[{}] Generating type stubs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Generating type stubs for: {}", format_languages(&languages));
                }
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let files = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let hashes: Vec<(String, String)> = files
                    .iter()
                    .flat_map(|(_, fs)| {
                        fs.iter().map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                    })
                    .collect();

                let cache_key = format!("{}.stubs", resolved_cfg.name);
                let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                if all_match {
                    if multi {
                        eprintln!("[{}] Stubs up to date (skipping)", resolved_cfg.name);
                    } else {
                        println!("Stubs up to date (skipping)");
                    }
                    continue;
                }

                let count = pipeline::write_files(&files, &base_dir)?;
                let _ = cache::write_generation_hashes(&cache_key, &hashes);

                pipeline::format_generated(&files, resolved_cfg, &base_dir, None);

                let stub_paths: std::collections::HashSet<PathBuf> = files
                    .iter()
                    .flat_map(|(_, fs)| fs.iter().map(|f| base_dir.join(&f.path)))
                    .collect();
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes(&stub_paths, &sources_hash, &alef_toml_bytes)?;
                grand_total += count;
            }
            println!("Generated {grand_total} stub files");
            Ok(None)
        }
        Commands::Scaffold { lang } => {
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                tracing::warn!("could not update alef.toml version pin: {e}");
            }

            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "scaffold", &config_toml, &[]);
                if cache::is_stage_cached(&resolved_cfg.name, "scaffold", &stage_hash) {
                    if multi {
                        eprintln!("[{}] Scaffold up to date (cached)", resolved_cfg.name);
                    } else {
                        println!("Scaffold up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    eprintln!(
                        "[{}] Generating scaffolding for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Generating scaffolding for: {}", format_languages(&languages));
                }
                let files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files(&files, &base_dir)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let scaffold_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&scaffold_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "scaffold", &stage_hash, &output_paths)?;
                grand_total += count;
            }

            pipeline::install_poly_hooks(&base_dir);

            // downstream crates can use `#[cfg_attr(feature = "alef-meta", alef(since = "..."))]`
            match pipeline::ensure_workspace_alef_meta_check_cfg() {
                Ok(true) => eprintln!(
                    "Patched Cargo.toml: added [workspace.lints.rust] unexpected_cfgs allowlist for alef-meta"
                ),
                Ok(false) => {}
                Err(e) => eprintln!("Warning: could not patch workspace lints for alef-meta: {e}"),
            }

            println!("Generated {grand_total} scaffold files");
            Ok(None)
        }
        Commands::Readme { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = crate::readme::expand_configured_readme_languages(
                    resolved_cfg,
                    &resolve_readme_languages(resolved_cfg, lang.as_deref())?,
                );
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "readme", &config_toml, &[]);
                if cache::is_stage_cached(&resolved_cfg.name, "readme", &stage_hash) {
                    if multi {
                        eprintln!("[{}] READMEs up to date (cached)", resolved_cfg.name);
                    } else {
                        println!("READMEs up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    eprintln!(
                        "[{}] Generating READMEs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Generating READMEs for: {}", format_languages(&languages));
                }
                let files = pipeline::readme(&api, resolved_cfg, &languages)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let readme_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&readme_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "readme", &stage_hash, &output_paths)?;
                grand_total += count;
            }
            println!("Generated {grand_total} README files");
            Ok(None)
        }
        Commands::Docs { lang, output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_doc_languages(resolved_cfg, lang.as_deref())?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "docs", &config_toml, &[]);
                let use_stage_cache = resolved_cfg.docs.is_none();
                if use_stage_cache && cache::is_stage_cached(&resolved_cfg.name, "docs", &stage_hash) {
                    if multi {
                        eprintln!("[{}] Docs up to date (cached)", resolved_cfg.name);
                    } else {
                        println!("Docs up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    eprintln!(
                        "[{}] Generating docs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Generating docs for: {}", format_languages(&languages));
                }
                let files =
                    crate::docs::generate_docs_stage(&api, resolved_cfg, &languages, output.as_deref(), &base_dir)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let doc_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&doc_paths, &sources_hash, &alef_toml_bytes)?;
                if use_stage_cache {
                    cache::write_stage_hash(&resolved_cfg.name, "docs", &stage_hash, &output_paths)?;
                }
                grand_total += count;
            }
            println!("Generated {grand_total} doc files");
            Ok(None)
        }
        Commands::SyncVersions {
            bump,
            set,
            regen,
            skip_swift_checksum,
            release_date,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                if let Some(version) = &set {
                    if multi {
                        eprintln!("[{}] Setting version to {version}", resolved_cfg.name);
                    } else {
                        eprintln!("Setting version to {version}");
                    }
                    pipeline::set_version(resolved_cfg, version)?;
                }
                if multi {
                    eprintln!("[{}] Syncing versions from Cargo.toml", resolved_cfg.name);
                } else {
                    eprintln!("Syncing versions from Cargo.toml");
                }
                pipeline::sync_versions(
                    resolved_cfg,
                    config_path,
                    bump.as_deref(),
                    !regen,
                    skip_swift_checksum,
                    release_date.as_deref(),
                )?;
            }
            println!("Version sync complete");
            Ok(None)
        }
        Commands::Build { lang, release } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let profile = if release { "release" } else { "dev" };
                if multi {
                    eprintln!(
                        "[{}] Building bindings ({profile}) for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Building bindings ({profile}) for: {}", format_languages(&languages));
                }
                pipeline::build(resolved_cfg, &languages, release)?;
            }
            println!("Build complete");
            Ok(None)
        }
        Commands::Fmt { lang: _ } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                if multi {
                    eprintln!("[{}] Formatting generated output...", resolved_cfg.name);
                } else {
                    eprintln!("Formatting generated output...");
                }
                pipeline::fmt(resolved_cfg, &base_dir)?;
            }
            println!("Format complete");
            Ok(None)
        }
        Commands::Lint { lang: _ } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                if multi {
                    eprintln!("[{}] Linting generated output...", resolved_cfg.name);
                } else {
                    eprintln!("Linting generated output...");
                }
                pipeline::lint(resolved_cfg, &base_dir)?;
            }
            println!("Lint complete");
            Ok(None)
        }
        Commands::Test { lang, e2e, coverage } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_test_languages(resolved_cfg, lang.as_deref(), e2e)?;
                if multi {
                    eprintln!(
                        "[{}] Running tests for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Running tests for: {}", format_languages(&languages));
                }
                if e2e {
                    eprintln!("  (with e2e tests)");
                }
                if coverage {
                    eprintln!("  (with coverage)");
                }
                pipeline::test(resolved_cfg, &languages, e2e, coverage)?;
            }
            println!("Tests complete");
            Ok(None)
        }
        Commands::Setup { lang, timeout } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    eprintln!(
                        "[{}] Setting up dependencies for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Setting up dependencies for: {}", format_languages(&languages));
                }
                pipeline::setup(resolved_cfg, &languages, timeout)?;
            }
            println!("Setup complete");
            Ok(None)
        }
        Commands::Clean { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    eprintln!(
                        "[{}] Cleaning build artifacts for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Cleaning build artifacts for: {}", format_languages(&languages));
                }
                pipeline::clean(resolved_cfg, &languages)?;
            }
            println!("Clean complete");
            Ok(None)
        }
        Commands::Update { lang, latest } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                let mode = if latest { "latest" } else { "compatible" };
                if multi {
                    eprintln!(
                        "[{}] Updating dependencies ({mode}) for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Updating dependencies ({mode}) for: {}", format_languages(&languages));
                }
                pipeline::update(resolved_cfg, &languages, latest)?;
            }
            println!("Update complete");
            Ok(None)
        }
        Commands::Verify {
            exit_code,
            compile: _,
            lint: _,
            lang: _,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            eprintln!("Verifying alef-generated files (inputs-hash mode)");
            let base_dir = std::env::current_dir()?;

            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            let all_inputs_hashes: Vec<String> = crates_to_process
                .iter()
                .filter_map(|c| cache::sources_hash(&c.sources).ok())
                .map(|sh| crate::core::hash::compute_inputs_hash(&sh, &alef_toml_bytes))
                .collect();

            let stale = verify_walk_multi(&base_dir, &all_inputs_hashes)?;

            let mut all_version_mismatches: Vec<String> = Vec::new();
            for resolved_cfg in &crates_to_process {
                let mismatches = pipeline::verify_versions(resolved_cfg)?;
                all_version_mismatches.extend(mismatches);
            }
            let has_version_issues = !all_version_mismatches.is_empty();
            if has_version_issues {
                println!("Version mismatches detected:");
                for mismatch in &all_version_mismatches {
                    println!("  {mismatch}");
                }
            }

            if stale.is_empty() && !has_version_issues {
                println!("All bindings and versions are up to date.");
            } else {
                if !stale.is_empty() {
                    println!("Stale bindings detected:");
                    for s in &stale {
                        println!("  {}", s.path);
                        if context.verbose > 0 {
                            println!("    embedded:  {}", s.embedded);
                            let computed_str = s.computed.join(", ");
                            println!("    computed:  {computed_str}");
                        }
                    }
                }
                if exit_code {
                    process::exit(1);
                }
            }
            Ok(None)
        }
        Commands::Diff { exit_code } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            eprintln!("Computing diff of generated bindings...");
            let base_dir = std::env::current_dir()?;
            let mut all_diffs: Vec<String> = Vec::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, true, config_path)?;
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;
                all_diffs.extend(pipeline::diff_files(&bindings, &base_dir)?);
                all_diffs.extend(pipeline::diff_files(&stubs, &base_dir)?);
            }

            if all_diffs.is_empty() {
                println!("No changes detected.");
            } else {
                println!("Files that would change:");
                for diff in &all_diffs {
                    println!("  {diff}");
                }
                if exit_code {
                    process::exit(1);
                }
            }
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}
