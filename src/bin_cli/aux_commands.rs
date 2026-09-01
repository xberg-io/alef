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
        Commands::Init { lang } => {
            tracing::info!("Initializing alef project");
            if let Some(langs) = &lang {
                tracing::info!("  Languages: {}", langs.join(", "));
            }
            pipeline::init(config_path, lang.clone())?;
            tracing::info!("  Created alef.toml");

            let (_workspace, resolved) = load_config(config_path)?;
            let resolved_cfg = &resolved[0];
            let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
            let base_dir = std::env::current_dir()?;

            let api = pipeline::extract(resolved_cfg, config_path, false)?;
            let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

            tracing::info!("  Generating bindings...");
            let bindings = pipeline::generate(&api, resolved_cfg, &languages, false, config_path, true)?;
            let mut binding_count: usize = 0;
            let mut all_paths = std::collections::HashSet::new();
            for (lang_key, lang_files) in &bindings {
                all_paths.extend(pipeline::stampable_output_paths(lang_files, &base_dir));
                let single = vec![(*lang_key, lang_files.clone())];
                binding_count += pipeline::write_files(&single, &base_dir)?;
            }
            if languages.contains(&crate::core::config::Language::Ffi) {
                pipeline::check_ffi_header_freshness(resolved_cfg, &base_dir)?;
            }

            tracing::info!("  Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
            all_paths.extend(pipeline::stampable_output_paths(&scaffold_files, &base_dir));

            tracing::info!("  Formatting...");
            // `alef init` bootstraps a fresh clone, which is precisely the machine least likely
            // to have every formatter installed, so it exposes no `--strict` flag and always
            // takes the lenient default. It still goes through the reporting entry point so the
            // skipped steps are named rather than swallowed. ~keep
            pipeline::format_generated_reporting(resolved_cfg, &base_dir, None, false)?;

            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
            pipeline::finalize_hashes_after_tree_format(&all_paths, &base_dir, &sources_hash, &alef_toml_bytes)?;

            pipeline::install_poly_hooks(&base_dir);

            tracing::info!("Initialized: {binding_count} binding files, {scaffold_count} scaffold files");
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
                tracing::info!("Schema is up to date: {}", output.display());
            } else {
                crate::core::config::write_alef_config_schema(&output, version)?;
                tracing::info!("Wrote schema to {}", output.display());
            }
            Ok(None)
        }
        Commands::Adopt {
            targets,
            write,
            converged_only,
            clobber_create_once_seeds,
        } => {
            super::adopt_command::handle(targets, write, converged_only, clobber_create_once_seeds, context)?;
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
                E2eAction::Generate {
                    lang,
                    registry,
                    strict,
                    no_strict_assertions,
                } => {
                    if registry {
                        tracing::warn!(
                            "`alef e2e generate --registry` is deprecated -- `alef e2e generate` is \
                             local-mode only. Use `alef test-apps generate` instead."
                        );
                    }
                    if no_strict_assertions {
                        // SAFETY: single-threaded CLI dispatch; no concurrent env access here.
                        unsafe { std::env::set_var(crate::e2e::codegen::STRICT_ASSERTIONS_ENV, "0") };
                    }
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    // Deferred the same way `all_commands::handle` defers it, and for the same
                    // reason: `sweep_manifest_orphans` and `cache::write_stage_hash` right after the
                    // write below are unsafe to run on a generator failure (stale-cache and
                    // last-good-output-deletion hazards). Both are gated on this being `None`. ~keep
                    let mut e2e_stage_error: Option<anyhow::Error> = None;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let cache_key = e2e_stage_cache_key(registry, lang.as_deref());
                        let effective_e2e_config;
                        let e2e_ref = if registry {
                            let mut cloned = this_e2e_config.clone();
                            cloned.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                            effective_e2e_config = cloned;
                            &effective_e2e_config
                        } else {
                            this_e2e_config
                        };
                        let stage_hash = cache::compute_stage_hash(&ir_json, &cache_key, &config_toml, &fixture_hash);
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        if cache::is_stage_cached(&e2e_crate.name, &cache_key, &stage_hash) {
                            let cached_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                            grand_count += cached_paths.len();
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters_for_cached_paths(
                                &cached_paths,
                                &base_dir,
                                e2e_ref,
                                strict,
                            )?);
                            if let Some(snippets) = &this_e2e_config.snippets {
                                let coverage_path = base_dir
                                    .join(&snippets.output)
                                    .join(crate::e2e::snippets::COVERAGE_MANIFEST);
                                crate::e2e::report_cached_snippet_coverage(&coverage_path)?;
                            }
                            tracing::info!("E2E tests up to date (cached)");
                            continue;
                        }
                        if registry {
                            tracing::info!("Generating e2e test apps (registry mode)...");
                        } else {
                            tracing::info!("Generating e2e test suites...");
                        }
                        let languages = lang.as_deref();
                        let (files, generator_error) = crate::e2e::generate_e2e(
                            e2e_crate,
                            e2e_ref,
                            languages,
                            &api.types,
                            &api.enums,
                            &api.functions,
                            &api.errors,
                        )?;
                        let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        pipeline::report_refused_writes(&report);
                        pipeline::report_user_owned_skips(&report);
                        let count = report.expected_count();
                        let managed_files = pipeline::managed_generated_files(&files);

                        if managed_files
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters(
                                &managed_files,
                                e2e_ref,
                                strict,
                            )?);
                        }

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let e2e_output_root = base_dir.join(e2e_ref.effective_output());
                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            let snippet_output_root = e2e_ref
                                .snippets
                                .as_ref()
                                .map(|snippets| base_dir.join(&snippets.output));
                            pipeline::targeted_e2e_sweep_roots(
                                &output_paths,
                                &e2e_output_root,
                                snippet_output_root.as_deref(),
                            )
                        } else {
                            vec![e2e_output_root]
                        };
                        if let Some(error) = generator_error {
                            if e2e_stage_error.is_some() {
                                tracing::error!("[{}] e2e codegen failed: {error:#}", e2e_crate.name);
                            }
                            e2e_stage_error.get_or_insert(error);
                        } else {
                            let previous_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                            pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &sweep_roots, &[])?;

                            cache::write_stage_hash(&e2e_crate.name, &cache_key, stage_hash.as_str(), &output_paths)?;
                        }
                        grand_count += count;
                    }
                    tracing::info!("Generated {grand_count} e2e files");
                    if let Some(error) = e2e_stage_error {
                        return Err(error);
                    }
                    Ok(None)
                }
                E2eAction::SnippetsMigrate {
                    existing_root,
                    lang,
                    json,
                } => {
                    let snippet_config = e2e_config
                        .snippets
                        .as_ref()
                        .context("no [e2e.snippets] section in alef.toml")?;
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let api = pipeline::extract(resolved_cfg, config_path, false)?;
                    let fallback_languages = effective_e2e_languages(e2e_config, &resolved_cfg.languages);
                    let languages = lang
                        .as_deref()
                        .unwrap_or_else(|| snippet_config.languages_or(&fallback_languages));
                    let generated = crate::e2e::snippets::generate_snippets(
                        &fixtures,
                        languages,
                        e2e_config,
                        snippet_config,
                        resolved_cfg,
                        &api.types,
                        &api.enums,
                        &api.functions,
                    )?;
                    // The project root is the process working directory, matching how
                    // `snippets.output` and `curated_snippets` are already resolved at
                    // generation time (`e2e::snippets::generate_snippet_report_with_extensions`
                    // resolves curated globs against `Path::new(".")` for the same reason -- see
                    // its own comment). `config_path.parent()` used to stand in for this, which
                    // is only equivalent when `--config` is left at its default (`alef.toml`,
                    // relative to the working directory): passing `--config` pointed at a file
                    // OUTSIDE the project (a staging copy, a config kept in a scripts directory)
                    // made every `curated_snippets` glob -- written project-root-relative --
                    // resolve against that unrelated directory instead, and fail to match
                    // anything the project actually generates. ~keep
                    let cwd = std::env::current_dir().context("failed to read the current working directory")?;
                    let entries =
                        crate::bin_cli::snippet_migration::compare(&cwd, &existing_root, snippet_config, &generated)?;
                    crate::bin_cli::snippet_migration::write_report(&entries, json)?;
                    Ok(None)
                }
                E2eAction::Init => {
                    tracing::info!("Initializing e2e fixtures directory...");
                    let created = crate::e2e::scaffold::init_fixtures(e2e_config, resolved_cfg)?;
                    for path in &created {
                        tracing::info!("  created {path}");
                    }
                    tracing::info!("Initialized {} file(s)", created.len());
                    Ok(None)
                }
                E2eAction::Scaffold {
                    id,
                    category,
                    description,
                } => {
                    let path =
                        crate::e2e::scaffold::scaffold_fixture(e2e_config, resolved_cfg, &id, &category, &description)?;
                    tracing::info!("Created {path}");
                    Ok(None)
                }
                E2eAction::List => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let groups = crate::e2e::fixture::group_fixtures(&fixtures);

                    crate::bin_cli::output::line(format_args!("Fixtures: {} total", fixtures.len()));
                    for group in &groups {
                        crate::bin_cli::output::line(format_args!(
                            "  {}: {} fixture(s)",
                            group.category,
                            group.fixtures.len()
                        ));
                    }
                    Ok(None)
                }
                E2eAction::Validate => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    tracing::info!("Validating fixtures in {}...", fixtures_dir.display());

                    let mut all_errors = crate::e2e::validate::validate_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to validate fixtures from {}", fixtures_dir.display()))?;

                    let fixtures = crate::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    // ~keep An unset `[e2e].languages` is the common case (it's documented to
                    // default to the top-level `languages` list) and must fall back the same way
                    // `E2eAction::SnippetsMigrate` and `TestAppsAction::Run` already do -- passing
                    // the raw empty list here silently disabled the "0 test functions" and
                    // "unsupported language not in the resolved set" checks below.
                    let validated_languages = effective_e2e_languages(e2e_config, &resolved_cfg.languages);
                    let semantic_errors =
                        crate::e2e::validate::validate_fixtures_semantic(&fixtures, e2e_config, &validated_languages);
                    all_errors.extend(semantic_errors);

                    if all_errors.is_empty() {
                        if fixtures.is_empty() {
                            crate::bin_cli::output::line(format_args!(
                                "No fixtures found under {} -- nothing was validated.",
                                fixtures_dir.display()
                            ));
                        } else {
                            crate::bin_cli::output::line(format_args!(
                                "All {} fixture(s) are valid ({} language(s) checked: {}).",
                                fixtures.len(),
                                validated_languages.len(),
                                validated_languages.join(", ")
                            ));
                        }
                        Ok(None)
                    } else {
                        use crate::e2e::validate::Severity;
                        let error_count = all_errors.iter().filter(|e| e.severity == Severity::Error).count();
                        let warning_count = all_errors.iter().filter(|e| e.severity == Severity::Warning).count();
                        crate::bin_cli::output::line(format_args!(
                            "Found {} error(s) and {} warning(s):",
                            error_count, warning_count
                        ));
                        for err in &all_errors {
                            crate::bin_cli::output::line(format_args!("  {err}"));
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
                    jobs: _,
                    strict,
                } => {
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let base_dir = std::env::current_dir()?;
                    let mut grand_count: usize = 0;
                    // Deferred the same way `all_commands::handle` defers it -- see the `e2e`
                    // command's `e2e_stage_error` above for the cache-poisoning and
                    // orphan-deletion hazard this gates against. ~keep
                    let mut e2e_stage_error: Option<anyhow::Error> = None;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };

                        let mut registry_config = this_e2e_config.clone();
                        registry_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        let e2e_ref = &registry_config;
                        let output_root = base_dir.join(e2e_ref.effective_output());

                        let fixtures_dir = std::path::Path::new(&this_e2e_config.fixtures);
                        let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                        let api = pipeline::extract(e2e_crate, config_path, false)?;
                        let ir_json = serde_json::to_string(&api)?;
                        let selector = lang
                            .as_deref()
                            .map(|languages| languages.join("-"))
                            .unwrap_or_else(|| "all".to_string());
                        let cache_key = format!("test-apps-{selector}");
                        let previous_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                        let stage_hash = cache::compute_stage_hash(&ir_json, &cache_key, &config_toml, &fixture_hash);
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        if !clean && cache::is_stage_cached(&e2e_crate.name, &cache_key, &stage_hash) {
                            let cached_paths = cache::read_stage_paths(&e2e_crate.name, &cache_key);
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters_for_cached_paths(
                                &cached_paths,
                                &base_dir,
                                e2e_ref,
                                strict,
                            )?);
                            tracing::info!("Test apps up to date (cached)");
                            continue;
                        }

                        tracing::info!("Generating registry-mode test apps...");
                        let languages = lang.as_deref();
                        let (files, generator_error) = crate::e2e::generate_e2e(
                            e2e_crate,
                            e2e_ref,
                            languages,
                            &api.types,
                            &api.enums,
                            &api.functions,
                            &api.errors,
                        )?;
                        let report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        pipeline::report_refused_writes(&report);
                        pipeline::report_user_owned_skips(&report);
                        let count = report.changed_count();
                        let managed_files: Vec<_> = files
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .cloned()
                            .collect();

                        let generated_langs: Vec<String> = languages
                            .map(|ls| ls.iter().map(|s| s.to_string()).collect())
                            .unwrap_or_else(|| e2e_ref.languages.clone());
                        for lang_name in &generated_langs {
                            let lock_missing = matches!(lang_name.as_str(), "node" | "wasm")
                                && !output_root.join(lang_name).join("pnpm-lock.yaml").exists();
                            if !lock_missing
                                && !report
                                    .changed_paths
                                    .iter()
                                    .any(|path| path.starts_with(output_root.join(lang_name)))
                            {
                                continue;
                            }
                            if lang_name == "node" || lang_name == "wasm" {
                                let test_app_dir = output_root.join(lang_name);
                                let package_json = test_app_dir.join("package.json");
                                if package_json.exists() {
                                    tracing::info!("Regenerating {}/pnpm-lock.yaml...", lang_name);
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
                            } else if lang_name == "php" {
                                let test_app_dir = output_root.join(lang_name);
                                let composer_json = test_app_dir.join("composer.json");
                                if composer_json.exists() {
                                    tracing::info!("Regenerating {}/composer.lock...", lang_name);
                                    run_optional(
                                        "composer",
                                        &[
                                            "update",
                                            "--lock",
                                            "--no-install",
                                            "--working-dir",
                                            test_app_dir.to_string_lossy().as_ref(),
                                        ],
                                    );
                                }
                            }
                        }

                        if managed_files
                            .iter()
                            .any(|file| report.changed_paths.contains(&base_dir.join(&file.path)))
                        {
                            crate::e2e::format::warn_deferred(&crate::e2e::format::run_formatters(
                                &managed_files,
                                e2e_ref,
                                strict,
                            )?);
                        }

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            let snippet_output_root = e2e_ref
                                .snippets
                                .as_ref()
                                .map(|snippets| base_dir.join(&snippets.output));
                            pipeline::targeted_e2e_sweep_roots(
                                &output_paths,
                                &output_root,
                                snippet_output_root.as_deref(),
                            )
                        } else {
                            vec![output_root]
                        };
                        if let Some(error) = generator_error {
                            if e2e_stage_error.is_some() {
                                tracing::error!("[{}] test-apps codegen failed: {error:#}", e2e_crate.name);
                            }
                            e2e_stage_error.get_or_insert(error);
                        } else {
                            pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &sweep_roots, &[])?;

                            cache::write_stage_hash(&e2e_crate.name, &cache_key, stage_hash.as_str(), &output_paths)?;
                        }
                        grand_count += count;
                    }
                    tracing::info!("Generated {grand_count} test-app files");
                    if let Some(error) = e2e_stage_error {
                        return Err(error);
                    }
                    Ok(None)
                }
                TestAppsAction::Run { lang } => {
                    let mut ran_any = false;
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let all_names: Vec<String> = effective_e2e_languages(this_e2e_config, &e2e_crate.languages);
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
                        ran_any = true;
                        tracing::info!("Running test apps for: {}", names.join(", "));
                        pipeline::test_apps_run(e2e_crate, config_path, &names)?;
                    }
                    ensure_requested_test_app_targets_ran(lang.as_deref(), ran_any)?;
                    Ok(None)
                }
            }
        }
        other => Ok(Some(other)),
    }
}

