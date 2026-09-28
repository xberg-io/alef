//! Structural guard: every production call to [`super::fail_on_unavailable_field_markers`] must be
//! paired with a nearby call to [`super::fail_on_unsupported_assertion_type_markers`] on the same
//! rendered body.
//!
//! ~keep The two funnels are deliberately unchained — see `assertion_type_skip.rs`'s module doc,
//! and the negative controls `mod::unavailable_field_marker_tests::unsupported_assertion_type_comments_are_not_recorded`
//! / `field_skip::tests::unsupported_assertion_type_wordings_stay_uncounted` that pin the field
//! funnel to never recognise a type-skip wording. But every backend that hands a rendered body to
//! the field funnel is equally exposed to an assertion-*type* skip landing in that same body — the
//! wording just names a different token — and if only the field funnel is wired in, the
//! assertion-type marker is never recorded on the shared ledger. [`super::inert_example::inert_verdict`]
//! then can't tell that body apart from one that rendered nothing at all: it comes back
//! `RenderedNothing` instead of `AwaitedOrLimited`.
//!
//! This walks the actual source tree rather than pinning a hand-copied list of call sites: a
//! hand-copied list goes stale by omission the moment a twentieth backend is added, and nothing
//! would notice.

use std::path::{Path, PathBuf};

/// This file's own path, so its own doc comments and helper text are never scanned.
const GUARD_FILE: &str = "src/e2e/codegen/assertion_type_funnel_pairing.rs";

const FIELD_FUNNEL: &str = "fail_on_unavailable_field_markers";
const TYPE_FUNNEL: &str = "fail_on_unsupported_assertion_type_markers";

/// How many lines apart a paired call may sit. Every call this guard is meant to require sits one
/// statement away from its sibling (at most a handful of rustfmt-wrapped lines); a much wider
/// window would risk accepting an unrelated call meant for a different fixture entirely.
const PAIRING_WINDOW: usize = 8;

fn collect_rust_files(directory: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory).expect("the source tree must be readable");
    for entry in entries {
        let path = entry.expect("a directory entry must be readable").path();
        if path.is_dir() {
            collect_rust_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

fn insert_explicit_path_declaration(
    explicit_path: &mut Option<String>,
    path: &Path,
    declared: &mut std::collections::BTreeSet<PathBuf>,
) {
    let Some(relative) = explicit_path.take() else { return };
    let Some(parent) = path.parent() else { return };
    declared.insert(parent.join(relative));
}

/// Every file in `files` that some other file declares as `#[cfg(test)] mod <name>;`.
///
/// ~keep [`blank_test_modules`] only recognises a test module written INLINE as
/// `#[cfg(test)] mod x { .. }`. A test module split into its own file — which is how this tree
/// keeps oversized modules under the size cap, e.g. `unavailable_field_marker_tests.rs` — is
/// still test code, but arrives here as an ordinary source file and every call it makes reads
/// as a production call site. Resolving the declaration to `<dir>/<name>.rs` or
/// `<dir>/<name>/mod.rs` rather than matching on the file's name keeps a *production* module
/// that merely ends in `_tests` from being skipped by accident.
fn test_only_files(files: &[PathBuf]) -> std::collections::BTreeSet<PathBuf> {
    let mut declared = std::collections::BTreeSet::new();
    for path in files {
        let source = std::fs::read_to_string(path).expect("a Rust source file must be readable");
        let directory = match path.file_name().and_then(|name| name.to_str()) {
            Some("mod.rs") => path.parent().map(Path::to_path_buf),
            _ => Some(path.with_extension("")),
        };
        let Some(directory) = directory else { continue };
        let mut saw_cfg_test = false;
        let mut explicit_path: Option<String> = None;
        for line in source.lines() {
            let trimmed = line.trim();
            if saw_cfg_test {
                // ~keep `#[cfg(test)]` and the `mod` it gates are not always adjacent: a split test
                // module is normally registered as `#[cfg(test)] #[path = "..."] mod name;`, three
                // lines. Treating any attribute line as a reset made every `#[path]`-registered test
                // file read as production, and the whole point of this guard is that a test file's
                // calls must not count. Carry the `#[path]` through, because it — not
                // `<dir>/<name>.rs` — is where the module actually lives.
                if let Some(rest) = trimmed
                    .strip_prefix("#[path = \"")
                    .and_then(|rest| rest.strip_suffix("\"]"))
                {
                    explicit_path = Some(rest.to_string());
                    continue;
                }
                if trimmed.starts_with("#[") {
                    continue;
                }
                saw_cfg_test = false;
                if let Some(name) = trimmed.strip_prefix("mod ").and_then(|rest| rest.strip_suffix(';')) {
                    insert_explicit_path_declaration(&mut explicit_path, path, &mut declared);
                    declared.insert(directory.join(format!("{name}.rs")));
                    declared.insert(directory.join(name).join("mod.rs"));
                }
                explicit_path = None;
            }
            if trimmed == "#[cfg(test)]" {
                saw_cfg_test = true;
                explicit_path = None;
            }
        }
    }
    declared
}

/// Blank out the body of every `#[cfg(test)] mod ... { .. }` block, preserving line count (and
/// therefore line numbers) so callers can keep indexing into the original file by line.
///
/// ~keep A `#[cfg(test)]` that gates a single item (an import, e.g. `c.rs`'s
/// `EffectiveConfigSource`) rather than a `mod` is left untouched: it never introduces a
/// production call site or shadows one, and blanking it would risk swallowing real code.
fn blank_test_modules(source: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(source.lines().count());
    let mut in_test_mod = false;
    let mut depth: i32 = 0;
    let mut saw_cfg_test = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if in_test_mod {
            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            out.push(String::new());
            if depth <= 0 {
                in_test_mod = false;
            }
            continue;
        }
        if saw_cfg_test {
            saw_cfg_test = false;
            if trimmed.starts_with("mod ") {
                in_test_mod = true;
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                out.push(String::new());
                continue;
            }
        }
        if trimmed == "#[cfg(test)]" {
            saw_cfg_test = true;
        }
        out.push(line.to_string());
    }
    out
}

/// True when `line` is an actual call to `funnel` — not its own `fn` definition, and not a `//`,
/// `///` or `//!` mention of its name.
fn calls_funnel(line: &str, funnel: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return false;
    }
    if line.contains(&format!("fn {funnel}(")) {
        return false;
    }
    line.contains(&format!("{funnel}("))
}

