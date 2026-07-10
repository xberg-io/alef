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
/// with a trailing comma — taplo's canonical multi-line form. An empty slice
/// renders as the inline empty array `[]` (taplo's canonical empty form).
fn toml_array(entries: &[&str]) -> String {
    if entries.is_empty() {
        return "[]".to_string();
    }
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

    let extra_excludes: Vec<&str> = config.poly.exclude.iter().map(String::as_str).collect();
    let all_excludes: Vec<&str> = EXCLUDES
        .iter()
        .copied()
        .chain(POLY_FORMAT_EXCLUDES.iter().copied())
        .chain(extra_excludes)
        .collect();
    let excludes = toml_array(&all_excludes);

    // file-safety-only globs (e.g. Rust `#![...]` inner attributes misread as
    let file_safety_extra: Vec<&str> = config.poly.file_safety_exclude.iter().map(String::as_str).collect();
    let file_safety_excludes = if file_safety_extra.is_empty() {
        excludes.clone()
    } else {
        let all: Vec<&str> = all_excludes.iter().copied().chain(file_safety_extra).collect();
        toml_array(&all)
    };

    let mut out = String::new();

    out.push_str(&format!("[discovery]\nexclude = {excludes}\n\n"));

    let md_disable = toml_array(RUMDL_DISABLE);
    out.push_str(&format!("[lint.markdown.rumdl]\ndisable = {md_disable}\n\n"));
    out.push_str(&format!("[fmt.markdown.rumdl]\ndisable = {md_disable}\n\n"));

    // NOTE: alef deliberately does NOT enable poly's opt-in native-toolchain

    if has(Language::Python) {
        out.push_str(&format!(
            "[lint.python.ruff]\nselect = [ \"ALL\" ]\nignore = {ignore}\n",
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
            "[lint.php.mago]\nselect = [ \"correctness\", \"security\" ]\nignore = {ignore}\nphp_version = \"8.2\"\n\n",
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

    out.push_str("[hooks]\nstages = [ \"pre-commit\" ]\n\n[hooks.builtin]\n");
    out.push_str(&format!("lint = {{ exclude = {excludes} }}\n"));
    out.push_str(&format!("fmt = {{ exclude = {excludes} }}\n"));
    out.push_str(&format!("file_safety = {{ exclude = {file_safety_excludes} }}\n"));
    out.push_str("cargo = true\n");
    out.push_str("commit = { stages = [ \"commit-msg\" ] }\n");

    if has(Language::Python) {
        let py_dir = config.package_dir(Language::Python);
        out.push_str(&format!(
            "\n[hooks.pre-commit.commands.pyrefly]\nrun = \"pyrefly check {py_dir}\"\nfiles = \"{py_dir}/**/*.py\"\n"
        ));
    }

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