/// Stage-cache key for `alef e2e generate`, fed to `cache::compute_stage_hash` as the stage name
/// and so encoded into the stage hash as well as the manifest filename.
///
/// The `--lang` selection is part of the key because a scoped run only writes the languages it was
/// asked for. Recorded under an unscoped key, that partial output is a complete stage as far as
/// the next run can tell: an unscoped `alef e2e generate` reads the hit and regenerates nothing
/// for any other language, leaving them stale with no diagnostic. `all` stands for "no selector
/// given", the same spelling `alef test-apps generate` uses, and the key stays selector-shaped
/// even when unscoped rather than collapsing to a bare `e2e` -- an entry keyed `e2e` carries no
/// evidence of which scope wrote it, and that is precisely the ambiguity being removed.
///
/// Consequence, and the same one `test-apps-{selector}` already has: `all_commands`'s e2e stage
/// still uses the bare `e2e` key (correctly -- it is unconditionally unscoped, passing `None` to
/// `generate_e2e`), so `alef all` and `alef e2e generate` no longer warm each other's stage cache
/// and each pays one regeneration after the other ran. ~keep
fn e2e_stage_cache_key(registry: bool, lang: Option<&[String]>) -> String {
    let stage = if registry { "e2e-registry" } else { "e2e" };
    let selector = lang
        .map(|languages| languages.join("-"))
        .unwrap_or_else(|| "all".to_string());
    format!("{stage}-{selector}")
}

