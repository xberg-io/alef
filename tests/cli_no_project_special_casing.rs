//! Guard test: alef must not contain literal references to specific downstream
//! polyglot projects in its production code paths.
//!
//! Alef is a generic polyglot binding generator and must remain
//! project-agnostic — codegen, extraction, and CLI source must never branch on
//! literal project names like "kreuzberg", "html-to-markdown", "liter-llm",
//! etc. If special-case logic is genuinely needed for a downstream consumer,
//! drive it from `alef.toml` configuration instead of hard-coded matches in
//! the generator.
//!
//! Allowed contexts (not flagged):
//! - Lines inside `#[cfg(test)]` mod blocks (tracked by brace depth).
//! - Files named `tests.rs` (convention for inline-tests-by-file).
//! - Anything under `tests/`, `examples/`, `benches/`, `target/` (not scanned).

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

const FORBIDDEN_TOKENS: &[&str] = &[
    "kreuzberg",
    "kreuzbreg",
    "kreuzcrawl",
    "h2m",
    "lllm",
    "tslp",
    "ts-pack",
    "html_to_markdown",
    "html-to-markdown",
    "tree_sitter_language_pack",
    "tree-sitter-language-pack",
    "liter_llm",
    "liter-llm",
    "spikard",
];

/// Source directories scanned by this guard. Paths are relative to the
/// workspace root (resolved via `CARGO_MANIFEST_DIR`).
const SCAN_ROOTS: &[&str] = &[
    "src/backends",
    "src/core",
    "src/e2e",
    "src/scaffold",
    "src/cli",
    "src/codegen",
    "src/extract",
];

/// Known existing violations grandfathered in pending follow-up cleanup. Each
/// entry is `(relative_file_path, line_substring)` — the line must match
/// exactly one of these substrings to be excluded. Keep this list as short as
/// possible; every entry is a debt item to be eliminated.
const KNOWN_ALLOWED_VIOLATIONS: &[(&str, &str)] = &[];

#[derive(Debug)]
struct Violation {
    file: PathBuf,
    line: usize,
    token: &'static str,
    content: String,
}

#[test]
fn no_project_name_special_casing_in_production_code() {
    let workspace_root = workspace_root();
    let mut violations: Vec<Violation> = Vec::new();

    for relative_root in SCAN_ROOTS {
        let root = workspace_root.join(relative_root);
        assert!(root.is_dir(), "scan root does not exist: {}", root.display());
        scan_directory(&root, &mut violations);
    }

    if !violations.is_empty() {
        let report = format_report(&workspace_root, &violations);
        panic!("Found project-name special-casing:\n{report}");
    }
}

fn scan_directory(root: &Path, violations: &mut Vec<Violation>) {
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !is_scannable_file(path) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        scan_file(path, &content, violations);
    }
}

fn is_scannable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if file_name == "tests.rs" {
        return false;
    }

    matches!(path.extension().and_then(|ext| ext.to_str()), Some("rs" | "jinja"))
}

fn scan_file(path: &Path, content: &str, violations: &mut Vec<Violation>) {
    let mut state = ScanState::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let line_number = idx + 1;
        let in_test_before = state.in_test_block();
        state.observe_line(raw_line);

        // If the line was inside (or transitioned into) a test block, skip it.
        if in_test_before || state.in_test_block() {
            continue;
        }

        for token in FORBIDDEN_TOKENS {
            if contains_forbidden_token(raw_line, token) {
                if is_allowed_provenance(path, raw_line) {
                    continue;
                }
                if is_grandfathered(path, raw_line) {
                    continue;
                }
                violations.push(Violation {
                    file: path.to_path_buf(),
                    line: line_number,
                    token,
                    content: raw_line.to_string(),
                });
            }
        }
    }
}

/// Tracks whether the parser is currently inside a `#[cfg(test)]` mod block.
///
/// The state machine is intentionally simple — Rust's full grammar is not
/// reproduced. We track:
/// - Pending `#[cfg(test)]` attribute (true after such a line, cleared on the
///   next non-blank, non-attribute line).
/// - A stack of `(start_brace_depth)` for each entered test block; while the
///   stack is non-empty, lines are considered in-test.
/// - Brace depth, naively counted by `{` and `}` occurrences per line. Strings
///   and comments may produce slight over-counts but this only ever produces
///   false-positive *test* membership (i.e. skipping legitimate production
///   lines), which is acceptable for a guard test — never the reverse.
struct ScanState {
    pending_cfg_test: bool,
    brace_depth: i64,
    test_block_starts: Vec<i64>,
}

