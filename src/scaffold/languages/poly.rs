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

/// Gitignore-style globs pruned from every lint/format pass. Emitted into
/// `[discovery] exclude` (direct `poly lint`/`poly fmt`/CI path) and mirrored
/// into the `lint`/`fmt`/`file_safety` builtin excludes (the git-hook
/// path, which filters per-builtin rather than via discovery).
///
/// Covers build output + lock files, plus the conventional non-source trees a
/// polyglot repo keeps under version control that must NOT be linted/reformatted:
/// `fixtures/` and `test_documents/` (exact-byte test data), `docs/assets/`
/// (canonical SVG/image assets), and `docs/snippets/` (compiled separately by the
/// snippet runner). Generated code under `packages/`, `e2e/`, `test_apps/` is
/// deliberately NOT excluded — poly owns its formatting.
const EXCLUDES: &[&str] = &[
    "**/*.freezed.dart",
    "**/*.g.dart",
    // Jinja templates (readme + e2e harness) contain `{{ ... }}` and are NOT valid
    // standalone source; poly must not lint or reformat them (reformatting corrupts
    // the template placeholders).
    "**/*.jinja",
    "**/*.lock",
    "**/Cargo.lock",
    "**/go.sum",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/uv.lock",
    ".alef/**",
    "artifacts/**",
    "dist/**",
    "docs/assets/**",
    "docs/snippets/**",
    "fixtures/**",
    "node_modules/**",
    // Readme templates are Jinja-in-Markdown (`{{ package_name }}` etc.); the two
    // conventional locations across repos. Same corruption risk as `**/*.jinja`.
    "readme_templates/**",
    "target/**",
    "templates/readme/**",
    "test_documents/**",
    "vendor/**",
];

/// Globs excluded specifically to keep poly's whole-repo format pass from
/// fighting alef's residual native passes (see `cli::pipeline::format`):
///
/// * `**/Cargo.toml` — poly's taplo and `cargo sort` (run as a residual + by the
///   `cargo` hook) canonicalize TOML differently; letting both touch Cargo.toml
///   produces an infinite format/regen loop on the embedded hash. cargo-sort owns
///   Cargo.toml.
/// * `packages/elixir/**/*.ex` / `*.exs` — poly's tree-sitter tier would reindent
///   Elixir before the residual `mix format` runs, breaking hash stability. mix
///   owns Elixir source.
// `Cargo.toml` is excluded from poly's whole-repo format pass: `cargo sort`
// (a residual step) owns dependency ordering there, and poly's taplo would fight
// its array formatting. Everything else — including Elixir `.ex`/`.exs` — is
// formatted by poly's tier-2 tree-sitter tier.
const POLY_FORMAT_EXCLUDES: &[&str] = &["**/Cargo.toml"];

/// Ruff rules ignored repo-wide for generated Python (ported verbatim from the
/// former pyproject `[tool.ruff] lint.ignore`).
const RUFF_IGNORE: &[&str] = &[
    "ANN401", "ASYNC109", "ASYNC110", "BLE001", "COM812", "D100", "D104", "D107", "D205", "E501", "EM", "FBT", "FIX",
    "ISC001", "PD011", "PGH003", "PLR2004", "PLW0603", "S104", "S110", "S603", "TD", "TRY",
];

/// rumdl rules disabled repo-wide for Markdown — the Zensical docs convention
/// shared across every polyglot repo's `.rumdl.toml`: tables/code run long
/// (MD013), the docs use inline HTML grids/cards (MD033), Zensical tabs indent
/// fenced blocks (MD046) and rewrite anchors (MD051), READMEs intentionally skip
/// a leading H1 (MD041) and use emphasis-as-heading (MD036), etc.
const RUMDL_DISABLE: &[&str] = &[
    "MD012", "MD013", "MD024", "MD033", "MD036", "MD041", "MD046", "MD051", "MD076",
];

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
    // Generated Python e2e/test-app suites carry codegen-shaped nits that are not
    // defects in the binding surface: redundant `# noqa` (RUF100), unused/duplicate
    // imports and redefinitions (F401/F811/I001), pytest composite asserts (PT018),
    // unused harness parameters (ARG001/ARG002), and assorted style/upgrade nits
    // (D403/UP035/UP012/RUF015/F541/EXE001).
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
    // Generated e2e tests take an `input` param shadowing the Python builtin (A001);
    // generated plugin trait-bridge stub classes aren't CapWords (N801).
    "A001",
    "N801",
];

/// Render a TOML array of strings indented under `key = [`, one entry per line
/// with a trailing comma — taplo's canonical multi-line form.
fn toml_array(entries: &[&str]) -> String {
    let inner = entries
        .iter()
        .map(|e| format!("    \"{e}\","))
        .collect::<Vec<_>>()
        .join("\n");
    format!("[\n{inner}\n]")
}