/// `alef test-apps run`'s "did the requested targets actually run" gate.
///
/// `TestAppsAction::Run`'s per-crate loop silently `continue`s whenever a crate's matched
/// `names` is empty, then falls through to `Ok(None)` -- exit 0 with no distinct signal -- once
/// every crate has been visited. Before this gate existed, a mistyped or stale `--lang` filter
/// (e.g. `--lang python` after a crate's `[e2e].languages` dropped python) matched nothing in
/// every crate and reported the identical clean exit as a run that installed and exercised every
/// requested package. `alef test-apps run` exists specifically to verify a release end-to-end
/// (issue #87) -- the same shape as the tslp incident this fix is named for, where every publish
/// job was skipped and the run still reported success. Mirrors
/// `cli::pipeline::commands::test::ensure_requested_suites_will_run`'s semantics: an *explicit*
/// `--lang` request that matched nothing is fatal. `lang: None` matching nothing is left alone --
/// that means no crate in this run has any `[e2e].languages` configured at all, which is this
/// checkout's own declared configuration rather than an unfulfilled request, so it stays a
/// silent no-op exactly like it did before this fix. ~keep
fn ensure_requested_test_app_targets_ran(lang_filter: Option<&[String]>, ran_any: bool) -> Result<()> {
    if ran_any {
        return Ok(());
    }
    let Some(filter) = lang_filter else {
        return Ok(());
    };
    anyhow::bail!(
        "requested test-app target(s) {} matched no configured `[e2e].languages` in any processed crate. \
         Fix: correct the typo, or add the target to that crate's `[e2e].languages`",
        filter.join(", ")
    );
}

