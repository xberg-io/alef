use std::path::{Path, PathBuf};
use tracing::debug;

/// Normalize content the same way `write_files` does before hashing.
///
/// `project_root` anchors every filesystem lookup `format_rust_content` performs (rustfmt
/// config discovery, `Cargo.toml` edition detection) -- see that function's doc for why the
/// process's current working directory must never be the source of truth for those lookups.
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
pub fn normalize_content(project_root: &Path, path: &Path, content: &str) -> String {
    let pre = if path.extension().is_some_and(|ext| ext == "rs") {
        format_rust_content(project_root, path, content)
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

/// A generation-time hard-failure on this trim was tried and reverted (alef-task #557
/// follow-up): a file-level trailing-whitespace check cannot tell "layout the trim is right
/// to clean" from "the tail of a raw/verbatim string literal's value" -- only the emitter
/// choosing that literal form knows which one it is. A real downstream regeneration run
/// against the fix hit 409 trailing-whitespace violations across 8 files (Java parameter-list
/// continuations, `///`/`##`/`#`/`*` doc-comment continuations, ...) and every single one was
/// layout, not a literal value. The check belongs in the emitter's literal-rendering path
/// instead -- see `go_needs_quoted` / `rust_needs_quoted` in `src/e2e/escape.rs`, and the
/// table-driven regression test next to them that exercises every backend's string-literal
/// function for exactly this shape. ~keep
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

/// Resolve `path` to an absolute location under `project_root`.
///
/// Several callers pass a path already relative to `project_root` (e.g. a `GeneratedFile`'s
/// own `path` field) while others pass one already joined onto it (`base_dir.join(&file.path)`).
/// Both must resolve to the identical absolute location before any upward filesystem walk
/// (rustfmt config discovery, `Cargo.toml` edition detection) runs, or the two forms silently
/// disagree about where that walk starts -- see alef #465. An already-absolute `path` is
/// returned unchanged (it may point outside `project_root` entirely; the bounded walks below
/// are what refuse to escape it, not this helper). ~keep
fn absolute_under(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

/// Walk up from `path` to find the nearest `Cargo.toml` at or below `project_root` and read its
/// `[package] edition = "YYYY"` value. Returns `"2024"` if no `Cargo.toml` is found within that
/// bound or the edition field is absent.
///
/// The walk never climbs above `project_root`: an earlier revision walked all the way to the
/// filesystem root, which could read a `Cargo.toml` belonging to a workspace *above* the
/// consumer's own repo (this polyrepo's root, a CI checkout's parent, ...) and silently adopt
/// its edition instead of the consumer's own. ~keep
pub(super) fn detect_crate_edition(project_root: &Path, path: &Path) -> String {
    let resolved = absolute_under(project_root, path);
    let start = if resolved.is_dir() {
        Some(resolved.as_path())
    } else {
        resolved.parent()
    };
    let Some(mut current) = start else {
        return "2024".to_string();
    };

    loop {
        if !current.starts_with(project_root) {
            return "2024".to_string();
        }
        let candidate = current.join("Cargo.toml");
        if candidate.is_file() {
            if let Ok(text) = std::fs::read_to_string(&candidate)
                && let Some(edition) = parse_package_edition(&text)
            {
                return edition;
            }
            return "2024".to_string();
        }
        if current == project_root {
            return "2024".to_string();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return "2024".to_string(),
        }
    }
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

/// The nearest `rustfmt.toml`/`.rustfmt.toml` at or above `path` but at or below
/// `project_root`, or `None` if neither file exists anywhere in that bounded range.
///
/// Bounded on both ends deliberately: the walk never starts outside `project_root` (a `path`
/// argument pointing elsewhere is never used to go config-hunting in a foreign tree) and never
/// climbs above it either (matching [`detect_crate_edition`]'s own bound, for the same reason --
/// alef #465). `None` here means "the consumer genuinely has no rustfmt config", which
/// [`format_rust_content`] must treat as "use rustfmt's own defaults", not as an error. ~keep
fn rustfmt_config_file(project_root: &Path, path: &Path) -> Option<PathBuf> {
    const CONFIG_FILE_NAMES: [&str; 2] = ["rustfmt.toml", ".rustfmt.toml"];

    let resolved = absolute_under(project_root, path);
    let start = if resolved.is_dir() {
        Some(resolved.as_path())
    } else {
        resolved.parent()
    };
    let mut current = start?;

    loop {
        if !current.starts_with(project_root) {
            return None;
        }
        for name in CONFIG_FILE_NAMES {
            let candidate = current.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if current == project_root {
            return None;
        }
        current = current.parent()?;
    }
}

/// Format a Rust source string by piping through `rustfmt`.
///
/// The edition is detected from the nearest `Cargo.toml` at or below `project_root`
/// (see [`detect_crate_edition`]), defaulting to `"2024"` when none is found. The rustfmt
/// config is resolved the same way, bounded to `project_root` (see [`rustfmt_config_file`]):
/// when a `rustfmt.toml`/`.rustfmt.toml` is found, its exact file is passed via
/// `--config-path`; when none is found, the flag is omitted entirely rather than pointed at a
/// directory with nothing in it.
///
/// That distinction matters: `--config-path <a directory containing no config, searched all
/// the way up to the filesystem root>` is a hard error for rustfmt ("unable to find a config
/// file for the given path"), not a fall-back to defaults -- passing `project_root`
/// unconditionally here (the previous, buggy behaviour, keyed off `std::env::current_dir()`
/// instead of `project_root`) made this function's non-success arm fire on every first
/// `alef all` in a project with no committed `rustfmt.toml` yet, so the freshly generated
/// bindings landed completely unformatted (alef #465). Omitting the flag when
/// `rustfmt_config_file` finds nothing lets rustfmt fall back to its own built-in defaults
/// instead.
///
/// The child process's working directory is pinned to `project_root` regardless of which
/// branch above is taken. This is a second, independent line of defense against the same root
/// cause: even with the flag omitted, rustfmt reading from stdin still performs its own
/// upward config search starting from its process's current directory, so leaving that
/// directory as whatever the *alef* process happened to be launched from would silently
/// reintroduce ambient, cwd-dependent behaviour for the "no config found" case. Pinning it to
/// `project_root` does not make the search perfectly hermetic (rustfmt can still climb above
/// `project_root` looking for one if we omit `--config-path`, since rustfmt has no flag to
/// disable config discovery outright), but it anchors the search at the consumer's own project
/// instead of an unrelated directory, which is the actual complaint in alef #465. ~keep
///
/// Returns the formatted content on success, or the original content if
/// rustfmt is unavailable or fails (best-effort).
pub fn format_rust_content(project_root: &Path, path: &Path, content: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let edition = detect_crate_edition(project_root, path);
    let config_file = rustfmt_config_file(project_root, path);

    let mut command = Command::new("rustfmt");
    command.arg("--edition").arg(&edition);
    if let Some(config_file) = &config_file {
        command.arg("--config-path").arg(config_file);
    }
    command.current_dir(project_root);

    let mut child = match command
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
