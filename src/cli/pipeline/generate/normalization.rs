use std::path::Path;
use tracing::debug;

/// Normalize content the same way `write_files` does before hashing.
///
/// Rust files go through rustfmt for canonical formatting, then through
/// `normalize_whitespace` so trailing-whitespace and trailing-newline rules
/// hold even when rustfmt could not parse the file (e.g. cextendr `lib.rs`
/// with non-standard `parameter: T = "default"` syntax that rustfmt rejects;
/// without the second pass, the raw codegen output retains trailing
/// whitespace on blank lines, and prek's `trailing-whitespace` hook then
/// rewrites the file post-finalisation, breaking `alef verify`).
///
/// Non-rust files skip rustfmt and go straight to whitespace normalization.
pub fn normalize_content(path: &Path, content: &str) -> String {
    let pre = if path.extension().is_some_and(|ext| ext == "rs") {
        format_rust_content(path, content)
    } else {
        content.to_string()
    };
    let is_markdown = path.extension().is_some_and(|ext| ext == "md");
    normalize_whitespace_with_policy(&pre, is_markdown)
}

/// Normalize whitespace for comparison: strip trailing whitespace per line,
/// collapse runs of 3+ blank lines to 2 (1 for markdown), and ensure a single
/// trailing newline.
///
/// Markdown files get an aggressive 1-blank-line cap because the canonical
/// downstream pre-commit pipeline runs `rumdl-fmt` after every commit,
/// and rumdl's MD012 rule collapses any multi-blank run to a single blank.
/// Without the matching cap inside alef, `alef all` output (which goes
/// through pre-commit `rumdl-fmt` before being committed) diverges from the
/// cold `alef readme` output (which does not invoke any markdown formatter),
/// and CI's `Validate READMEs` step — which runs `alef readme` cold and
/// diffs against the committed file — fails on every regen with the
/// noisy "extra blank line between `##` headings" diff. Capping at 1
/// inside alef itself produces rumdl-clean output natively, so cold and
/// hot paths converge and CI is stable.
///
/// Empty input stays empty — the canonical pre-commit `end-of-file-fixer`
/// hook truncates whitespace-only files (including a lone `"\n"`) to zero
/// bytes, so re-inflating empty content to `"\n"` here would create an
/// infinite emit/format ping-pong (e.g. for `.gitkeep` placeholders).
pub(super) fn normalize_whitespace(content: &str) -> String {
    normalize_whitespace_with_policy(content, false)
}

fn normalize_whitespace_with_policy(content: &str, is_markdown: bool) -> String {
    if content.is_empty() {
        return String::new();
    }
    let max_blanks: usize = if is_markdown { 1 } else { 2 };
    let mut result = String::with_capacity(content.len());
    let mut blank_count = 0usize;
    for line in content.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= max_blanks {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    while result.ends_with("\n\n") {
        result.pop();
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Walk up from `path` to find the nearest `Cargo.toml` and read its
/// `[package] edition = "YYYY"` value.  Returns `"2024"` if no `Cargo.toml`
/// is found or the edition field is absent.
pub(super) fn detect_crate_edition(path: &Path) -> String {
    let start = if path.is_dir() {
        path
    } else {
        match path.parent() {
            Some(p) => p,
            None => return "2024".to_string(),
        }
    };

    let mut current = start;
    loop {
        let candidate = current.join("Cargo.toml");
        if candidate.is_file() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                if let Some(edition) = parse_package_edition(&text) {
                    return edition;
                }
            }
            return "2024".to_string();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    "2024".to_string()
}

/// Parse the `edition = "YYYY"` value from the `[package]` section of a
/// `Cargo.toml` string.  Returns `None` if not found.
pub(super) fn parse_package_edition(toml_text: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("edition") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let value = rest.trim().trim_matches('"');
                if value.len() == 4 && value.chars().all(|c| c.is_ascii_digit()) {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Format a Rust source string by piping through `rustfmt`.
///
/// The edition is detected from the nearest `Cargo.toml` above `path`,
/// defaulting to `"2024"` when none is found.  `rustfmt` also discovers the
/// project's `rustfmt.toml` from the working directory.
///
/// Returns the formatted content on success, or the original content if
/// rustfmt is unavailable or fails (best-effort).
pub fn format_rust_content(path: &Path, content: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let edition = detect_crate_edition(path);
    let config_dir = std::env::current_dir().unwrap_or_default();

    let mut child = match Command::new("rustfmt")
        .arg("--edition")
        .arg(&edition)
        .arg("--config-path")
        .arg(&config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            debug!("rustfmt not available: {e}");
            return content.to_string();
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(content.as_bytes());
    }

    match child.wait_with_output() {
        Ok(output) if output.status.success() => {
            String::from_utf8(output.stdout).unwrap_or_else(|_| content.to_string())
        }
        Ok(output) => {
            debug!("rustfmt failed: {}", String::from_utf8_lossy(&output.stderr));
            content.to_string()
        }
        Err(e) => {
            debug!("rustfmt process error: {e}");
            content.to_string()
        }
    }
}