/// The e2e language set a crate's config actually resolves to: `[e2e].languages` when the
/// crate set it, otherwise the fallback derived from the crate's scaffolded `languages` list.
///
/// `E2eConfig::languages` is documented to "default to the top-level `languages` list", but
/// `#[serde(default)]` only gives an empty `Vec` when the key is omitted -- nothing applies that
/// documented fallback automatically. Every call site that reads e2e languages must go through
/// this helper instead of `e2e_config.languages` directly, or an unset `[e2e].languages` (the
/// common case) silently narrows whatever that call site does to zero languages instead of the
/// intended default set. ~keep
fn effective_e2e_languages(
    e2e_config: &crate::core::config::e2e::E2eConfig,
    crate_languages: &[crate::core::config::extras::Language],
) -> Vec<String> {
    if e2e_config.languages.is_empty() {
        crate::e2e::default_e2e_languages(crate_languages)
    } else {
        e2e_config.languages.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The outcome under test is the cache *decision*, taken through `cache::is_stage_cached` --
    /// the same predicate the command branches on -- rather than a comparison of key strings. Two
    /// keys can differ and still collide in the stage cache (they are also the stage-hash input
    /// and the manifest filename), and it is the collision, not the string, that skips a full
    /// regeneration; an assertion on the strings alone would pass while the defect survived. ~keep
    #[test]
    fn scoped_e2e_stage_cache_does_not_satisfy_a_later_full_run() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cwd = crate::test_support::CwdGuard::enter(tmp.path());

        let ir_json = r#"{"crate_name":"sample_crate"}"#;
        let config_toml = "[e2e]\nlanguages = [\"python\", \"node\"]\n";
        let scoped_langs = vec!["python".to_string()];
        let scoped_key = e2e_stage_cache_key(false, Some(&scoped_langs));
        let full_key = e2e_stage_cache_key(false, None);

        let python_output = tmp.path().join("e2e/python/test_smoke.py");
        std::fs::create_dir_all(python_output.parent().expect("output parent")).expect("create output dir");
        std::fs::write(&python_output, "# generated\n").expect("write the scoped run's only output");
        let scoped_hash = cache::compute_stage_hash(ir_json, &scoped_key, config_toml, &[]);
        cache::write_stage_hash("sample-crate", &scoped_key, scoped_hash.as_str(), &[python_output])
            .expect("record the scoped run");

        assert!(
            cache::is_stage_cached("sample-crate", &scoped_key, &scoped_hash),
            "a repeat of the same scoped run must still hit its own cache"
        );
        let full_hash = cache::compute_stage_hash(ir_json, &full_key, config_toml, &[]);
        assert!(
            !cache::is_stage_cached("sample-crate", &full_key, &full_hash),
            "an unscoped run must regenerate rather than inherit a --lang-scoped run's partial output"
        );
        assert!(
            cache::read_stage_paths("sample-crate", &full_key).is_empty(),
            "no unscoped stage has been generated, so the unscoped manifest must not exist"
        );
    }

    #[test]
    fn e2e_stage_cache_key_separates_registry_mode_and_language_selections() {
        let python = vec!["python".to_string()];
        let node = vec!["node".to_string()];

        assert_eq!(e2e_stage_cache_key(false, None), "e2e-all");
        assert_eq!(e2e_stage_cache_key(true, None), "e2e-registry-all");
        assert_eq!(e2e_stage_cache_key(false, Some(&python)), "e2e-python");
        assert_eq!(e2e_stage_cache_key(true, Some(&python)), "e2e-registry-python");
        assert_ne!(
            e2e_stage_cache_key(false, Some(&python)),
            e2e_stage_cache_key(false, Some(&node))
        );
    }

    /// The regression this fix closes: before `ensure_requested_test_app_targets_ran` existed,
    /// `TestAppsAction::Run`'s loop silently `continue`d past every crate whose matched `names`
    /// was empty and fell through to `Ok(None)` -- an explicit `--lang` filter that matched
    /// nothing anywhere reported the identical clean exit as a run that actually installed and
    /// exercised the requested package(s). This proves the caller can now tell the difference.
    #[test]
    fn an_explicit_lang_filter_that_matched_nothing_is_fatal() {
        let requested = vec!["python".to_string()];

        let error = ensure_requested_test_app_targets_ran(Some(&requested), false)
            .expect_err("an explicit --lang filter that matched nothing anywhere must not exit clean");

        assert!(error.to_string().contains("python"), "{error}");
    }

    #[test]
    fn an_explicit_lang_filter_that_matched_something_is_fine() {
        assert!(ensure_requested_test_app_targets_ran(Some(&["python".to_string()]), true).is_ok());
    }

    #[test]
    fn effective_e2e_languages_falls_back_when_config_languages_is_empty() {
        use crate::core::config::e2e::E2eConfig;
        use crate::core::config::extras::Language;

        let e2e_config = E2eConfig::default();
        assert!(
            e2e_config.languages.is_empty(),
            "test assumes the default E2eConfig has no explicit [e2e].languages"
        );

        let names = effective_e2e_languages(&e2e_config, &[Language::Python, Language::Node]);

        assert_eq!(
            names,
            crate::e2e::default_e2e_languages(&[Language::Python, Language::Node]),
            "an unset [e2e].languages must fall back to the crate's scaffolded languages, not resolve to zero"
        );
        assert!(!names.is_empty());
    }

    #[test]
    fn effective_e2e_languages_honours_explicit_config_languages() {
        use crate::core::config::e2e::E2eConfig;
        use crate::core::config::extras::Language;

        let e2e_config = E2eConfig {
            languages: vec!["swift".to_string()],
            ..E2eConfig::default()
        };

        let names = effective_e2e_languages(&e2e_config, &[Language::Python, Language::Node]);

        assert_eq!(
            names,
            vec!["swift".to_string()],
            "an explicit [e2e].languages must win over the crate's scaffolded languages"
        );
    }

    /// The control: with no `--lang` filter, `ran_any == false` means no crate in this run has
    /// any `[e2e].languages` configured at all -- this checkout's own declared configuration,
    /// not an unfulfilled request -- so it must stay non-fatal exactly like it did before this
    /// fix. ~keep
    #[test]
    fn no_lang_filter_and_nothing_configured_stays_non_fatal() {
        assert!(ensure_requested_test_app_targets_ran(None, false).is_ok());
    }
}
