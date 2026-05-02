use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

mod cache;
mod commands;
mod dispatch;
mod pipeline;
mod registry;
mod version_pin;

#[derive(Parser)]
#[command(
    name = "alef",
    version,
    about = "Opinionated polyglot binding generator",
    long_about = None,
)]
struct Cli {
    /// Path to alef.toml config file.
    #[arg(long, default_value = "alef.toml")]
    config: PathBuf,

    /// Maximum parallel jobs (0 = all cores, 1 = sequential).
    #[arg(short, long, default_value = "0", global = true)]
    jobs: usize,

    /// Increase verbosity (-v info, -vv debug, -vvv trace). Overridden by RUST_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Suppress all output below `error`. Overridden by RUST_LOG.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Disable ANSI colour in log output.
    #[arg(long, global = true)]
    no_color: bool,

    /// Restrict the command to one or more crates by name. May be passed multiple times.
    /// When omitted, every crate in the workspace is processed.
    #[arg(long = "crate", value_name = "NAME", global = true)]
    crate_filter: Vec<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extract API surface from Rust source into IR.
    Extract {
        /// Output IR JSON file.
        #[arg(short, long, default_value = ".alef/ir.json")]
        output: PathBuf,
    },
    /// Generate bindings for selected languages.
    Generate {
        /// Comma-separated list of languages (default: all from config).
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Ignore cache, regenerate everything.
        #[arg(long)]
        clean: bool,
        /// Skip post-generation formatters (formatters run by default).
        #[arg(long)]
        no_format: bool,
    },
    /// Generate type stubs (.pyi, .rbs).
    Stubs {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Generate package scaffolding (pyproject.toml, package.json, etc.).
    Scaffold {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Generate README files from templates.
    Readme {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Generate API reference documentation (Markdown for mkdocs).
    Docs {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Output directory (default: docs/reference).
        #[arg(long, default_value = "docs/reference")]
        output: String,
    },
    /// Sync version from Cargo.toml to all package manifests.
    SyncVersions {
        /// Bump version before syncing (major, minor, patch).
        #[arg(long)]
        bump: Option<String>,
        /// Set version explicitly (e.g., "0.1.0-rc.1").
        #[arg(long)]
        set: Option<String>,
    },
    /// Run format commands on generated output.
    Fmt {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Run configured lint/format commands on generated output.
    Lint {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Run configured test suites for each language.
    Test {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Also run e2e tests.
        #[arg(long)]
        e2e: bool,
        /// Run with coverage collection.
        #[arg(long)]
        coverage: bool,
    },
    /// Install dependencies for each language.
    Setup {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Override the per-language setup timeout in seconds (default: 600).
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Clean build artifacts for each language.
    Clean {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Update dependencies for each language.
    Update {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Upgrade to latest versions, including incompatible/major bumps.
        #[arg(long)]
        latest: bool,
    },
    /// Verify bindings are up to date and API surface parity.
    Verify {
        /// Exit with code 1 if any binding is stale (CI mode).
        #[arg(long)]
        exit_code: bool,
        /// Also run compilation check.
        #[arg(long)]
        compile: bool,
        /// Also run lint check.
        #[arg(long)]
        lint: bool,
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Show diff of what would change without writing.
    Diff {
        /// Exit with code 1 if changes exist (CI mode).
        #[arg(long)]
        exit_code: bool,
    },
    /// Build language bindings using native tools (napi, maturin, wasm-pack, etc.).
    Build {
        /// Comma-separated list of languages (default: all from config).
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Build with release optimizations.
        #[arg(long, short)]
        release: bool,
    },
    /// Run all: generate + stubs + scaffold + readme + sync.
    All {
        /// Ignore cache.
        #[arg(long)]
        clean: bool,
        /// Skip post-generation formatters (formatters run by default).
        #[arg(long)]
        no_format: bool,
    },
    /// Initialize a new alef.toml config.
    Init {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Run post-generation formatters on emitted files (off by default).
        #[arg(long)]
        format: bool,
    },
    /// Migrate legacy alef.toml schema to new [workspace] / [[crates]] layout.
    Migrate {
        /// Path to alef.toml (default: alef.toml from --config flag).
        path: Option<PathBuf>,
        /// Write migrated config back to file (dry-run by default).
        #[arg(long)]
        write: bool,
    },
    /// Generate e2e test suites from fixture files.
    E2e {
        #[command(subcommand)]
        action: E2eAction,
    },
    /// Prepare, build, and package artifacts for publishing.
    Publish {
        #[command(subcommand)]
        action: PublishAction,
    },
    /// Manage the build cache.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Cross-manifest version consistency checker and release utilities.
    Validate {
        #[command(subcommand)]
        action: ValidateAction,
    },
    /// Emit release metadata JSON consumed by CI workflows.
    ReleaseMetadata {
        /// Release tag (e.g. v4.1.0 or v4.1.0-rc.1). Required.
        #[arg(long, short)]
        tag: String,
        /// Comma-separated target list (e.g. "python,node") or "all" (default).
        #[arg(long, default_value = "all")]
        targets: String,
        /// Git ref override (branch, tag, or commit SHA).
        #[arg(long)]
        git_ref: Option<String>,
        /// GitHub event name (release/workflow_dispatch/repository_dispatch).
        #[arg(long, default_value = "")]
        event: String,
        /// Dry-run flag — include in metadata without actually publishing.
        #[arg(long)]
        dry_run: bool,
        /// Force-republish flag — republish even if version already exists.
        #[arg(long)]
        force_republish: bool,
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check whether a package version exists in a registry.
    CheckRegistry {
        /// Registry to check.
        #[arg(long, value_enum)]
        registry: commands::check_registry::Registry,
        /// Package name (use `groupId:artifactId` for Maven).
        #[arg(long)]
        package: String,
        /// Version to check.
        #[arg(long)]
        version: String,
        /// Homebrew tap repository (`owner/repo`).
        #[arg(long)]
        tap_repo: Option<String>,
        /// GitHub repository (`owner/repo`) for github-release check.
        #[arg(long)]
        repo: Option<String>,
        /// NuGet source URL (defaults to https://api.nuget.org).
        #[arg(long)]
        source: Option<String>,
        /// Asset name prefix (github-release): require at least one asset on
        /// the release whose name starts with this prefix.
        #[arg(long)]
        asset_prefix: Option<String>,
        /// Comma-separated list of required asset names (github-release): all
        /// must be present on the release.
        #[arg(long, value_delimiter = ',')]
        required_assets: Vec<String>,
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create and push Go module tags for a release.
    GoTag {
        /// Version string (e.g. "4.1.0" or "v4.1.0").
        #[arg(long, short)]
        version: String,
        /// Git remote name (default: origin).
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Print tags that would be created without executing.
        #[arg(long)]
        dry_run: bool,
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PublishAction {
    /// Prepare for publishing: vendor deps, stage FFI artifacts.
    Prepare {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Rust target triple for cross-compilation (e.g. x86_64-unknown-linux-gnu).
        #[arg(long)]
        target: Option<String>,
        /// Show what would be done without executing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Build release artifacts for a specific platform.
    Build {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Rust target triple (defaults to host).
        #[arg(long)]
        target: Option<String>,
        /// Use `cross` instead of `cargo` for cross-compilation.
        #[arg(long)]
        use_cross: bool,
    },
    /// Package built artifacts into distributable archives.
    Package {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Rust target triple (auto-maps to language-specific platform names).
        #[arg(long)]
        target: Option<String>,
        /// Output directory for packages.
        #[arg(long, short, default_value = "dist")]
        output: String,
        /// Version string (auto-detected from Cargo.toml if absent).
        #[arg(long)]
        version: Option<String>,
        /// Show what would be packaged without executing.
        #[arg(long)]
        dry_run: bool,
        /// PHP minor version (e.g. "8.5"). Required when --lang php.
        #[arg(long)]
        php_version: Option<String>,
        /// PHP thread-safety mode: "nts" or "ts". Defaults to "nts".
        #[arg(long, default_value = "nts")]
        php_ts: String,
        /// Linux libc override: "glibc" or "musl". Auto-detected from target triple if absent.
        #[arg(long)]
        php_libc: Option<String>,
        /// Windows compiler tag (e.g. "vs16", "vs17"). Required when target OS is windows and --lang php.
        #[arg(long)]
        windows_compiler: Option<String>,
    },
    /// Validate that all package manifests are consistent and ready for publishing.
    Validate,
}

#[derive(Subcommand)]
enum E2eAction {
    /// Generate e2e test projects from fixtures.
    Generate {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Generate standalone test apps using registry (published) package
        /// versions instead of local path dependencies.
        #[arg(long)]
        registry: bool,
    },
    /// Initialize fixture directory with schema and example.
    Init,
    /// Scaffold a new fixture file.
    Scaffold {
        /// Fixture ID (snake_case).
        #[arg(long)]
        id: String,
        /// Category name.
        #[arg(long)]
        category: String,
        /// Description.
        #[arg(long)]
        description: String,
    },
    /// List all fixtures with counts per category.
    List,
    /// Validate fixture files against the JSON schema.
    Validate,
}

#[derive(Subcommand)]
enum CacheAction {
    /// Clear the .alef/ cache directory.
    Clear,
    /// Show cache status.
    Status,
}

#[derive(Subcommand)]
enum ValidateAction {
    /// Check that all language manifest versions match the Cargo.toml workspace version.
    Versions {
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Exit with code 1 if any mismatch is found.
        #[arg(long)]
        exit_code: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet, cli.no_color);

    // Configure rayon thread pool based on --jobs flag
    if cli.jobs > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cli.jobs)
            .build_global()
            .ok();
    }

    let config_path = &cli.config;

    match cli.command {
        Commands::Extract { output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                // For multi-crate: derive per-crate output path so each crate
                // gets its own IR file instead of overwriting a shared path.
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
            Ok(())
        }
        Commands::Generate { lang, clean, no_format } => {
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
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
                let files = pipeline::generate(&api, resolved_cfg, &languages, clean)?;
                // Pure source-only fingerprint. The embedded `alef:hash:` line in
                // every generated file combines this with the file's own (post-format)
                // content, so the hash stays stable across alef CLI bumps as long as
                // the rust sources and emitted bytes are unchanged.
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                // Collect all files generated in this run for cleanup pass
                let mut current_gen_paths = std::collections::HashSet::new();
                let mut changed_languages: std::collections::HashSet<alef_core::config::Language> =
                    std::collections::HashSet::new();

                let mut total_written: usize = 0;
                let mut any_written = false;
                for (lang, lang_files) in &files {
                    let lang_str = lang.to_string();
                    for file in lang_files {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }

                    // Per-language up-to-date short-circuit: hash the codegen output
                    // (pre-format) and compare with the stored hashes from the last
                    // run. Independent of the embedded `alef:hash:` line, which is
                    // finalised on-disk after formatters run.
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
                    let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                    if all_match && !clean {
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

                // Generate public API wrappers — cache by content hash like
                // bindings, otherwise we rewrite hundreds of files on every warm
                // run for no net change.
                if resolved_cfg.generate.public_api {
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages)?;
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

                // Generate type stubs (e.g., .pyi for Python, .d.ts for TypeScript)
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

                    // Always register stub paths in `current_gen_paths` so the
                    // orphan-sweep pass never deletes them when the cache is warm.
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
                            // Track stub-changed languages so formatters run even when
                            // no bindings changed for this language (e.g. ruff on .pyi).
                            changed_languages.insert(*lang);
                        }
                    } else {
                        eprintln!("  [stubs] up to date (skipping)");
                    }
                }

                if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                    if removed > 0 {
                        eprintln!("Removed {removed} stale alef-generated file(s)");
                    }
                }

                if any_written && !no_format && !changed_languages.is_empty() {
                    eprintln!("Formatting generated files...");
                    pipeline::format_generated(&files, resolved_cfg, &base_dir, Some(&changed_languages));
                    let changed_list: Vec<alef_core::config::Language> = changed_languages.iter().copied().collect();
                    pipeline::fmt_post_generate(resolved_cfg, &changed_list);
                }

                // Finalise per-file hashes after all formatters have run, so the
                // embedded `alef:hash:` line describes the actual on-disk content
                // and `alef verify` can recompute it without regenerating.
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash)?;

                // Always re-sync versions across user-owned manifests.
                if let Err(e) = pipeline::sync_versions(resolved_cfg, config_path, None) {
                    tracing::warn!("version sync failed: {e}");
                }

                // Stamp alef.toml with the CLI version that produced this generate.
                if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                    tracing::warn!("could not update alef.toml version pin: {e}");
                }

                grand_total_written += total_written;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total_written} files");
            Ok(())
        }
        Commands::Stubs { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
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

                // Compute content hashes and compare against stored values; write
                // only when something has actually changed.
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

                // Run language-native formatters on the freshly written stubs before
                // computing the embedded hash.  Without this step, `alef:hash:` is
                // computed over the raw codegen output (e.g. with unused `Any` imports
                // or brace-heavy PHP style); when host-language tools (ruff, php-cs-fixer,
                // mix format, …) reformat those files the hash no longer matches and
                // `alef verify` reports them as stale.  Formatter failures are warnings —
                // they must not abort the stubs command.
                let stub_langs: Vec<alef_core::config::Language> = files.iter().map(|(lang, _)| *lang).collect();
                pipeline::format_generated(&files, resolved_cfg, &base_dir, None);
                pipeline::fmt_post_generate(resolved_cfg, &stub_langs);

                // Finalise per-file hashes for the freshly written (and formatted) stubs.
                let stub_paths: std::collections::HashSet<PathBuf> = files
                    .iter()
                    .flat_map(|(_, fs)| fs.iter().map(|f| base_dir.join(&f.path)))
                    .collect();
                pipeline::finalize_hashes(&stub_paths, &sources_hash)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total} stub files");
            Ok(())
        }
        Commands::Scaffold { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
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
                let files = pipeline::scaffold(&api, resolved_cfg, &languages)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let count = pipeline::write_scaffold_files(&files, &base_dir)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let scaffold_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&scaffold_paths, &sources_hash)?;
                cache::write_stage_hash(&resolved_cfg.name, "scaffold", &stage_hash, &output_paths)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total} scaffold files");
            Ok(())
        }
        Commands::Readme { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
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
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let readme_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&readme_paths, &sources_hash)?;
                cache::write_stage_hash(&resolved_cfg.name, "readme", &stage_hash, &output_paths)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total} README files");
            Ok(())
        }
        Commands::Docs { lang, output } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let mut grand_total: usize = 0;
            for resolved_cfg in &crates_to_process {
                let languages = resolve_doc_languages(resolved_cfg, lang.as_deref())?;
                // Use filtered IR so docs only cover the public API surface.
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let ir_json = serde_json::to_string(&api)?;
                let stage_hash = cache::compute_stage_hash(&ir_json, "docs", &config_toml, &[]);
                if cache::is_stage_cached(&resolved_cfg.name, "docs", &stage_hash) {
                    if multi {
                        eprintln!("[{}] API docs up to date (cached)", resolved_cfg.name);
                    } else {
                        println!("API docs up to date (cached)");
                    }
                    continue;
                }
                if multi {
                    eprintln!(
                        "[{}] Generating API docs for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Generating API docs for: {}", format_languages(&languages));
                }
                let files = alef_docs::generate_docs(&api, resolved_cfg, &languages, &output)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let doc_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&doc_paths, &sources_hash)?;
                cache::write_stage_hash(&resolved_cfg.name, "docs", &stage_hash, &output_paths)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total} API doc files");
            Ok(())
        }
        Commands::SyncVersions { bump, set } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
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
                pipeline::sync_versions(resolved_cfg, config_path, bump.as_deref())?;
            }
            println!("Version sync complete");
            Ok(())
        }
        Commands::Build { lang, release } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
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
            Ok(())
        }
        Commands::Fmt { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    eprintln!(
                        "[{}] Formatting generated output for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Formatting generated output for: {}", format_languages(&languages));
                }
                pipeline::fmt(resolved_cfg, &languages)?;
            }
            println!("Format complete");
            Ok(())
        }
        Commands::Lint { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                if multi {
                    eprintln!(
                        "[{}] Linting generated output for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    eprintln!("Linting generated output for: {}", format_languages(&languages));
                }
                pipeline::lint(resolved_cfg, &languages)?;
            }
            println!("Lint complete");
            Ok(())
        }
        Commands::Test { lang, e2e, coverage } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
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
            Ok(())
        }
        Commands::Setup { lang, timeout } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
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
            Ok(())
        }
        Commands::Clean { lang } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
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
            Ok(())
        }
        Commands::Update { lang, latest } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
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
            Ok(())
        }
        Commands::Verify {
            exit_code,
            compile: _,
            lint: _,
            lang: _,
        } => {
            // alef verify is **idempotent across alef versions**: for each
            // alef-headered file on disk it recomputes
            // `blake3(sources_hash || file_content_without_hash_line)` and
            // compares with the embedded `alef:hash:<hex>` line. There is no
            // alef-version dimension and no `alef.toml` dimension, so a green
            // `alef verify` stays green after upgrading the alef CLI as long
            // as the rust sources and on-disk file contents are unchanged.
            //
            // Verify never regenerates and never writes — pure read+rehash.
            // The legacy `--compile` / `--lint` / `--lang` flags are accepted
            // but ignored; run `alef build` / `alef lint` / `alef test` for
            // those concerns.
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            eprintln!("Verifying alef-generated files (per-file hash mode)");
            let base_dir = std::env::current_dir()?;

            // Collect sources hashes for all selected crates so that the
            // file walk can validate each file against ANY crate's hash.
            // A file is valid if it matches at least one crate's sources hash
            // (each file was generated by exactly one crate).
            let all_sources_hashes: Vec<String> = crates_to_process
                .iter()
                .filter_map(|c| cache::sources_hash(&c.sources).ok())
                .collect();

            let stale = verify_walk_multi(&base_dir, &all_sources_hashes)?;

            // Version consistency check: run per crate, accumulate mismatches.
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
                        println!("  {s}");
                    }
                }
                if exit_code {
                    process::exit(1);
                }
            }
            Ok(())
        }
        Commands::Diff { exit_code } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            eprintln!("Computing diff of generated bindings...");
            let base_dir = std::env::current_dir()?;
            let mut all_diffs: Vec<String> = Vec::new();
            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                let api = pipeline::extract(resolved_cfg, config_path, false)?;
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, true)?;
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
            Ok(())
        }
        Commands::All { clean, no_format } => {
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            let mut grand_binding_count: usize = 0;
            let mut grand_stub_count: usize = 0;
            let mut grand_api_count: usize = 0;
            let mut grand_scaffold_count: usize = 0;
            let mut grand_readme_count: usize = 0;
            let mut grand_e2e_count: usize = 0;
            let mut grand_doc_count: usize = 0;

            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
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

                // Collect all files generated in this run for cleanup pass
                let mut current_gen_paths = std::collections::HashSet::new();
                let mut changed_languages: std::collections::HashSet<alef_core::config::Language> =
                    std::collections::HashSet::new();

                eprintln!("Generating bindings...");
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, clean)?;

                // Per-language: hash content, skip writing if all hashes match.
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
                    let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                    if all_match && !clean {
                        eprintln!("  [{lang_str}] up to date (skipping)");
                        continue;
                    }

                    let single = vec![(*lang, lang_files.clone())];
                    binding_count += pipeline::write_files(&single, &base_dir)?;
                    changed_languages.insert(*lang);
                    let _ = cache::write_generation_hashes(&cache_key, &hashes);
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
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages)?;
                    if !public_api_files.is_empty() {
                        api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                        for (_, files) in &public_api_files {
                            for file in files {
                                current_gen_paths.insert(base_dir.join(&file.path));
                            }
                        }
                    }
                }

                eprintln!("Generating scaffolding...");
                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages)?;
                let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
                for file in &scaffold_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                eprintln!("Generating READMEs...");
                let readme_files = pipeline::readme(&api, resolved_cfg, &languages)?;
                let readme_count = pipeline::write_scaffold_files_with_overwrite(&readme_files, &base_dir, clean)?;
                for file in &readme_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                let mut e2e_count = 0;
                if let Some(e2e_config) = &resolved_cfg.e2e {
                    // Validate that every call config's (module, function) pair is
                    // actually exported at the declared path in the IR. This catches
                    // C1 (unexported function) and C2 (wrong definition selected) early
                    // so codegen never emits an unresolvable use statement.
                    let all_calls = std::iter::once(("_default", &e2e_config.call))
                        .chain(e2e_config.calls.iter().map(|(k, v)| (k.as_str(), v)));
                    for (call_name, call_config) in all_calls {
                        if call_config.function.is_empty() || call_config.module.is_empty() {
                            continue;
                        }
                        // Derive the Rust module path from the module field:
                        // replace hyphens with underscores to match rust_path convention.
                        let module_path = call_config.module.replace('-', "_");
                        let function_name = &call_config.function;
                        match alef_extract::validate_call_export(&api, &module_path, function_name) {
                            alef_extract::ExportValidation::Ok => {}
                            alef_extract::ExportValidation::NotFound { function } => {
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' was not found in the extracted API surface. \
                                 Check that it is declared `pub` and that its source file is listed in \
                                 [[crate.sources]] or [[crate.source_crates]]."
                                );
                            }
                            alef_extract::ExportValidation::WrongPath {
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

                    eprintln!("Generating e2e test suites...");
                    let files = alef_e2e::generate_e2e(resolved_cfg, e2e_config, None)?;
                    e2e_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, clean)?;
                    alef_e2e::format::run_formatters(&files, e2e_config);
                    for file in &files {
                        current_gen_paths.insert(base_dir.join(&file.path));
                    }
                }

                eprintln!("Generating API docs...");
                let docs_api = pipeline::extract(resolved_cfg, config_path, false)?;
                let doc_languages = resolve_doc_languages(resolved_cfg, None)?;
                let doc_files = alef_docs::generate_docs(&docs_api, resolved_cfg, &doc_languages, "docs/reference")?;
                let doc_count = pipeline::write_scaffold_files_with_overwrite(&doc_files, &base_dir, clean)?;
                for file in &doc_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                    if removed > 0 {
                        eprintln!("Removed {removed} stale alef-generated file(s)");
                    }
                }

                // Formatters run by default. They are best-effort: a missing
                // formatter or non-zero exit must not abort the pipeline.
                // Two passes when enabled:
                //  1. `format_generated` runs language-native defaults (cargo fmt,
                //     ruff format, mix format, oxfmt, etc.) on the freshly
                //     emitted files.
                //  2. `fmt_post_generate` runs any extra repo-configured
                //     `[lint.<lang>].format` commands (linters, custom passes).
                // Both are scoped to languages that actually regenerated this run.
                if !no_format && !changed_languages.is_empty() {
                    eprintln!("Formatting generated files...");
                    pipeline::format_generated(&bindings, resolved_cfg, &base_dir, Some(&changed_languages));

                    eprintln!("Running formatters...");
                    let changed_list: Vec<alef_core::config::Language> = changed_languages.iter().copied().collect();
                    pipeline::fmt_post_generate(resolved_cfg, &changed_list);
                }

                // Finalise per-file hashes after every formatter has run, so
                // `alef verify` can recompute the same hash from on-disk content.
                eprintln!("Finalising hashes...");
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash)?;

                grand_binding_count += binding_count;
                grand_stub_count += stub_count;
                grand_api_count += api_count;
                grand_scaffold_count += scaffold_count;
                grand_readme_count += readme_count;
                grand_e2e_count += e2e_count;
                grand_doc_count += doc_count;
            } // end for resolved_cfg in crates_to_process

            // Stamp alef.toml with the CLI version that produced this run.
            if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                tracing::warn!("could not update alef.toml version pin: {e}");
            }

            println!(
                "Done: {grand_binding_count} binding files, {grand_stub_count} stub files, {grand_api_count} API files, {grand_scaffold_count} scaffold files, {grand_readme_count} readme files, {grand_e2e_count} e2e files, {grand_doc_count} doc files"
            );
            Ok(())
        }
        Commands::Init { lang, format } => {
            eprintln!("Initializing alef project");
            if let Some(langs) = &lang {
                eprintln!("  Languages: {}", langs.join(", "));
            }
            pipeline::init(config_path, lang.clone())?;
            eprintln!("  Created alef.toml");

            // Load the generated config and bootstrap the project
            let (_workspace, resolved) = load_config(config_path)?;
            let resolved_cfg = &resolved[0];
            let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
            let base_dir = std::env::current_dir()?;

            // Extract API surface
            let api = pipeline::extract(resolved_cfg, config_path, false)?;
            let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

            // Generate bindings
            eprintln!("  Generating bindings...");
            let bindings = pipeline::generate(&api, resolved_cfg, &languages, false)?;
            let mut binding_count: usize = 0;
            let mut all_paths = std::collections::HashSet::new();
            for (lang_key, lang_files) in &bindings {
                for file in lang_files {
                    all_paths.insert(base_dir.join(&file.path));
                }
                let single = vec![(*lang_key, lang_files.clone())];
                binding_count += pipeline::write_files(&single, &base_dir)?;
            }

            // Scaffold package manifests and lint configs
            eprintln!("  Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages)?;
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
            for file in &scaffold_files {
                all_paths.insert(base_dir.join(&file.path));
            }

            // Format generated code only when --format is requested.
            if format {
                eprintln!("  Formatting...");
                pipeline::fmt_post_generate(resolved_cfg, &languages);
            }

            // Finalise per-file hashes after formatting.
            pipeline::finalize_hashes(&all_paths, &sources_hash)?;

            println!("Initialized: {binding_count} binding files, {scaffold_count} scaffold files");
            Ok(())
        }
        Commands::Migrate { path, write } => {
            let migrate_path = path.unwrap_or_else(|| cli.config.clone());
            let options = commands::migrate::MigrateOptions {
                path: migrate_path,
                write,
            };
            commands::migrate::run(options)?;
            Ok(())
        }
        Commands::E2e { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            // E2e operates on per-crate e2e config. Use first crate that has an
            // e2e section, or error if none has one. For multi-crate workspaces,
            // all crates with an e2e section are processed in the loop below.
            // The action dispatch still uses the first crate's e2e config for
            // non-Generate actions (Init, Scaffold, List, Validate) since those
            // are fixture-directory operations.
            let resolved_cfg = crates_to_process
                .iter()
                .find(|c| c.e2e.is_some())
                .copied()
                .unwrap_or_else(|| crates_to_process[0]);
            let e2e_config = resolved_cfg.e2e.as_ref().context("no [e2e] section in alef.toml")?;
            match action {
                E2eAction::Generate { lang, registry } => {
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
                        // When --registry is set, clone the e2e config and switch to
                        // registry dependency mode so generators emit version-based
                        // dependencies instead of local paths.
                        let effective_e2e_config;
                        let e2e_ref = if registry {
                            let mut cloned = this_e2e_config.clone();
                            cloned.dep_mode = alef_core::config::e2e::DependencyMode::Registry;
                            effective_e2e_config = cloned;
                            eprintln!("Generating e2e test apps (registry mode)...");
                            &effective_e2e_config
                        } else {
                            eprintln!("Generating e2e test suites...");
                            this_e2e_config
                        };
                        let languages = lang.as_deref();
                        let files = alef_e2e::generate_e2e(e2e_crate, e2e_ref, languages)?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

                        // Run per-language formatters
                        alef_e2e::format::run_formatters(&files, e2e_ref);

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash)?;

                        // Sweep orphan alef-generated files.  When a --lang filter is
                        // active, scope the sweep to only the per-language subdirectories
                        // that were regenerated — sweeping the full e2e root would delete
                        // other languages' files that were intentionally left on disk.
                        // Without a filter, sweep the entire e2e output root as before.
                        let e2e_output_root = if registry {
                            base_dir.join(&e2e_ref.registry.output)
                        } else {
                            base_dir.join(&e2e_ref.output)
                        };
                        let sweep_roots: Vec<PathBuf> = if lang.is_some() {
                            // Derive sweep roots from the top-level subdirectories of the
                            // e2e output root that appear in the generated file set.  Each
                            // generator writes into `<output>/<lang>/`, so taking the first
                            // two path components relative to the e2e root gives us the
                            // per-language directory.
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
                    Ok(())
                }
                E2eAction::Init => {
                    eprintln!("Initializing e2e fixtures directory...");
                    let created = alef_e2e::scaffold::init_fixtures(e2e_config, resolved_cfg)?;
                    for path in &created {
                        println!("  created {path}");
                    }
                    println!("Initialized {} file(s)", created.len());
                    Ok(())
                }
                E2eAction::Scaffold {
                    id,
                    category,
                    description,
                } => {
                    let path =
                        alef_e2e::scaffold::scaffold_fixture(e2e_config, resolved_cfg, &id, &category, &description)?;
                    println!("Created {path}");
                    Ok(())
                }
                E2eAction::List => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = alef_e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let groups = alef_e2e::fixture::group_fixtures(&fixtures);

                    println!("Fixtures: {} total", fixtures.len());
                    for group in &groups {
                        println!("  {}: {} fixture(s)", group.category, group.fixtures.len());
                    }
                    Ok(())
                }
                E2eAction::Validate => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    eprintln!("Validating fixtures in {}...", fixtures_dir.display());

                    // Schema validation
                    let mut all_errors = alef_e2e::validate::validate_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to validate fixtures from {}", fixtures_dir.display()))?;

                    // Semantic validation
                    let fixtures = alef_e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let semantic_errors =
                        alef_e2e::validate::validate_fixtures_semantic(&fixtures, e2e_config, &e2e_config.languages);
                    all_errors.extend(semantic_errors);

                    if all_errors.is_empty() {
                        println!("All fixtures are valid.");
                        Ok(())
                    } else {
                        use alef_e2e::validate::Severity;
                        let error_count = all_errors.iter().filter(|e| e.severity == Severity::Error).count();
                        let warning_count = all_errors.iter().filter(|e| e.severity == Severity::Warning).count();
                        println!("Found {} error(s) and {} warning(s):", error_count, warning_count);
                        for err in &all_errors {
                            println!("  {err}");
                        }
                        if error_count > 0 {
                            process::exit(1);
                        }
                        Ok(())
                    }
                }
            }
        }
        Commands::Publish { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            match action {
                PublishAction::Prepare { lang, target, dry_run } => {
                    let rust_target = target
                        .as_deref()
                        .map(alef_publish::platform::RustTarget::parse)
                        .transpose()?;
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        if multi {
                            eprintln!(
                                "[{}] Preparing publish for: {}",
                                resolved_cfg.name,
                                format_languages(&languages)
                            );
                        } else {
                            eprintln!("Preparing publish for: {}", format_languages(&languages));
                        }
                        alef_publish::prepare(resolved_cfg, &languages, rust_target.as_ref(), dry_run)?;
                    }
                    println!("Prepare complete");
                    Ok(())
                }
                PublishAction::Build {
                    lang,
                    target,
                    use_cross,
                } => {
                    let rust_target = target
                        .as_deref()
                        .map(alef_publish::platform::RustTarget::parse)
                        .transpose()?;
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        if multi {
                            eprintln!(
                                "[{}] Building publish artifacts for: {}",
                                resolved_cfg.name,
                                format_languages(&languages)
                            );
                        } else {
                            eprintln!("Building publish artifacts for: {}", format_languages(&languages));
                        }
                        alef_publish::build(resolved_cfg, &languages, rust_target.as_ref(), use_cross)?;
                    }
                    println!("Build complete");
                    Ok(())
                }
                PublishAction::Package {
                    lang,
                    target,
                    output,
                    version,
                    dry_run,
                    php_version,
                    php_ts,
                    php_libc,
                    windows_compiler,
                } => {
                    let rust_target = target
                        .as_deref()
                        .map(alef_publish::platform::RustTarget::parse)
                        .transpose()?;
                    let output_dir = std::path::Path::new(&output);
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        let ver = version
                            .clone()
                            .or_else(|| resolved_cfg.resolved_version())
                            .context("could not determine version — set --version or version_from in alef.toml")?;

                        // Build PHP-specific options when any PHP language is in the list.
                        let needs_php = languages.contains(&alef_core::config::Language::Php);
                        let pie_opts: Option<alef_publish::package::php::PiePackageOptions<'_>> = if needs_php {
                            let php_ver = php_version
                                .as_deref()
                                .context("--php-version is required when packaging --lang php")?;
                            let ts_mode = alef_publish::package::php::TsMode::parse(&php_ts)?;
                            // Validate: Windows target requires --windows-compiler.
                            if let Some(ref rt) = rust_target {
                                if rt.os == alef_publish::platform::Os::Windows && windows_compiler.is_none() {
                                    anyhow::bail!(
                                        "--windows-compiler is required when packaging PHP for a Windows target"
                                    );
                                }
                            }
                            Some(alef_publish::package::php::PiePackageOptions {
                                php_version: php_ver,
                                ts_mode,
                                libc_override: php_libc.as_deref(),
                                windows_compiler: windows_compiler.as_deref(),
                            })
                        } else {
                            None
                        };

                        let pkg_options = alef_publish::PackageOptions { php: pie_opts };

                        if multi {
                            eprintln!(
                                "[{}] Packaging {} (v{ver}) for: {}",
                                resolved_cfg.name,
                                output_dir.display(),
                                format_languages(&languages)
                            );
                        } else {
                            eprintln!(
                                "Packaging {} (v{ver}) for: {}",
                                output_dir.display(),
                                format_languages(&languages)
                            );
                        }
                        alef_publish::package(
                            resolved_cfg,
                            &languages,
                            rust_target.as_ref(),
                            output_dir,
                            &ver,
                            dry_run,
                            &pkg_options,
                        )?;
                    }
                    println!("Package complete");
                    Ok(())
                }
                PublishAction::Validate => {
                    let mut all_issues: Vec<String> = Vec::new();
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, None)?;
                        let issues = alef_publish::validate(resolved_cfg, &languages)?;
                        all_issues.extend(issues);
                    }
                    if all_issues.is_empty() {
                        println!("All package manifests are consistent");
                    } else {
                        eprintln!("Validation issues:");
                        for issue in &all_issues {
                            eprintln!("  - {issue}");
                        }
                        anyhow::bail!("{} validation issue(s) found", all_issues.len());
                    }
                    Ok(())
                }
            }
        }
        Commands::Cache { action } => match action {
            CacheAction::Clear => {
                cache::clear_cache()?;
                println!("Cache cleared.");
                Ok(())
            }
            CacheAction::Status => {
                cache::show_status();
                Ok(())
            }
        },
        Commands::Validate { action } => match action {
            ValidateAction::Versions { json, exit_code } => {
                let (_workspace, resolved) = load_config(config_path)?;
                let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
                let workspace_root = std::env::current_dir()?;
                let mut has_mismatches = false;
                for resolved_cfg in &crates_to_process {
                    let checks = commands::validate_versions::run(resolved_cfg, &workspace_root, json)?;
                    if checks.iter().any(|c| !c.matches) {
                        has_mismatches = true;
                    }
                }
                if has_mismatches && exit_code {
                    process::exit(1);
                }
                Ok(())
            }
        },
        Commands::ReleaseMetadata {
            tag,
            targets,
            git_ref,
            event,
            dry_run,
            force_republish,
            json: _,
        } => {
            // Sniff event from env when not provided.
            let effective_event = if event.is_empty() {
                std::env::var("GITHUB_EVENT_NAME").unwrap_or_default()
            } else {
                event.clone()
            };
            let resolved_opt = load_config(config_path).ok().map(|(_ws, r)| r);
            // For release metadata, use the first crate matching the filter (or first crate overall).
            // This command emits a single JSON object per invocation; multi-crate is an
            // unusual case. If the user needs per-crate metadata they can filter with --crate.
            let resolved_cfg_opt: Option<&alef_core::config::ResolvedCrateConfig> =
                resolved_opt.as_ref().and_then(|r| {
                    dispatch::select_crates(r, &cli.crate_filter)
                        .ok()
                        .and_then(|v| v.into_iter().next())
                });
            let meta = commands::release_metadata::compute(
                &tag,
                &targets,
                git_ref.as_deref(),
                &effective_event,
                dry_run,
                force_republish,
                resolved_cfg_opt,
            )?;
            println!("{}", meta.to_json()?);
            Ok(())
        }
        Commands::CheckRegistry {
            registry,
            package,
            version,
            tap_repo,
            repo,
            source,
            asset_prefix,
            required_assets,
            json,
        } => {
            let extra = commands::check_registry::ExtraParams {
                nuget_source: source,
                tap_repo,
                repo,
                asset_prefix,
                required_assets,
            };
            commands::check_registry::check(registry, &package, &version, &extra, json)?;
            Ok(())
        }
        Commands::GoTag {
            version,
            remote,
            dry_run,
            json,
        } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let workspace_root = std::env::current_dir()?;
            for resolved_cfg in &crates_to_process {
                let params = commands::go_tag::GoTagParams {
                    version: &version,
                    remote: &remote,
                    dry_run,
                    output_json: json,
                    config: resolved_cfg,
                    workspace_root: &workspace_root,
                };
                commands::go_tag::run(&params)?;
            }
            Ok(())
        }
    }
}

