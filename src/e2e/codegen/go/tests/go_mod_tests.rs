//! Coverage for `render_go_mod`: registry vs local mode, testify indirect deps, and extras
//! (dependencies/dev-dependencies) merging, sorting, and idempotency. Also covers the direct
//! require block's canonical alphabetical ordering and the merge of a required module's own
//! on-disk go.mod requires into the generated `// indirect` block (the
//! `dependency_go_mod_dir` parameter) -- see `dependency_requires` for the lower-level parser
//! and merge/sort unit tests.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use super::render_go_mod;

#[test]
fn render_go_mod_without_extras() {
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", None, None);
    assert!(
        out.contains("github.com/example/mylib v1.0.0"),
        "should contain main module require"
    );
    assert!(
        out.contains("github.com/stretchr/testify v1.11.1"),
        "should contain testify require"
    );
    assert!(
        !out.contains("github.com/tree-sitter"),
        "should not contain tree-sitter without extras"
    );
}

#[test]
fn render_go_mod_includes_testify_indirect_deps() {
    // A go.mod that lists testify but omits its transitive deps makes `go test`
    // abort with "updates to go.mod needed; ... go mod tidy". The generated
    // test_app must carry a complete dependency graph so it builds without a
    // manual tidy (and offline).
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", None, None);
    for indirect in [
        "github.com/davecgh/go-spew v1.1.1 // indirect",
        "github.com/pmezard/go-difflib v1.0.0 // indirect",
        "gopkg.in/yaml.v3 v3.0.1 // indirect",
    ] {
        assert!(
            out.contains(indirect),
            "go.mod must contain testify indirect dep `{indirect}`, got:\n{out}"
        );
    }
}

#[test]
fn render_go_mod_registry_mode_uses_sibling_module_path() {
    // Registry mode (no replace): the main module must NOT be a subpath of the
    // module under test, or Go ignores the `require` directive and resolves a
    // stray upstream tag instead of the pinned version.
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", None, None);
    assert!(
        out.contains("module github.com/example/mylib-e2e"),
        "registry-mode main module must be a sibling path, got: {out}"
    );
    assert!(
        !out.contains("module github.com/example/mylib/e2e"),
        "registry-mode main module must not shadow the module under test, got: {out}"
    );
}

#[test]
fn render_go_mod_local_mode_uses_nested_module_path() {
    // Local mode (replace present): a nested `/e2e` main module resolves via the
    // replace directive, so keep the historical nested path.
    let out = render_go_mod(
        "github.com/example/mylib",
        Some("../../packages/go"),
        "v0.0.0",
        None,
        None,
    );
    assert!(
        out.contains("module github.com/example/mylib/e2e"),
        "local-mode main module should stay nested, got: {out}"
    );
}

#[test]
fn render_go_mod_with_extras_includes_requires() {
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};
    let mut extras = ManifestExtras::default();
    extras.dev_dependencies.insert(
        "github.com/tree-sitter/go-tree-sitter".to_string(),
        ExtraDepSpec::Simple("v0.24.0".to_string()),
    );
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    assert!(
        out.contains("github.com/tree-sitter/go-tree-sitter v0.24.0"),
        "should include tree-sitter extra, got: {out}"
    );
    assert!(
        out.contains("github.com/example/mylib v1.0.0"),
        "should still contain main module"
    );
}

#[test]
fn render_go_mod_extras_with_replace_directive() {
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};
    let mut extras = ManifestExtras::default();
    extras.dependencies.insert(
        "github.com/upstream/lib".to_string(),
        ExtraDepSpec::Simple("v0.5.0".to_string()),
    );
    let out = render_go_mod(
        "github.com/example/mylib",
        Some("../../packages/go"),
        "v0.0.0",
        Some(&extras),
        None,
    );
    assert!(
        out.contains("github.com/upstream/lib v0.5.0"),
        "should include upstream lib"
    );
    assert!(
        out.contains("replace github.com/example/mylib => ../../packages/go"),
        "should include replace directive"
    );
}

#[test]
fn render_go_mod_empty_extras_matches_no_extras() {
    use crate::core::config::manifest_extras::ManifestExtras;
    let extras = ManifestExtras::default();
    let without_empty = render_go_mod("github.com/example/mylib", None, "v1.0.0", None, None);
    let with_empty = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    assert_eq!(without_empty, with_empty, "empty extras should be equivalent to None");
}

