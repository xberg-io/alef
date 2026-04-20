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
    /// Manage the build cache.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
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
        Commands::Generate { lang, clean } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Generating bindings for: {}", format_languages(&languages));
            let api = pipeline::extract(&config, config_path, clean)?;
            let files = pipeline::generate(&api, &config, &languages, clean)?;
            let base_dir = std::env::current_dir()?;
            let count = pipeline::write_files(&files, &base_dir)?;
            // Auto-format generated Rust files
            pipeline::format_rust_files(&files, &base_dir);

            // Generate public API wrappers
            if config.generate.public_api {
                let public_api_files = pipeline::generate_public_api(&api, &config, &languages)?;
                if !public_api_files.is_empty() {
                    let api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                    eprintln!("Generated {api_count} public API files");
                }
            }

            // Generate type stubs (e.g., .pyi for Python, .d.ts for TypeScript)
            let stub_files = pipeline::generate_stubs(&api, &config, &languages)?;
            if !stub_files.is_empty() {
                let stub_count = pipeline::write_files(&stub_files, &base_dir)?;
                eprintln!("Generated {stub_count} type stub files");
            }

            // Format and lint all generated files via prek (best-effort)
            pipeline::run_prek();

            // Recompute input + output hashes AFTER prek (prek may modify config).
            let post_config_struct = load_config(config_path)?;
            let post_api = pipeline::extract(&post_config_struct, config_path, true)?;
            let post_ir = serde_json::to_string(&post_api)?;
            let post_config = toml::to_string(&post_config_struct).unwrap_or_default();
            for lang in &languages {
                let lang_str = lang.to_string();
                let lang_hash = cache::compute_lang_hash(&post_ir, &lang_str, &post_config);
                if let Ok(paths) = cache::read_manifest_paths(&lang_str) {
                    let _ = cache::write_lang_hash(&lang_str, &lang_hash, &paths);
                    let _ = cache::write_output_hashes(&lang_str, &paths);
                }
            }

            println!("Generated {count} files");
            Ok(())
        }
        Commands::Stubs { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            let config_toml = std::fs::read_to_string(config_path)?;
            let api = pipeline::extract(&config, config_path, false)?;
            let ir_json = serde_json::to_string(&api)?;
            let stage_hash = cache::compute_stage_hash(&ir_json, "stubs", &config_toml, &[]);
            if cache::is_stage_cached("stubs", &stage_hash) {
                println!("Stubs up to date (cached)");
                return Ok(());
            }
            eprintln!("Generating type stubs for: {}", format_languages(&languages));
            let files = pipeline::generate_stubs(&api, &config, &languages)?;
            let base_dir = std::env::current_dir()?;
            let count = pipeline::write_files(&files, &base_dir)?;
            let output_paths: Vec<PathBuf> = files
                .iter()
                .flat_map(|(_, fs)| fs.iter().map(|f| base_dir.join(&f.path)))
                .collect();
            cache::write_stage_hash("stubs", &stage_hash, &output_paths)?;
            let _ = cache::write_output_hashes("stubs", &output_paths);
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
            let has_pre_commit = files.iter().any(|f| f.path.ends_with(".pre-commit-config.yaml"));
            let base_dir = std::env::current_dir()?;
            let count = pipeline::write_scaffold_files(&files, &base_dir)?;
            let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
            cache::write_stage_hash("scaffold", &stage_hash, &output_paths)?;
            // If a new .pre-commit-config.yaml was scaffolded, run prek autoupdate
            // to bump hook revisions to the latest available versions.
            if has_pre_commit {
                pipeline::run_prek_autoupdate();
            }
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
            // Use unfiltered IR for docs so ALL public types are documented,
            // not just the subset that survives [include]/[exclude] binding filters.
            let api = pipeline::extract_unfiltered(&config, config_path)?;
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
            pipeline::sync_versions(&config, bump.as_deref())?;
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
        Commands::Lint { lang } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Linting generated output for: {}", format_languages(&languages));
            pipeline::lint(&config, &languages)?;
            println!("Lint complete");
            Ok(())
        }
        Commands::Test { lang, e2e } => {
            let config = load_config(config_path)?;
            let languages = resolve_languages(&config, lang.as_deref())?;
            eprintln!("Running tests for: {}", format_languages(&languages));
            if e2e {
                eprintln!("  (with e2e tests)");
            }
            pipeline::test(&config, &languages, e2e)?;
            println!("Tests complete");
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

            let mut all_stale: Vec<String> = Vec::new();

            // Verify each language's output files via blake3 content hashing.
            // We compare current on-disk content against hashes stored after the
            // last `alef generate` / `alef all` (post-prek).
            for lang in &languages {
                let lang_str = lang.to_string();

                if !cache::has_output_hashes(&lang_str) {
                    // No output hashes yet — fall back to regenerate-and-diff.
                    let api = pipeline::extract(&config, config_path, false)?;
                    let bindings = pipeline::generate(&api, &config, &[*lang], true)?;
                    let base_dir = std::env::current_dir()?;
                    all_stale.extend(pipeline::diff_files(&bindings, &base_dir)?);
                    continue;
                }

                match cache::verify_output_hashes(&lang_str) {
                    Ok(stale_files) => {
                        for f in stale_files {
                            all_stale.push(format!("[{lang_str}] {f}"));
                        }
                    }
                    Err(e) => {
                        all_stale.push(format!("[{lang_str}] failed to verify: {e}"));
                    }
                }
            }

            // Verify stubs
            if cache::has_output_hashes("stubs") {
                match cache::verify_output_hashes("stubs") {
                    Ok(stale_files) => {
                        for f in stale_files {
                            all_stale.push(format!("[stubs] {f}"));
                        }
                    }
                    Err(e) => {
                        all_stale.push(format!("[stubs] failed to verify: {e}"));
                    }
                }
            } else {
                // Fallback: regenerate stubs and diff
                let api = pipeline::extract(&config, config_path, false)?;
                let stubs = pipeline::generate_stubs(&api, &config, &languages)?;
                let base_dir = std::env::current_dir()?;
                all_stale.extend(pipeline::diff_files(&stubs, &base_dir)?);
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

            eprintln!("Generating bindings...");
            let bindings = pipeline::generate(&api, &config, &languages, clean)?;
            let base_dir = std::env::current_dir()?;
            let binding_count = pipeline::write_files(&bindings, &base_dir)?;
            pipeline::format_rust_files(&bindings, &base_dir);

            eprintln!("Generating type stubs...");
            let stubs = pipeline::generate_stubs(&api, &config, &languages)?;
            let stub_count = pipeline::write_files(&stubs, &base_dir)?;

            // Generate public API wrappers
            let mut api_count = 0;
            if config.generate.public_api {
                let public_api_files = pipeline::generate_public_api(&api, &config, &languages)?;
                if !public_api_files.is_empty() {
                    api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                }
            }

            eprintln!("Generating scaffolding...");
            let scaffold_files = pipeline::scaffold(&api, &config, &languages)?;
            let has_pre_commit = scaffold_files
                .iter()
                .any(|f| f.path.ends_with(".pre-commit-config.yaml"));
            let scaffold_count = pipeline::write_scaffold_files(&scaffold_files, &base_dir)?;
            if has_pre_commit {
                pipeline::run_prek_autoupdate();
            }

            eprintln!("Generating READMEs...");
            let readme_files = pipeline::readme(&api, &config, &languages)?;
            let readme_count = pipeline::write_scaffold_files_with_overwrite(&readme_files, &base_dir, clean)?;

            // Generate e2e tests if [e2e] section is present in config
            let mut e2e_count = 0;
            if let Some(e2e_config) = &config.e2e {
                eprintln!("Generating e2e test suites...");
                let files = alef_e2e::generate_e2e(&config, e2e_config, None)?;
                e2e_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, clean)?;
                alef_e2e::format::run_formatters(&files, e2e_config);
            }

            // Generate API docs using unfiltered IR so ALL public types are documented,
            // not just the subset that survives [include]/[exclude] binding filters.
            eprintln!("Generating API docs...");
            let docs_api = pipeline::extract_unfiltered(&config, config_path)?;
            let doc_files = alef_docs::generate_docs(&docs_api, &config, &languages, "docs/reference")?;
            let doc_count = pipeline::write_scaffold_files_with_overwrite(&doc_files, &base_dir, clean)?;

            // Format and lint all generated files via prek (best-effort)
            eprintln!("Running formatters and linters...");
            pipeline::run_prek();

            // Recompute input + output hashes AFTER prek.  Prek may have
            // modified the config file (e.g. alef-sync-versions updates
            // alef.toml) so the input hash recorded during generation is stale.
            // Re-load config from disk and re-hash so `alef verify` sees
            // consistent values.
            eprintln!("Computing output hashes...");
            let post_config_struct = load_config(config_path)?;
            let post_api = pipeline::extract(&post_config_struct, config_path, true)?;
            let post_ir = serde_json::to_string(&post_api)?;
            let post_config = toml::to_string(&post_config_struct).unwrap_or_default();
            for lang in &languages {
                let lang_str = lang.to_string();
                let lang_hash = cache::compute_lang_hash(&post_ir, &lang_str, &post_config);
                if let Ok(paths) = cache::read_manifest_paths(&lang_str) {
                    let _ = cache::write_lang_hash(&lang_str, &lang_hash, &paths);
                    let _ = cache::write_output_hashes(&lang_str, &paths);
                }
            }
            // Stubs
            let stubs_hash = cache::compute_stage_hash(&post_ir, "stubs", &post_config, &[]);
            if let Ok(paths) = cache::read_manifest_paths("stubs") {
                let _ = cache::write_stage_hash("stubs", &stubs_hash, &paths);
                let _ = cache::write_output_hashes("stubs", &paths);
            }

            println!(
                "Done: {binding_count} binding files, {stub_count} stub files, {api_count} API files, {scaffold_count} scaffold files, {readme_count} readme files, {e2e_count} e2e files, {doc_count} doc files"
            );
            Ok(())
        }
        Commands::Init { lang } => {
            eprintln!("Initializing alef.toml");
            if let Some(langs) = &lang {
                eprintln!("  Languages: {}", langs.join(", "));
            }
            pipeline::init(config_path, lang)?;
            println!("Initialized alef.toml");
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
                    let errors = alef_e2e::validate::validate_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to validate fixtures from {}", fixtures_dir.display()))?;

                    if errors.is_empty() {
                        println!("All fixtures are valid.");
                        Ok(())
                    } else {
                        println!("Found {} validation error(s):", errors.len());
                        for err in &errors {
                            println!("  {err}");
                        }
                        process::exit(1);
                    }
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
        None => Ok(config.languages.clone()),
    }
}

fn format_languages(languages: &[alef_core::config::Language]) -> String {
    languages.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
}
