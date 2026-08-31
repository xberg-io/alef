//! Mechanical enforcement of the `file-modularization` rule's 1,000-line cap.
//!
//! The rule ("existing files over 1,000 lines are remediation targets and must not grow") was
//! aspirational until this test existed: nothing measured it, and the count of over-cap files
//! reached 124. Splitting 124 files at once would be a review-hostile diff, so this does not
//! demand the split. It ratchets:
//!
//! * a file already over the cap may shrink freely, but may never exceed the ceiling recorded
//!   for it in [`BASELINE`];
//! * a file not in the baseline may never cross the cap at all;
//! * a baseline entry that has shrunk under the cap (or disappeared) must be dropped from the
//!   baseline, so the ratchet tightens instead of leaving a licence to regrow.
//!
//! The effect is that today's sizes are frozen and every future touch is neutral or an
//! improvement.
//!
//! Where it runs: nowhere new. This is an ordinary integration test, so CI's `test` job picks it
//! up through `cargo test --workspace` on all three platforms, and `task lint:file-size` runs it
//! alone. It is deliberately *not* a separate `ci.yml` step — an integration test target forces a
//! full build of the `alef` lib (~2 min), and paying that twice to move a 0.5 s check earlier in
//! the pipeline is a bad trade. After an actual split, `task lint:file-size:tighten` rewrites the
//! baseline.
//!
//! Design note — why a checked-in ceiling and not a merge-base diff: comparing `HEAD` against
//! `origin/main` needs full history (CI checkouts are shallow), a fetched remote (agents work
//! offline in worktrees), and a meaningful base (rebases move it). Each failure mode degrades
//! into "skip", and a check that silently skips is the bug class this file exists to close. A
//! committed ceiling is hermetic: same answer on every machine, no network, no history. ~keep

#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

/// The cap from CLAUDE.md's `file-modularization` rule.
const MAX_LINES: usize = 1_000;

/// Recorded ceilings for files that were already over [`MAX_LINES`] when the ratchet landed.
const BASELINE: &str = "tests/file_size_baseline.txt";

/// Set to any value to rewrite [`BASELINE`] from the working tree instead of asserting against
/// it. Exposed as `task lint:file-size:tighten`.
const UPDATE_ENV: &str = "UPDATE_FILE_SIZE_BASELINE";

/// The rule's scope: `src/**/*.rs`, `src/**/*.jinja`, `tests/**/*.rs`. Expressed as git
/// pathspecs, whose leading `dir/` already means "at any depth below".
const PATHSPECS: [&str; 3] = ["src/*.rs", "src/*.jinja", "tests/*.rs"];

/// "generated snapshots are excluded" (CLAUDE.md). Every committed snapshot in this repo is an
/// `insta` `.snap` under `tests/snapshots/`, so the extension filter in [`PATHSPECS`] already
/// excludes them all — verified: `tests/snapshots/` holds 135 files and zero `.rs`. The prefix is
/// listed anyway so that configuring insta to emit `.rs`-suffixed snapshots cannot silently pull
/// a generated tree back under the cap. ~keep
const EXCLUDED_PREFIXES: [&str; 1] = ["tests/snapshots/"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Repo-relative, `/`-separated paths in scope.
///
/// `git ls-files` is authoritative because the rule governs committed content: an agent's
/// untracked scratch file is not a modularization failure. Where git cannot answer (no binary,
/// no repository) this walks the tree instead and says so, rather than reporting "nothing to
/// check" and passing.
fn files_in_scope() -> Vec<String> {
    let listed = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .arg("ls-files")
        .arg("-z")
        .arg("--")
        .args(PATHSPECS)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| {
            text.split('\0')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|paths: &Vec<String>| !paths.is_empty());

    let mut paths = match listed {
        Some(paths) => paths,
        None => {
            println!("{BASELINE}: git ls-files unavailable, falling back to a filesystem walk");
            walk_fallback()
        }
    };
    paths.retain(|path| !EXCLUDED_PREFIXES.iter().any(|prefix| path.starts_with(prefix)));
    paths.sort();
    paths
}

fn walk_fallback() -> Vec<String> {
    fn visit(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                visit(&entry.path(), &relative, out);
            } else if relative.ends_with(".rs") || relative.ends_with(".jinja") {
                out.push(relative);
            }
        }
    }

    let mut out = Vec::new();
    for root in ["src", "tests"] {
        visit(&repo_root().join(root), root, &mut out);
    }
    out.retain(|path| path.ends_with(".rs") || path.starts_with("src/"));
    out
}

