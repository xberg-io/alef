use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::cli::commands;

#[derive(Parser)]
#[command(
    name = "alef",
    version,
    about = "Opinionated polyglot binding generator",
    long_about = None,
)]
pub(crate) struct Cli {
    /// Path to alef.toml config file.
    #[arg(long, default_value = "alef.toml")]
    pub(crate) config: PathBuf,

    /// Maximum parallel jobs (0 = all cores, 1 = sequential).
    #[arg(short, long, default_value = "0", global = true)]
    pub(crate) jobs: usize,

    /// Increase verbosity (-v info, -vv debug, -vvv trace). Overridden by RUST_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub(crate) verbose: u8,

    /// Suppress all output below `error`. Overridden by RUST_LOG.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub(crate) quiet: bool,

    /// Disable ANSI colour in log output.
    #[arg(long, global = true)]
    pub(crate) no_color: bool,

    /// Restrict the command to one or more crates by name. May be passed multiple times.
    /// When omitted, every crate in the workspace is processed.
    #[arg(long = "crate", value_name = "NAME", global = true)]
    pub(crate) crate_filter: Vec<String>,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
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
            hide = true,
        )]
        format: bool,
        /// Skip the flutter_rust_bridge_codegen post-build step.
        ///
        /// Useful when `flutter_rust_bridge` is not installed on the host (e.g.
        /// CI environments or developer machines without the Flutter SDK).
        /// Equivalent to setting `ALEF_SKIP_COMMANDS=flutter_rust_bridge_codegen`
        /// or `[crates.dart] skip_frb = true` in alef.toml.
        #[arg(long)]
        skip_frb: bool,
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
        /// Override reference output directory.
        #[arg(long)]
        output: Option<String>,
    },
    /// Sync version from Cargo.toml to all package manifests.
    ///
    /// Updates version fields in all package manifests and alef.toml registry
    /// package pins atomically. Does not regenerate code — use `alef generate`,
    /// `alef all`, or `task alef:generate` to regenerate test_apps/ and scaffold
    /// files after syncing versions.
    SyncVersions {
        /// Bump version before syncing (major, minor, patch).
        #[arg(long)]
        bump: Option<String>,
        /// Set version explicitly (e.g., "0.1.0-rc.1").
        #[arg(long)]
        set: Option<String>,
        /// Regenerate test_apps/ and scaffold files after syncing versions.
        /// By default, sync-versions only updates manifests; use this flag to
        /// also regenerate code (expensive, normally run separately as `alef generate`).
        #[arg(long)]
        regen: bool,
        /// Skip the swift artifactbundle build and checksum substitution.
        /// Use when Xcode / the required Apple targets are not available on the
        /// current host, or during fast dev iterations where the checksum
        /// placeholder in Package.swift is acceptable.
        #[arg(long)]
        skip_swift_checksum: bool,
        /// Stamp CITATION.cff `date-released:` with this value (YYYY-MM-DD).
        ///
        /// When passed, overrides any `[workspace.citation].date-released`
        /// configured in `alef.toml` and the default of "today's system date".
        /// Intended for release engineers cutting a release on a date other
        /// than the current system date (e.g. backports, replayed releases).
        /// Default: unset — behaviour matches the pre-flag policy
        /// (configured `date-released` if any, else today's date).
        #[arg(long, value_name = "YYYY-MM-DD")]
        release_date: Option<String>,
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
    /// Run all: generate + stubs + scaffold + readme + docs + sync + e2e.
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
            hide = true,
        )]
        format: bool,
        /// Skip the flutter_rust_bridge_codegen post-build step.
        ///
        /// Useful when `flutter_rust_bridge` is not installed on the host (e.g.
        /// CI environments or developer machines without the Flutter SDK).
        /// Equivalent to setting `ALEF_SKIP_COMMANDS=flutter_rust_bridge_codegen`
        /// or `[crates.dart] skip_frb = true` in alef.toml.
        #[arg(long)]
        skip_frb: bool,
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
            hide = true,
        )]
        format: bool,
    },
    /// Generate or check the versioned alef.toml JSON Schema.
    Schema {
        /// Output JSON Schema file.
        #[arg(long, short, default_value = crate::core::config::DEFAULT_SCHEMA_PATH)]
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
pub(crate) enum PublishAction {
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
pub(crate) enum E2eAction {
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
            hide = true,
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
pub(crate) enum TestAppsAction {
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
            hide = true,
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
pub(crate) enum CacheAction {
    /// Clear the .alef/ cache directory.
    Clear,
    /// Show cache status.
    Status,
}

#[derive(Subcommand)]
pub(crate) enum ValidateAction {
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
