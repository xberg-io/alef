//! Repo-root `poly.toml` scaffolding.
//!
//! Emits the single config that drives `poly lint`, `poly fmt`, `poly hooks`,
//! and `poly commit` — replacing the former `.pre-commit-config.yaml` and the
//! per-tool config files (`[tool.ruff]`, `[tool.mypy]`, `phpstan.neon`,
//! `.php-cs-fixer.dist.php`, `.lintr`, `.typos.toml`).
//!
//! poly covers most languages natively with zero system dependencies (ruff for
//! Python lint+fmt, oxc for JS/TS/JSON, taplo for TOML, rumdl for Markdown,
//! mago for PHP, jarl+air for R, rubyfmt for Ruby format, rustfmt + the `cargo`
//! builtin for Rust). It auto-detects languages and dispatches the right engine,
//! so only languages that need non-default config get an explicit table here:
//! Python (ruff rule selection + pyrefly type-check hook) and PHP (mago
//! correctness/security ruleset).
//!
//! `[defaults]` is omitted because poly's built-in defaults already match ours
//! (line_length 120, lf, final_newline, trim_trailing_whitespace).

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use std::path::PathBuf;

/// Anchoring behavior for an emitted poly exclude pattern.
///
/// `[discovery] exclude` is consumed by `ignore::overrides`, which uses
/// gitignore semantics: a pattern with no `/` matches at any depth. `[hooks.builtin]`
/// lint/fmt/file_safety `exclude` is consumed by `globset::Glob::new`, which always
/// matches the whole repo-relative path — there is no bare-pattern "any depth"
/// special case there. The same literal pattern (e.g. `target/**`) therefore means
/// something different to the two consumers, and syntax alone cannot disambiguate
/// it: alef's own list has bare patterns that need OPPOSITE scopes (`fixtures/**`
/// must stay root-anchored so it doesn't prune nested `tests/fixtures` trees;
/// `target/**` must match at any depth so it prunes nested build dirs). Scope is
/// therefore tagged per entry via [`ExcludeEntry`] rather than inferred from `/`
/// characters in the pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExcludeScope {
    /// Matches only at the repository root.
    RepoRoot,
    /// Matches at any depth in the tree.
    AnyDepth,
}

/// One exclude pattern with its anchoring [`ExcludeScope`] attached. `pattern`
/// itself carries NO anchor marker (no leading `/`, no `**/` prefix) — that is
/// added by [`ExcludeEntry::for_discovery`] / [`ExcludeEntry::for_hooks`], which
/// lower the same entry differently for the two consumers described on
/// [`ExcludeScope`].
#[derive(Debug, Clone)]
struct ExcludeEntry {
    pattern: String,
    scope: ExcludeScope,
}

impl ExcludeEntry {
    fn root(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            scope: ExcludeScope::RepoRoot,
        }
    }

    fn any_depth(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            scope: ExcludeScope::AnyDepth,
        }
    }

    /// Lower for `[discovery] exclude` (gitignore semantics via `ignore::overrides`).
    fn for_discovery(&self) -> String {
        match self.scope {
            ExcludeScope::RepoRoot => format!("/{}", self.pattern),
            ExcludeScope::AnyDepth => format!("**/{}", self.pattern),
        }
    }

    /// Lower for `[hooks.builtin]` lint/fmt/file_safety excludes (whole-path
    /// matching via `globset`, which needs an explicit `**/` for "any depth" but no
    /// anchor at all for "root only" — a bare pattern already only matches from the
    /// start of the repo-relative path).
    ///
    /// The load-bearing property, measured against `globset` 0.4 rather than assumed:
    /// a leading `**/` matches ZERO or more components, so `**/target/**` still matches
    /// a ROOT-level `target/x` as well as a nested `crates/a/target/x`. If it required
    /// at least one component, [`ExcludeScope::AnyDepth`] would silently stop excluding
    /// root-level `target/` and `node_modules/` from the commit hooks — a regression in
    /// the exact direction this split exists to fix, and one that every string-equality
    /// test here would still pass. The same probe confirms the defect being fixed: bare
    /// `target/**` does NOT match `crates/a/target/x`, which is why mirroring the
    /// discovery list verbatim under-excluded nested build dirs in the hook path. ~keep
    fn for_hooks(&self) -> String {
        match self.scope {
            ExcludeScope::RepoRoot => self.pattern.clone(),
            ExcludeScope::AnyDepth => format!("**/{}", self.pattern),
        }
    }
}

