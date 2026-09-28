//! Every spelling a backend answers to must be a spelling `exclude_languages` accepts.
//!
//! The two lived apart: each backend declares its own `TARGET_SPELLINGS` (`["python",
//! "pyo3"]`, `["node", "napi"]`, ...) and `bridge_targets_language` honours all of them, while
//! config resolution validated the entry against `Language` names alone. So
//! `exclude_languages = ["pyo3"]` -- the form the backend's own doc comment advertises --
//! produced a config that would not resolve (alef #476).
//!
//! This scans the constants out of the source rather than importing them, because three of
//! them (`php`, `wasm`, `extendr`) are private to their modules and an integration test cannot
//! name them. A new backend that adds a spelling nobody taught the validator fails here. ~keep

#![allow(clippy::print_stdout)]

use std::path::Path;

use alef::core::config::is_known_bridge_language;

/// Every `TARGET_SPELLINGS: [&str; N] = ["a", "b"];` literal under `src/backends/`, as
/// `(file, spelling)` pairs.
fn declared_spellings(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            for line in text.lines() {
                let Some(rest) = line.split_once("TARGET_SPELLINGS").map(|(_, r)| r) else {
                    continue;
                };
                let Some(open) = rest.find('[').and_then(|i| rest[i + 1..].find('[').map(|j| i + 1 + j)) else {
                    continue;
                };
                let Some(close) = rest[open..].find(']').map(|i| open + i) else {
                    continue;
                };
                for raw in rest[open + 1..close].split(',') {
                    let spelling = raw.trim().trim_matches('"');
                    if !spelling.is_empty() {
                        out.push((path.display().to_string(), spelling.to_string()));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn every_backend_target_spelling_is_accepted_by_exclude_languages() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backends");
    let spellings = declared_spellings(&root);

    // The scan itself is the thing most likely to be silently broken: a changed literal
    // shape, a moved directory, and it reports a clean pass over zero rows. Assert the job
    // count before reading the result. ~keep
    assert!(
        spellings.len() >= 14,
        "expected at least 14 declared spellings across the backends, scanned {} -- the \
         TARGET_SPELLINGS scan found nothing to check: {spellings:?}",
        spellings.len()
    );
    let distinct: std::collections::BTreeSet<&str> = spellings.iter().map(|(_, s)| s.as_str()).collect();
    assert!(
        distinct.contains("pyo3") && distinct.contains("napi") && distinct.contains("extendr"),
        "the scan must reach the private constants too; found: {distinct:?}"
    );

    for (file, spelling) in &spellings {
        assert!(
            is_known_bridge_language(spelling),
            "`{spelling}` is declared in {file} as a target this backend answers to, but \
             `exclude_languages` rejects it -- a config using the documented spelling will not \
             resolve"
        );
    }
    println!(
        "checked {} declared spellings ({} distinct)",
        spellings.len(),
        distinct.len()
    );
}

/// The per-language carrier prune asks `Language::bridge_spellings`, not a backend's own
/// `TARGET_SPELLINGS`, because the generation pipeline has a `Language` and cannot reach the
/// backend consts (three are private). That makes `bridge_spellings` a hand-written second
/// copy of the same fact, and a copy that drifts is worse than no copy at all: a spelling
/// missing from it means `exclude_languages = ["pyo3"]` suppresses the bridge but silently
/// fails to prune the carrier field, which is the #480 defect wearing the #476 costume.
///
/// Asserted as a union rather than per backend on purpose -- mapping a source directory back
/// to a `Language` would be a third hand-written copy of the same mapping. ~keep
#[test]
fn every_backend_target_spelling_is_reachable_from_language_bridge_spellings() {
    use alef::core::config::Language;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backends");
    let spellings = declared_spellings(&root);
    assert!(
        spellings.len() >= 14,
        "the TARGET_SPELLINGS scan found nothing to check, so this test examined nothing"
    );

    let reachable: std::collections::BTreeSet<&str> = Language::ALL
        .iter()
        .flat_map(|language| language.bridge_spellings().iter().copied())
        .collect();
    assert!(
        reachable.len() >= 14,
        "expected at least 14 spellings across every Language, got {} -- {reachable:?}",
        reachable.len()
    );

    for (file, spelling) in &spellings {
        assert!(
            reachable.contains(spelling.as_str()),
            "`{spelling}` is declared in {file} as a target this backend answers to, but no \
             `Language::bridge_spellings()` lists it -- `exclude_languages = [\"{spelling}\"]` \
             would suppress the bridge and still leave its carrier field on the binding"
        );
    }
}