/// Generate the repo-root `poly.toml` from the configured language set.
pub(crate) fn scaffold_poly_config(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    let has = |lang: Language| languages.contains(&lang);

    // Build the merged exclude list: built-in defaults first, then repo extras.
    let extra_excludes: Vec<&str> = config.poly.exclude.iter().map(String::as_str).collect();
    let all_excludes: Vec<&str> = EXCLUDES
        .iter()
        .copied()
        .chain(POLY_FORMAT_EXCLUDES.iter().copied())
        .chain(extra_excludes)
        .collect();
    let excludes = toml_array(&all_excludes);

    let mut out = String::new();

    // Direct-CLI / CI discovery prune.
    out.push_str(&format!("[discovery]\nexclude = {excludes}\n\n"));

    // Markdown (rumdl) — universal; every repo carries READMEs + Zensical docs.
    let md_disable = toml_array(RUMDL_DISABLE);
    out.push_str(&format!("[lint.markdown.rumdl]\ndisable = {md_disable}\n\n"));
    out.push_str(&format!("[fmt.markdown.rumdl]\ndisable = {md_disable}\n\n"));

    // NOTE: alef deliberately does NOT enable poly's opt-in native-toolchain
    // formatters (shfmt, zig fmt, google-java-format, ktfmt, swift-format, dart
    // format, gleam format, styler). Those require the language's system
    // toolchain and make the formatted output environment-dependent — which
    // would break `alef verify` hash stability across machines. Instead every
    // language without a pure-Rust tier-1 poly engine is formatted by poly's
    // deterministic, zero-dependency tree-sitter (tier-2) generic formatter.
    // Go (gofmt) and Rust (rustfmt) stay at poly's default-on: alef already
    // requires those toolchains (Layer B formats Rust/Go in-memory pre-hash),
    // and consumer Go/Rust CI expects canonical gofmt/rustfmt output.

    // Native lint/format tables — only for languages needing non-default config.
    if has(Language::Python) {
        out.push_str(&format!(
            "[lint.python.ruff]\nselect = [ \"ALL\" ]\nignore = {ignore}\n",
            ignore = toml_array(RUFF_IGNORE)
        ));
        // Per-plugin params (poly >= 0.1.6 honors these). `pydocstyle_convention`
        // both selects the convention and disables the D-rules it turns off, so
        // no explicit D-set ignore is needed; INP001 resolves via poly's package
        // -root detection. Generated code relies on these to stay lint-clean.
        out.push_str(
            "mccabe_max_complexity = 15\n\
             pydocstyle_convention = \"google\"\n\
             pylint_max_args = 10\n\
             pylint_max_branches = 15\n\
             pylint_max_returns = 10\n\n",
        );
    }

    if has(Language::Php) {
        // mago replaces phpstan + php-cs-fixer (no PHP runtime). Generated
        // bindings target correctness + security; mago's style/complexity rules
        // are intentionally not selected, and a few correctness-category rules
        // that only fire on generated phpunit assertions are ignored.
        out.push_str(&format!(
            "[lint.php.mago]\nselect = [ \"correctness\", \"security\" ]\nignore = {ignore}\nphp_version = \"8.2\"\n\n",
            ignore = toml_array(MAGO_IGNORE)
        ));
    }

    // Typos spell-checker allowlists from [workspace.poly.typos].
    // Only emitted when at least one sub-table is non-empty; omitted entirely
    // when the consumer declares no typos overrides.
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

    // Cross-engine per-file suppressions. Always emitted: every alef repo ships
    // generated test/e2e suites. Python repos add wrapper-specific relaxations.
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
    // Repo-specific per-file suppressions from [workspace.poly.per-file-ignores].
    // BTreeMap iteration is deterministic (alphabetical key order).
    for (glob, codes) in &config.poly.per_file_ignores {
        let code_refs: Vec<&str> = codes.iter().map(String::as_str).collect();
        out.push_str(&format!("\"{glob}\" = {}\n", toml_array(&code_refs)));
    }
    out.push('\n');

    // Git-hook orchestration.
    out.push_str("[hooks]\nstages = [ \"pre-commit\" ]\n\n[hooks.builtin]\n");
    out.push_str(&format!("lint = {{ exclude = {excludes} }}\n"));
    out.push_str(&format!("fmt = {{ exclude = {excludes} }}\n"));
    out.push_str(&format!("file_safety = {{ exclude = {excludes} }}\n"));
    // Whole-workspace clippy/sort/machete/deny; capability-probed (skipped when
    // the cargo toolchain is absent). Per-crate clippy excludes for binding
    // crates await a polylint feature (tracked with the owner).
    out.push_str("cargo = true\n");
    // gitfluff-equivalent conventional-commit + AI-attribution stripping.
    out.push_str("commit = { stages = [ \"commit-msg\" ] }\n");

    if has(Language::Python) {
        // pyrefly type-check (replaces mypy) as a pre-commit hook, run in
        // project mode from the package root so it resolves the pyo3 _native
        // module and the [tool.pyrefly] sub-config.
        let py_dir = config.package_dir(Language::Python);
        out.push_str(&format!(
            "\n[hooks.pre-commit.commands.pyrefly]\nrun = \"pyrefly check {py_dir}\"\nfiles = \"{py_dir}/**/*.py\"\n"
        ));
    }

    // Canonical rustfmt config. poly's Rust formatter defers to rustfmt's own
    // config discovery (matching `cargo fmt`), so an explicit `rustfmt.toml`
    // pins the width both tools use. Without it rustfmt falls back to its 100
    // default; every alef repo standardizes on 120 to match poly's global
    // `line_length` default and stay consistent across the polyglot ecosystem.
    vec![
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
    ]
}