/// Classify an author-supplied `config.poly.exclude` / `file_safety_exclude`
/// pattern into a scoped [`ExcludeEntry`], stripping any anchor marker so the
/// stored pattern carries none (matching [`EXCLUDES`]'s own convention):
///
/// * A `**/` prefix is explicit "any depth" intent — strip it, tag [`ExcludeScope::AnyDepth`].
/// * A leading `/` or `./` is explicit "repo root" intent — strip it, tag [`ExcludeScope::RepoRoot`].
/// * A bare pattern (no anchor marker) defaults to [`ExcludeScope::RepoRoot`] — the
///   dominant real intent for a repo-specific exclude and the field's own doc
///   example (`vendor/generated/**`, a project-specific root path). `**/foo/**`
///   remains the idiomatic escape hatch for "any depth": both matchers read it
///   identically either way.
fn classify_extra(raw: &str) -> ExcludeEntry {
    if let Some(rest) = raw.strip_prefix("**/") {
        return ExcludeEntry::any_depth(rest);
    }
    if let Some(rest) = raw.strip_prefix("./") {
        return ExcludeEntry::root(rest);
    }
    if let Some(rest) = raw.strip_prefix('/') {
        return ExcludeEntry::root(rest);
    }
    ExcludeEntry::root(raw)
}

/// Globs pruned from every lint/format pass, tagged with their [`ExcludeScope`].
/// Lowered into `[discovery] exclude` (direct `poly lint`/`poly fmt`/CI path) and
/// the `lint`/`fmt`/`file_safety` builtin excludes (the git-hook path, which
/// filters per-builtin rather than via discovery) — see [`ExcludeEntry`].
///
/// Covers build output + lock files, plus the conventional non-source trees a
/// polyglot repo keeps under version control that must NOT be linted/reformatted:
/// `fixtures/` and `test_documents/` (exact-byte test data). Generated code under
/// `packages/`, `e2e/`, `test_apps/` is deliberately NOT excluded — poly owns its
/// formatting.
///
/// Deliberately NOT here: a `docs/assets/**`-style entry for canonical doc images.
/// Nothing in [`crate::core::config`] names a docs-asset directory, so a hardcoded
/// `docs/` guess is unverifiable and, for a consumer whose docs live in a different
/// tree (e.g. an Astro `docs-site/`), reinstates a phantom path on every run — see
/// [`docs_snippets_excludes`] for the sibling entry this replaced, moved to a
/// config-derived path for exactly that reason. A consumer that does keep
/// hand-authored assets under a fixed path can add it via `[workspace.poly]
/// exclude` (`PolyConfig::exclude`), which this list is unioned with. ~keep
///
/// `artifacts/**`, `dist/**`, and `vendor/**` are tagged `AnyDepth` on the same
/// reasoning as `target/**` / `node_modules/**`: in a polyglot monorepo these are
/// per-package build-output / vendored-dependency directories (a Go module's
/// `vendor/`, a JS package's `dist/`, a CI job's `artifacts/`) that legitimately
/// recur under `crates/*` or `packages/*`, not just at the repo root.
///
/// `schemas/**` excludes the directory holding [`crate::core::config::DEFAULT_SCHEMA_PATH`]
/// (a consumer's vendored `alef.schema.json`, written only by `alef schema`) on the same
/// "exact-byte data" reasoning as `fixtures/**`/`test_documents/**`: `alef schema --check`
/// and `alef verify` classify that file byte-for-byte (see
/// `core::config::schema::check_alef_config_schema`'s doc), so a poly JSON-format pass
/// reshaping it after `alef schema` writes it put alef's own formatter and its own
/// byte-exact gate in permanent disagreement -- `alef schema --check` passed right after
/// `alef schema` ran, then failed stale the moment `alef all`'s whole-tree `poly fmt --fix`
/// pass (which, unlike alef's own dev-repo `poly.toml`, had nothing excluding this path)
/// touched the same file. Excluding it here, matching what alef's own repo already
/// configures for itself, keeps the byte-exact gate honest without trying to make
/// `serde_json::to_string_pretty`'s output byte-match an external formatter's opinion --
/// a coupling that would silently break again the next time that formatter's JSON style
/// changes. ~keep
const EXCLUDES: &[(&str, ExcludeScope)] = &[
    ("*.freezed.dart", ExcludeScope::AnyDepth),
    ("*.g.dart", ExcludeScope::AnyDepth),
    ("*.jinja", ExcludeScope::AnyDepth),
    ("*.lock", ExcludeScope::AnyDepth),
    ("Cargo.lock", ExcludeScope::AnyDepth),
    ("go.sum", ExcludeScope::AnyDepth),
    ("package-lock.json", ExcludeScope::AnyDepth),
    ("pnpm-lock.yaml", ExcludeScope::AnyDepth),
    ("uv.lock", ExcludeScope::AnyDepth),
    (".alef/**", ExcludeScope::RepoRoot),
    ("artifacts/**", ExcludeScope::AnyDepth),
    ("dist/**", ExcludeScope::AnyDepth),
    ("fixtures/**", ExcludeScope::RepoRoot),
    ("node_modules/**", ExcludeScope::AnyDepth),
    ("readme_templates/**", ExcludeScope::RepoRoot),
    ("schemas/**", ExcludeScope::RepoRoot),
    ("target/**", ExcludeScope::AnyDepth),
    ("templates/readme/**", ExcludeScope::RepoRoot),
    ("test_documents/**", ExcludeScope::RepoRoot),
    ("vendor/**", ExcludeScope::AnyDepth),
];