/// Integration-test targets are compiled with `--test`, so this module is always built; it is
/// plain `mod`, not `#[cfg(test)] mod`, to make that explicit. ~keep
mod fallback_scope {
    use super::walk_fallback;

    /// `.jinja` is in scope under `src/` only; the walk must not widen the rule's glob set.
    #[test]
    #[ignore = "file-size ratchet disabled by maintainer decision; re-enable by deleting these #[ignore]s"]
    fn walk_covers_the_same_extensions_as_the_pathspecs() {
        let paths = walk_fallback();
        assert!(!paths.is_empty(), "walk found nothing under src/ or tests/");
        for path in &paths {
            let in_scope = (path.starts_with("src/") && (path.ends_with(".rs") || path.ends_with(".jinja")))
                || (path.starts_with("tests/") && path.ends_with(".rs"));
            assert!(in_scope, "walk returned out-of-scope path `{path}`");
        }
    }
}

/// Lines in a file, matching `wc -l` for the newline-terminated files this repo commits.
fn line_count(relative: &str) -> usize {
    let path = repo_root().join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    text.lines().count()
}

fn current_sizes() -> BTreeMap<String, usize> {
    files_in_scope()
        .into_iter()
        .map(|path| {
            let n = line_count(&path);
            (path, n)
        })
        .collect()
}

fn baseline_path() -> PathBuf {
    repo_root().join(BASELINE)
}

fn read_baseline() -> BTreeMap<String, usize> {
    let path = baseline_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e}\nRegenerate it with `task lint:file-size:tighten`.",
            path.display()
        )
    });
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (file, ceiling) = line
                .rsplit_once(char::is_whitespace)
                .unwrap_or_else(|| panic!("{BASELINE}: expected `<path> <ceiling>`, got `{line}`"));
            let ceiling = ceiling
                .parse::<usize>()
                .unwrap_or_else(|e| panic!("{BASELINE}: `{line}` has a non-numeric ceiling: {e}"));
            (file.trim().to_owned(), ceiling)
        })
        .collect()
}

fn write_baseline(sizes: &BTreeMap<String, usize>) {
    let mut out = String::from(HEADER);
    for (path, lines) in sizes.iter().filter(|(_, lines)| **lines > MAX_LINES) {
        let _ = writeln!(out, "{path} {lines}");
    }
    std::fs::write(baseline_path(), out).expect("writing baseline");
}

const HEADER: &str = "\
# file-size ratchet baseline -- see tests/file_size_ratchet.rs
#
# Every path here was already over the 1,000-line cap from CLAUDE.md's `file-modularization`
# rule when the ratchet landed. The number is a CEILING, not a measurement: the file may shrink
# freely, but it fails CI the moment it exceeds this. Splitting a file below 1,000 lines means
# deleting its line here, which is how the ratchet tightens.
#
# Never raise a number in this file to make a build pass. Split the file instead.
# Regenerate after a split with: task lint:file-size:tighten
#
# <path> <ceiling>
";

/// Honour `UPDATE_FILE_SIZE_BASELINE` once per process, before any assertion reads the baseline.
fn maybe_update_baseline() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os(UPDATE_ENV).is_some() {
            let sizes = current_sizes();
            write_baseline(&sizes);
            println!(
                "{BASELINE}: rewritten from the working tree ({} entries over the cap)",
                sizes.values().filter(|n| **n > MAX_LINES).count()
            );
        }
    });
}