fn init_tracing(verbose: u8, quiet: bool, no_color: bool) {
    use tracing_subscriber::EnvFilter;
    let default_level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(!no_color)
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();
}

/// Load and resolve an alef.toml, returning the workspace-level config and
/// the per-crate resolved configs.  Detects legacy schema and returns an error
/// with a migration hint rather than a confusing parse error.
fn load_config(
    path: &std::path::Path,
) -> Result<(
    alef_core::config::WorkspaceConfig,
    Vec<alef_core::config::ResolvedCrateConfig>,
)> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config: {}", path.display()))?;
    alef_core::config::detect_legacy_keys(&content).with_context(|| {
        format!(
            "legacy schema detected in {} — run `alef migrate` to update automatically",
            path.display()
        )
    })?;
    let cfg: alef_core::config::NewAlefConfig =
        toml::from_str(&content).with_context(|| format!("Failed to parse alef.toml ({})", path.display()))?;
    let resolved = cfg
        .resolve()
        .with_context(|| format!("failed to resolve crates in {}", path.display()))?;
    Ok((cfg.workspace, resolved))
}

fn resolve_languages(
    config: &alef_core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef_core::config::Language>> {
    resolve_languages_inner(config, filter, false)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
/// Docs can always be generated for Rust since it's the source language.
fn resolve_doc_languages(
    config: &alef_core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef_core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

fn resolve_languages_inner(
    config: &alef_core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    allow_rust: bool,
) -> Result<Vec<alef_core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang: alef_core::config::Language = toml::Value::String(lang_str.clone())
                    .try_into()
                    .with_context(|| format!("Unknown language: {lang_str}"))?;
                if config.languages.contains(&lang) || (allow_rust && lang == alef_core::config::Language::Rust) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if allow_rust && !langs.contains(&alef_core::config::Language::Rust) {
                langs.push(alef_core::config::Language::Rust);
            }
            Ok(langs)
        }
    }
}

