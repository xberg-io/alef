use alef_core::hash;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

mod cache;
mod pipeline;
mod registry;

#[derive(Parser)]
#[command(name = "alef", about = "Opinionated polyglot binding generator")]
struct Cli {
    /// Path to alef.toml config file.
    #[arg(long, default_value = "alef.toml")]
    config: PathBuf,

    /// Maximum parallel jobs (0 = all cores, 1 = sequential).
    #[arg(short, long, default_value = "0", global = true)]
    jobs: usize,

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
        /// Skip post-generation formatting of emitted files.
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
    },
    /// Initialize a new alef.toml config.
    Init {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
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

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

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
            let config = load_config(config_path)?;
            let api = pipeline::extract(&config, config_path, false)?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, serde_json::to_string_pretty(&api)?)?;
            println!("Wrote IR to {}", output.display());
            Ok(())
        }
        Commands::Generate { lang, clean, no_format } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Generating bindings for: {}", format_languages(&languages));
            let api = pipeline::extract(&config, config_path, clean)?;
            let files = pipeline::generate(&api, &config, &languages, clean)?;
            let base_dir = std::env::current_dir()?;

            // Collect all files generated in this run for cleanup pass
            let mut current_gen_paths = std::collections::HashSet::new();

            // For each language: compute content hashes, compare against stored
            // hashes, write only when something changed.
            let mut total_written: usize = 0;
            let mut any_written = false;
            for (lang, lang_files) in &files {
                let lang_str = lang.to_string();

                // Track all generated paths for cleanup BEFORE any skip-on-up-to-date check.
                // The cleanup pass uses this set to know which files the current run *would*
                // produce — including unchanged files that we skip writing.
                for file in lang_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                let hashes: Vec<(String, String)> = lang_files
                    .iter()
                    .map(|f| {
                        let normalized = pipeline::normalize_content(&f.path, &f.content);
                        let content_hash = hash::hash_content(&normalized);
                        let final_content = hash::inject_hash_line(&normalized, &content_hash);
                        (
                            base_dir.join(&f.path).display().to_string(),
                            cache::hash_content(&final_content),
                        )
                    })
                    .collect();

                let stored = cache::read_generation_hashes(&lang_str).unwrap_or_default();
                let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                if all_match && !clean {
                    eprintln!("  [{lang_str}] up to date (skipping)");
                    continue;
                }

                // Write all files for this language and store updated hashes.
                let single = vec![(*lang, lang_files.clone())];
                let written = pipeline::write_files(&single, &base_dir)?;
                total_written += written;
                any_written = true;
                let _ = cache::write_generation_hashes(&lang_str, &hashes);
            }

            // Generate public API wrappers
            if config.generate.public_api {
                let public_api_files = pipeline::generate_public_api(&api, &config, &languages)?;
                if !public_api_files.is_empty() {
                    let api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                    eprintln!("Generated {api_count} public API files");

                    // Track public API files for cleanup
                    for (_, files) in &public_api_files {
                        for file in files {
                            current_gen_paths.insert(base_dir.join(&file.path));
                        }
                    }
                }
            }

            // Generate type stubs (e.g., .pyi for Python, .d.ts for TypeScript)
            let stub_files = pipeline::generate_stubs(&api, &config, &languages)?;
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

                let stored_stubs = cache::read_generation_hashes("stubs").unwrap_or_default();
                let stubs_match =
                    !stub_hashes.is_empty() && stub_hashes.iter().all(|(p, h)| stored_stubs.get(p) == Some(h));

                if !stubs_match || clean {
                    let stub_count = pipeline::write_files(&stub_files, &base_dir)?;
                    eprintln!("Generated {stub_count} type stub files");
                    any_written = true;
                    let _ = cache::write_generation_hashes("stubs", &stub_hashes);

                    // Track stub files for cleanup
                    for (_, files) in &stub_files {
                        for file in files {
                            current_gen_paths.insert(base_dir.join(&file.path));
                        }
                    }
                } else {
                    eprintln!("  [stubs] up to date (skipping)");
                }
            }

            // Clean up orphaned alef-generated files
            if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                if removed > 0 {
                    eprintln!("Removed {removed} stale alef-generated file(s)");
                }
            }

            if any_written && !no_format {
                // Auto-format generated files using language-native formatters
                // (ruff, mix format, cargo fmt, etc.). This ensures CI formatter
                // checks pass without requiring users to run formatters manually.
                // Formatting failures are logged as warnings and do not fail the
                // generate command, since formatter quirks shouldn't block codegen.
                eprintln!("Formatting generated files...");
                pipeline::format_generated(&files, &config, &base_dir);
            }

            if any_written {
                // Format generated files using configured formatters.
                // Generation hashes are already stored from in-memory content
                // (pre-formatter), so formatter modifications don't affect
                // staleness detection. `alef verify` compares generation-to-
                // generation, never consulting on-disk state.
                //
                // Post-generation formatting is best-effort: formatters are
                // expected to modify files, and a missing tool / non-zero
                // exit must not abort the generate run. Failures are logged
                // and skipped per-language.
                pipeline::fmt_post_generate(&config, &languages);
            }

            // Always re-sync versions across user-owned manifests (gemspec,
            // composer.json, package.json, *.csproj, mix.exs, ...). These are
            // scaffold-once files alef can't safely overwrite, but their version
            // strings must track Cargo.toml or `alef verify` flags them as stale.
            // Running sync after every generate makes verify a true successor of
            // generate without the consumer needing a second `alef sync-versions`.
            if let Err(e) = pipeline::sync_versions(&config, config_path, None) {
                tracing::warn!("version sync failed: {e}");
            }

            println!("Generated {total_written} files");
            Ok(())
        }
        Commands::Stubs { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Generating type stubs for: {}", format_languages(&languages));
            let api = pipeline::extract(&config, config_path, false)?;
            let files = pipeline::generate_stubs(&api, &config, &languages)?;
            let base_dir = std::env::current_dir()?;

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

            let stored = cache::read_generation_hashes("stubs").unwrap_or_default();
            let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

            if all_match {
                println!("Stubs up to date (skipping)");
                return Ok(());
            }

            let count = pipeline::write_files(&files, &base_dir)?;
            let _ = cache::write_generation_hashes("stubs", &hashes);
            println!("Generated {count} stub files");
            Ok(())
        }
        Commands::Scaffold { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let api = pipeline::extract(&config, config_path, false)?;
            let ir_json = serde_json::to_string(&api)?;
            let stage_hash = cache::compute_stage_hash(&ir_json, "scaffold", &config_toml, &[]);
            if cache::is_stage_cached("scaffold", &stage_hash) {
                println!("Scaffold up to date (cached)");
                return Ok(());
            }
            eprintln!("Generating scaffolding for: {}", format_languages(&languages));
            let files = pipeline::scaffold(&api, &config, &languages)?;
            let base_dir = std::env::current_dir()?;
            let count = pipeline::write_scaffold_files(&files, &base_dir)?;
            let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
            cache::write_stage_hash("scaffold", &stage_hash, &output_paths)?;
            println!("Generated {count} scaffold files");
            Ok(())
        }
        Commands::Readme { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let api = pipeline::extract(&config, config_path, false)?;
            let ir_json = serde_json::to_string(&api)?;
            let stage_hash = cache::compute_stage_hash(&ir_json, "readme", &config_toml, &[]);
            if cache::is_stage_cached("readme", &stage_hash) {
                println!("READMEs up to date (cached)");
                return Ok(());
            }
            eprintln!("Generating READMEs for: {}", format_languages(&languages));
            let files = pipeline::readme(&api, &config, &languages)?;
            let base_dir = std::env::current_dir()?;
            let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
            let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
            cache::write_stage_hash("readme", &stage_hash, &output_paths)?;
            println!("Generated {count} README files");
            Ok(())
        }
        Commands::Docs { lang, output } => {
            let config = load_config(config_path)?;
            let languages = resolve_doc_languages(&config, lang.as_deref())?;
            let config_toml = std::fs::read_to_string(config_path)?;
            // Use filtered IR so docs only cover the public API surface.
            let api = pipeline::extract(&config, config_path, false)?;
            let ir_json = serde_json::to_string(&api)?;
            let stage_hash = cache::compute_stage_hash(&ir_json, "docs", &config_toml, &[]);
            if cache::is_stage_cached("docs", &stage_hash) {
                println!("API docs up to date (cached)");
                return Ok(());
            }
            eprintln!("Generating API docs for: {}", format_languages(&languages));
            let files = alef_docs::generate_docs(&api, &config, &languages, &output)?;
            let base_dir = std::env::current_dir()?;
            let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
            let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
            cache::write_stage_hash("docs", &stage_hash, &output_paths)?;
            println!("Generated {count} API doc files");
            Ok(())
        }
        Commands::SyncVersions { bump, set } => {
            let config = load_config(config_path)?;
            if let Some(version) = &set {
                eprintln!("Setting version to {version}");
                pipeline::set_version(&config, version)?;
            }
            eprintln!("Syncing versions from Cargo.toml");
            pipeline::sync_versions(&config, config_path, bump.as_deref())?;
            println!("Version sync complete");
            Ok(())
        }
        Commands::Build { lang, release } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            let profile = if release { "release" } else { "dev" };
            eprintln!("Building bindings ({profile}) for: {}", format_languages(&languages));
            pipeline::build(&config, &languages, release)?;
            println!("Build complete");
            Ok(())
        }
        Commands::Fmt { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Formatting generated output for: {}", format_languages(&languages));
            pipeline::fmt(&config, &languages)?;
            println!("Format complete");
            Ok(())
        }
        Commands::Lint { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Linting generated output for: {}", format_languages(&languages));
            pipeline::lint(&config, &languages)?;
            println!("Lint complete");
            Ok(())
        }
        Commands::Test { lang, e2e, coverage } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Running tests for: {}", format_languages(&languages));
            if e2e {
                eprintln!("  (with e2e tests)");
            }
            if coverage {
                eprintln!("  (with coverage)");
            }
            pipeline::test(&config, &languages, e2e, coverage)?;
            println!("Tests complete");
            Ok(())
        }
        Commands::Setup { lang, timeout } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Setting up dependencies for: {}", format_languages(&languages));
            pipeline::setup(&config, &languages, timeout)?;
            println!("Setup complete");
            Ok(())
        }
        Commands::Clean { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Cleaning build artifacts for: {}", format_languages(&languages));
            pipeline::clean(&config, &languages)?;
            println!("Clean complete");
            Ok(())
        }
        Commands::Update { lang, latest } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            let mode = if latest { "latest" } else { "compatible" };
            eprintln!("Updating dependencies ({mode}) for: {}", format_languages(&languages));
            pipeline::update(&config, &languages, latest)?;
            println!("Update complete");
            Ok(())
        }
        Commands::Verify {
            exit_code,
            compile,
            lint,
            lang,
        } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Verifying bindings for: {}", format_languages(&languages));
            if compile {
                eprintln!("  (with compilation check)");
            }
            if lint {
                eprintln!("  (with lint check)");
            }

            // Regenerate everything in-memory once — shared across all checks.
            let api = pipeline::extract(&config, config_path, false)?;
            let base_dir = std::env::current_dir()?;

            let mut all_stale: Vec<String> = Vec::new();

            // Verify by regenerating in-memory, normalizing (same as write_files),
            // hashing, then comparing against the hash embedded in the on-disk
            // file header.  No external cache needed — the file is its own proof.
            let bindings = pipeline::generate(&api, &config, &languages, true)?;
            for (lang, lang_files) in &bindings {
                let lang_str = lang.to_string();
                for f in lang_files {
                    let full_path = base_dir.join(&f.path);
                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                    let expected_hash = alef_core::hash::hash_content(&normalized);
                    verify_file(&full_path, &expected_hash, &lang_str, &mut all_stale);
                }
            }

            // Verify stubs the same way.
            let stubs = pipeline::generate_stubs(&api, &config, &languages)?;
            for (_, stub_files) in &stubs {
                for f in stub_files {
                    let full_path = base_dir.join(&f.path);
                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                    let expected_hash = alef_core::hash::hash_content(&normalized);
                    verify_file(&full_path, &expected_hash, "stubs", &mut all_stale);
                }
            }

            // Verify READMEs the same way.
            let readmes = pipeline::readme(&api, &config, &languages)?;
            for f in &readmes {
                let full_path = base_dir.join(&f.path);
                let normalized = pipeline::normalize_content(&f.path, &f.content);
                let expected_hash = alef_core::hash::hash_content(&normalized);
                verify_file(&full_path, &expected_hash, "readme", &mut all_stale);
            }

            // Also verify version consistency across all package manifests
            let version_mismatches = pipeline::verify_versions(&config)?;
            let has_version_issues = !version_mismatches.is_empty();
            if has_version_issues {
                println!("Version mismatches detected:");
                for mismatch in &version_mismatches {
                    println!("  {mismatch}");
                }
            }

            if all_stale.is_empty() && !has_version_issues {
                println!("All bindings and versions are up to date.");
            } else {
                if !all_stale.is_empty() {
                    println!("Stale bindings detected:");
                    for s in &all_stale {
                        println!("  {s}");
                    }
                }
                if exit_code && (!all_stale.is_empty() || has_version_issues) {
                    process::exit(1);
                }
            }
            Ok(())
        }
        Commands::Diff { exit_code } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, None)?;
            eprintln!("Computing diff of generated bindings...");

            let api = pipeline::extract(&config, config_path, false)?;
            let bindings = pipeline::generate(&api, &config, &languages, true)?;
            let stubs = pipeline::generate_stubs(&api, &config, &languages)?;

            let base_dir = std::env::current_dir()?;
            let mut all_diffs = pipeline::diff_files(&bindings, &base_dir)?;
            all_diffs.extend(pipeline::diff_files(&stubs, &base_dir)?);

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
        Commands::All { clean } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, None)?;
            eprintln!("Running all for: {}", format_languages(&languages));

            let api = pipeline::extract(&config, config_path, clean)?;
            let base_dir = std::env::current_dir()?;

            // Collect all files generated in this run for cleanup pass
            let mut current_gen_paths = std::collections::HashSet::new();

            eprintln!("Generating bindings...");
            let bindings = pipeline::generate(&api, &config, &languages, clean)?;

            // Per-language: hash content, skip writing if all hashes match.
            let mut binding_count: usize = 0;
            for (lang, lang_files) in &bindings {
                let lang_str = lang.to_string();

                // Track all generated paths for cleanup BEFORE the skip-if-up-to-date check,
                // so unchanged but legitimate files are not deleted as orphans.
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

                let stored = cache::read_generation_hashes(&lang_str).unwrap_or_default();
                let all_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                if all_match && !clean {
                    eprintln!("  [{lang_str}] up to date (skipping)");
                    continue;
                }

                let single = vec![(*lang, lang_files.clone())];
                binding_count += pipeline::write_files(&single, &base_dir)?;
                let _ = cache::write_generation_hashes(&lang_str, &hashes);
            }

            eprintln!("Generating type stubs...");
            let stubs = pipeline::generate_stubs(&api, &config, &languages)?;

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
            let stored_stubs = cache::read_generation_hashes("stubs").unwrap_or_default();
            let stubs_match =
                !stub_hashes.is_empty() && stub_hashes.iter().all(|(p, h)| stored_stubs.get(p) == Some(h));

            let stub_count = if !stubs_match || clean {
                let count = pipeline::write_files(&stubs, &base_dir)?;
                let _ = cache::write_generation_hashes("stubs", &stub_hashes);
                count
            } else {
                eprintln!("  [stubs] up to date (skipping)");
                0
            };

            // Track stub paths for cleanup regardless of whether they were just rewritten;
            // up-to-date stubs are still legitimate output of this run.
            for (_, files) in &stubs {
                for file in files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }
            }

            // Generate public API wrappers
            let mut api_count = 0;
            if config.generate.public_api {
                let public_api_files = pipeline::generate_public_api(&api, &config, &languages)?;
                if !public_api_files.is_empty() {
                    api_count = pipeline::write_files(&public_api_files, &base_dir)?;

                    // Track public API files for cleanup
                    for (_, files) in &public_api_files {
                        for file in files {
                            current_gen_paths.insert(base_dir.join(&file.path));
                        }
                    }
                }
            }

            eprintln!("Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, &config, &languages)?;
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
            for file in &scaffold_files {
                current_gen_paths.insert(base_dir.join(&file.path));
            }

            eprintln!("Generating READMEs...");
            let readme_files = pipeline::readme(&api, &config, &languages)?;
            let readme_count = pipeline::write_scaffold_files_with_overwrite(&readme_files, &base_dir, clean)?;
            for file in &readme_files {
                current_gen_paths.insert(base_dir.join(&file.path));
            }

            // Generate e2e tests if [e2e] section is present in config
            let mut e2e_count = 0;
            if let Some(e2e_config) = &config.e2e {
                eprintln!("Generating e2e test suites...");
                let files = alef_e2e::generate_e2e(&config, e2e_config, None)?;
                e2e_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, clean)?;
                alef_e2e::format::run_formatters(&files, e2e_config);
                for file in &files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }
            }

            // Generate API docs using filtered IR so docs match the public API surface.
            eprintln!("Generating API docs...");
            let docs_api = pipeline::extract(&config, config_path, false)?;
            let doc_languages = resolve_doc_languages(&config, None)?;
            let doc_files = alef_docs::generate_docs(&docs_api, &config, &doc_languages, "docs/reference")?;
            let doc_count = pipeline::write_scaffold_files_with_overwrite(&doc_files, &base_dir, clean)?;
            for file in &doc_files {
                current_gen_paths.insert(base_dir.join(&file.path));
            }

            // Clean up orphaned alef-generated files
            if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                if removed > 0 {
                    eprintln!("Removed {removed} stale alef-generated file(s)");
                }
            }

            // Format all generated files using configured formatters.
            // Best-effort: a missing formatter or non-zero exit must not
            // abort the orchestrated pipeline.
            eprintln!("Running formatters...");
            pipeline::fmt_post_generate(&config, &languages);

            // Update input hashes AFTER formatting. Formatters may have modified files
            // so the input hash recorded during generation is stale. Re-load config
            // from disk and re-hash so `alef verify` sees consistent values.
            // Generation content hashes were stored before formatting — no need to recompute.
            eprintln!("Updating input hashes...");
            let post_config_struct = load_config(config_path)?;
            let post_api = pipeline::extract(&post_config_struct, config_path, true)?;
            let post_ir = serde_json::to_string(&post_api)?;
            let post_config = toml::to_string(&post_config_struct).unwrap_or_default();
            for lang in &languages {
                let lang_str = lang.to_string();
                let lang_hash = cache::compute_lang_hash(&post_ir, &lang_str, &post_config);
                if let Ok(paths) = cache::read_manifest_paths(&lang_str) {
                    let _ = cache::write_lang_hash(&lang_str, &lang_hash, &paths);
                }
            }
            let post_stubs_hash = cache::compute_stage_hash(&post_ir, "stubs", &post_config, &[]);
            if let Ok(paths) = cache::read_manifest_paths("stubs") {
                let _ = cache::write_stage_hash("stubs", &post_stubs_hash, &paths);
            }

            println!(
                "Done: {binding_count} binding files, {stub_count} stub files, {api_count} API files, {scaffold_count} scaffold files, {readme_count} readme files, {e2e_count} e2e files, {doc_count} doc files"
            );
            Ok(())
        }
        Commands::Init { lang } => {
            eprintln!("Initializing alef project");
            if let Some(langs) = &lang {
                eprintln!("  Languages: {}", langs.join(", "));
            }
            pipeline::init(config_path, lang.clone())?;
            eprintln!("  Created alef.toml");

            // Load the generated config and bootstrap the project
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            let base_dir = std::env::current_dir()?;

            // Extract API surface
            let api = pipeline::extract(&config, config_path, false)?;

            // Generate bindings
            eprintln!("  Generating bindings...");
            let bindings = pipeline::generate(&api, &config, &languages, false)?;
            let mut binding_count: usize = 0;
            for (lang_key, lang_files) in &bindings {
                let single = vec![(*lang_key, lang_files.clone())];
                binding_count += pipeline::write_files(&single, &base_dir)?;
            }

            // Scaffold package manifests and lint configs
            eprintln!("  Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, &config, &languages)?;
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;

            // Format generated code (best-effort).
            eprintln!("  Formatting...");
            pipeline::fmt_post_generate(&config, &languages);

            println!("Initialized: {binding_count} binding files, {scaffold_count} scaffold files");
            Ok(())
        }
        Commands::E2e { action } => {
            let config = load_config(config_path)?;
            let e2e_config = config.e2e.as_ref().context("no [e2e] section in alef.toml")?;
            match action {
                E2eAction::Generate { lang, registry } => {
                    let config_toml = std::fs::read_to_string(config_path)?;
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                    let api = pipeline::extract(&config, config_path, false)?;
                    let ir_json = serde_json::to_string(&api)?;
                    let cache_key = if registry { "e2e-registry" } else { "e2e" };
                    let stage_hash = cache::compute_stage_hash(&ir_json, cache_key, &config_toml, &fixture_hash);
                    if cache::is_stage_cached(cache_key, &stage_hash) {
                        println!("E2E tests up to date (cached)");
                        return Ok(());
                    }
                    // When --registry is set, clone the e2e config and switch to
                    // registry dependency mode so generators emit version-based
                    // dependencies instead of local paths.
                    let effective_e2e_config;
                    let e2e_ref = if registry {
                        let mut cloned = e2e_config.clone();
                        cloned.dep_mode = alef_core::config::e2e::DependencyMode::Registry;
                        effective_e2e_config = cloned;
                        eprintln!("Generating e2e test apps (registry mode)...");
                        &effective_e2e_config
                    } else {
                        eprintln!("Generating e2e test suites...");
                        e2e_config
                    };
                    let languages = lang.as_deref();
                    let files = alef_e2e::generate_e2e(&config, e2e_ref, languages)?;
                    let base_dir = std::env::current_dir()?;
                    let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

                    // Run per-language formatters
                    alef_e2e::format::run_formatters(&files, e2e_ref);

                    let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                    cache::write_stage_hash(cache_key, &stage_hash, &output_paths)?;
                    println!("Generated {count} e2e files");
                    Ok(())
                }
                E2eAction::Init => {
                    eprintln!("Initializing e2e fixtures directory...");
                    let created = alef_e2e::scaffold::init_fixtures(e2e_config, &config)?;
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
                    let path = alef_e2e::scaffold::scaffold_fixture(e2e_config, &config, &id, &category, &description)?;
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
            let config = load_config(config_path)?;
            match action {
                PublishAction::Prepare { lang, target, dry_run } => {
                    let languages = resolve_languages(&config, lang.as_deref())?;
                    let rust_target = target
                        .as_deref()
                        .map(alef_publish::platform::RustTarget::parse)
                        .transpose()?;
                    eprintln!("Preparing publish for: {}", format_languages(&languages));
                    alef_publish::prepare(&config, &languages, rust_target.as_ref(), dry_run)?;
                    println!("Prepare complete");
                    Ok(())
                }
                PublishAction::Build {
                    lang,
                    target,
                    use_cross,
                } => {
                    let languages = resolve_languages(&config, lang.as_deref())?;
                    let rust_target = target
                        .as_deref()
                        .map(alef_publish::platform::RustTarget::parse)
                        .transpose()?;
                    eprintln!("Building publish artifacts for: {}", format_languages(&languages));
                    alef_publish::build(&config, &languages, rust_target.as_ref(), use_cross)?;
                    println!("Build complete");
                    Ok(())
                }
                PublishAction::Package {
                    lang,
                    target,
                    output,
                    version,
                    dry_run,
                } => {
                    let languages = resolve_languages(&config, lang.as_deref())?;
                    let rust_target = target
                        .as_deref()
                        .map(alef_publish::platform::RustTarget::parse)
                        .transpose()?;
                    let ver = version
                        .or_else(|| config.resolved_version())
                        .context("could not determine version — set --version or version_from in alef.toml")?;
                    let output_dir = std::path::Path::new(&output);
                    eprintln!(
                        "Packaging {} (v{ver}) for: {}",
                        output_dir.display(),
                        format_languages(&languages)
                    );
                    alef_publish::package(&config, &languages, rust_target.as_ref(), output_dir, &ver, dry_run)?;
                    println!("Package complete");
                    Ok(())
                }
                PublishAction::Validate => {
                    let languages = resolve_languages(&config, None)?;
                    let issues = alef_publish::validate(&config, &languages)?;
                    if issues.is_empty() {
                        println!("All package manifests are consistent");
                    } else {
                        eprintln!("Validation issues:");
                        for issue in &issues {
                            eprintln!("  - {issue}");
                        }
                        anyhow::bail!("{} validation issue(s) found", issues.len());
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
    }
}

fn load_config(path: &std::path::Path) -> Result<alef_core::config::AlefConfig> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config: {}", path.display()))?;
    let config: alef_core::config::AlefConfig =
        toml::from_str(&content).with_context(|| "Failed to parse alef.toml")?;
    config
        .validate()
        .with_context(|| format!("alef.toml validation failed ({})", path.display()))?;
    Ok(config)
}

fn resolve_languages(
    config: &alef_core::config::AlefConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef_core::config::Language>> {
    resolve_languages_inner(config, filter, false)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
/// Docs can always be generated for Rust since it's the source language.
fn resolve_doc_languages(
    config: &alef_core::config::AlefConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef_core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

fn resolve_languages_inner(
    config: &alef_core::config::AlefConfig,
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

/// Check a single generated file against its expected content hash.
///
/// Reads the on-disk file and compares its embedded `alef:hash:` line against
/// `expected_hash` (computed from freshly generated + normalized content).
///
/// alef writes the hash into the header at generation time, so verification is
/// purely a hash comparison — `alef verify` never inspects the file body.  This
/// makes it formatting-agnostic: any external formatter (cargo fmt, php-cs-fixer,
/// rubocop, ruff, biome, etc.) can rewrite the body without breaking verify, as
/// long as the hash header line survives.
///
/// A file without an embedded hash is treated as user-owned (scaffold-once) and
/// is skipped — alef never claims ownership of files it didn't tag.
fn verify_file(path: &std::path::Path, expected_hash: &str, label: &str, stale: &mut Vec<String>) {
    let disk_content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            stale.push(format!("[{label}] {} (missing)", path.display()));
            return;
        }
    };

    let Some(disk_hash) = alef_core::hash::extract_hash(&disk_content) else {
        // No alef hash on disk — file is either user-owned scaffold or was
        // hand-edited to remove the marker. Either way, alef has no claim.
        return;
    };

    if disk_hash != expected_hash {
        stale.push(format!("[{label}] {}", path.display()));
    }
}
