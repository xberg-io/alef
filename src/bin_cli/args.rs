use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::cli::commands;

#[derive(Parser)]
// `version` keeps `-V` a single bare `alef <semver>` line; `long_version` gives `--version` the
// build provenance stamped by `build.rs` — commit, build time, and whether the tree was dirty.
// The semver stays alone on the first line of both. ~keep
#[command(
    name = "alef",
    version,
    long_version = crate::bin_cli::build_info::long_version(),
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
    ///
    /// Writes bindings and scaffold files, then syncs version pins. It does NOT
    /// regenerate test_apps/ — that tree belongs to `alef all`'s test-apps stage
    /// and to `alef test-apps generate`. A stale test_apps/ after this command is
    /// expected, not a bug.
    Generate {
        /// Comma-separated list of languages (default: all from config).
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Ignore cache, regenerate everything.
        #[arg(long)]
        clean: bool,
        /// Skip the flutter_rust_bridge_codegen post-build step.
        ///
        /// Useful when `flutter_rust_bridge` is not installed on the host (e.g.
        /// CI environments or developer machines without the Flutter SDK).
        /// Equivalent to setting `ALEF_SKIP_COMMANDS=flutter_rust_bridge_codegen`
        /// or `[crates.dart] skip_frb = true` in alef.toml.
        #[arg(long)]
        skip_frb: bool,
        /// Fail the run when a configured formatter's executable is not installed.
        ///
        /// Means exactly what `alef all --strict` means, for the same reason it exists
        /// there: by default a missing formatter is recorded as a deferred step and the
        /// run continues, because `poly`, `rustfmt`, `cargo-sort` and `mix` are host
        /// toolchains a fresh clone may legitimately lack. A formatter that RUNS and
        /// rejects the code always fails, with or without this.
        #[arg(long)]
        strict: bool,
        /// Write and post-process source without invoking a compiler.
        ///
        /// Generation is documented as a source-writing step, but two of its post-build stages
        /// legitimately shell out to cargo: Swift's, to run the swift-bridge crate's `build.rs`
        /// so the `SwiftBridgeCore.swift`/`{crate}.swift`/`RustBridgeC.h` trio exists in
        /// `OUT_DIR` for `MaterializeSwiftBridge` to copy out, and the FFI header gate, to let
        /// cbindgen re-emit a header before checking it against the generated source. Even as a
        /// `cargo check`, the first walks the consumer's entire dependency graph.
        ///
        /// This flag skips exactly those two and nothing else. It is OFF by default, so a
        /// workflow that relies on `alef all` producing them keeps getting them; pass it only
        /// when a separate build task compiles the crate. The artifacts they derive then keep
        /// whatever content is already on disk until `alef build` runs, and every skipped step
        /// says so in the log. A cbindgen header that is present but *stale* still fails the
        /// run -- this flag suppresses the rebuild, never the check.
        #[arg(long)]
        skip_compile: bool,
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
        /// Skip compile/type-check/run snippet validation (`docs.snippets.validation_level`).
        ///
        /// Snippet discovery, the reference-page audit, and gap detection still run; only
        /// the step that spawns a compiler, interpreter, or type-checker per snippet is
        /// skipped. Useful when the language toolchains or built artifacts that snippet
        /// validation depends on are not present yet (e.g. before `alef build`, or on a
        /// clean checkout with no build cache), so docs stay cheap to regenerate.
        #[arg(long)]
        skip_snippet_validation: bool,
    },
    /// Sync version from Cargo.toml to all package manifests.
    ///
    /// Updates version fields in all package manifests and alef.toml registry
    /// package pins atomically. Does not regenerate code.
    ///
    /// To pick the follow-up command, note which tree you need rewritten.
    /// `alef generate` writes bindings and scaffold files only — it does NOT
    /// regenerate test_apps/. Only `alef all` (its test-apps stage) and
    /// `alef test-apps generate` write that tree; `--regen` below reaches it too.
    SyncVersions {
        /// Bump version before syncing (major, minor, patch).
        #[arg(long)]
        bump: Option<String>,
        /// Set version explicitly (e.g., "0.1.0-rc.1").
        #[arg(long)]
        set: Option<String>,
        /// Regenerate test_apps/ and scaffold files after syncing versions.
        /// By default, sync-versions only updates manifests; use this flag to
        /// also regenerate code (expensive, normally run separately as `alef all`).
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
    ///
    /// Fails by default (no `--exit-code` needed -- see that flag) on every drift/staleness/
    /// coverage finding this command reports, with one deliberate exception: a drifted
    /// create-once seed -- a pre-existing, unmarked file the write guard refuses to touch
    /// forever, such as a version-bearing installer or manifest whose baked-in content has gone
    /// stale. That finding is printed on every run, including a passing one, under "frozen
    /// path(s) DRIFTED", but it never gates the exit code: failing the build
    /// for content a consumer cannot fix from their own `alef.toml` (the write guard will not
    /// overwrite it) would only buy a blanket `[workspace.ownership] user_owned` opt-out rather
    /// than a fix -- the same reasoning this codebase already applies to `GeneratorGap`. To make
    /// this fatal in your own CI, grep the heading above in the `alef verify` log, or declare
    /// the path `user_owned` in `alef.toml` to silence it once reviewed.
    Verify {
        /// Deprecated compatibility flag; verification now fails by default -- see this
        /// command's own doc for exactly what that default failure does and does not cover.
        #[arg(long, hide = true)]
        exit_code: bool,
        /// Report drift without returning a failure status.
        #[arg(long)]
        report_only: bool,
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
        /// Fail the run when a language was skipped because its toolchain is not on PATH.
        ///
        /// Off by default: a developer without e.g. `gradle` installed must still be able to
        /// build the languages they do have. In CI, a skipped language means its bindings were
        /// never built and never validated -- pass this flag there so that gap surfaces as a
        /// non-zero exit instead of a log line nobody read.
        #[arg(long)]
        strict: bool,
    },
    /// Run all: generate + stubs + scaffold + readme + docs + sync + e2e + test-apps.
    ///
    /// The test-apps stage runs whenever an `[e2e]` block is configured, and it is
    /// the only stage that writes test_apps/. It is cache-gated like every other
    /// stage: on a cache hit it logs `[test-apps] up to date (skipping)` and reuses
    /// the recorded paths. That log line means the stage ran and found nothing to
    /// do — it does NOT mean `alef all` skips test apps. Use `--clean` to force it.
    All {
        /// Ignore cache, recomputing every stage from source.
        ///
        /// Purely a cache flag: it deletes nothing, and it does not widen which pre-existing
        /// files alef is allowed to write over. Snippet validation session scratch under
        /// `<cwd>/.alef/snippets/sessions/` is not cache and is not this flag's business — stale
        /// sessions there are swept on every run, with or without `--clean`.
        ///
        /// This flag used to ALSO be threaded into `write_scaffold_files_report`'s `overwrite`
        /// for the scaffold and docs stages, which disabled the create-only branch that leaves
        /// an already-existing unmarked file alone — so a routine cache-cold rerun could replace
        /// a hand-grown seed with alef's placeholder. That behaviour now lives on
        /// `--clobber-create-once-seeds`; pass both flags together for the old meaning.
        #[arg(long)]
        clean: bool,
        // Deliberately the same name as `alef adopt --clobber-create-once-seeds`: the two
        // authorise the same destruction through different doors -- adopt stamps a marker so a
        // LATER regen replaces the file, this one replaces it on THIS run -- and one concept
        // wearing two spellings is how `--clean` came to mean two things in the first place.
        // Named for the damage rather than the mechanism, for the same reason as adopt's: a
        // `--force` or `--overwrite` spelling reads as routine scope-widening, and reading it
        // that way is exactly what put `--clean` into five consumer repos' default regeneration
        // task. ~keep
        /// DANGEROUS: overwrite pre-existing create-once seeds (composer.json, package.json,
        /// placeholder test files, ...) with freshly generated content instead of leaving them
        /// alone. Only files alef can prove it authored are affected; anything the ownership
        /// guard cannot vouch for is still refused, with or without this flag.
        #[arg(long)]
        clobber_create_once_seeds: bool,
        /// Fail the run when a configured formatter's executable is not installed.
        ///
        /// By default a missing formatter is recorded as a deferred step and the run
        /// continues, so generation still reaches finalisation and the tree is stamped
        /// rather than left unstamped and indistinguishable from a stripping bug. A
        /// formatter that RUNS and rejects the code always fails, with or without this.
        #[arg(long)]
        strict: bool,
        /// Skip the flutter_rust_bridge_codegen post-build step.
        ///
        /// Useful when `flutter_rust_bridge` is not installed on the host (e.g.
        /// CI environments or developer machines without the Flutter SDK).
        /// Equivalent to setting `ALEF_SKIP_COMMANDS=flutter_rust_bridge_codegen`
        /// or `[crates.dart] skip_frb = true` in alef.toml.
        #[arg(long)]
        skip_frb: bool,
        /// Skip compile/type-check/run snippet validation (`docs.snippets.validation_level`)
        /// in the docs stage.
        ///
        /// Snippet discovery, the reference-page audit, and gap detection still run; only
        /// the step that spawns a compiler, interpreter, or type-checker per snippet is
        /// skipped. `alef all` never builds the full per-language artifacts that
        /// `typecheck`/`compile`/`run` validation levels need (only `alef build` does), so
        /// this flag is useful whenever those artifacts are not already present.
        #[arg(long)]
        skip_snippet_validation: bool,
        /// Write and post-process source without invoking a compiler.
        ///
        /// Generation is documented as a source-writing step, but two of its post-build stages
        /// legitimately shell out to cargo: Swift's, to run the swift-bridge crate's `build.rs`
        /// so the `SwiftBridgeCore.swift`/`{crate}.swift`/`RustBridgeC.h` trio exists in
        /// `OUT_DIR` for `MaterializeSwiftBridge` to copy out, and the FFI header gate, to let
        /// cbindgen re-emit a header before checking it against the generated source. Even as a
        /// `cargo check`, the first walks the consumer's entire dependency graph.
        ///
        /// This flag skips exactly those two and nothing else. It is OFF by default, so a
        /// workflow that relies on `alef all` producing them keeps getting them; pass it only
        /// when a separate build task compiles the crate. The artifacts they derive then keep
        /// whatever content is already on disk until `alef build` runs, and every skipped step
        /// says so in the log. A cbindgen header that is present but *stale* still fails the
        /// run -- this flag suppresses the rebuild, never the check.
        #[arg(long)]
        skip_compile: bool,
    },
    /// Initialize a new alef.toml config.
    Init {
        /// Comma-separated list of languages.
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
    },
    /// Generate or check the versioned alef.toml JSON Schema.
    ///
    /// The schema describes alef.toml itself, so vendoring a copy is optional and explicit:
    /// no other command writes it, and `alef generate`/`alef build`/`alef all` neither create
    /// nor refresh it. `alef verify` reports an existing copy at the default path when it has
    /// drifted -- failing only when it describes a different config surface, not when the
    /// embedded version stamp alone is behind.
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
    // Never add this to `alef all`, `alef generate`, or any other command's sequence.
    // The ownership guard refuses to overwrite a file it cannot prove it authored, and
    // no predicate over file content can tell "alef wrote this under an older release"
    // apart from "someone hand-wrote this" -- both are the same bytes. Only a human
    // reading the diff can, which is why this is a separate command with a `--write`
    // gate. See `cli::commands::adopt`. ~keep
    /// Take ownership of a pre-existing generated file so alef can regenerate it.
    ///
    /// Prints the full diff between the file on disk and what alef would generate,
    /// and changes nothing unless --write is given.
    Adopt {
        /// One or more repo-relative paths or globs to adopt, e.g.
        /// `crates/foo-ffi/Cargo.toml` or `crates/foo-ffi/Cargo.toml packages/**/*.gemspec`.
        #[arg(value_name = "PATH_OR_GLOB", required = true)]
        targets: Vec<String>,
        /// Stamp the marker. Without this, adopt only prints the diff.
        #[arg(long)]
        write: bool,
        // Subtractive only: it narrows what `--write` adopts, never widens it. Nothing
        // additive exists on this axis -- no `--all`/`--yes` -- since the only thing such
        // a flag could buy is skipping the drifted diffs, and a drifted file may be a
        // deliberate hand-edit, which is the content the guard exists to protect. ~keep
        /// Adopt only files already byte-identical to generated output, leaving every
        /// drifted match untouched. For large migrations.
        #[arg(long)]
        converged_only: bool,
        // Named for the damage rather than the mechanism. Both "clobber" and "seeds" are
        // in the flag so the one-line form a user pastes out of a runbook still says what
        // it does; an `--include-*` or `--force` spelling reads as routine scope-widening,
        // and this widens scope onto precisely the paths where adoption destroys work on a
        // later, unrelated command. ~keep
        // The timing is stated exactly. Adoption itself writes no seed byte -- for an
        // unmarkable seed (LICENSE, mvnw, .gitkeep) it only records the path in
        // `.alef-ownership.toml` -- and a plain `alef generate` skips the seed regardless
        // (`write_scaffold_files_report`'s `can_skip`). The loss lands on the next write that
        // passes `overwrite: true`, where the record or marker adoption left behind is what
        // clears the ownership guard. Saying "on the next generate" invited an operator to
        // run one, see the file untouched, and dismiss a warning that is true. ~keep
        /// DANGEROUS: also adopt create-once seeds (real test suites, build.zig, LICENSE, ...).
        /// alef emits these only when absent, so adopting one consents to alef replacing its
        /// contents with a placeholder seed on the next OVERWRITING regen -- an `alef version`
        /// sync or `alef all --clobber-create-once-seeds`. A plain `alef generate` skips them.
        #[arg(long)]
        clobber_create_once_seeds: bool,
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
    /// Build, package, lock, and verify downloadable native components.
    Component {
        #[command(subcommand)]
        action: ComponentAction,
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
        /// Homebrew tap repository, or Scoop bucket repository (`owner/repo`).
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
pub(crate) enum ComponentAction {
    /// Compile configured component feature profiles as native dynamic libraries.
    Build {
        /// Component profile names (default: every configured profile).
        #[arg(long, value_delimiter = ',')]
        component: Vec<String>,
        /// Rust target triples (default: each profile's configured targets).
        #[arg(long, value_delimiter = ',')]
        target: Vec<String>,
        /// Build without release optimizations.
        #[arg(long)]
        debug: bool,
        /// Print build commands without executing them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Create deterministic signed component archives and artifact records.
    Package {
        /// Component profile names (default: every configured profile).
        #[arg(long, value_delimiter = ',')]
        component: Vec<String>,
        /// Rust target triples (default: each profile's configured targets).
        #[arg(long, value_delimiter = ',')]
        target: Vec<String>,
        /// Output directory for component archives and records.
        #[arg(long, short, default_value = "dist/components")]
        output: PathBuf,
        /// Component version (default: resolved from version_from).
        #[arg(long)]
        version: Option<String>,
        /// Ed25519 private key accepted by `openssl pkeyutl` (PEM or DER).
        #[arg(long, requires = "key_id", conflicts_with = "unsigned")]
        signing_key: Option<PathBuf>,
        /// Stable ID of the public key embedded in generated binding packages.
        #[arg(long, requires = "signing_key")]
        key_id: Option<String>,
        /// Produce an unsigned CI intermediate archive.
        #[arg(long, conflicts_with = "signing_key")]
        unsigned: bool,
        /// Explicit built library path; valid only for one profile/target.
        #[arg(long)]
        library: Option<PathBuf>,
    },
    /// Generate and stage a binding-embeddable components.lock.json from signed artifact records.
    Lock {
        /// Directory containing `*.record.json` files from `component package`.
        #[arg(long, short, default_value = "dist/components")]
        input: PathBuf,
        /// Output lock manifest path.
        #[arg(long, short, default_value = "components.lock.json")]
        output: PathBuf,
    },
    /// Verify artifact hashes, identities, library contents, and Ed25519 signatures.
    Verify {
        /// Artifact record file or directory containing `*.record.json` files.
        #[arg(long, short, default_value = "dist/components")]
        input: PathBuf,
    },
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
        /// Fail the run when a configured formatter's executable is not installed.
        ///
        /// See `alef all --strict`; a formatter that runs and rejects the code always
        /// fails regardless.
        #[arg(long)]
        strict: bool,
        /// Downgrade an unresolvable e2e assertion field to a warning instead of failing
        /// generation. Equivalent to `ALEF_E2E_STRICT_ASSERTIONS=0`; see
        /// `e2e::codegen::STRICT_ASSERTIONS_ENV`. For an emergency regeneration only --
        /// the debt is still counted in the end-of-run summary either way.
        #[arg(long)]
        no_strict_assertions: bool,
    },
    /// Compare handwritten snippets with fixture-generated equivalents.
    SnippetsMigrate {
        /// Root directory containing the existing handwritten snippets.
        #[arg(value_name = "EXISTING_ROOT")]
        existing_root: PathBuf,
        /// Comma-separated list of snippet languages (default: snippet or e2e config).
        #[arg(long, value_delimiter = ',')]
        lang: Option<Vec<String>>,
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
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
        /// Maximum parallel jobs (0 = all cores, 1 = sequential).
        #[arg(short, long, default_value = "0")]
        jobs: usize,
        /// Fail the run when a configured formatter's executable is not installed.
        ///
        /// See `alef all --strict`; a formatter that runs and rejects the code always
        /// fails regardless.
        #[arg(long)]
        strict: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect `(name, about + long_about)` for every subcommand, recursively.
    fn help_texts(command: &clap::Command, prefix: &str, out: &mut Vec<(String, String)>) {
        for sub in command.get_subcommands() {
            let name = if prefix.is_empty() {
                sub.get_name().to_string()
            } else {
                format!("{prefix} {}", sub.get_name())
            };
            let mut text = String::new();
            if let Some(about) = sub.get_about() {
                text.push_str(&about.to_string());
                text.push('\n');
            }
            if let Some(long_about) = sub.get_long_about() {
                text.push_str(&long_about.to_string());
            }
            for arg in sub.get_arguments() {
                if let Some(help) = arg.get_long_help().or_else(|| arg.get_help()) {
                    text.push('\n');
                    text.push_str(&help.to_string());
                }
            }
            out.push((name.clone(), text));
            help_texts(sub, &name, out);
        }
    }

    /// `alef generate` does not write `test_apps/`: its handler calls
    /// `pipeline::sync_versions(.., no_regen = true, ..)`, which skips
    /// `regenerate_test_apps_after_sync` entirely. Only `alef all`'s test-apps stage and
    /// `alef test-apps generate` write that tree.
    ///
    /// The help text used to claim the opposite, and an operator who believes it goes looking
    /// for a regeneration bug that does not exist. This pins the property rather than a
    /// literal: any help text that names `alef generate` alongside `test_apps` must be
    /// disclaiming the write, not promising it. ~keep
    #[test]
    fn no_help_text_claims_alef_generate_writes_test_apps() {
        use clap::CommandFactory;

        let command = Cli::command();
        let mut texts = Vec::new();
        help_texts(&command, "", &mut texts);
        assert!(!texts.is_empty(), "expected to walk at least one subcommand");

        for (name, text) in &texts {
            if !text.contains("test_apps") || !text.contains("alef generate") {
                continue;
            }
            assert!(
                text.contains("NOT"),
                "`alef {name}` help mentions both `alef generate` and test_apps without \
                 disclaiming the write. `alef generate` does not regenerate test_apps/ -- it \
                 calls sync_versions with no_regen = true. Attribute that tree to `alef all` \
                 or `alef test-apps generate`, or spell the exclusion with an explicit NOT.\n\
                 help text was:\n{text}"
            );
        }
    }

    /// The `[test-apps] up to date (skipping)` cache-hit log reads as "this stage did not run",
    /// which is how the belief that `alef all` excludes test apps survives. `alef all`'s own
    /// summary omitting the stage is the other half. Keep the stage named there. ~keep
    #[test]
    fn all_command_help_names_the_test_apps_stage() {
        use clap::CommandFactory;

        let command = Cli::command();
        let all = command
            .get_subcommands()
            .find(|sub| sub.get_name() == "all")
            .expect("`alef all` subcommand");
        let about = all.get_about().map(|a| a.to_string()).unwrap_or_default();
        assert!(
            about.contains("test-apps"),
            "`alef all` runs a test-apps stage whenever an [e2e] block is configured, but its \
             one-line summary does not name it: {about:?}"
        );
    }

    #[test]
    fn parses_e2e_snippets_migrate_options() {
        let cli = Cli::try_parse_from([
            "alef",
            "e2e",
            "snippets-migrate",
            "docs/handwritten",
            "--lang",
            "python,rust",
            "--json",
        ])
        .expect("parse snippets migration command");

        let Commands::E2e {
            action:
                E2eAction::SnippetsMigrate {
                    existing_root,
                    lang,
                    json,
                },
        } = cli.command
        else {
            panic!("expected snippets migration command");
        };
        assert_eq!(existing_root, PathBuf::from("docs/handwritten"));
        assert_eq!(lang, Some(vec!["python".into(), "rust".into()]));
        assert!(json);
    }

    /// Regression for task #542: `alef snippets check` had no way to request compile-level
    /// checking for one invocation without weakening `[docs.snippets].validation_level` for
    /// `alef all`/`alef docs`, which never build first and would then warn on every run for a
    /// config that is entirely correct for `check`. `--level` is the explicit override. ~keep
    #[test]
    fn parses_snippets_check_level_override() {
        let cli = Cli::try_parse_from([
            "alef",
            "snippets",
            "check",
            "--config",
            "alef.toml",
            "--level",
            "compile",
        ])
        .expect("parse snippets check with --level");

        let Commands::Snippets {
            action: crate::cli::commands::snippets::SnippetsAction::Check { level, .. },
        } = cli.command
        else {
            panic!("expected snippets check command");
        };
        assert_eq!(level, Some("compile".to_string()));
    }

    /// Negative control: `--level` is optional, and omitting it must not change parsing.
    #[test]
    fn snippets_check_level_defaults_to_unset() {
        let cli = Cli::try_parse_from(["alef", "snippets", "check", "--config", "alef.toml"])
            .expect("parse snippets check without --level");

        let Commands::Snippets {
            action: crate::cli::commands::snippets::SnippetsAction::Check { level, .. },
        } = cli.command
        else {
            panic!("expected snippets check command");
        };
        assert_eq!(level, None);
    }

    #[test]
    fn verify_is_strict_by_default_and_accepts_compatibility_flags() {
        let strict = Cli::try_parse_from(["alef", "verify"]).expect("strict verify command");
        let Commands::Verify {
            exit_code, report_only, ..
        } = strict.command
        else {
            panic!("expected verify command");
        };
        assert!(!exit_code);
        assert!(!report_only);

        let compatible =
            Cli::try_parse_from(["alef", "verify", "--exit-code", "--report-only"]).expect("compatible verify command");
        let Commands::Verify {
            exit_code, report_only, ..
        } = compatible.command
        else {
            panic!("expected verify command");
        };
        assert!(exit_code);
        assert!(report_only);
    }
}