fn format_languages(languages: &[alef_core::config::Language]) -> String {
    languages.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
}

/// Multi-crate variant of [`verify_walk`].
///
/// A file is considered valid if its embedded `alef:hash:` matches the hash
/// computed using ANY of the provided `sources_hashes`.  In a multi-crate
/// workspace each file was generated by exactly one crate, so the file passes
/// verification when it matches its generating crate's hash.
fn verify_walk_multi(base_dir: &std::path::Path, sources_hashes: &[String]) -> anyhow::Result<Vec<String>> {
    if sources_hashes.is_empty() {
        return Ok(Vec::new());
    }
    if sources_hashes.len() == 1 {
        return verify_walk(base_dir, &sources_hashes[0]);
    }

    const SKIP_DIRS: &[&str] = &[
        ".git",
        ".alef",
        "target",
        "node_modules",
        "_build",
        "deps",
        "parsers",
        "dist",
        "dist-node",
        "vendor",
        ".venv",
        ".cache",
        ".remote-cache",
        "__pycache__",
        "build",
        "tmp",
        "out",
        ".idea",
        ".vscode",
    ];
    const SCAN_EXTENSIONS: &[&str] = &[
        "rs", "py", "pyi", "ts", "tsx", "js", "mjs", "cjs", "rb", "rbs", "php", "phpstub", "go", "java", "cs", "ex",
        "exs", "R", "r", "toml", "json", "md", "h", "c", "yaml", "yml",
    ];

    let mut stale: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![base_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&name) || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| SCAN_EXTENSIONS.iter().any(|allowed| allowed.eq_ignore_ascii_case(e)))
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let Some(disk_hash) = alef_core::hash::extract_hash(&content) else {
                continue;
            };
            // A file is valid if it matches ANY crate's sources hash.
            let valid = sources_hashes
                .iter()
                .any(|sh| alef_core::hash::compute_file_hash(sh, &content) == disk_hash);
            if !valid {
                stale.push(path.display().to_string());
            }
        }
    }

    stale.sort();
    Ok(stale)
}