/// Build the `docs/snippets/**`-equivalent excludes from where the repo actually
/// configured its snippet roots, instead of a hardcoded `docs/snippets/**` that
/// assumes every consumer keeps a `docs/` tree. Snippets are compiled separately
/// by the snippet runner and must not be linted/reformatted as ordinary source.
///
/// Reads two independent config tables, not one:
///
/// - `[workspace.docs.snippets] dirs` — the roots snippet *validation* discovers files in.
/// - `[crates.e2e.snippets] output` — the root [`crate::e2e::snippets`] actually
///   *writes* generated fixture-snippet Markdown into. `docs.snippets` is a separate,
///   optional feature (MkDocs `--8<--` include discovery); a consumer using only
///   e2e-generated snippets has no reason to configure it too, and reading `dirs`
///   alone left `[e2e.snippets] output` — the tree that exists as soon as
///   `[e2e.snippets]` is configured at all — outside every exclude this function
///   emits.
///
/// The hazard motivating the second table is **latent, not observed**, and deserves that
/// label: poly's `[lint.uncomment]` pass (on by default in this same generated file) does
/// not list alef's marker among its preserved patterns, so it *could* strip the
/// `<!-- alef:hash: -->` comment `docs::render::with_html_header` embeds in generated
/// snippet Markdown — the only ownership proof such a file carries, since
/// `marker_comment_style` deliberately excludes `.md` (see its doc) — and losing it makes
/// the write guard refuse that file permanently. Nobody has watched that happen. The gap
/// also requires a consumer whose two keys *diverge*; in the trees surveyed they name the
/// same directory, so it is inert there and none of them is a test case for it. Fixed
/// because the divergent-key case is real and the fix is cheap, not because a live
/// instance is known.
///
/// Returns no entries (not even a default) when the repo has configured neither —
/// nothing to exclude yet, and guessing `docs/snippets` would reintroduce the
/// phantom-path defect this replaces. Deduplicated so a consumer who points both
/// configs at the same directory gets one exclude entry, not two identical ones. ~keep
fn docs_snippets_excludes(config: &ResolvedCrateConfig) -> Vec<ExcludeEntry> {
    let mut dirs: Vec<String> = config
        .docs
        .as_ref()
        .and_then(|docs| docs.snippets.as_ref())
        .into_iter()
        .flat_map(|snippets| snippets.dirs.iter())
        .map(|dir| dir.to_string_lossy().trim_end_matches('/').to_string())
        .collect();
    if let Some(output) = config
        .e2e
        .as_ref()
        .and_then(|e2e| e2e.snippets.as_ref())
        .map(|snippets| snippets.output.trim_end_matches('/'))
        && !output.is_empty()
        && !dirs.iter().any(|dir| dir.as_str() == output)
    {
        dirs.push(output.to_string());
    }
    dirs.into_iter()
        .map(|dir| ExcludeEntry::root(format!("{dir}/**")))
        .collect()
}

/// Globs excluded specifically to keep poly's whole-repo format pass from
/// fighting alef's residual native passes (see `cli::pipeline::format`):
///
/// * `Cargo.toml` — poly's taplo and `cargo sort` (run as a residual + by the
///   `cargo` hook) canonicalize TOML differently; letting both touch Cargo.toml
///   produces an infinite format/regen loop on the embedded hash. cargo-sort owns
///   Cargo.toml. Any depth: a workspace has one per crate, not just at the root.
///
/// Elixir is deliberately NOT excluded here any more: poly ≥0.19.6 lets a declared
/// `[tools.mix]` displace its generic tree-sitter reindenter, so `mix format` owns
/// Elixir source through poly rather than around it.
const POLY_FORMAT_EXCLUDES: &[(&str, ExcludeScope)] = &[("Cargo.toml", ExcludeScope::AnyDepth)];

/// Canonical clang-format style for cbindgen-generated C FFI headers, shipped so
/// every repo with an FFI target formats its headers identically (paired with the
/// `[tools.clang-format]` catalog opt-in emitted into `poly.toml`). Matches the
/// LLVM/4-space style the polyglot repos already commit.
const CLANG_FORMAT: &str = "\
---
BasedOnStyle: LLVM
IndentWidth: 4
ColumnLimit: 100
BreakBeforeBraces: Attach
AllowShortFunctionsOnASingleLine: Empty
AllowShortIfStatementsOnASingleLine: false
SortIncludes: true
";

/// Ruff rule families selected for generated + hand-written Python. This is an
/// explicit allowlist rather than `select = ["ALL"]`: enabling every rule then
/// suppressing the noise meant each ruff release could silently start firing a new
/// deny-by-default rule on the generated binding surface (the `CPY` copyright-header
/// family is the canonical example). We instead enable the families we actually
/// want. Families that used to be carried only to be fully suppressed via `ignore`
/// (`COM`, `FBT`, `FIX`, `TD`, `PD`, `EM`, `TRY`, `BLE`) are simply not selected.
const RUFF_SELECT: &[&str] = &[
    "F", "E", "W", "I", "N", "D", "UP", "ANN", "ASYNC", "S", "B", "A", "C4", "DTZ", "T10", "T20", "ISC", "ICN", "PIE",
    "PT", "Q", "RSE", "RET", "SIM", "TID", "TC", "ARG", "PTH", "PGH", "PL", "PERF", "FURB", "RUF",
];

