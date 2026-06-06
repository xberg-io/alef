use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

use alef::cli::{cache, commands, dispatch, pipeline, registry, version_pin};

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
        /// Run post-generation formatters on emitted files. Default: false for
        /// fast regeneration; pass `--format` to opt into formatter-stable output.
        #[arg(
            long,
            default_value_t = false,
            default_missing_value = "true",
            num_args = 0..=1,
            action = clap::ArgAction::Set,
        )]
        format: bool,
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
    ///
    /// After updating manifest versions and alef.toml registry package pins,
    /// automatically regenerates test_apps/ scaffold files so generated files
    /// (pyproject.toml, mix.exs, build.zig.zon, Package.swift, etc.) reflect
    /// the new version atomically. Use --no-regen to opt out of this behaviour
    /// and keep the legacy two-step workflow (sync-versions + alef:generate).
    SyncVersions {
        /// Bump version before syncing (major, minor, patch).
        #[arg(long)]
        bump: Option<String>,
        /// Set version explicitly (e.g., "0.1.0-rc.1").
        #[arg(long)]
        set: Option<String>,
        /// Skip automatic test_apps/ regeneration after syncing registry package
        /// versions. Use when you want to run alef:generate separately.
        #[arg(long)]
        no_regen: bool,
        /// Skip the swift artifactbundle build and checksum substitution.
        /// Use when Xcode / the required Apple targets are not available on the
        /// current host, or during fast dev iterations where the checksum
        /// placeholder in Package.swift is acceptable.
        #[arg(long)]
        skip_swift_checksum: bool,
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
    /// Run all: generate + stubs + scaffold + readme + sync + e2e.
    All {
        /// Ignore cache.
        #[arg(long)]
        clean: bool,
        /// Run post-generation formatters on emitted files. Default: false for
        /// fast regeneration; pass `--format` to opt into formatter-stable output.
        #[arg(
            long,
            default_value_t = false,
            default_missing_value = "true",
            num_args = 0..=1,
            action = clap::ArgAction::Set,
        )]
        format: bool,
    },
    /// Initialize a new alef.toml config.
    Init {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Run post-generation formatters on emitted files. Default: false.
        #[arg(
            long,
            default_value_t = false,
            default_missing_value = "true",
            num_args = 0..=1,
            action = clap::ArgAction::Set,
        )]
        format: bool,
    },
    /// Generate or check the versioned alef.toml JSON Schema.
    Schema {
        /// Output JSON Schema file.
        #[arg(long, short, default_value = alef::core::config::DEFAULT_SCHEMA_PATH)]
        output: PathBuf,
        /// Schema version to embed. Defaults to the compiled alef package version.
        #[arg(long)]
        schema_version: Option<String>,
        /// Fail if the existing schema file is stale instead of writing it.
        #[arg(long)]
        check: bool,
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
    /// Generate standalone registry-mode test apps (test_apps/).
    TestApps {
        #[command(subcommand)]
        action: TestAppsAction,
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
    /// Discover, validate, audit, and gap-check documentation snippets.
    Snippets {
        #[command(subcommand)]
        action: commands::snippets::SnippetsAction,
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
        /// Require referenced workspace-member versions to already be published to
        /// the registry: regenerate the Cargo.lock and fail hard if resolution
        /// fails (i.e. a member version is not yet published). Use in CI/release;
        /// leave off for local/pre-release dev.
        #[arg(long)]
        require_registry: bool,
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
        /// Run e2e formatters on emitted files. Default: false for fast
        /// regeneration; pass `--format` to opt in.
        #[arg(
            long,
            default_value_t = false,
            default_missing_value = "true",
            num_args = 0..=1,
            action = clap::ArgAction::Set,
        )]
        format: bool,
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
enum TestAppsAction {
    /// Generate registry-mode test apps from fixtures into test_apps/.
    Generate {
        /// Comma-separated list of languages to generate (default: all).
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Delete the test_apps/<lang>/ directory before regenerating.
        #[arg(long)]
        clean: bool,
        /// Run e2e formatters on emitted files. Default: false for fast
        /// regeneration; pass `--format` to opt in.
        #[arg(
            long,
            default_value_t = false,
            default_missing_value = "true",
            num_args = 0..=1,
            action = clap::ArgAction::Set,
        )]
        format: bool,
        /// Maximum parallel jobs (0 = all cores, 1 = sequential).
        #[arg(short, long, default_value = "0")]
        jobs: usize,
    },
    /// Run the registry-mode test apps: install each published package from its
    /// registry and exercise it, reporting pass/skip/fail per target. Verifies a
    /// release end-to-end (e.g. the Ruby gem builds its native ext — issue #87).
    Run {
        /// Comma-separated list of test-app targets to run (default: all in `[e2e].languages`).
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
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
        Commands::Generate { lang, clean, format } => {
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
                let mut changed_languages: std::collections::HashSet<alef::core::config::Language> =
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

                // Generate service API (idiomatic app/handler bridge) for backends
                // that support it — only runs when surface.services is non-empty.
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

                // Sweep language package directories to catch stale alef-generated files
                // in directories the current run no longer writes to. cleanup_orphaned_files
                // only walks directories touched by current_gen_paths; this pass covers
                // the case where a backend stopped emitting files in a directory entirely
                // (e.g. generate_public_api removed from alef-backend-wasm left behind
                // packages/wasm/src/index.ts which cleanup_orphaned_files never visits
                // because no current wasm file lives in packages/wasm/).
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
                    // Legacy paths that alef previously wrote generate_public_api shims
                    // into but no longer touches after their respective backends stopped
                    // emitting those files.
                    sweep_roots.insert(base_dir.join("packages/wasm"));
                    sweep_roots.insert(base_dir.join("packages/typescript"));
                    let roots: Vec<std::path::PathBuf> = sweep_roots.into_iter().filter(|d| d.exists()).collect();
                    if let Ok(removed) = pipeline::sweep_orphans(&roots, &current_gen_paths) {
                        if removed > 0 {
                            eprintln!("Removed {removed} stale alef-generated file(s)");
                        }
                    }
                }

                if any_written && format && !changed_languages.is_empty() {
                    eprintln!("Formatting generated files...");
                    // Include stubs in the format pass so that languages where only
                    // stubs changed (no bindings written) still trigger their
                    // formatter (e.g. ruff on .pyi). Without this, `format_generated`
                    // would iterate over `files` (bindings only) and skip the language
                    // entirely, leaving stub content unformatted before hash finalisation.
                    let mut files_to_format = files.clone();
                    files_to_format.extend(stub_files.clone());
                    pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, Some(&changed_languages));
                    let changed_list: Vec<alef::core::config::Language> = changed_languages.iter().copied().collect();
                    pipeline::fmt_post_generate(resolved_cfg, &changed_list);
                }

                // Finalise per-file hashes after all formatters have run.
                // The embedded hash is derived from generation inputs (alef rev +
                // sources + alef.toml), not from file content, so formatter rewrites
                // never invalidate it.
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes(&current_gen_paths, &sources_hash, &alef_toml_bytes)?;

                // Always re-sync versions across user-owned manifests.
                // Pass no_regen=true: alef generate owns the test_apps/ stage
                // itself and will regenerate them in its own pass below.
                if let Err(e) = pipeline::sync_versions(resolved_cfg, config_path, None, true, true) {
                    tracing::warn!("version sync failed: {e}");
                }

                // Stamp alef.toml with the CLI version that produced this generate.
                if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                    tracing::warn!("could not update alef.toml version pin: {e}");
                }
                // Keep the .pre-commit-config.yaml alef hook rev in lockstep.
                if let Err(e) = version_pin::sync_precommit_alef_rev(&base_dir) {
                    tracing::warn!("could not update .pre-commit-config.yaml alef hook rev: {e}");
                }

                // Warn if [e2e] is configured but not regenerated
                if resolved_cfg.e2e.is_some() {
                    tracing::warn!("[e2e] block detected — run 'alef e2e generate' to regenerate e2e test suites");
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
                let stub_langs: Vec<alef::core::config::Language> = files.iter().map(|(lang, _)| *lang).collect();
                pipeline::format_generated(&files, resolved_cfg, &base_dir, None);
                pipeline::fmt_post_generate(resolved_cfg, &stub_langs);

                // Finalise per-file hashes for the freshly written (and formatted) stubs.
                let stub_paths: std::collections::HashSet<PathBuf> = files
                    .iter()
                    .flat_map(|(_, fs)| fs.iter().map(|f| base_dir.join(&f.path)))
                    .collect();
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                pipeline::finalize_hashes(&stub_paths, &sources_hash, &alef_toml_bytes)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total} stub files");
            Ok(())
        }
        Commands::Scaffold { lang } => {
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
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
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files(&files, &base_dir)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let scaffold_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&scaffold_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "scaffold", &stage_hash, &output_paths)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            // Stamp alef.toml + the pre-commit alef hook rev with the CLI version.
            if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                tracing::warn!("could not update alef.toml version pin: {e}");
            }
            if let Err(e) = version_pin::sync_precommit_alef_rev(&base_dir) {
                tracing::warn!("could not update .pre-commit-config.yaml alef hook rev: {e}");
            }
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
                let languages = resolve_readme_languages(resolved_cfg, lang.as_deref())?;
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
                let files = alef::docs::generate_docs(&api, resolved_cfg, &languages, &output)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;
                let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                let doc_paths: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                pipeline::finalize_hashes(&doc_paths, &sources_hash, &alef_toml_bytes)?;
                cache::write_stage_hash(&resolved_cfg.name, "docs", &stage_hash, &output_paths)?;
                grand_total += count;
            } // end for resolved_cfg in crates_to_process
            println!("Generated {grand_total} API doc files");
            Ok(())
        }
        Commands::SyncVersions {
            bump,
            set,
            no_regen,
            skip_swift_checksum,
        } => {
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
                pipeline::sync_versions(
                    resolved_cfg,
                    config_path,
                    bump.as_deref(),
                    no_regen,
                    skip_swift_checksum,
                )?;
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
            // Verify never regenerates and never writes — pure read+compare.
            // The embedded hash is a generation-inputs fingerprint; verify
            // re-derives it from current (alef rev + sources + alef.toml) and
            // compares, so formatter drift never causes false-positive failures.
            // The legacy `--compile` / `--lint` / `--lang` flags are accepted
            // but ignored; run `alef build` / `alef lint` / `alef test` for
            // those concerns.
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            eprintln!("Verifying alef-generated files (inputs-hash mode)");
            let base_dir = std::env::current_dir()?;

            // Read the alef.toml bytes once — same bytes used at generate time.
            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            // Collect inputs hashes for all selected crates so that the file
            // walk can validate each file against ANY crate's inputs hash.
            // A file is valid if it matches at least one crate's inputs hash.
            let all_inputs_hashes: Vec<String> = crates_to_process
                .iter()
                .filter_map(|c| cache::sources_hash(&c.sources).ok())
                .map(|sh| alef::core::hash::compute_inputs_hash(&sh, &alef_toml_bytes))
                .collect();

            let stale = verify_walk_multi(&base_dir, &all_inputs_hashes)?;

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
        Commands::All { clean, format } => {
            let (workspace, resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;
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
                let mut changed_languages: std::collections::HashSet<alef::core::config::Language> =
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

                // Run post-build processing (e.g., FRB codegen, post-processing rewrites)
                eprintln!("Running post-build processing...");
                for &lang in &languages {
                    let Some(backend) = registry::try_get_backend(lang) else {
                        continue;
                    };
                    if let Some(bc) = backend.build_config_with_config(resolved_cfg) {
                        if !bc.post_build.is_empty() {
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
                        // Track stub-changed languages so formatters run even when
                        // no bindings changed for this language (e.g. ruff on .pyi).
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

                // Generate service API (idiomatic app/handler bridge) for backends
                // that support it — only runs when surface.services is non-empty.
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

                let mut api_count = 0;
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
                            api_count = pipeline::write_files(&public_api_files, &base_dir)?;
                            eprintln!("Generated {api_count} public API files");
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                        } else {
                            eprintln!("  [public_api] up to date (skipping)");
                        }
                    }
                }

                eprintln!("Generating scaffolding...");
                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages)?;
                let scaffold_count = pipeline::write_scaffold_files_with_overwrite(&scaffold_files, &base_dir, clean)?;
                for file in &scaffold_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                eprintln!("Generating READMEs...");
                let readme_files = pipeline::readme(&api, resolved_cfg, &languages)?;
                let readme_count = pipeline::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
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
                        match alef::extract::validate_call_export(&api, &module_path, function_name) {
                            alef::extract::ExportValidation::Ok => {}
                            alef::extract::ExportValidation::NotFound { function } => {
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' was not found in the extracted API surface. \
                                 Check that it is declared `pub` and that its source file is listed in \
                                 [[crate.sources]] or [[crate.source_crates]]."
                                );
                            }
                            alef::extract::ExportValidation::WrongPath {
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

                    // Check e2e stage cache: skip regeneration if fixtures + IR + config
                    // are all unchanged (unless --clean forces a full regeneration).
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                    let ir_json = serde_json::to_string(&api)?;
                    let e2e_stage_hash = cache::compute_stage_hash(&ir_json, "e2e", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "e2e", &e2e_stage_hash) {
                        eprintln!("  [e2e] up to date (skipping)");
                        // Repopulate `current_gen_paths` from the cached manifest so the
                        // orphan-cleanup pass below does not treat previously-generated
                        // e2e files as stale. Without this, every cached `alef all` run
                        // would delete every e2e file in the workspace.
                        for path in cache::read_stage_paths(&resolved_cfg.name, "e2e") {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        eprintln!("Generating e2e test suites...");
                        let files = alef::e2e::generate_e2e(resolved_cfg, e2e_config, None, &api.types, &api.enums)?;
                        e2e_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                        if format {
                            alef::e2e::format::run_formatters(&files, e2e_config);
                        }

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        // Sweep orphan alef-generated e2e files from e2e/ only —
                        // never touches test_apps/ (owned by the test-apps stage below).
                        let e2e_output_root = base_dir.join(&e2e_config.output);
                        pipeline::sweep_orphans(&[e2e_output_root], &path_set)?;

                        cache::write_stage_hash(&resolved_cfg.name, "e2e", &e2e_stage_hash, &output_paths)?;

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }

                    // Test-apps stage: regenerate registry-mode test apps in test_apps/.
                    // Runs as a distinct pipeline stage so its stale-file sweep is scoped
                    // to test_apps/ and cannot delete e2e/ files (and vice versa).
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
                        registry_e2e_config.dep_mode = alef::core::config::e2e::DependencyMode::Registry;
                        let registry_e2e_ref = &registry_e2e_config;

                        let files =
                            alef::e2e::generate_e2e(resolved_cfg, registry_e2e_ref, None, &api.types, &api.enums)?;
                        let test_apps_count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;
                        e2e_count += test_apps_count;
                        if format {
                            alef::e2e::format::run_formatters(&files, registry_e2e_ref);
                        }

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        // Sweep orphans scoped to test_apps/ only — never touches e2e/.
                        let test_apps_root = base_dir.join(registry_e2e_ref.effective_output());
                        pipeline::sweep_orphans(&[test_apps_root], &path_set)?;

                        cache::write_stage_hash(&resolved_cfg.name, "test-apps", &test_apps_stage_hash, &output_paths)?;

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }
                }

                eprintln!("Generating API docs...");
                let docs_api = pipeline::extract(resolved_cfg, config_path, false)?;
                let doc_languages = resolve_doc_languages(resolved_cfg, None)?;
                let doc_files = alef::docs::generate_docs(&docs_api, resolved_cfg, &doc_languages, "docs/reference")?;
                let doc_count = pipeline::write_scaffold_files_with_overwrite(&doc_files, &base_dir, clean)?;
                for file in &doc_files {
                    current_gen_paths.insert(base_dir.join(&file.path));
                }

                if let Ok(removed) = pipeline::cleanup_orphaned_files(&current_gen_paths) {
                    if removed > 0 {
                        eprintln!("Removed {removed} stale alef-generated file(s)");
                    }
                }

                // Sweep language package directories to catch stale alef-generated files
                // in directories the current run no longer writes to (same rationale as
                // in Commands::Generate above).
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

                // Formatters run by default. They are best-effort: a missing
                // formatter or non-zero exit must not abort the pipeline.
                // Two passes when enabled:
                //  1. `format_generated` runs language-native defaults (cargo fmt,
                //     ruff format, mix format, oxfmt, etc.) on the freshly
                //     emitted files.
                //  2. `fmt_post_generate` runs any extra repo-configured
                //     `[lint.<lang>].format` commands (linters, custom passes).
                // Both are scoped to languages that actually regenerated this run.
                if format && !changed_languages.is_empty() {
                    eprintln!("Formatting generated files...");
                    // Include stubs in the format pass so that languages where only
                    // stubs changed (no bindings written) still trigger their formatter.
                    let mut files_to_format = bindings.clone();
                    files_to_format.extend(stubs.clone());
                    pipeline::format_generated(&files_to_format, resolved_cfg, &base_dir, Some(&changed_languages));

                    eprintln!("Running formatters...");
                    let changed_list: Vec<alef::core::config::Language> = changed_languages.iter().copied().collect();
                    pipeline::fmt_post_generate(resolved_cfg, &changed_list);
                }

                // Finalise per-file hashes after every formatter has run.
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
            } // end for resolved_cfg in crates_to_process

            // Stamp alef.toml with the CLI version that produced this run.
            if let Err(e) = version_pin::write_alef_toml_version(config_path) {
                tracing::warn!("could not update alef.toml version pin: {e}");
            }
            // Keep the .pre-commit-config.yaml alef hook rev in lockstep.
            if let Err(e) = version_pin::sync_precommit_alef_rev(&base_dir) {
                tracing::warn!("could not update .pre-commit-config.yaml alef hook rev: {e}");
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
            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
            pipeline::finalize_hashes(&all_paths, &sources_hash, &alef_toml_bytes)?;

            println!("Initialized: {binding_count} binding files, {scaffold_count} scaffold files");
            Ok(())
        }
        Commands::Schema {
            output,
            schema_version,
            check,
        } => {
            let version = schema_version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));
            if check {
                alef::core::config::check_alef_config_schema(&output, version)?;
                println!("Schema is up to date: {}", output.display());
            } else {
                alef::core::config::write_alef_config_schema(&output, version)?;
                println!("Wrote schema to {}", output.display());
            }
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
                        // When --registry is set (deprecated path), clone the e2e config
                        // and switch to registry dependency mode.
                        let effective_e2e_config;
                        let e2e_ref = if registry {
                            let mut cloned = this_e2e_config.clone();
                            cloned.dep_mode = alef::core::config::e2e::DependencyMode::Registry;
                            effective_e2e_config = cloned;
                            eprintln!("Generating e2e test apps (registry mode)...");
                            &effective_e2e_config
                        } else {
                            eprintln!("Generating e2e test suites...");
                            this_e2e_config
                        };
                        let languages = lang.as_deref();
                        let files = alef::e2e::generate_e2e(e2e_crate, e2e_ref, languages, &api.types, &api.enums)?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

                        if format {
                            alef::e2e::format::run_formatters(&files, e2e_ref);
                        }

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        // Sweep orphan alef-generated files scoped to effective_output() of
                        // the current mode — local mode sweeps e2e/, registry mode sweeps
                        // test_apps/. This prevents each mode from deleting the other's files.
                        let e2e_output_root = base_dir.join(e2e_ref.effective_output());
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
                    let created = alef::e2e::scaffold::init_fixtures(e2e_config, resolved_cfg)?;
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
                        alef::e2e::scaffold::scaffold_fixture(e2e_config, resolved_cfg, &id, &category, &description)?;
                    println!("Created {path}");
                    Ok(())
                }
                E2eAction::List => {
                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixtures = alef::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let groups = alef::e2e::fixture::group_fixtures(&fixtures);

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
                    let mut all_errors = alef::e2e::validate::validate_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to validate fixtures from {}", fixtures_dir.display()))?;

                    // Semantic validation
                    let fixtures = alef::e2e::fixture::load_fixtures(fixtures_dir)
                        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
                    let semantic_errors =
                        alef::e2e::validate::validate_fixtures_semantic(&fixtures, e2e_config, &e2e_config.languages);
                    all_errors.extend(semantic_errors);

                    if all_errors.is_empty() {
                        println!("All fixtures are valid.");
                        Ok(())
                    } else {
                        use alef::e2e::validate::Severity;
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
        Commands::TestApps { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let _resolved_cfg = crates_to_process
                .iter()
                .find(|c| c.e2e.is_some())
                .copied()
                .unwrap_or_else(|| crates_to_process[0]);
            // Validate that at least one crate has an [e2e] section.
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

                        // Build a registry-mode clone of the e2e config.
                        let mut registry_config = this_e2e_config.clone();
                        registry_config.dep_mode = alef::core::config::e2e::DependencyMode::Registry;
                        let e2e_ref = &registry_config;
                        let output_root = base_dir.join(e2e_ref.effective_output());

                        // --clean: delete the per-language directories before regen.
                        // Preserve lock files (go.sum, go.mod is regenerated, etc.) — generators
                        // should not own dependency lock files.
                        if clean {
                            let langs_to_clean: Vec<String> = lang
                                .as_deref()
                                .map(|ls| ls.iter().map(|s| s.to_string()).collect())
                                .unwrap_or_else(|| e2e_ref.languages.clone());
                            let lock_files = [
                                "go.sum",
                                "go.mod",
                                "package-lock.json",
                                "pnpm-lock.yaml",
                                "yarn.lock",
                                "Gemfile.lock",
                                "composer.lock",
                                "uv.lock",
                                "pubspec.lock",
                            ];
                            for lang_name in &langs_to_clean {
                                let lang_dir = output_root.join(lang_name);
                                if lang_dir.exists() {
                                    // Save lock files before deletion
                                    let mut saved_locks = std::collections::HashMap::new();
                                    for lock_file in &lock_files {
                                        let lock_path = lang_dir.join(lock_file);
                                        if lock_path.exists() {
                                            if let Ok(content) = std::fs::read(&lock_path) {
                                                saved_locks.insert(lock_path.clone(), content);
                                            }
                                        }
                                    }

                                    std::fs::remove_dir_all(&lang_dir)
                                        .with_context(|| format!("failed to remove {}", lang_dir.display()))?;

                                    // Restore lock files after deletion
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
                        let files = alef::e2e::generate_e2e(e2e_crate, e2e_ref, languages, &api.types, &api.enums)?;
                        let sources_hash = cache::sources_hash(&e2e_crate.sources)?;
                        let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
                        let count = pipeline::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

                        if format {
                            alef::e2e::format::run_formatters(&files, e2e_ref);
                        }

                        let output_paths: Vec<PathBuf> = files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        // Sweep orphans scoped to test_apps/ only — never touches e2e/.
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
                    Ok(())
                }
                TestAppsAction::Run { lang } => {
                    for e2e_crate in &crates_to_process {
                        let Some(this_e2e_config) = e2e_crate.e2e.as_ref() else {
                            continue;
                        };
                        let all_names: Vec<String> = if this_e2e_config.languages.is_empty() {
                            alef::e2e::default_e2e_languages(&e2e_crate.languages)
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
                    Ok(())
                }
            }
        }
        Commands::Publish { action } => {
            let (_workspace, resolved) = load_config(config_path)?;
            let crates_to_process = dispatch::select_crates(&resolved, &cli.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            match action {
                PublishAction::Prepare {
                    lang,
                    target,
                    dry_run,
                    require_registry,
                } => {
                    let rust_target = target
                        .as_deref()
                        .map(alef::publish::platform::RustTarget::parse)
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
                        alef::publish::prepare(
                            resolved_cfg,
                            &languages,
                            rust_target.as_ref(),
                            dry_run,
                            require_registry,
                        )?;
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
                        .map(alef::publish::platform::RustTarget::parse)
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
                        alef::publish::build(resolved_cfg, &languages, rust_target.as_ref(), use_cross)?;
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
                        .map(alef::publish::platform::RustTarget::parse)
                        .transpose()?;
                    let output_dir = std::path::Path::new(&output);
                    for resolved_cfg in &crates_to_process {
                        let languages = resolve_languages(resolved_cfg, lang.as_deref())?;
                        let ver = version
                            .clone()
                            .or_else(|| resolved_cfg.resolved_version())
                            .context("could not determine version — set --version or version_from in alef.toml")?;

                        // Build PHP-specific options when any PHP language is in the list.
                        let needs_php = languages.contains(&alef::core::config::Language::Php);
                        let pie_opts: Option<alef::publish::package::php::PiePackageOptions<'_>> = if needs_php {
                            let php_ver = php_version
                                .as_deref()
                                .context("--php-version is required when packaging --lang php")?;
                            let ts_mode = alef::publish::package::php::TsMode::parse(&php_ts)?;
                            // Validate: Windows target requires --windows-compiler.
                            if let Some(ref rt) = rust_target {
                                if rt.os == alef::publish::platform::Os::Windows && windows_compiler.is_none() {
                                    anyhow::bail!(
                                        "--windows-compiler is required when packaging PHP for a Windows target"
                                    );
                                }
                            }
                            Some(alef::publish::package::php::PiePackageOptions {
                                php_version: php_ver,
                                ts_mode,
                                debug_mode: alef::publish::package::php::DebugMode::NoDebug,
                                libc_override: php_libc.as_deref(),
                                windows_compiler: windows_compiler.as_deref(),
                            })
                        } else {
                            None
                        };

                        let pkg_options = alef::publish::PackageOptions { php: pie_opts };

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
                        alef::publish::package(
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
                        let issues = alef::publish::validate(resolved_cfg, &languages)?;
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
            let resolved_cfg_opt: Option<&alef::core::config::ResolvedCrateConfig> =
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
        Commands::Snippets { action } => {
            let exit_code = commands::snippets::run(action);
            if exit_code != std::process::ExitCode::SUCCESS {
                process::exit(1);
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
    alef::core::config::WorkspaceConfig,
    Vec<alef::core::config::ResolvedCrateConfig>,
)> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read config: {}", path.display()))?;
    alef::core::config::detect_legacy_keys(&content).with_context(|| {
        format!(
            "legacy schema detected in {} — run `alef migrate` to update automatically",
            path.display()
        )
    })?;
    let cfg: alef::core::config::NewAlefConfig =
        toml::from_str(&content).with_context(|| format!("Failed to parse alef.toml ({})", path.display()))?;
    let resolved = cfg
        .resolve()
        .with_context(|| format!("failed to resolve crates in {}", path.display()))?;
    for resolved_cfg in &resolved {
        alef::core::config::validation::validate_resolved(resolved_cfg)
            .with_context(|| format!("invalid resolved config for crate `{}`", resolved_cfg.name))?;
    }
    Ok((cfg.workspace, resolved))
}

fn resolve_languages(
    config: &alef::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef::core::config::Language>> {
    resolve_languages_inner(config, filter, false)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
/// Docs can always be generated for Rust since it's the source language.
fn resolve_doc_languages(
    config: &alef::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Like `resolve_languages` but also allows `rust` regardless of the config languages list.
///
/// Every Rust crate that publishes to crates.io needs a `crates/<lib>/README.md`,
/// so the readme command must regenerate it from the same templates that produce
/// the per-binding READMEs. Configure with `[crates.readme.languages.rust]` in
/// `alef.toml` to opt in.
fn resolve_readme_languages(
    config: &alef::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
) -> Result<Vec<alef::core::config::Language>> {
    resolve_languages_inner(config, filter, true)
}

/// Resolve languages for `alef test`.
///
/// Test suites can exist for targets that do not generate host bindings, such
/// as Rust e2e tests for the source crate. Keep binding language resolution
/// strict for generation/build commands, but allow explicit test targets and
/// include e2e-only entries when `alef test --e2e` runs without a filter.
fn resolve_test_languages(
    config: &alef::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    include_e2e: bool,
) -> Result<Vec<alef::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || config.test.contains_key(&lang.to_string()) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list or test configuration");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if include_e2e {
                let mut extra_test_langs = vec![];
                for (lang_str, test_config) in &config.test {
                    if test_config.e2e.is_none() {
                        continue;
                    }
                    let lang = parse_language(lang_str)
                        .with_context(|| format!("Invalid test language in alef.toml: {lang_str}"))?;
                    if !langs.contains(&lang) {
                        extra_test_langs.push(lang);
                    }
                }
                extra_test_langs.sort_by_key(|lang| lang.to_string());
                for lang in extra_test_langs {
                    if !langs.contains(&lang) {
                        langs.push(lang);
                    }
                }
            }
            Ok(langs)
        }
    }
}

fn resolve_languages_inner(
    config: &alef::core::config::ResolvedCrateConfig,
    filter: Option<&[String]>,
    allow_rust: bool,
) -> Result<Vec<alef::core::config::Language>> {
    match filter {
        Some(langs) => {
            let mut result = vec![];
            for lang_str in langs {
                let lang = parse_language(lang_str)?;
                if config.languages.contains(&lang) || (allow_rust && lang == alef::core::config::Language::Rust) {
                    result.push(lang);
                } else {
                    anyhow::bail!("Language '{lang_str}' not in config languages list");
                }
            }
            Ok(result)
        }
        None => {
            let mut langs = config.languages.clone();
            if allow_rust && !langs.contains(&alef::core::config::Language::Rust) {
                langs.push(alef::core::config::Language::Rust);
            }
            Ok(langs)
        }
    }
}

fn parse_language(lang_str: &str) -> Result<alef::core::config::Language> {
    toml::Value::String(lang_str.to_string())
        .try_into()
        .with_context(|| format!("Unknown language: {lang_str}"))
}

fn format_languages(languages: &[alef::core::config::Language]) -> String {
    languages.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", ")
}

/// Multi-crate variant of [`verify_walk`].
///
/// A file is considered valid if its embedded `alef:hash:` matches the hash
/// Walk the repo from `base_dir`, find every alef-headered file, and return
/// the list of stale ones — where the embedded `alef:hash:<hex>` does not match
/// any of the provided `inputs_hashes`.  In a multi-crate workspace each file
/// was generated by exactly one crate, so the file passes verification when it
/// matches its generating crate's inputs hash.
fn verify_walk_multi(base_dir: &std::path::Path, inputs_hashes: &[String]) -> anyhow::Result<Vec<String>> {
    if inputs_hashes.is_empty() {
        return Ok(Vec::new());
    }
    if inputs_hashes.len() == 1 {
        return verify_walk(base_dir, &inputs_hashes[0]);
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
            let Some(disk_hash) = alef::core::hash::extract_hash(&content) else {
                continue;
            };
            // A file is valid if its embedded hash matches ANY crate's inputs hash.
            // The comparison is a simple string equality — no file content is rehashed.
            let valid = inputs_hashes.iter().any(|ih| ih == &disk_hash);
            if !valid {
                stale.push(path.display().to_string());
            }
        }
    }

    stale.sort();
    Ok(stale)
}

/// Walk the consumer's repo from `base_dir`, find every alef-headered file, and
/// return the list of stale ones — where the embedded `alef:hash:<hex>` does not
/// equal `inputs_hash`.
///
/// Verification is a direct string equality check against the generation-inputs
/// hash (alef rev + sources + alef.toml). File content is never rehashed, so
/// post-generation formatter rewrites cannot cause false-positive staleness.
///
/// Skips obvious build/cache directories (`target/`, `node_modules/`, `_build/`,
/// `.alef/`, `parsers/`, `dist/`, `vendor/`, `.git/`) so verify stays fast on
/// large repos. Files without the alef header marker are skipped silently —
/// those are user-owned (scaffold-once Cargo.toml templates, composer.json,
/// gemspec, package.json, lockfiles, etc.) and alef has no claim.
fn verify_walk(base_dir: &std::path::Path, inputs_hash: &str) -> anyhow::Result<Vec<String>> {
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
            let Some(disk_hash) = alef::core::hash::extract_hash(&content) else {
                continue;
            };
            // Direct string comparison: the embedded hash is an inputs fingerprint,
            // not derived from file content. No rehashing needed.
            if disk_hash != inputs_hash {
                stale.push(path.display().to_string());
            }
        }
    }

    stale.sort();
    Ok(stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef::core::config::Language;

    fn resolved_test_config() -> alef::core::config::ResolvedCrateConfig {
        let cfg: alef::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
command = "pytest"

[crates.test.rust]
e2e = "cargo test"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn resolve_test_languages_allows_explicit_test_only_language() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, Some(&["rust".to_string()]), true).unwrap();
        assert_eq!(langs, vec![Language::Rust]);
    }

    #[test]
    fn resolve_test_languages_appends_e2e_only_languages() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, None, true).unwrap();
        assert_eq!(langs, vec![Language::Python, Language::Rust]);
    }

    #[test]
    fn resolve_test_languages_omits_e2e_only_languages_without_e2e() {
        let config = resolved_test_config();
        let langs = resolve_test_languages(&config, None, false).unwrap();
        assert_eq!(langs, vec![Language::Python]);
    }
}