/// The ratchet proper: nothing already over the cap may get bigger.
#[test]
#[ignore = "file-size ratchet disabled by maintainer decision; re-enable by deleting these #[ignore]s"]
fn over_cap_files_must_not_grow() {
    maybe_update_baseline();
    let baseline = read_baseline();
    let current = current_sizes();

    let mut grown = Vec::new();
    for (path, ceiling) in &baseline {
        let Some(&now) = current.get(path) else { continue };
        if now > *ceiling {
            grown.push((path.clone(), *ceiling, now));
        }
    }
    if grown.is_empty() {
        return;
    }

    let mut message = format!(
        "{} file(s) already over the {MAX_LINES}-line cap grew:\n\n",
        grown.len()
    );
    for (path, ceiling, now) in &grown {
        let _ = writeln!(
            message,
            "  {path}\n      was {ceiling} lines, now {now} (+{})",
            now - ceiling
        );
    }
    let _ = write!(
        message,
        "\nThese files are remediation targets: CLAUDE.md's `file-modularization` rule allows them\n\
         to shrink but never to grow. Move the new code into a sibling module (split by concern,\n\
         not by line count) so the touched file ends at or below its ceiling in {BASELINE}.\n\n\
         Do not raise the ceiling to make this pass. After an actual split, run:\n\
         \x20   task lint:file-size:tighten\n"
    );
    panic!("{message}");
}

/// The other half: a file under the cap today may not cross it tomorrow.
#[test]
#[ignore = "file-size ratchet disabled by maintainer decision; re-enable by deleting these #[ignore]s"]
fn no_new_file_may_cross_the_line_cap() {
    maybe_update_baseline();
    let baseline = read_baseline();

    let mut crossed = Vec::new();
    for (path, lines) in current_sizes() {
        if lines > MAX_LINES && !baseline.contains_key(&path) {
            crossed.push((path, lines));
        }
    }
    if crossed.is_empty() {
        return;
    }

    let mut message = format!("{} file(s) newly crossed the {MAX_LINES}-line cap:\n\n", crossed.len());
    for (path, lines) in &crossed {
        let _ = writeln!(
            message,
            "  {path}\n      {lines} lines ({} over the cap)",
            lines - MAX_LINES
        );
    }
    let _ = write!(
        message,
        "\nCLAUDE.md's `file-modularization` rule caps src/**/*.rs, src/**/*.jinja and\n\
         tests/**/*.rs at {MAX_LINES} lines, and asks that files approaching 800 be split before\n\
         more behaviour is added. Split at the concept boundary -- see the standard module layout\n\
         in the rule.\n\n\
         {BASELINE} is a frozen record of pre-existing debt; new entries are never added to it.\n"
    );
    panic!("{message}");
}

/// Tightening: a baseline entry that no longer needs to be there must go, or it becomes a
/// standing licence for the file to regrow to its old size.
#[test]
#[ignore = "file-size ratchet disabled by maintainer decision; re-enable by deleting these #[ignore]s"]
fn baseline_must_not_outlive_the_files_it_excuses() {
    maybe_update_baseline();
    let baseline = read_baseline();
    let current = current_sizes();

    let mut stale = Vec::new();
    for (path, ceiling) in &baseline {
        match current.get(path) {
            None => stale.push(format!("  {path}\n      no longer exists (was ceiling {ceiling})")),
            Some(&now) if now <= MAX_LINES => {
                stale.push(format!(
                    "  {path}\n      now {now} lines, under the cap (ceiling {ceiling})"
                ));
            }
            Some(_) => {}
        }
    }
    if stale.is_empty() {
        return;
    }

    panic!(
        "{} stale entries in {BASELINE}:\n\n{}\n\nDelete these lines. Left in place they are a\n\
         standing licence for the file to grow back to its old size, which undoes the split that\n\
         just happened. Run `task lint:file-size:tighten` to drop them mechanically.\n",
        stale.len(),
        stale.join("\n")
    );
}