/// Ruff sub-rules suppressed within the selected families (see `RUFF_SELECT`):
/// specific checks that would otherwise fire on the generated binding surface
/// without indicating a defect — missing module/package docstrings, line length
/// (owned by the formatter), and the security lints for bind-all-interfaces /
/// try-except-pass / subprocess use in generated glue.
const RUFF_IGNORE: &[&str] = &[
    "ANN401", "ASYNC109", "ASYNC110", "D100", "D104", "D107", "D205", "E501", "ISC001", "PGH003", "PLR2004", "PLW0603",
    "S104", "S110", "S603",
];

/// rumdl rules disabled repo-wide for Markdown — the Zensical docs convention
/// shared across every polyglot repo's `.rumdl.toml`: tables/code run long
/// (MD013), the docs use inline HTML grids/cards (MD033), Zensical tabs indent
/// fenced blocks (MD046) and rewrite anchors (MD051), READMEs intentionally skip
/// a leading H1 (MD041) and use emphasis-as-heading (MD036), etc.
const RUMDL_DISABLE: &[&str] = &[
    "MD012", "MD013", "MD024", "MD033", "MD036", "MD041", "MD046", "MD051", "MD076",
];

/// rumdl rules additionally disabled for `fmt` only, on top of `RUMDL_DISABLE`.
/// MD025 (multiple top-level headings) still lints, so a stray `#` heading is
/// reported, but its autofix is destructive: rumdl demotes the offending H1 *and
/// every heading after it*, which silently reparents an entire CHANGELOG under
/// `## [Unreleased]` and makes heading-scoped release-note extraction emit the
/// wrong section. Observed on alef's own CHANGELOG (338 lines rewritten, no
/// content lost). ~keep
const RUMDL_FMT_ONLY_DISABLE: &[&str] = &["MD025"];

/// mago rules suppressed: test-assertion style/consistency checks that fire on
/// the generated PHP e2e suites (phpunit assertions), plus `sensitive-parameter`
/// which fires on generated extension stubs (a codegen-shaped suggestion, not a
/// defect in the binding surface). Scoped here rather than narrowed to tests
/// because the generated binding code is already clean of the rest.
const MAGO_IGNORE: &[&str] = &[
    "strict-assertions",
    "use-specific-assertions",
    "no-redundant-variable",
    "sensitive-parameter",
];

/// Cross-engine rule codes relaxed for the GENERATED test/e2e suites
/// (`tests/`, `e2e/`, `test_apps/`). These are conventional test-code allowances
/// — unused test imports (`no-unused-vars`), `print`/diagnostics (`T201`),
/// pytest `raises` shape (`PT011`/`PT012`), assert/random/URL-in-test
/// (`S101`/`S310`/`S311`), literal creds in fixtures (`no-literal-password`),
/// missing annotations/docstrings, and magic values — none of which indicate a
/// defect in the binding surface (which lints clean). Engine-agnostic: a code
/// simply no-ops on files of other languages.
const TEST_IGNORES: &[&str] = &[
    "ANN",
    "D103",
    "PLR2004",
    "PLR0915",
    "PLR0913",
    "S101",
    "S105",
    "S106",
    "S108",
    "S310",
    "S311",
    "PT011",
    "PT012",
    "PERF401",
    "PTH123",
    "T201",
    "TC001",
    "TC002",
    "TC003",
    "INP001",
    "no-unused-vars",
    "no-literal-password",
    "no-unescaped-output",
    // defects in the binding surface: redundant `# noqa` (RUF100), unused/duplicate
    "RUF100",
    "F401",
    "F811",
    "I001",
    "PT018",
    "ARG001",
    "ARG002",
    "D403",
    "E713",
    "UP035",
    "UP012",
    "RUF015",
    "F541",
    "EXE001",
    "A001",
    "N801",
];

/// Render a TOML array of strings indented under `key = [`, one entry per line
/// with a trailing comma. An empty slice renders as the inline empty array `[]`.
///
/// The indent is 2 spaces because that is what taplo — and therefore
/// `poly fmt --check` in every consumer repo — emits; this used to be 4, which
/// meant the freshly written file was never poly-clean. taplo also collapses an
/// array onto one line when it fits, which this cannot know without the key
/// prefix, so [`normalize_poly_config`] hands the finished file to poly.
///
/// [`normalize_poly_config`]: crate::cli::pipeline::generate::scaffold
fn toml_array<S: AsRef<str>>(entries: &[S]) -> String {
    if entries.is_empty() {
        return "[]".to_string();
    }
    let inner = entries
        .iter()
        .map(|e| format!("  \"{}\",", e.as_ref()))
        .collect::<Vec<_>>()
        .join("\n");
    format!("[\n{inner}\n]")
}