/// Walk the consumer's repo from `base_dir`, find every alef-headered file, and
/// return the list of stale ones — where
/// `compute_file_hash(sources_hash, on_disk_content)` doesn't match the
/// embedded `alef:hash:` line.
///
/// Skips obvious build/cache directories (`target/`, `node_modules/`, `_build/`,
/// `.alef/`, `parsers/`, `dist/`, `vendor/`, `.git/`) so verify stays fast on
/// large repos. Files without the alef header marker are skipped silently —
/// those are user-owned (scaffold-once Cargo.toml templates, composer.json,
/// gemspec, package.json, lockfiles, etc.) and alef has no claim.
fn verify_walk(base_dir: &std::path::Path, sources_hash: &str) -> anyhow::Result<Vec<String>> {
    const SKIP_DIRS: &[&str] = &[
        ".git",
        ".alef",
        "target",
        "node_modules",
        "_build",
        "deps",
        "parsers",
        "dist",
        "dist-node",
        "vendor",
        ".venv",
        ".cache",
        ".remote-cache",
        "__pycache__",
        "build",
        "tmp",
        "out",
        ".idea",
        ".vscode",
    ];

    // Only scan files alef plausibly emits. The check is cheap (extension
    // match + read-first-10-lines), but constraining the set keeps the walk
    // O(generated files) instead of O(every file in the repo).
    const SCAN_EXTENSIONS: &[&str] = &[
        "rs", "py", "pyi", "ts", "tsx", "js", "mjs", "cjs", "rb", "rbs", "php", "phpstub", "go", "java", "cs", "ex",
        "exs", "R", "r", "toml", "json", "md", "h", "c", "yaml", "yml",
    ];

    let mut stale: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![base_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if SKIP_DIRS.contains(&name) || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| SCAN_EXTENSIONS.iter().any(|allowed| allowed.eq_ignore_ascii_case(e)))
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let Some(disk_hash) = alef_core::hash::extract_hash(&content) else {
                continue;
            };
            // Recompute the per-file hash from the on-disk byte content.
            // `compute_file_hash` strips the existing `alef:hash:` line so the
            // computation is symmetric with the post-format finalisation in
            // `pipeline::finalize_hashes`.
            let expected = alef_core::hash::compute_file_hash(sources_hash, &content);
            if disk_hash != expected {
                stale.push(path.display().to_string());
            }
        }
    }

    stale.sort();
    Ok(stale)
}
