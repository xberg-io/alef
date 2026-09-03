//! Fold the on-disk `go.mod` of a generated go.mod's required package into that generated
//! go.mod's `// indirect` require block.
//!
//! Go's module-graph-pruning rule (go >= 1.17, go.dev/ref/mod#graph-pruning) requires the
//! IMPORTING module's own go.mod to carry explicit `// indirect` requirements for every module
//! that provides a package transitively imported through a directly required module. Emitting
//! only testify's own pinned indirect deps (`super::TESTIFY_INDIRECT_DEPS`) silently drops the
//! required module's own dependency graph. `go build -mod=readonly` still passes (module pruning
//! means the build itself doesn't need them), but `go test -mod=readonly ./...` fails with
//! "updates to go.mod needed, disabled by -mod=readonly; to update it: go mod tidy" -- a
//! build-only check would report the generated file fixed while it is still broken.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::TESTIFY_INDIRECT_DEPS;

/// Every `require` pin declared in `go_mod_text`, whether written as a single-line `require
/// module version` or inside a parenthesized `require ( ... )` block. Trailing `// indirect`
/// comments (or any other trailing comment) are stripped before reading the version field --
/// direct and indirect requires alike are folded into the caller's `// indirect` block, since
/// both provide packages a pruned module graph must still enumerate for the importing module.
fn parse_go_mod_requires(go_mod_text: &str) -> BTreeMap<String, String> {
    let mut requires = BTreeMap::new();
    let mut in_require_block = false;
    for raw_line in go_mod_text.lines() {
        let line = raw_line.split("//").next().unwrap_or(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if in_require_block {
            if line == ")" {
                in_require_block = false;
                continue;
            }
            if let Some((module, version)) = parse_module_version_pair(line) {
                requires.insert(module, version);
            }
            continue;
        }
        if line == "require (" {
            in_require_block = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ")
            && let Some((module, version)) = parse_module_version_pair(rest)
        {
            requires.insert(module, version);
        }
    }
    requires
}

/// Split a `module version` pair on the first run of whitespace.
fn parse_module_version_pair(text: &str) -> Option<(String, String)> {
    let mut parts = text.split_whitespace();
    let module = parts.next()?;
    let version = parts.next()?;
    Some((module.to_string(), version.to_string()))
}

/// The full `// indirect` require block for a generated go.mod: `TESTIFY_INDIRECT_DEPS` merged
/// with every require (direct or indirect) declared in the required module's own `go.mod`,
/// deduplicated by module path and sorted alphabetically (`BTreeMap` iteration order). Excludes
/// any module already in `direct_modules` -- a module cannot be required both directly and as
/// `// indirect` in the same go.mod. `direct_modules` always contains the required module path
/// itself and `testify`, and in Local dep mode may also contain a `harness_extras` entry that
/// duplicates one of the dependency's own direct requires (e.g. `tree-sitter-language-pack`'s
/// `e2e/go` harness extra for `go-tree-sitter`, which its `packages/go/go.mod` also requires
/// directly).
///
/// `dependency_go_mod_dir` is the on-disk directory the required module resolves to (the
/// generated go.mod's unconditional `replace_path`, joined onto `output_base` at the call site
/// in `super::GoCodegen::generate` -- unlike the `replace` directive itself, this lookup is not
/// gated on dep mode, since the dependency's own go.mod lives on disk regardless of whether this
/// run emits a `replace`). When it's `None`, or the go.mod there is missing or unreadable (e.g.
/// a partial/bootstrap run where the dependency hasn't been scaffolded yet), this degrades
/// gracefully to the testify-only set via `tracing::debug!` rather than failing generation.
pub(super) fn resolve_indirect_requires(
    dependency_go_mod_dir: Option<&Path>,
    direct_modules: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut merged: BTreeMap<String, String> = TESTIFY_INDIRECT_DEPS
        .iter()
        .map(|(module, version)| ((*module).to_string(), (*version).to_string()))
        .collect();

    if let Some(dir) = dependency_go_mod_dir {
        let go_mod_path = dir.join("go.mod");
        match std::fs::read_to_string(&go_mod_path) {
            Ok(text) => {
                for (module, version) in parse_go_mod_requires(&text) {
                    merged.insert(module, version);
                }
            }
            Err(error) => {
                tracing::debug!(
                    path = %go_mod_path.display(),
                    %error,
                    "could not read dependency go.mod for indirect requires; falling back to testify-only set"
                );
            }
        }
    }

    merged.retain(|module, _| !direct_modules.contains(module));
    merged.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_go_mod_requires_reads_single_line_and_block_forms() {
        let text = "module example.com/foo\n\ngo 1.26\n\nrequire github.com/tree-sitter/go-tree-sitter v0.25.0\n\nrequire (\n\tgithub.com/mattn/go-pointer v0.0.1 // indirect\n)\n";
        let requires = parse_go_mod_requires(text);
        assert_eq!(
            requires
                .get("github.com/tree-sitter/go-tree-sitter")
                .map(String::as_str),
            Some("v0.25.0")
        );
        assert_eq!(
            requires.get("github.com/mattn/go-pointer").map(String::as_str),
            Some("v0.0.1")
        );
        assert_eq!(requires.len(), 2, "unexpected requires: {requires:?}");
    }

    #[test]
    fn parse_go_mod_requires_returns_empty_for_bare_module() {
        let text = "module example.com/bare\n\ngo 1.26\n";
        assert!(parse_go_mod_requires(text).is_empty());
    }

    fn direct_modules(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn resolve_indirect_requires_falls_back_to_testify_only_when_dir_is_none() {
        let resolved = resolve_indirect_requires(None, &direct_modules(&["github.com/example/mylib"]));
        let expected: Vec<(String, String)> = TESTIFY_INDIRECT_DEPS
            .iter()
            .map(|(m, v)| ((*m).to_string(), (*v).to_string()))
            .collect();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_indirect_requires_falls_back_to_testify_only_when_go_mod_missing() {
        let dir = tempfile::tempdir().expect("create temp dir");
        // No go.mod written into `dir`.
        let resolved = resolve_indirect_requires(Some(dir.path()), &direct_modules(&["github.com/example/mylib"]));
        let expected: Vec<(String, String)> = TESTIFY_INDIRECT_DEPS
            .iter()
            .map(|(m, v)| ((*m).to_string(), (*v).to_string()))
            .collect();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_indirect_requires_merges_dependency_go_mod_requires() {
        let dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/xberg-io/tree-sitter-language-pack/packages/go\n\n\
             go 1.26\n\n\
             require github.com/tree-sitter/go-tree-sitter v0.25.0\n\n\
             require github.com/mattn/go-pointer v0.0.1 // indirect\n",
        )
        .expect("write fixture go.mod");

        let resolved = resolve_indirect_requires(
            Some(dir.path()),
            &direct_modules(&[
                "github.com/xberg-io/tree-sitter-language-pack/packages/go",
                "github.com/stretchr/testify",
            ]),
        );

        assert_eq!(
            resolved,
            vec![
                ("github.com/davecgh/go-spew".to_string(), "v1.1.1".to_string()),
                ("github.com/mattn/go-pointer".to_string(), "v0.0.1".to_string()),
                ("github.com/pmezard/go-difflib".to_string(), "v1.0.0".to_string()),
                (
                    "github.com/tree-sitter/go-tree-sitter".to_string(),
                    "v0.25.0".to_string()
                ),
                ("gopkg.in/yaml.v3".to_string(), "v3.0.1".to_string()),
            ],
            "expected testify indirects merged alphabetically with the dependency's own requires"
        );
    }

    #[test]
    fn resolve_indirect_requires_excludes_self_reference() {
        let dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/example/mylib\n\ngo 1.26\n\nrequire github.com/example/mylib v1.0.0\n",
        )
        .expect("write fixture go.mod");

        let resolved = resolve_indirect_requires(Some(dir.path()), &direct_modules(&["github.com/example/mylib"]));
        assert!(
            !resolved.iter().any(|(module, _)| module == "github.com/example/mylib"),
            "a module must not appear as its own indirect require, got: {resolved:?}"
        );
    }

    #[test]
    fn resolve_indirect_requires_excludes_modules_already_required_directly() {
        // Models `tree-sitter-language-pack`'s `e2e/go`: `harness_extras` already lists
        // `go-tree-sitter` as a DIRECT require, and `packages/go/go.mod` also requires it
        // directly. It must not additionally appear in the `// indirect` block -- a module
        // cannot be required both directly and as `// indirect` in the same go.mod.
        let dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/xberg-io/tree-sitter-language-pack/packages/go\n\n\
             go 1.26\n\n\
             require github.com/tree-sitter/go-tree-sitter v0.25.0\n\n\
             require github.com/mattn/go-pointer v0.0.1 // indirect\n",
        )
        .expect("write fixture go.mod");

        let resolved = resolve_indirect_requires(
            Some(dir.path()),
            &direct_modules(&[
                "github.com/xberg-io/tree-sitter-language-pack/packages/go",
                "github.com/stretchr/testify",
                "github.com/tree-sitter/go-tree-sitter",
            ]),
        );

        assert!(
            !resolved
                .iter()
                .any(|(module, _)| module == "github.com/tree-sitter/go-tree-sitter"),
            "a module already required directly must not also appear as `// indirect`, got: {resolved:?}"
        );
        assert!(
            resolved
                .iter()
                .any(|(module, _)| module == "github.com/mattn/go-pointer"),
            "go-pointer (only indirect) should still be included, got: {resolved:?}"
        );
    }
}