/// Force forward slashes in a directory value about to be embedded in generated `poly.toml`
/// text.
///
/// `poly.toml` is a portable manifest, committed once and then parsed by every consumer of the
/// scaffolded repo regardless of which OS ran `alef scaffold`. Most `ResolvedCrateConfig::
/// package_dir` results are literal forward-slash strings, but Java's (and an explicitly
/// configured Kotlin/Kotlin Android's) project root is rebuilt through `PathBuf::join`, which
/// inserts the *host* separator -- on Windows that is `\`. `\` is a TOML basic-string escape
/// character, so a raw Windows path landing in `root = "{dir}"` either fails to parse outright
/// (an escape sequence TOML doesn't recognize, e.g. `\d`) or silently mangles the value (a
/// recognized one, e.g. `\r` becoming a literal carriage return) -- corrupting the checked-in
/// file for every other consumer, not just the machine that generated it. `package_dir` itself
/// must keep returning the host separator: its other callers feed real shell commands (`cd
/// {output_dir} && gradle`, `mvn -f {output_dir}/pom.xml`) that need it. Normalize only at this
/// text-embedding boundary. ~keep
fn portable_dir(dir: &str) -> String {
    dir.replace('\\', "/")
}

/// Emit a `[hooks.pre-commit.commands.<name>]` job that `poly lint` runs ONCE
/// over the whole project (`workspace = true`), from `dir`, delegating to an
/// external linter poly does not bundle (golangci-lint, checkstyle, credo, …). The
/// tool discovers its own native config file relative to `dir`; poly skips the job
/// gracefully when the binary is not installed. This is the only poly mechanism that
/// runs a whole-project tool once on `poly lint .` (the per-file `[tools.*]` catalog
/// tier cannot) — see the poly workspace-hook runner. Type-checkers and project-graph
/// linters belong here, not in `[tools]`.
///
/// A tool only belongs here if it can run in poly's ISOLATED staged snapshot, which
/// holds tracked content only — see `unrunnable_snapshot_hooks_are_not_emitted`.
fn workspace_hook(name: &str, dir: &str, run: &str, files_glob: &str) -> String {
    let dir = portable_dir(dir);
    format!(
        "\n[hooks.pre-commit.commands.{name}]\n\
         run = \"{run}\"\n\
         root = \"{dir}\"\n\
         workspace = true\n\
         files = \"{dir}/{files_glob}\"\n"
    )
}