#[test]
fn render_go_mod_extras_are_sorted_deterministically() {
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};
    let mut extras = ManifestExtras::default();
    extras.dev_dependencies.insert(
        "github.com/z-last/lib".to_string(),
        ExtraDepSpec::Simple("v1.0.0".to_string()),
    );
    extras.dev_dependencies.insert(
        "github.com/a-first/lib".to_string(),
        ExtraDepSpec::Simple("v2.0.0".to_string()),
    );
    extras.dependencies.insert(
        "github.com/m-middle/lib".to_string(),
        ExtraDepSpec::Simple("v3.0.0".to_string()),
    );
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    let first_idx = out.find("github.com/a-first/lib").expect("should find a-first");
    let middle_idx = out.find("github.com/m-middle/lib").expect("should find m-middle");
    let last_idx = out.find("github.com/z-last/lib").expect("should find z-last");
    assert!(
        first_idx < middle_idx && middle_idx < last_idx,
        "extras should be sorted alphabetically: {out}"
    );
}

#[test]
fn render_go_mod_extras_handles_detailed_form_with_version() {
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};
    let mut extras = ManifestExtras::default();
    let mut table = toml::Table::new();
    table.insert("version".to_string(), toml::Value::String("v0.25.0".to_string()));
    table.insert("features".to_string(), toml::Value::String("debug".to_string()));
    extras.dev_dependencies.insert(
        "github.com/example/with-features".to_string(),
        ExtraDepSpec::Detailed(table),
    );
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    assert!(
        out.contains("github.com/example/with-features v0.25.0"),
        "should extract version from detailed form, got: {out}"
    );
}

#[test]
fn render_go_mod_extras_skips_entries_without_version() {
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};
    let mut extras = ManifestExtras::default();
    let mut table = toml::Table::new();
    table.insert(
        "git".to_string(),
        toml::Value::String("https://example.com".to_string()),
    );
    extras.dev_dependencies.insert(
        "github.com/example/no-version".to_string(),
        ExtraDepSpec::Detailed(table),
    );
    let out = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    assert!(
        !out.contains("github.com/example/no-version"),
        "should skip extras without version field, got: {out}"
    );
}

#[test]
fn render_go_mod_extras_idempotent() {
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};
    let mut extras = ManifestExtras::default();
    extras.dev_dependencies.insert(
        "github.com/tree-sitter/go-tree-sitter".to_string(),
        ExtraDepSpec::Simple("v0.24.0".to_string()),
    );
    let first = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    let second = render_go_mod("github.com/example/mylib", None, "v1.0.0", Some(&extras), None);
    assert_eq!(first, second, "re-rendering with same extras should be stable");
}

#[test]
fn render_go_mod_direct_require_block_is_alphabetical() {
    // Every consumer module path is `github.com/xberg-io/...`, which always sorts after
    // `github.com/stretchr/testify` ('s' < 'x') -- so testify must come first in the generated
    // block, not the required module.
    let out = render_go_mod(
        "github.com/xberg-io/tree-sitter-language-pack/packages/go",
        None,
        "v1.16.2",
        None,
        None,
    );
    let testify_idx = out
        .find("github.com/stretchr/testify")
        .expect("should contain testify require");
    let module_idx = out
        .find("github.com/xberg-io/tree-sitter-language-pack/packages/go v1.16.2")
        .expect("should contain module require");
    assert!(
        testify_idx < module_idx,
        "testify must sort before the xberg module in the direct require block, got:\n{out}"
    );
}

#[test]
fn render_go_mod_merges_dependency_go_mod_indirect_requires() {
    // Models `tree-sitter-language-pack`'s real `packages/go/go.mod`: a direct require of
    // `go-tree-sitter` and an indirect require of `go-pointer`. Both must appear in the
    // generated go.mod's `// indirect` block, merged alphabetically with the testify indirects
    // -- dropping them makes `go build -mod=readonly` still pass (module pruning) but `go test
    // -mod=readonly ./...` fail with "updates to go.mod needed".
    let dir = tempfile::tempdir().expect("create temp dir");
    std::fs::write(
        dir.path().join("go.mod"),
        "module github.com/xberg-io/tree-sitter-language-pack/packages/go\n\n\
         go 1.26\n\n\
         require github.com/tree-sitter/go-tree-sitter v0.25.0\n\n\
         require github.com/mattn/go-pointer v0.0.1 // indirect\n",
    )
    .expect("write fixture go.mod");

    let out = render_go_mod(
        "github.com/xberg-io/tree-sitter-language-pack/packages/go",
        Some("../../packages/go"),
        "v1.16.2",
        None,
        Some(dir.path()),
    );

    // (a) direct require block is alphabetical: testify before the xberg module.
    let testify_idx = out
        .find("github.com/stretchr/testify")
        .expect("should contain testify require");
    let module_idx = out
        .find("github.com/xberg-io/tree-sitter-language-pack/packages/go v1.16.2")
        .expect("should contain module require");
    assert!(
        testify_idx < module_idx,
        "testify must sort before the xberg module, got:\n{out}"
    );

    // (b) indirect block merges testify's pinned indirects with the dependency's own requires,
    // alphabetically.
    let expected_indirect_order = [
        "github.com/davecgh/go-spew v1.1.1 // indirect",
        "github.com/mattn/go-pointer v0.0.1 // indirect",
        "github.com/pmezard/go-difflib v1.0.0 // indirect",
        "github.com/tree-sitter/go-tree-sitter v0.25.0 // indirect",
        "gopkg.in/yaml.v3 v3.0.1 // indirect",
    ];
    let mut last_idx = None;
    for indirect in expected_indirect_order {
        let idx = out
            .find(indirect)
            .unwrap_or_else(|| panic!("go.mod must contain indirect dep `{indirect}`, got:\n{out}"));
        if let Some(last) = last_idx {
            assert!(last < idx, "indirect block out of alphabetical order, got:\n{out}");
        }
        last_idx = Some(idx);
    }
}