impl ScanState {
    fn new() -> Self {
        Self {
            pending_cfg_test: false,
            brace_depth: 0,
            test_block_starts: Vec::new(),
        }
    }

    fn in_test_block(&self) -> bool {
        !self.test_block_starts.is_empty()
    }

    fn observe_line(&mut self, raw_line: &str) {
        let trimmed = raw_line.trim();

        if is_cfg_test_attribute(trimmed) {
            self.pending_cfg_test = true;
        }

        // If we're awaiting a mod/fn after `#[cfg(test)]` and this line opens
        // a brace block, treat it as entering a test block.
        let opens = count_unescaped(raw_line, '{');
        let closes = count_unescaped(raw_line, '}');

        if self.pending_cfg_test && opens > 0 {
            // The new block starts at the current depth; record it so we know
            // when the brace count drops back below.
            self.test_block_starts.push(self.brace_depth);
            self.pending_cfg_test = false;
        } else if self.pending_cfg_test
            && !trimmed.is_empty()
            && !trimmed.starts_with("//")
            && !trimmed.starts_with('#')
        {
            // Attribute applied to a non-block item (e.g. `#[cfg(test)] mod tests;`)
            // — no opening brace on this line, so just clear the pending flag.
            self.pending_cfg_test = false;
        }

        self.brace_depth += opens as i64;
        self.brace_depth -= closes as i64;
        if self.brace_depth < 0 {
            self.brace_depth = 0;
        }

        // Pop any test-block frames whose start depth is now above the current
        // depth (meaning their closing `}` has been consumed).
        while let Some(&start_depth) = self.test_block_starts.last() {
            if self.brace_depth <= start_depth {
                self.test_block_starts.pop();
            } else {
                break;
            }
        }
    }
}

fn is_grandfathered(path: &Path, raw_line: &str) -> bool {
    let workspace_root = workspace_root();
    let Ok(rel) = path.strip_prefix(&workspace_root) else {
        return false;
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    for (allowed_path, allowed_substring) in KNOWN_ALLOWED_VIOLATIONS {
        if rel_str == *allowed_path && raw_line.contains(allowed_substring) {
            return true;
        }
    }
    false
}

fn is_allowed_provenance(path: &Path, raw_line: &str) -> bool {
    let workspace_root = workspace_root();
    let Ok(rel) = path.strip_prefix(&workspace_root) else {
        return false;
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");

    rel_str == "src/core/hash.rs" && raw_line.contains("https://github.com/kreuzberg-dev/alef")
}

fn is_cfg_test_attribute(trimmed: &str) -> bool {
    // Matches `#[cfg(test)]`, `#[cfg(all(test, ...))]`, `#[cfg(any(test, ...))]`,
    // `#[test]` (single-fn test attribute) — anything declaring a test-only
    // item should be treated as opening a test scope.
    if trimmed.starts_with("#[cfg(test)") {
        return true;
    }
    if trimmed.starts_with("#[cfg(all(test") || trimmed.starts_with("#[cfg(any(test") {
        return true;
    }
    if trimmed.starts_with("#[test]") {
        return true;
    }
    false
}

fn contains_forbidden_token(line: &str, token: &str) -> bool {
    let mut search_start = 0;
    while let Some(relative_index) = line[search_start..].find(token) {
        let start = search_start + relative_index;
        let end = start + token.len();
        if is_token_boundary(line, start, end) {
            return true;
        }
        search_start = end;
    }
    false
}

fn is_token_boundary(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();

    !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn count_unescaped(line: &str, target: char) -> usize {
    // Naive counter — does not strip strings/chars/comments. The state machine
    // is forgiving by design (see `ScanState` docs).
    line.chars().filter(|c| *c == target).count()
}

fn workspace_root() -> PathBuf {
    // After the workspace-to-single-crate consolidation, CARGO_MANIFEST_DIR
    // IS the workspace root (the single Cargo.toml sits at the repo root).
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn format_report(workspace_root: &Path, violations: &[Violation]) -> String {
    let mut out = String::new();
    for v in violations {
        let rel = v.file.strip_prefix(workspace_root).unwrap_or(&v.file).display();
        out.push_str(&format!(
            "  {}:{}: forbidden token `{}` in: {}\n",
            rel,
            v.line,
            v.token,
            v.content.trim()
        ));
    }
    out.push_str(&format!(
        "\n{} violation(s) found. Alef must remain project-agnostic — drive any\n",
        violations.len()
    ));
    out.push_str("downstream-specific behavior through `alef.toml` configuration, not\n");
    out.push_str("hard-coded references in codegen, extract, or CLI source.\n");
    out
}