/// Generate the repo-root `poly.toml` from the configured language set.
pub(crate) fn scaffold_poly_config(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    let has = |lang: Language| languages.contains(&lang);

    // `all_excludes` merges alef's own tagged EXCLUDES/POLY_FORMAT_EXCLUDES with
    // author-supplied `config.poly.exclude` entries (classified per `classify_extra`).
    // It backs BOTH [discovery] and the lint/fmt/file_safety hook builtins, but each
    // lowers it differently — see `ExcludeEntry::for_discovery` / `for_hooks`.
    let mut all_excludes: Vec<ExcludeEntry> = EXCLUDES
        .iter()
        .chain(POLY_FORMAT_EXCLUDES.iter())
        .map(|(pattern, scope)| ExcludeEntry {
            pattern: (*pattern).to_string(),
            scope: *scope,
        })
        .collect();
    all_excludes.extend(docs_snippets_excludes(config));
    all_excludes.extend(config.poly.exclude.iter().map(|raw| classify_extra(raw)));

    let discovery_excludes: Vec<String> = all_excludes.iter().map(ExcludeEntry::for_discovery).collect();
    let hooks_excludes: Vec<String> = all_excludes.iter().map(ExcludeEntry::for_hooks).collect();
    let discovery_excludes_toml = toml_array(&discovery_excludes);
    let hooks_excludes_toml = toml_array(&hooks_excludes);

    // file-safety-only globs (e.g. Rust `#![...]` inner attributes misread as
    // a shebang) are appended ONLY to the file_safety builtin, on top of the same
    // shared base + author `exclude` list, then lowered with hooks (whole-path)
    // semantics like every other `[hooks.builtin]` entry.
    let file_safety_excludes = if config.poly.file_safety_exclude.is_empty() {
        hooks_excludes_toml.clone()
    } else {
        let mut entries = all_excludes.clone();
        entries.extend(config.poly.file_safety_exclude.iter().map(|raw| classify_extra(raw)));
        let lowered: Vec<String> = entries.iter().map(ExcludeEntry::for_hooks).collect();
        toml_array(&lowered)
    };

    let mut out = String::new();

    out.push_str(&format!("[discovery]\nexclude = {discovery_excludes_toml}\n\n"));

    // The `[lint]` super-table must precede every `[lint.*]` sub-table below: a super-table
    // defined after its children parses but reads as a redefinition to a human. ~keep
    if let Some(workspace) = config.poly.lint_workspace {
        out.push_str(&format!("[lint]\nworkspace = {workspace}\n\n"));
    }

    let md_lint_disable = toml_array(RUMDL_DISABLE);
    let md_fmt_disable = toml_array(
        &RUMDL_DISABLE
            .iter()
            .chain(RUMDL_FMT_ONLY_DISABLE.iter())
            .copied()
            .collect::<Vec<_>>(),
    );
    out.push_str(&format!("[lint.markdown.rumdl]\ndisable = {md_lint_disable}\n\n"));
    out.push_str(&format!("[fmt.markdown.rumdl]\ndisable = {md_fmt_disable}\n\n"));

    // NOTE: alef deliberately does NOT enable poly's opt-in native-toolchain

    if has(Language::Python) {
        out.push_str(&format!(
            "[lint.python.ruff]\nselect = {select}\nignore = {ignore}\n",
            select = toml_array(RUFF_SELECT),
            ignore = toml_array(RUFF_IGNORE)
        ));
        out.push_str(
            "mccabe_max_complexity = 15\n\
             pydocstyle_convention = \"google\"\n\
             pylint_max_args = 10\n\
             pylint_max_branches = 15\n\
             pylint_max_returns = 10\n\n",
        );
    }

    if has(Language::Php) {
        out.push_str(&format!(
            "[lint.php.mago]\nselect = [\"correctness\", \"security\"]\nignore = {ignore}\nphp_version = \"8.2\"\n\n",
            ignore = toml_array(MAGO_IGNORE)
        ));
    }

    if !config.poly.typos.extend_words.is_empty() {
        out.push_str("[lint.typos.extend_words]\n");
        for (word, correct) in &config.poly.typos.extend_words {
            out.push_str(&format!("{word} = \"{correct}\"\n"));
        }
        out.push('\n');
    }
    if !config.poly.typos.extend_identifiers.is_empty() {
        out.push_str("[lint.typos.extend_identifiers]\n");
        for (ident, correct) in &config.poly.typos.extend_identifiers {
            out.push_str(&format!("{ident} = \"{correct}\"\n"));
        }
        out.push('\n');
    }

    if let Some(uncomment) = &config.poly.uncomment {
        out.push_str("[lint.uncomment]\n");
        out.push_str(&format!("enabled = {}\n", uncomment.enabled));
        out.push_str(&format!("remove_todos = {}\n", uncomment.remove_todos));
        out.push_str(&format!("remove_fixme = {}\n", uncomment.remove_fixme));
        out.push_str(&format!("remove_docs = {}\n", uncomment.remove_docs));
        out.push_str(&format!("use_default_ignores = {}\n", uncomment.use_default_ignores));
        let patterns: Vec<&str> = uncomment.preserve_patterns.iter().map(String::as_str).collect();
        out.push_str(&format!("preserve_patterns = {}\n\n", toml_array(&patterns)));
    }

    // cbindgen writes the C FFI header (crates/*-ffi/include/*.h) at BUILD time,
    // so alef's post-generate `poly fmt` pass never sees it. poly ships no native
    // C formatter, so enable its clang-format catalog tool: `poly fmt` and the
    // pre-commit hook then format those headers (using the scaffolded
    // `.clang-format`) whenever they exist. The headers are deliberately NOT in
    // EXCLUDES so poly can reach them.
    if has(Language::Ffi) {
        out.push_str("[tools.clang-format]\nenabled = true\n\n");
    }

    // poly has no native Elixir formatter and no bundled `indents.scm`, so without
    // this it reindents `.ex`/`.exs` with a hand-rolled tree-sitter query that models
    // only `do…end` and `fn…end` — every other construct captured nothing and was
    // re-emitted at column 0, so poly and `mix format` flattened and re-indented the
    // same file forever. Declaring the catalog tool hands the language to `mix
    // format`; poly ≥0.19.6 then drops its own reindenter for Elixir. ~keep
    if has(Language::Elixir) {
        let dir = portable_dir(&config.package_dir(Language::Elixir));
        out.push_str(&format!("[tools.mix]\nenabled = true\nroot = \"{dir}\"\n\n"));
    }

    out.push_str("[per-file-ignores]\n");
    if has(Language::Python) {
        out.push_str(
            "\"**/api.py\" = [ \"F401\", \"I001\", \"TC006\", \"UP035\" ]\n\
             \"**/*.pyi\" = [ \"A002\", \"F401\", \"I001\", \"PYI033\", \"TC006\", \"UP035\" ]\n\
             \"**/options.py\" = [ \"F401\", \"I001\", \"RUF100\" ]\n\
             \"**/__init__.py\" = [ \"I001\" ]\n",
        );
    }
    let test_ignores = toml_array(TEST_IGNORES);
    for glob in ["**/tests/**", "**/e2e/**", "**/test_apps/**"] {
        out.push_str(&format!("\"{glob}\" = {test_ignores}\n"));
    }
    for (glob, codes) in &config.poly.per_file_ignores {
        let code_refs: Vec<&str> = codes.iter().map(String::as_str).collect();
        out.push_str(&format!("\"{glob}\" = {}\n", toml_array(&code_refs)));
    }
    out.push('\n');

    out.push_str("[hooks]\nstages = [\"pre-commit\"]\n\n[hooks.builtin]\n");
    out.push_str(&format!("lint = {{ exclude = {hooks_excludes_toml} }}\n"));
    out.push_str(&format!("fmt = {{ exclude = {hooks_excludes_toml} }}\n"));
    out.push_str(&format!("file_safety = {{ exclude = {file_safety_excludes} }}\n"));
    out.push_str("cargo = true\n");
    out.push_str("commit = { stages = [\"commit-msg\"] }\n");

    // Whole-project linters / type-checkers that poly does not bundle. Each runs
    // once on `poly lint .` via a `workspace = true` hook (the per-file `[tools.*]`
    // tier cannot host project-graph tools), delegating to the language toolchain
    // and its native config. Absent toolchains are skipped by poly, so a consumer
    // that lacks e.g. maven simply doesn't run checkstyle.
    if has(Language::Python) {
        let py_dir = portable_dir(&config.package_dir(Language::Python));
        // pyrefly takes the dir as an argument rather than via `root`.
        out.push_str(&format!(
            "\n[hooks.pre-commit.commands.pyrefly]\nrun = \"pyrefly check {py_dir}\"\nworkspace = true\nfiles = \"{py_dir}/**/*.py\"\n"
        ));
    }
    if has(Language::Go) {
        let dir = config.package_dir(Language::Go);
        out.push_str(&workspace_hook(
            "golangci-lint",
            &dir,
            "golangci-lint run ./...",
            "**/*.go",
        ));
    }
    if has(Language::Java) {
        let dir = config.package_dir(Language::Java);
        out.push_str(&workspace_hook(
            "checkstyle",
            &dir,
            "mvn -q checkstyle:check",
            "**/*.java",
        ));
    }
    if has(Language::Elixir) {
        let dir = config.package_dir(Language::Elixir);
        // `mix deps.get` first: poly runs hooks from a staged snapshot outside the repo,
        // and Elixir resolves dependencies strictly project-locally into a gitignored
        // `deps/`, so credo's own package is missing there and mix aborts with "Unchecked
        // dependencies for environment dev". The snapshot persists between runs, so the
        // fetch is a one-time cost. The linters that remain beside it (golangci-lint,
        // checkstyle, pyrefly) resolve from a global store -- go module cache, `~/.m2`,
        // the active interpreter -- and need no such priming. ~keep
        out.push_str(&workspace_hook(
            "credo",
            &dir,
            "mix deps.get && mix credo --strict",
            "**/*.{ex,exs}",
        ));
    }

    for source in &config.poly.hooks_sources {
        let hook_refs: Vec<&str> = source.hooks.iter().map(String::as_str).collect();
        out.push_str(&format!(
            "\n[[hooks.sources]]\nid = \"{}\"\ngit = \"{}\"\nrevision = \"{}\"\nhooks = {}\n",
            source.id,
            source.git,
            source.revision,
            toml_array(&hook_refs),
        ));
    }

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: out,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("rustfmt.toml"),
            content: "max_width = 120\n".to_string(),
            generated_header: true,
        },
    ];

    // Ship the canonical `.clang-format` for repos with a C FFI header so cbindgen
    // output formats identically everywhere. alef-owned (overwritten every run) to
    // keep the C style uniform across all consumer repos.
    if has(Language::Ffi) {
        files.push(GeneratedFile {
            path: PathBuf::from(".clang-format"),
            content: CLANG_FORMAT.to_string(),
            generated_header: true,
        });
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{E2eConfig, ResolvedCrateConfig};

    fn poly_toml() -> String {
        scaffold_poly_config(&ResolvedCrateConfig::default(), &[Language::Rust])
            .into_iter()
            .find(|file| file.path == std::path::Path::new("poly.toml"))
            .expect("scaffold emits poly.toml")
            .content
    }

    fn poly_toml_for(config: &ResolvedCrateConfig, languages: &[Language]) -> toml::Value {
        let content = scaffold_poly_config(config, languages)
            .into_iter()
            .find(|file| file.path == std::path::Path::new("poly.toml"))
            .expect("scaffold emits poly.toml")
            .content;
        toml::from_str(&content).expect("generated poly.toml is valid")
    }

    /// The pre-commit hook set is deliberately limited to linters that can actually pass
    /// there. poly runs hooks against an ISOLATED staged snapshot (a fresh copy under
    /// `~/.cache/poly/<hash>/staged/`) holding TRACKED content only, so a tool that needs its
    /// dependency graph materialized into a gitignored, project-local directory fails before it
    /// examines a single file: `dart analyze` reports `uri_does_not_exist` for every `package:`
    /// import (no `.dart_tool/package_config.json`), and rubocop/steep never start because alef
    /// pins Bundler to a gitignored `BUNDLE_PATH=vendor/bundle` (`Bundler::GemNotFound`). Those
    /// four hooks are not emitted at all -- CI lints Ruby through `scripts/ci/ruby/*` and Dart
    /// is not statically analysed in CI today. credo stays because it is primed with
    /// `mix deps.get`; the rest resolve from a global store. ~keep
    #[test]
    fn unrunnable_snapshot_hooks_are_not_emitted() {
        // An e2e Dart target IS configured, so the absence below is a real decision rather
        // than a vacuous "nothing asked for a Dart e2e hook". ~keep
        let config = ResolvedCrateConfig {
            e2e: Some(E2eConfig {
                output: "e2e".to_string(),
                ..E2eConfig::default()
            }),
            ..ResolvedCrateConfig::default()
        };
        let document = poly_toml_for(
            &config,
            &[
                Language::Dart,
                Language::Ruby,
                Language::Elixir,
                Language::Go,
                Language::Java,
                Language::Python,
            ],
        );
        let commands = &document["hooks"]["pre-commit"]["commands"];

        for hook in ["dart-analyze", "dart-e2e-analyze", "rubocop", "steep"] {
            assert!(
                commands.get(hook).is_none(),
                "`{hook}` cannot pass in poly's staged snapshot and must not be emitted"
            );
        }

        // Exact equality, never `contains`: `"mix credo --strict"` is a SUBSTRING of the primed
        // command, so a containment check would still pass if the priming were dropped. ~keep
        for (hook, expected) in [
            ("credo", "mix deps.get && mix credo --strict"),
            ("golangci-lint", "golangci-lint run ./..."),
            ("checkstyle", "mvn -q checkstyle:check"),
            ("pyrefly", "pyrefly check packages/python"),
        ] {
            assert_eq!(commands[hook]["run"].as_str(), Some(expected), "wrong `{hook}` command");
        }
    }

    fn disable_list(content: &str, table: &str) -> String {
        let start = content
            .find(table)
            .unwrap_or_else(|| panic!("{table} present in poly.toml"));
        let rest = &content[start + table.len()..];
        let open = rest.find('[').expect("disable array opens");
        let close = rest[open..].find(']').expect("disable array closes") + open;
        rest[open..=close].to_string()
    }

    #[test]
    fn should_disable_md025_for_fmt_only() {
        let content = poly_toml();
        assert!(
            disable_list(&content, "[fmt.markdown.rumdl]").contains("\"MD025\""),
            "fmt must not run MD025's autofix: it demotes every heading after a stray H1"
        );
        assert!(
            !disable_list(&content, "[lint.markdown.rumdl]").contains("\"MD025\""),
            "lint must keep reporting MD025 so a stray H1 is still surfaced"
        );
    }

    #[test]
    fn should_keep_shared_rumdl_disables_in_both_tables() {
        let content = poly_toml();
        let lint = disable_list(&content, "[lint.markdown.rumdl]");
        let fmt = disable_list(&content, "[fmt.markdown.rumdl]");
        for rule in RUMDL_DISABLE {
            assert!(
                lint.contains(&format!("\"{rule}\"")),
                "{rule} missing from lint disable list"
            );
            assert!(
                fmt.contains(&format!("\"{rule}\"")),
                "{rule} missing from fmt disable list"
            );
        }
    }

    /// Guards [`portable_dir`] directly against a Windows-shaped input, since this crate's own
    /// test suite runs on macOS/Linux and never exercises a real `\`-separated `PathBuf::
    /// to_string_lossy` result: `ResolvedCrateConfig::package_dir`'s Java arm only produces one
    /// on a Windows host (see the function's doc for why). A mixed-separator input is included
    /// to prove this is a blanket replace, not a prefix/suffix-shaped check. ~keep
    #[test]
    fn portable_dir_replaces_every_backslash_with_a_forward_slash() {
        assert_eq!(portable_dir("packages\\java"), "packages/java");
        assert_eq!(portable_dir("packages/java"), "packages/java");
        assert_eq!(portable_dir("sdk\\java/nested\\deep"), "sdk/java/nested/deep");
    }

    /// End-to-end guard for the bug this module fixes: a `workspace_hook` built from a
    /// Windows-separated `dir` must still emit `poly.toml` text that is valid, parseable TOML --
    /// `\` is a basic-string escape character, so an un-normalized Windows path here can corrupt
    /// or (for a byte sequence TOML happens to recognize as an escape) silently mangle the
    /// checked-in file for every other consumer. ~keep
    #[test]
    fn workspace_hook_normalizes_a_windows_separated_dir_into_valid_toml() {
        let hook = workspace_hook("checkstyle", "packages\\java", "mvn -q checkstyle:check", "**/*.java");
        assert!(
            !hook.contains('\\'),
            "no backslash may reach the emitted poly.toml text: {hook}"
        );
        assert!(hook.contains("root = \"packages/java\""));
        assert!(hook.contains("files = \"packages/java/**/*.java\""));

        let wrapped = format!("[hooks]\nstages = [\"pre-commit\"]\n{hook}");
        toml::from_str::<toml::Value>(&wrapped).expect("normalized hook text must be valid TOML");
    }
}