#[test]
fn render_go_mod_excludes_extra_that_duplicates_dependency_direct_require() {
    // Models `tree-sitter-language-pack`'s real `e2e/go/go.mod`: `harness_extras` lists
    // `go-tree-sitter` as a direct require (Local dep mode injects the harness's own native
    // dep), and `packages/go/go.mod` ALSO requires it directly. It must appear exactly once, in
    // the direct block -- never additionally in the `// indirect` block, which would make it a
    // duplicate requirement for the same module.
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

    let dir = tempfile::tempdir().expect("create temp dir");
    std::fs::write(
        dir.path().join("go.mod"),
        "module github.com/xberg-io/tree-sitter-language-pack/packages/go\n\n\
         go 1.26\n\n\
         require github.com/tree-sitter/go-tree-sitter v0.25.0\n\n\
         require github.com/mattn/go-pointer v0.0.1 // indirect\n",
    )
    .expect("write fixture go.mod");

    let mut extras = ManifestExtras::default();
    extras.dev_dependencies.insert(
        "github.com/tree-sitter/go-tree-sitter".to_string(),
        ExtraDepSpec::Simple("v0.25.0".to_string()),
    );

    let out = render_go_mod(
        "github.com/xberg-io/tree-sitter-language-pack/packages/go",
        Some("../../packages/go"),
        "v0.0.0",
        Some(&extras),
        Some(dir.path()),
    );

    let occurrences = out.matches("github.com/tree-sitter/go-tree-sitter").count();
    assert_eq!(
        occurrences, 1,
        "go-tree-sitter must appear exactly once (direct, not also indirect), got:\n{out}"
    );
    assert!(
        !out.contains("github.com/tree-sitter/go-tree-sitter v0.25.0 // indirect"),
        "go-tree-sitter is already a direct require and must not also be `// indirect`, got:\n{out}"
    );
    assert!(
        out.contains("github.com/mattn/go-pointer v0.0.1 // indirect"),
        "go-pointer (only indirect) should still be included, got:\n{out}"
    );
}

#[test]
fn render_go_mod_dependency_with_no_extra_requires_matches_testify_only() {
    // Models `crawlberg`/`xberg`/`liter-llm`/`html-to-markdown`'s actual shape today: a bare
    // `module .../go.mod` `go 1.26` with zero requires. Output must be unchanged from the
    // testify-only indirect set (just reordered alphabetically in the direct block).
    let dir = tempfile::tempdir().expect("create temp dir");
    std::fs::write(
        dir.path().join("go.mod"),
        "module github.com/xberg-io/example/packages/go\n\ngo 1.26\n",
    )
    .expect("write fixture go.mod");

    let with_dependency_dir = render_go_mod(
        "github.com/xberg-io/example/packages/go",
        Some("../../packages/go"),
        "v1.0.0",
        None,
        Some(dir.path()),
    );
    let without_dependency_dir = render_go_mod(
        "github.com/xberg-io/example/packages/go",
        Some("../../packages/go"),
        "v1.0.0",
        None,
        None,
    );
    assert_eq!(
        with_dependency_dir, without_dependency_dir,
        "a dependency go.mod with no requires must not change the testify-only indirect output"
    );
    for indirect in [
        "github.com/davecgh/go-spew v1.1.1 // indirect",
        "github.com/pmezard/go-difflib v1.0.0 // indirect",
        "gopkg.in/yaml.v3 v3.0.1 // indirect",
    ] {
        assert!(
            with_dependency_dir.contains(indirect),
            "should still contain testify indirect `{indirect}`, got:\n{with_dependency_dir}"
        );
    }
}