#[test]
fn every_production_field_skip_call_site_also_calls_the_assertion_type_funnel() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scan_root = root.join("src/e2e/codegen");
    let mut files = Vec::new();
    collect_rust_files(&scan_root, &mut files);
    assert!(
        files.len() > 20,
        "the source scan found only {} file(s) under src/e2e/codegen — the walk is broken, and a \
         guard that examines nothing passes for a healthy tree",
        files.len()
    );

    let mut field_site_count = 0usize;
    let mut unpaired: Vec<String> = Vec::new();
    let test_only = test_only_files(&files);

    for path in &files {
        let relative = path
            .strip_prefix(root)
            .expect("every scanned file lives under the manifest directory")
            .to_string_lossy()
            .replace('\\', "/");
        if relative == GUARD_FILE || test_only.contains(path) {
            continue;
        }
        let source = std::fs::read_to_string(path).expect("a Rust source file must be readable");
        let stripped = blank_test_modules(&source);

        for (index, line) in stripped.iter().enumerate() {
            if !calls_funnel(line, FIELD_FUNNEL) {
                continue;
            }
            field_site_count += 1;
            let window_start = index.saturating_sub(PAIRING_WINDOW);
            let window_end = (index + PAIRING_WINDOW + 1).min(stripped.len());
            let paired = stripped[window_start..window_end]
                .iter()
                .any(|candidate| calls_funnel(candidate, TYPE_FUNNEL));
            if !paired {
                unpaired.push(format!("  {relative}:{}", index + 1));
            }
        }
    }

    // ~keep A guard that finds nothing passes vacuously. This repo had exactly 23 production call
    // sites of the field funnel when this guard was written across 18 backend files; 20 is a floor
    // with headroom, not a ceiling, so an unrelated refactor is free to add or consolidate sites.
    assert!(
        field_site_count >= 20,
        "the scan found only {field_site_count} production call site(s) of `{FIELD_FUNNEL}` under \
         src/e2e/codegen — expected at least 20; the scan is finding too little to trust, not too \
         much"
    );

    assert!(
        unpaired.is_empty(),
        "the following production call(s) of `{FIELD_FUNNEL}` have no nearby call to \
         `{TYPE_FUNNEL}` on the same rendered body, so any assertion-type skip marker this backend \
         renders is never recorded and `inert_verdict` will misclassify the example as \
         `RenderedNothing`:\n{}",
        unpaired.join("\n")
    );
}

#[test]
fn the_guard_recognises_a_call_and_ignores_prose_about_it() {
    assert!(calls_funnel(
        "    crate::e2e::codegen::fail_on_unavailable_field_markers(body, \"go\", &fixture.id, &fixture.assertions);",
        FIELD_FUNNEL
    ));
    assert!(!calls_funnel(
        "/// the shared `fail_on_unavailable_field_markers` mechanism (src/e2e/codegen/mod.rs)",
        FIELD_FUNNEL
    ));
    assert!(!calls_funnel(
        "pub(crate) fn fail_on_unavailable_field_markers(",
        FIELD_FUNNEL
    ));
}

#[test]
fn blanking_test_modules_hides_calls_inside_them_but_keeps_line_count() {
    let source =
        "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { fail_on_unavailable_field_markers(); }\n}\nfn c() {}\n";
    let stripped = blank_test_modules(source);
    assert_eq!(stripped.len(), source.lines().count());
    assert!(!stripped.join("\n").contains(FIELD_FUNNEL));
    assert!(stripped[0].contains("fn a"));
    assert!(stripped[5].contains("fn c"));
}

#[test]
fn a_cfg_test_on_a_single_item_is_not_treated_as_a_test_module() {
    let source = "#[cfg(test)]\nuse super::Thing;\nfn a() { fail_on_unavailable_field_markers(); }\n";
    let stripped = blank_test_modules(source);
    assert!(stripped.join("\n").contains(FIELD_FUNNEL));
}

/// The split-file counterpart of `blanking_test_modules_hides_calls_inside_them_but_keeps_line_count`.
/// Uses the real tree rather than a temp fixture, so it also fails if the tree stops using the
/// pattern the skip is granted for.
#[test]
fn a_test_module_split_into_its_own_file_is_recognised_but_a_production_sibling_is_not() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rust_files(&root.join("src/e2e/codegen"), &mut files);
    let test_only = test_only_files(&files);

    let split_test_module = root.join("src/e2e/codegen/unavailable_field_marker_tests.rs");
    assert!(
        test_only.contains(&split_test_module),
        "a module its parent declares `#[cfg(test)] mod ...;` must be recognised as test-only"
    );

    let production = root.join("src/e2e/codegen/declared_error_variant.rs");
    assert!(
        files.contains(&production),
        "the fixture file for the negative control must exist"
    );
    assert!(
        !test_only.contains(&production),
        "a module declared without `#[cfg(test)]` must stay in the production scan"
    );
}
