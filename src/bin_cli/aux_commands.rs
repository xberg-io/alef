use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process;

use crate::cli::pipeline::run_optional;
use crate::cli::{cache, commands, dispatch, pipeline};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::Init { lang, format } => {
            eprintln!("Initializing alef project");
            if let Some(langs) = &lang {
                eprintln!("  Languages: {}", langs.join(", "));
            }
            pipeline::init(config_path, lang.clone())?;
            eprintln!("  Created alef.toml");

            let (_workspace, resolved) = load_config(config_path)?;
            let resolved_cfg = &resolved[0];
            let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
            let base_dir = std::env::current_dir()?;

            let api = pipeline::extract(resolved_cfg, config_path, false)?;
            let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

            eprintln!("  Generating bindings...");
            let bindings = pipeline::generate(&api, resolved_cfg, &languages, false, config_path)?;
            let mut binding_count: usize = 0;
            let mut all_paths = std::collections::HashSet::new();
            for (lang_key, lang_files) in &bindings {
                for file in lang_files {
                    all_paths.insert(base_dir.join(&file.path));
                }
                let single = vec![(*lang_key, lang_files.clone())];
                binding_count += pipeline::write_files(&single, &base_dir)?;
            }

            eprintln!("  Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
            for file in &scaffold_files {
                all_paths.insert(base_dir.join(&file.path));
            }

            if format {
                eprintln!("  Formatting...");
                pipeline::format_generated(&bindings, resolved_cfg, &base_dir, None);
            }

            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
            pipeline::finalize_hashes(&all_paths, &sources_hash, &alef_toml_bytes)?;

            pipeline::install_poly_hooks(&base_dir);

            println!("Initialized: {binding_count} binding files, {scaffold_count} scaffold files");
            Ok(None)
        }
        Commands::Schema {
            output,
            schema_version,
            check,
        } => {
            let version = schema_version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
            if check {
                crate::core::config::check_alef_config_schema(&output, version)?;
                println!("Schema is up to date: {}", output.display());
            } else {
                crate::core::config::write_alef_config_schema(&output, version)?;
                println!("Wrote schema to {}", output.display());
            }
            Ok(None)
        }
        Commands::Migrate { path, write } => {
            let migrate_path = path.unwrap_or_else(|| context.config_path.clone());
            let options = commands::migrate::MigrateOptions {
                path: migrate_path,
                write,
            };
            commands::migrate::run(options)?;
            Ok(None)
        }
        Commands::E2e { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let resolved_cfg = crates_to_process
                .iter()
                .find(|c| c.e2e.is_some())
                .copied()
                .unwrap_or_else(|| crates_to_process[0]);
            let e2e_config = resolved_cfg.e2e.as_ref().context("no [e2e] section in alef.toml")?;
            match action {
                E2eAction::Generate { lang, registry, format } => {
                    if registry {
                        eprintln!(
                            "warning: `alef e2e generate --registry` is deprecated. \
                             Use `alef test-apps generate` instead. \
                             `alef e2e generate` is local-mode only."
                        );
                    }
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let cache_key = if registry { "e2e-registry" } else { "e2e" };
                        let stage_hash = cache::compute_stage_hash(&ir_json, cache_key, &config_toml, &fixture_hash);
                        if cache::is_stage_cached(&e2e_crate.name, cache_key, &stage_hash) {
                            println!("E2E tests up to date (cached)");
                            continue;
                        }
                        let effective_e2e_config;
                        let e2e_ref = if registry {
                            let mut cloned = this_e2e_config.clone();
                            cloned.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                            effective_e2e_config = cloned;
                            eprintln!("Generating e2e test apps (registry mode)...");
                            &effective_e2e_config
                        } else {
                            eprintln!("Generating e2e test suites...");
                            this_e2e_config
                        };
                        let languages = lang.as_deref();
                        let files = crate::e2e::generate_e2e(e2e_crate, e2e_ref, languages, &api.types, &api.enums)?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

                        if format {
                            crate::e2e::format::run_formatters(&files, e2e_ref);
                        }

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let e2e_output_root = base_dir.join(e2e_ref.effective_output());
                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            let mut seen = std::collections::HashSet::new();
                            for path in &output_paths {
                                if let Ok(rel) = path.strip_prefix(&e2e_output_root) {
                                    if let Some(top) = rel.components().next() {
                                        let lang_dir = e2e_output_root.join(top.as_os_str());
                                        seen.insert(lang_dir);
                                    }
                                }
                            }
                            seen.into_iter().collect()
                        } else {
                            vec![e2e_output_root]
                        };
                        pipeline::sweep_orphans(&sweep_roots, &path_set)?;

                        cache::write_stage_hash(&e2e_crate.name, cache_key, &stage_hash, &output_paths)?;
                        grand_count += count;
                    }
                    println!("Generated {grand_count} e2e files");
                    Ok(None)
                }
                E2eAction::Init => {
                    eprintln!("Initializing e2e fixtures directory...");
                    let created = crate::e2e::scaffold::init_fixtures(e2e_config, resolved_cfg)?;
                    for path in &created {
                        println!("  created {path}");
                    }
                    println!("Initialized {} file(s)", created.len());
                    Ok(None)
                }
                E2eAction::Scaffold {
                    id,
                    category,
                    description,
                } => {
                    let path =
                        crate::e2e::scaffold::scaffold_fixture(e2e_config, resolved_cfg, &id, &category, &description)?;
                    println!("Created {path}");
                    Ok(None)
                }
                E2eAction::List => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let groups = crate::e2e::fixture::group_fixtures(&fixtures);

                    println!("Fixtures: {} total", fixtures.len());
                    for group in &groups {
                        println!("  {}: {} fixture(s)", group.category, group.fixtures.len());
                    }
                    Ok(None)
                }
                E2eAction::Validate => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    eprintln!("Validating fixtures in {}...", fixtures_dir.display());

                    let mut all_errors = crate::e2e::validate::validate_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to validate fixtures from {}", fixtures_dir.display()))?;

                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let semantic_errors =
                        crate::e2e::validate::validate_fixtures_semantic(&fixtures, e2e_config, &e2e_config.languages);
                    all_errors.extend(semantic_errors);

                    if all_errors.is_empty() {
                        println!("All fixtures are valid.");
                        Ok(None)
                    } else {
                        use crate::e2e::validate::Severity;
                        let error_count = all_errors.iter().filter(|e| e.severity == Severity::Error).count();
                        let warning_count = all_errors.iter().filter(|e| e.severity == Severity::Warning).count();
                        println!("Found {} error(s) and {} warning(s):", error_count, warning_count);
                        for err in &all_errors {
                            println!("  {err}");
                        }
                        if error_count > 0 {
                            process::exit(1);
                        }
                        Ok(None)
                    }
                }
            }
        }
        Commands::TestApps { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let _resolved_cfg = crates_to_process
                .iter()
                .find(|c| c.e2e.is_some())
                .copied()
                .unwrap_or_else(|| crates_to_process[0]);
            let _ = _resolved_cfg.e2e.as_ref().context("no [e2e] section in alef.toml")?;
            match action {
                TestAppsAction::Generate {
                    lang,
                    clean,
                    format,
                    jobs: _,
                } => {
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };

                        let mut registry_config = this_e2e_config.clone();
                        registry_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        let e2e_ref = &registry_config;
                        let output_root = base_dir.join(e2e_ref.effective_output());

                        if clean {
                            let langs_to_clean: Vec<String> = lang
                                .as_deref()
                                .map(|ls| ls.iter().map(|s| s.to_string()).collect())
                                .unwrap_or_else(|| e2e_ref.languages.clone());
                            let lock_files = [
                                "go.sum",
                                "go.mod",
                                "Gemfile.lock",
                                "composer.lock",
                                "uv.lock",
                                "pubspec.lock",
                            ];
                            // JS lockfiles are deliberately NOT preserved across --clean for the
                            // node/wasm test apps. A committed lockfile that pins an older version
                            // than package.json wants (e.g. `pnpm-lock.yaml` stuck at rc.25 while
                            // package.json wants ^rc.26) makes pnpm's `minimumReleaseAge`
                            // supply-chain gate reject the install
                            // (ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION). Dropping the stale lock lets
                            // the post-generate `pnpm install --lockfile-only` regenerate it fresh
                            // against the current package.json — and if that step is unavailable, no
                            // stale lock ships and smoke-time `pnpm install` resolves cleanly.
                            let js_lock_files = ["package-lock.json", "pnpm-lock.yaml", "yarn.lock"];
                            for lang_name in &langs_to_clean {
                                let preserve_js_locks = lang_name != "node" && lang_name != "wasm";
                                let lang_dir = output_root.join(lang_name);
                                if lang_dir.exists() {
                                    let mut saved_locks = std::collections::HashMap::new();
                                    let lock_files_iter = lock_files
                                        .iter()
                                        .chain(js_lock_files.iter().filter(|_| preserve_js_locks));
                                    for lock_file in lock_files_iter {
                                        let lock_path = lang_dir.join(lock_file);
                                        if lock_path.exists() {
                                            if let Ok(content) = std::fs::read(&lock_path) {
                                                saved_locks.insert(lock_path.clone(), content);
                                            }
                                        }
                                    }

                                    std::fs::remove_dir_all(&lang_dir)
                                        .with_context(|| format!("failed to remove {}", lang_dir.display()))?;

                                    std::fs::create_dir_all(&lang_dir)
                                        .with_context(|| format!("failed to recreate {}", lang_dir.display()))?;
                                    for (lock_path, content) in saved_locks {
                                        std::fs::write(&lock_path, content).with_context(|| {
                                            format!("failed to restore lock file {}", lock_path.display())
                                        })?;
                                    }
                                }
                            }
                        }

                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let cache_key = "test-apps";
                        let stage_hash = cache::compute_stage_hash(&ir_json, cache_key, &config_toml, &fixture_hash);
                        if !clean && cache::is_stage_cached(&e2e_crate.name, cache_key, &stage_hash) {
                            println!("Test apps up to date (cached)");
                            continue;
                        }

                        eprintln!("Generating registry-mode test apps...");
                        let languages = lang.as_deref();
                        let files = crate::e2e::generate_e2e(e2e_crate, e2e_ref, languages, &api.types, &api.enums)?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

                        let generated_langs: Vec<String> = languages
                            .map(|ls| ls.iter().map(|s| s.to_string()).collect())
                            .unwrap_or_else(|| e2e_ref.languages.clone());
                        for lang_name in &generated_langs {
                            if lang_name == "node" || lang_name == "wasm" {
                                let test_app_dir = output_root.join(lang_name);
                                let package_json = test_app_dir.join("package.json");
                                if package_json.exists() {
                                    eprintln!("Regenerating {}/pnpm-lock.yaml...", lang_name);
                                    run_optional(
                                        "pnpm",
                                        &[
                                            "install",
                                            "--lockfile-only",
                                            "-C",
                                            test_app_dir.to_string_lossy().as_ref(),
                                        ],
                                    );
                                }
                            }
                        }

                        if format {
                            crate::e2e::format::run_formatters(&files, e2e_ref);
                        }

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            let mut seen = std::collections::HashSet::new();
                            for path in &output_paths {
                                if let Ok(rel) = path.strip_prefix(&output_root) {
                                    if let Some(top) = rel.components().next() {
                                        seen.insert(output_root.join(top.as_os_str()));
                                    }
                                }
                            }
                            seen.into_iter().collect()
                        } else {
                            vec![output_root]
                        };
                        pipeline::sweep_orphans(&sweep_roots, &path_set)?;

                        cache::write_stage_hash(&e2e_crate.name, cache_key, &stage_hash, &output_paths)?;
                        grand_count += count;
                    }
                    println!("Generated {grand_count} test-app files");
                    Ok(None)
                }
                TestAppsAction::Run { lang } => {
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let all_names: Vec<String> = if this_e2e_config.languages.is_empty() {
                            crate::e2e::default_e2e_languages(&e2e_crate.languages)
                        } else {
                            this_e2e_config.languages.clone()
                        };
                        let names: Vec<String> = match lang.as_deref() {
                            Some(filter) => all_names
                                .into_iter()
                                .filter(|n| filter.iter().any(|f| f == n))
                                .collect(),
                            None => all_names,
                        };
                        if names.is_empty() {
                            continue;
                        }
                        eprintln!("Running test apps for: {}", names.join(", "));
                        pipeline::test_apps_run(e2e_crate, &names)?;
                    }
                    Ok(None)
                }
            }
        }
        other => Ok(Some(other)),
    }
}
