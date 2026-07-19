use anyhow::Result;
use std::path::PathBuf;

use crate::cli::{cache, dispatch, pipeline, registry, version_pin};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::All {
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

            let config_toml = std::fs::read_to_string(config_path)?;

            let mut grand_binding_count: usize = 0;
            let mut grand_stub_count: usize = 0;
            let mut grand_api_count: usize = 0;
            let mut grand_scaffold_count: usize = 0;
            let mut grand_readme_count: usize = 0;
            let mut grand_e2e_count: usize = 0;
            let mut grand_doc_count: usize = 0;

            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                pipeline::ensure_required_formatters(&languages)?;
                if multi {
                    eprintln!(
                        "[{}] Running all for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Running all for: {}", format_languages(&languages));
                }

                let api = pipeline::extract(resolved_cfg, config_path, clean)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                let mut current_gen_paths = std::collections::HashSet::new();
                let mut changed_languages: std::collections::HashSet<crate::core::config::Language> =
                    std::collections::HashSet::new();

                eprintln!("Generating bindings...");
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path)?;

                let mut binding_count: usize = 0;
                for (lang, lang_files) in &bindings {
                    let lang_str = lang.to_string();

                    for file in lang_files {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }

                    let hashes: Vec<(String, String)> = lang_files
                        .iter()
                        .map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
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
                    binding_count += pipeline::write_files(&single, &base_dir)?;
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
                        for (lang, _) in &svc_files {
                            changed_languages.insert(*lang);
                        }
                    }
                }

                eprintln!("Generating scaffolding...");
                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                let scaffold_count = pipeline::write_scaffold_files_with_overwrite(&scaffold_files, &base_dir, clean)?;
                for file in &scaffold_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                eprintln!("Running post-build processing...");
                for &lang in &languages {
                    let Some(backend) = registry::try_get_backend(lang) else {
                        continue;
                    };
                    let Some(bc) = backend.build_config_with_config(resolved_cfg) else {
                        continue;
                    };
                    if bc.post_build.is_empty() {
                        continue;
                    }
                    eprintln!("  [{lang}] running post-build...");
                    match pipeline::run_post_build(lang, &bc, resolved_cfg, &base_dir) {
                        Ok(()) => {
                            eprintln!("  [{lang}] post-build processing complete");
                        }
                        Err(e) => {
                            eprintln!("  [{lang}] post-build processing failed: {e}");
                            return Err(e);
                        }
                    }
                }

                eprintln!("Generating type stubs...");
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;

                let stub_hashes: Vec<(String, String)> = stubs
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

                let stub_count = if !stubs_match || clean {
                    let count = pipeline::write_files(&stubs, &base_dir)?;
                    let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);
                    for (lang, _) in &stubs {
                        changed_languages.insert(*lang);
                    }
                    count
                } else {
                    eprintln!("  [stubs] up to date (skipping)");
                    0
                };

                for (_, files) in &stubs {
                    for file in files {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }
                }

                let mut api_count = 0;
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
                            api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                            eprintln!("Generated {api_count} public API files");
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                        } else {
                            eprintln!("  [public_api] up to date (skipping)");
                        }
                    }
                }

                if !api.version.is_empty() {
                    let pkg = base_dir.join("Package.swift");
                    if let Ok(content) = std::fs::read_to_string(&pkg) {
                        let updated = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{}", api.version));
                        if updated != content {
                            std::fs::write(&pkg, updated)?;
                        }
                    }
                }

                eprintln!("Generating READMEs...");
                let readme_languages = crate::readme::expand_configured_readme_languages(resolved_cfg, &languages);
                let readme_files = pipeline::readme(&api, resolved_cfg, &readme_languages)?;
                let readme_count = pipeline::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
                for file in &readme_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                let mut e2e_count = 0;
                if let Some(e2e_config) = &resolved_cfg.e2e {
                    let all_calls = std::iter::once(("_default", &e2e_config.call))
                        .chain(e2e_config.calls.iter().map(|(k, v)| (k.as_str(), v)));
                    for (call_name, call_config) in all_calls {
                        if call_config.function.is_empty() || call_config.module.is_empty() {
                            continue;
                        }
                        let module_path = call_config.module.replace('-', "_");
                        let function_name = &call_config.function;
                        match crate::extract::validate_call_export(&api, &module_path, function_name) {
                            crate::extract::ExportValidation::Ok => {}
                            crate::extract::ExportValidation::NotFound { function } => {
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' was not found in the extracted API surface. \
                                 Check that it is declared `pub` and that its source file is listed in \
                                 [[crate.sources]] or [[crate.source_crates]]."
                                );
                            }
                            crate::extract::ExportValidation::WrongPath {
                                function,
                                declared_module,
                                actual_paths,
                            } => {
                                let paths = actual_paths.join(", ");
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' is not exported at module path \
                                 '{declared_module}' -- the Rust codegen would emit `use {declared_module}::{function};`. \
                                 Actual rust_path(s) found: {paths}. \
                                 Fix: either add `pub use <path>::{function};` at the crate root, \
                                 or update `module` in [e2e.calls.{call_name}] to the correct path."
                                );
                            }
                        }
                    }

                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                    let ir_json = serde_json::to_string(&api)?;
                    let e2e_stage_hash = cache::compute_stage_hash(&ir_json, "e2e", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "e2e", &e2e_stage_hash) {
                        eprintln!("  [e2e] up to date (skipping)");
                        for path in cache::read_stage_paths(&resolved_cfg.name, "e2e") {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        eprintln!("Generating e2e test suites...");
                        let files = crate::e2e::generate_e2e(resolved_cfg, e2e_config, None, &api.types, &api.enums)?;
                        e2e_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                        crate::e2e::format::run_formatters(&files, e2e_config);

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        let e2e_output_root = base_dir.join(&e2e_config.output);
                        pipeline::sweep_orphans(&[e2e_output_root], &path_set)?;

                        cache::write_stage_hash(&resolved_cfg.name, "e2e", &e2e_stage_hash, &output_paths)?;

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }

                    let test_apps_stage_hash =
                        cache::compute_stage_hash(&ir_json, "test-apps", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "test-apps", &test_apps_stage_hash) {
                        eprintln!("  [test-apps] up to date (skipping)");
                        for path in cache::read_stage_paths(&resolved_cfg.name, "test-apps") {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        eprintln!("Generating registry-mode test apps...");
                        let mut registry_e2e_config = e2e_config.clone();
                        registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        let registry_e2e_ref = &registry_e2e_config;

                        let files =
                            crate::e2e::generate_e2e(resolved_cfg, registry_e2e_ref, None, &api.types, &api.enums)?;
                        let test_apps_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                        e2e_count += test_apps_count;
                        crate::e2e::format::run_formatters(&files, registry_e2e_ref);

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        let test_apps_root = base_dir.join(registry_e2e_ref.effective_output());
                        pipeline::sweep_orphans(&[test_apps_root], &path_set)?;

                        cache::write_stage_hash(&resolved_cfg.name, "test-apps", &test_apps_stage_hash, &output_paths)?;

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }
                }

                eprintln!("Generating docs...");
                let docs_api = pipeline::extract(resolved_cfg, config_path, false)?;
                let doc_languages = resolve_doc_languages(resolved_cfg, None)?;
                let doc_files =
                    crate::docs::generate_docs_stage(&docs_api, resolved_cfg, &doc_languages, None, &base_dir)?;
                let doc_count = pipeline::write_scaffold_files_with_overwrite(&doc_files, &base_dir, clean)?;
                for file in &doc_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                // Protect committed reference-doc pages from host-dependent deletion (#184): ~keep
                // the pages `generate_docs_stage` emits vary with host state (CLI/MCP source ~keep
                // presence, doc languages), so a host that regenerates fewer pages must not let ~keep
                // orphan cleanup delete the committed pages it simply did not produce this run. ~keep
                let docs_reference_dir = base_dir.join(crate::docs::reference_output_dir(resolved_cfg));
                for path in pipeline::collect_alef_headered_paths(&docs_reference_dir) {
                    current_gen_paths.insert(path);
                }

                if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                    if removed > 0 {
                        eprintln!("Removed {removed} stale alef-generated file(s)");
                    }
                }

                {
                    // `alef all` always processes the full language set (no `--lang` ~keep
                    // filter), so the sweep is unfiltered and reclaims orphans across the ~keep
                    // whole binding tree, including the conventional wasm/typescript roots. ~keep
                    let roots: Vec<std::path::PathBuf> =
                        pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir)
                            .into_iter()
                            .filter(|d| d.exists())
                            .collect();
                    if let Ok(removed) = pipeline::sweep_orphans(&roots, &current_gen_paths) {
                        if removed > 0 {
                            eprintln!("Removed {removed} stale alef-generated file(s)");
                        }
                    }
                }

                if !changed_languages.is_empty() {
                    eprintln!("Formatting generated files...");
                    let mut files_to_format = bindings.clone();
                    files_to_format.extend(stubs.clone());
                    pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, Some(&changed_languages));
                }

                eprintln!("Finalising hashes...");
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                grand_binding_count += binding_count;
                grand_stub_count += stub_count;
                grand_api_count += api_count;
                grand_scaffold_count += scaffold_count;
                grand_readme_count += readme_count;
                grand_e2e_count += e2e_count;
                grand_doc_count += doc_count;
            }

            pipeline::install_poly_hooks(&base_dir);

            println!(
                "Done: {grand_binding_count} binding files, {grand_stub_count} stub files, {grand_api_count} API files, {grand_scaffold_count} scaffold files, {grand_readme_count} readme files, {grand_e2e_count} e2e files, {grand_doc_count} doc files"
            );
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}
