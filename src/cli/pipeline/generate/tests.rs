use super::*;

#[cfg(test)]
mod write_scaffold_normalize_tests {
    use super::*;
    use crate::core::backend::GeneratedFile;
    use std::path::PathBuf;

    fn make_file(name: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(name),
            content: content.to_owned(),
            generated_header: false,
        }
    }

    /// `write_scaffold_files_with_overwrite` must strip trailing whitespace and
    /// ensure a single trailing newline — matching what prek's
    /// `end-of-file-fixer` and `trailing-whitespace` hooks would do.
    #[test]
    fn test_scaffold_write_normalizes_trailing_whitespace_and_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content = "line one   \nline two\n\n";
        let files = vec![make_file("out.py", content)];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.py")).expect("read ok");
        assert_eq!(
            written, "line one\nline two\n",
            "trailing whitespace must be stripped and single newline ensured"
        );
    }

    #[test]
    fn test_scaffold_write_adds_missing_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![make_file("out.gleam", "pub fn main() {}")];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.gleam")).expect("read ok");
        assert!(
            written.ends_with('\n'),
            "file must end with newline, got: {:?}",
            written
        );
    }

    #[test]
    fn test_scaffold_write_does_not_add_double_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![make_file("out.zig", "const x = 1;\n")];
        write_scaffold_files_with_overwrite(&files, base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join("out.zig")).expect("read ok");
        assert!(!written.ends_with("\n\n"), "must not have double trailing newline");
        assert!(written.ends_with('\n'));
    }

    #[test]
    fn poly_scaffold_merges_generated_defaults_without_deleting_user_policy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let existing = r#"# Project policy. ~keep
[lint.python.ruff]
ignore = ["PT027", "S105"]

[per-file-ignores]
"tools/**" = ["S105"]

[hooks.builtin]
file_safety = { exclude = ["bindings/manual.rs"] }

[project-policy]
owner = "maintainers"
"#;
        std::fs::write(base.join("poly.toml"), existing).expect("write existing config");
        let generated = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: r#"[lint.python.ruff]
ignore = ["F401"]

[per-file-ignores]
"tests/**" = ["S101"]

[hooks.builtin]
file_safety = { exclude = ["target/**"] }
"#
            .into(),
            generated_header: true,
        };

        write_scaffold_files_with_overwrite(&[generated], base, true).expect("merge poly config");
        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        let parsed = merged.parse::<toml_edit::DocumentMut>().expect("merged TOML parses");

        assert!(merged.contains("# Project policy. ~keep"), "{merged}");
        assert_eq!(parsed["project-policy"]["owner"].as_str(), Some("maintainers"));
        assert!(parsed["per-file-ignores"]["tools/**"].is_value());
        assert!(parsed["per-file-ignores"]["tests/**"].is_value());
        let ruff_ignores = parsed["lint"]["python"]["ruff"]["ignore"]
            .as_array()
            .expect("ruff ignore array");
        assert!(ruff_ignores.iter().any(|value| value.as_str() == Some("PT027")));
        assert!(ruff_ignores.iter().any(|value| value.as_str() == Some("S105")));
        assert!(ruff_ignores.iter().any(|value| value.as_str() == Some("F401")));
        let file_safety = parsed["hooks"]["builtin"]["file_safety"]["exclude"]
            .as_array()
            .expect("file safety excludes");
        assert!(
            file_safety
                .iter()
                .any(|value| value.as_str() == Some("bindings/manual.rs"))
        );
        assert!(file_safety.iter().any(|value| value.as_str() == Some("target/**")));

        let generated_again = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: r#"[lint.python.ruff]
ignore = ["F401"]

[per-file-ignores]
"tests/**" = ["S101"]

[hooks.builtin]
file_safety = { exclude = ["target/**"] }
"#
            .into(),
            generated_header: true,
        };
        let count = write_scaffold_files_with_overwrite(&[generated_again], base, true).expect("repeat merge");
        assert_eq!(count, 0, "merged poly config must converge on a second scaffold pass");
    }

    /// Regression: the array-merge duplicate check used to compare
    /// `value.to_string().trim()`, which includes each value's decor. A value
    /// reformatted between runs (different quote style, different surrounding
    /// whitespace -- exactly what `poly fmt` does to a real consumer's
    /// `poly.toml` between alef passes) no longer textually matched the freshly
    /// generated value even though it decodes to the same string, so it got
    /// re-appended as a "new" entry -- the reported 4x-per-run growth of every
    /// default exclude. Decoding both sides before comparing must treat them
    /// as equal regardless of decor.
    #[test]
    fn poly_merge_does_not_duplicate_a_value_with_different_decor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let existing = "[discovery]\nexclude = [ \"target/**\" ]\n";
        std::fs::write(base.join("poly.toml"), existing).expect("write existing config");

        let generated = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[generated], base, true).expect("merge poly config");

        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        let parsed = merged.parse::<toml_edit::DocumentMut>().expect("merged TOML parses");
        let exclude = parsed["discovery"]["exclude"].as_array().expect("exclude array");
        assert_eq!(
            exclude.iter().filter(|v| v.as_str() == Some("target/**")).count(),
            1,
            "differently-decorated but identical values must not duplicate; got:\n{merged}"
        );
    }

    /// Companion to the decor regression above: a value already duplicated
    /// several times over on disk (the damage the decor bug already did to a
    /// consumer's committed `poly.toml` before this fix) must collapse to one
    /// occurrence on the very next merge. This is unconditionally safe --
    /// removing a redundant copy of a value that remains present at least
    /// once never changes the set of values the array represents -- so it
    /// needs no ownership/provenance information at all, unlike pruning a
    /// value alef no longer emits.
    #[test]
    fn poly_merge_collapses_pre_existing_duplicates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let existing =
            "[discovery]\nexclude = [\"target/**\", \"target/**\", \"target/**\", \"target/**\", \"vendor/**\"]\n";
        std::fs::write(base.join("poly.toml"), existing).expect("write existing config");

        let generated = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[generated], base, true).expect("merge poly config");

        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        let parsed = merged.parse::<toml_edit::DocumentMut>().expect("merged TOML parses");
        let exclude = parsed["discovery"]["exclude"].as_array().expect("exclude array");
        assert_eq!(
            exclude.iter().filter(|v| v.as_str() == Some("target/**")).count(),
            1,
            "four pre-existing copies must collapse to one; got:\n{merged}"
        );
        assert_eq!(
            exclude.iter().filter(|v| v.as_str() == Some("vendor/**")).count(),
            1,
            "an unrelated, non-duplicated value must be left alone; got:\n{merged}"
        );
    }

    /// The `merge_managed_toml` prune step: a value alef itself proposed on a
    /// prior run (recorded in the committed `.alef-toml-merge-provenance.toml`,
    /// straight from that run's own generated output) and no longer proposes
    /// must be removed once a baseline exists to compare against. The first
    /// run establishes the baseline and cannot prune anything yet (nothing to
    /// compare against); the second run, once alef's own template has
    /// stopped emitting the value, removes it.
    #[test]
    fn poly_merge_prunes_a_value_alef_stopped_emitting_once_a_baseline_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let first_generation = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"docs/assets/**\", \"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[first_generation], base, true).expect("first scaffold run");
        let after_first = std::fs::read_to_string(base.join("poly.toml")).expect("read after first run");
        assert!(
            after_first.contains("docs/assets/**"),
            "first run has no baseline to prune against; got:\n{after_first}"
        );

        // Simulate alef dropping `docs/assets/**` from its own EXCLUDES.
        let second_generation = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[second_generation], base, true).expect("second scaffold run");
        let after_second = std::fs::read_to_string(base.join("poly.toml")).expect("read after second run");
        assert!(
            !after_second.contains("docs/assets/**"),
            "a value alef itself proposed and then stopped emitting must be pruned; got:\n{after_second}"
        );
        assert!(
            after_second.contains("target/**"),
            "a value alef still emits must survive; got:\n{after_second}"
        );
    }

    /// A scoped run (`alef generate --lang java`) omits every out-of-scope language's
    /// `[lint.<lang>.<tool>]` table entirely, because `scaffold/languages/poly.rs` gates
    /// those blocks on `has(Language::Python)` / `has(Language::Php)`. The prune step must
    /// not read that absence as "alef stopped proposing these values" -- doing so strips the
    /// consumer's whole rule selection down to `select = []`, which every linter accepts and
    /// then checks nothing, so the gate goes green while proving nothing.
    ///
    /// Absence-from-output has three causes -- dropped on purpose, out of scope this run, or
    /// never reached -- and only the first licenses a removal. The contrast half of this test
    /// is load-bearing: it pins that prune still fires for a value missing from a table the
    /// run DID emit, so the fix cannot be satisfied by disabling prune altogether. ~keep
    #[test]
    fn poly_merge_never_prunes_a_table_a_scoped_run_did_not_emit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let all_languages = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"docs/assets/**\", \"target/**\"]\n\n\
                      [lint.python.ruff]\nselect = [\"E\", \"F\"]\n"
                .to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[all_languages], base, true).expect("all-languages run");

        // `--lang java`: no `[lint.python.ruff]` at all, and `docs/assets/**` dropped from a
        // table this run DOES emit.
        let scoped = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[scoped], base, true).expect("scoped run");

        let after = std::fs::read_to_string(base.join("poly.toml")).expect("read after scoped run");
        assert!(
            after.contains("\"E\"") && after.contains("\"F\""),
            "a scoped run must not empty a lint selection it was never asked to generate; got:\n{after}"
        );
        assert!(
            !after.contains("select = []"),
            "an emptied `select` is a linter that checks nothing while still exiting green; got:\n{after}"
        );
        assert!(
            !after.contains("docs/assets/**"),
            "prune must still fire for a value missing from a table the run DID emit; got:\n{after}"
        );
    }

    /// The `poly.toml` prune baseline must be determinable from the repository alone,
    /// identically on a fresh clone and on the machine that generated it -- the same
    /// #80-shaped reproducibility property already covered for scaffold ownership in
    /// `ownership_of_an_unmarkable_file_survives_a_cache_less_fresh_clone`.
    ///
    /// The baseline used to live at gitignored `.alef/toml-merge-provenance.json`, so a
    /// fresh clone or CI checkout never had it and the prune step could never fire there,
    /// no matter how long a value had been gone from alef's own template -- this is
    /// exactly the liter-llm `docs/assets/**` / `docs/snippets/**` staleness that had to
    /// be removed by hand in `12b1d0a69` instead of pruning itself.
    ///
    /// Deleting the whole `.alef/` cache between the baseline-establishing run and the
    /// pruning run is the load-bearing step -- it is what a fresh clone *is*. Without it
    /// the assertion would pass whether or not the baseline is committed, and prove
    /// nothing. ~keep
    #[test]
    fn poly_merge_prunes_a_stale_value_after_a_cache_less_fresh_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let first_generation = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"docs/assets/**\", \"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[first_generation], base, true).expect("first scaffold run");

        let record = base.join(".alef-toml-merge-provenance.toml");
        assert!(
            record.exists(),
            "establishing a merge baseline must leave a committable record"
        );

        std::fs::remove_dir_all(base.join(".alef")).ok();
        assert!(!base.join(".alef").exists(), "sanity: the machine-local cache is gone");

        // Simulate alef dropping `docs/assets/**` from its own EXCLUDES, on a checkout
        // that carries only the committed provenance record.
        let second_generation = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[second_generation], base, true).expect("second scaffold run");
        let after_second = std::fs::read_to_string(base.join("poly.toml")).expect("read after second run");
        assert!(
            !after_second.contains("docs/assets/**"),
            "a checkout carrying only the committed record must still prune a value alef stopped \
             emitting; got:\n{after_second}"
        );
        assert!(
            after_second.contains("target/**"),
            "a value alef still emits must survive; got:\n{after_second}"
        );
    }

    /// Negative control for the prune fix above: a value the CONSUMER hand-added to
    /// `poly.toml` -- never alef's own generated output -- must survive the same
    /// cache-less-fresh-clone scenario. A prune baseline that pruned everything on a
    /// fresh clone would pass the positive test above while destroying user config; this
    /// is what tells the two apart.
    #[test]
    fn poly_merge_never_prunes_a_consumer_value_after_a_cache_less_fresh_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let existing = "[discovery]\nexclude = [\"packages/**\"]\n";
        std::fs::write(base.join("poly.toml"), existing).expect("write existing, consumer-authored config");

        let generated = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(std::slice::from_ref(&generated), base, true).expect("first scaffold run");

        std::fs::remove_dir_all(base.join(".alef")).ok();
        assert!(!base.join(".alef").exists(), "sanity: the machine-local cache is gone");

        write_scaffold_files_with_overwrite(&[generated], base, true).expect("second scaffold run");
        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        assert!(
            merged.contains("packages/**"),
            "a value alef never generated must never be pruned, on a fresh clone or otherwise; \
             got:\n{merged}"
        );
    }

    /// The critical safety property: a value alef never once generated --
    /// standing in for a consumer's own `[workspace.poly] exclude` entry --
    /// must survive indefinitely, across any number of scaffold runs, even
    /// though it never appears in any of alef's own generated content. The
    /// prune step only ever removes values recorded straight from a *prior
    /// run's generated output*, never values merely present in `existing`;
    /// a value that was never alef's proposal is therefore never a candidate,
    /// regardless of how many runs pass or what alef's template does next.
    #[test]
    fn poly_merge_never_prunes_a_value_alef_never_generated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let existing = "[discovery]\nexclude = [\"packages/**\"]\n";
        std::fs::write(base.join("poly.toml"), existing).expect("write existing, consumer-authored config");

        for _ in 0..3 {
            let generated = GeneratedFile {
                path: PathBuf::from("poly.toml"),
                content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
                generated_header: true,
            };
            write_scaffold_files_with_overwrite(&[generated], base, true).expect("scaffold run");
        }

        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        assert!(
            merged.contains("packages/**"),
            "a value alef never generated must never be pruned, no matter how many runs pass; got:\n{merged}"
        );
    }

    /// Disclosed limitation, made explicit as a test rather than left only in
    /// prose: pruning only ever prevents *future* drift starting from the
    /// first run that establishes a baseline in a given working copy. A value
    /// that went stale *before* alef ever recorded a baseline here -- the
    /// exact shape of the already-reported damage in existing consumer repos
    /// -- is not retroactively cleaned up; there is nothing recorded to
    /// compare against, so nothing is removed, which is the same
    /// degrade-safely-to-no-op behaviour as the overwrite guard's ownership
    /// check when no local record exists.
    #[test]
    fn poly_merge_does_not_retroactively_prune_pre_existing_staleness_without_a_baseline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        // A value that leaked in before this mechanism ever ran -- no
        // `.alef-toml-merge-provenance.toml` record exists for it.
        let existing = "[discovery]\nexclude = [\"docs/assets/**\", \"target/**\"]\n";
        std::fs::write(base.join("poly.toml"), existing).expect("write existing, already-stale config");

        let generated = GeneratedFile {
            path: PathBuf::from("poly.toml"),
            content: "[discovery]\nexclude = [\"target/**\"]\n".to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(&[generated], base, true).expect("scaffold run");

        let merged = std::fs::read_to_string(base.join("poly.toml")).expect("read merged config");
        assert!(
            merged.contains("docs/assets/**"),
            "with no recorded baseline, pre-existing staleness must survive untouched (documented \
             limitation, not a bug); got:\n{merged}"
        );
    }

    /// `normalize_content` must strip trailing whitespace from `.rs` files even
    /// when rustfmt rejects them — e.g. cextendr `lib.rs` files use the
    /// `name: T = "default"` parameter-default syntax that rustfmt cannot
    /// parse, so it falls back to the raw codegen output. Without a final
    /// whitespace pass, the raw output's trailing-whitespace blank lines
    /// (e.g. `    \n` between `#[must_use]` and `pub fn …`) survive into the
    /// finalised `alef:hash`, and prek's `trailing-whitespace` hook then
    /// rewrites the file post-hash, breaking `alef verify`.
    #[test]
    fn test_normalize_content_strips_trailing_whitespace_when_rustfmt_fails() {
        let path = PathBuf::from("packages/r/src/rust/src/lib.rs");
        let content = "extendr_module! {\n    fn convert(\n    \n        title: String = \"\",\n    );\n}\n";
        let normalized = normalize_content(std::path::Path::new("."), &path, content);
        for (i, line) in normalized.lines().enumerate() {
            assert_eq!(
                line.trim_end(),
                line,
                "line {i} has trailing whitespace after normalize: {line:?}"
            );
        }
        assert!(normalized.ends_with('\n'), "must end with newline");
    }

    /// `sweep_orphans` must delete alef-marked files that aren't in the keep set,
    /// preserve user-owned files (no marker), and preserve files that are in the
    /// keep set even if they have the marker.
    #[test]
    fn test_sweep_orphans_removes_only_alef_marked_files_outside_keep_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let nested = base.join("e2e/elixir/test");
        std::fs::create_dir_all(&nested).expect("mkdir");

        let alef_marker = "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:abc\n";
        let kept = nested.join("keep_test.exs");
        let orphan = nested.join("orphan_test.exs");
        let user_owned = nested.join("user_helper.exs");

        std::fs::write(&kept, format!("{alef_marker}defmodule Keep do\nend\n")).unwrap();
        std::fs::write(&orphan, format!("{alef_marker}defmodule Orphan do\nend\n")).unwrap();
        std::fs::write(&user_owned, "defmodule UserHelper do\nend\n").unwrap();

        let mut keep = std::collections::HashSet::new();
        keep.insert(kept.clone());

        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 1, "should remove exactly one orphan");
        assert!(kept.exists(), "kept alef-marked file must remain");
        assert!(!orphan.exists(), "orphan alef-marked file must be removed");
        assert!(user_owned.exists(), "user-owned (no marker) file must remain");
    }

    /// `sweep_orphans` must skip dependency / build directories (target, node_modules,
    /// _build, deps, vendor, build, dist, .git, .venv) so it never deletes anything
    /// inside a vendored or compiled tree.
    #[test]
    fn test_sweep_orphans_skips_dependency_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let alef_marker = "// auto-generated by alef\n// alef:hash:def\n";
        for skip_dir in ["target", "node_modules", "_build", "vendor"] {
            let nested = base.join(skip_dir).join("nested");
            std::fs::create_dir_all(&nested).expect("mkdir");
            std::fs::write(nested.join("orphan.rs"), alef_marker).unwrap();
        }
        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 0, "must not descend into dependency directories");
    }

    #[test]
    fn targeted_e2e_sweep_ignores_snippet_outputs_nested_under_e2e_root() {
        let base = PathBuf::from("/workspace");
        let e2e_root = base.join("e2e");
        let snippet_root = e2e_root.join("ruby");
        let outputs = vec![snippet_root.join(".alef-snippet-coverage.json")];

        let roots = targeted_e2e_sweep_roots(&outputs, &e2e_root, Some(&snippet_root));

        assert!(roots.is_empty(), "snippet-only output must not authorize an e2e sweep");
    }

    #[test]
    fn targeted_e2e_sweep_includes_only_languages_with_current_e2e_outputs() {
        let base = PathBuf::from("/workspace");
        let e2e_root = base.join("e2e");
        let snippet_root = base.join("docs/snippets-generated");
        let outputs = vec![
            e2e_root.join("ruby/spec/example_spec.rb"),
            snippet_root.join("ruby/api/example.md"),
        ];

        let roots = targeted_e2e_sweep_roots(&outputs, &e2e_root, Some(&snippet_root));

        assert_eq!(roots, [e2e_root.join("ruby/spec")]);
    }

    #[test]
    fn targeted_e2e_sweep_preserves_specs_when_only_scaffold_and_snippets_are_generated() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let e2e_root = directory.path().join("e2e");
        let snippet_root = directory.path().join("docs/snippets-generated");
        let spec_directory = e2e_root.join("ruby/spec");
        std::fs::create_dir_all(&spec_directory).expect("create spec directory");
        let existing_spec = spec_directory.join("existing_spec.rb");
        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(
            &existing_spec,
            format!("# alef:hash:{HASH}\n# This file is auto-generated by alef — DO NOT EDIT.\n"),
        )
        .expect("write existing spec");
        let outputs = vec![
            e2e_root.join("ruby/Gemfile"),
            snippet_root.join("ruby/http/create_item.md"),
            snippet_root.join(".alef-snippet-coverage.json"),
        ];
        let keep = outputs.iter().cloned().collect();

        let roots = targeted_e2e_sweep_roots(&outputs, &e2e_root, Some(&snippet_root));
        let removed = sweep_orphans(&roots, &keep).expect("sweep targeted outputs");

        assert!(
            roots.is_empty(),
            "top-level scaffolding and snippets own no E2E test subtree"
        );
        assert_eq!(removed, 0);
        assert!(
            existing_spec.exists(),
            "an ungenerated E2E subtree must remain untouched"
        );
    }

    #[test]
    fn managed_toml_scaffold_replaces_marked_content_and_records_ownership() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let manifest = directory.path().join("crates/sample-py/Cargo.toml");
        std::fs::create_dir_all(manifest.parent().expect("manifest parent")).expect("create manifest directory");
        std::fs::write(
            &manifest,
            "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:old\n\
             [package]\nname = \"old-name\"\n\n[lints.clippy]\nunwrap_used = \"deny\"\n",
        )
        .expect("write existing manifest");
        let files = vec![crate::core::backend::GeneratedFile {
            path: PathBuf::from("crates/sample-py/Cargo.toml"),
            content: "[package]\nname = \"sample-py\"\nversion = \"1.0.0\"\n".into(),
            generated_header: true,
        }];

        let written = write_scaffold_files(&files, directory.path()).expect("refresh managed manifest");
        let refreshed = std::fs::read_to_string(&manifest).expect("read refreshed manifest");

        assert_eq!(written, 1);
        assert!(refreshed.contains("auto-generated by alef"), "{refreshed}");
        assert!(refreshed.contains("name = \"sample-py\""), "{refreshed}");
        assert!(!refreshed.contains("[lints.clippy]"), "{refreshed}");
        assert!(!refreshed.contains("unwrap_used = \"deny\""), "{refreshed}");
        let grouped = vec![(crate::core::config::Language::Rust, files)];
        assert!(
            diff_files(&grouped, directory.path())
                .expect("diff managed manifest")
                .is_empty()
        );
    }

    /// Regression: a file that contains loose "auto-generated" or "DO NOT EDIT"
    /// markers but lacks the `alef:hash:` line must NOT be deleted by
    /// `sweep_orphans`. This protects consumer-vendored files such as cgo headers.
    #[test]
    fn sweep_orphans_preserves_loose_marker_file_without_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let include_dir = base.join("packages/go/include");
        std::fs::create_dir_all(&include_dir).expect("mkdir");

        let vendored = include_dir.join("sample_crawler.h");
        std::fs::write(
            &vendored,
            "// DO NOT EDIT — vendored cgo header\n#ifndef FOO_H\n#define FOO_H\n\ntypedef void CrawlEngine;\n\n#endif\n",
        )
        .unwrap();

        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 0, "vendored file without alef:hash must not be deleted");
        assert!(vendored.exists(), "vendored cgo header must survive sweep_orphans");
    }

    /// Positive path: a file that contains the `alef:hash:` line IS alef-owned
    /// and must be deleted by `sweep_orphans` when not in the keep set.
    #[test]
    fn sweep_orphans_removes_file_with_alef_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out_dir = base.join("e2e/rust/src");
        std::fs::create_dir_all(&out_dir).expect("mkdir");

        const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let alef_file = out_dir.join("lib.rs");
        std::fs::write(
            &alef_file,
            format!(
                "// This file is auto-generated by alef — DO NOT EDIT.\n// alef:hash:{HASH}\npub fn hello() {{}}\n"
            ),
        )
        .unwrap();

        let keep: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let removed = sweep_orphans(&[base.to_path_buf()], &keep).expect("sweep ok");
        assert_eq!(removed, 1, "alef-owned file not in keep set must be deleted");
        assert!(!alef_file.exists(), "alef:hash file must be removed by sweep_orphans");
    }

    /// `collect_alef_headered_paths` must return all alef-headered files under
    /// the given root and skip user-owned (no marker) files.
    #[test]
    fn test_collect_alef_headered_paths_finds_headered_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let lang_dir = base.join("python");
        std::fs::create_dir_all(&lang_dir).expect("mkdir");

        let alef_marker = "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:abc123\nprint('hello')\n";
        let user_file = "print('user code')\n";

        let headered = lang_dir.join("test_chat.py");
        let plain = lang_dir.join("conftest.py");
        std::fs::write(&headered, alef_marker).unwrap();
        std::fs::write(&plain, user_file).unwrap();

        let collected = collect_alef_headered_paths(base);
        assert!(collected.contains(&headered), "alef-headered file must be collected");
        assert!(!collected.contains(&plain), "user-owned file must not be collected");
    }

    /// `collect_alef_headered_paths` on a non-existent root must return an
    /// empty set without panicking.
    #[test]
    fn test_collect_alef_headered_paths_missing_root_returns_empty() {
        let paths = collect_alef_headered_paths(std::path::Path::new("/nonexistent/test_apps"));
        assert!(paths.is_empty(), "missing root must yield empty set");
    }

    /// Regression for #524: `collect_alef_headered_paths` must find an
    /// alef-headered file that is missing its `alef:hash:` line. Matching on
    /// `extract_hash(..).is_some()` instead of the marker (the pre-fix
    /// behaviour) makes an already-unstamped file permanently invisible to
    /// this scan, which is exactly how a stripped file stays stripped
    /// forever.
    #[test]
    fn test_collect_alef_headered_paths_finds_files_missing_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let lang_dir = base.join("java");
        std::fs::create_dir_all(&lang_dir).expect("mkdir");

        let headered_no_hash = "// This file is auto-generated by alef — DO NOT EDIT.\npublic class Foo {}\n";
        let headered_path = lang_dir.join("Foo.java");
        std::fs::write(&headered_path, headered_no_hash).unwrap();

        let collected = collect_alef_headered_paths(base);
        assert!(
            collected.contains(&headered_path),
            "a headered file missing its alef:hash: line must still be collected, got: {collected:?}"
        );
    }

    /// Regression for #524: `finalize_hashes_sweeping` must re-stamp an
    /// alef-headered, hash-less file found on disk under `roots` even when
    /// that file's path is absent from the caller-supplied `paths` set. This
    /// is the exact scenario a language dropped from the per-language cache
    /// (`generation::generate`) produces: its output is never added to
    /// `current_gen_paths`, so plain `finalize_hashes` would leave it
    /// unstamped forever.
    #[test]
    fn test_finalize_hashes_sweeping_restamps_file_absent_from_explicit_path_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("java");
        std::fs::create_dir_all(&root).expect("mkdir");

        let stripped_content = "// This file is auto-generated by alef — DO NOT EDIT.\npublic class Foo {}\n";
        let cached_language_file = root.join("Foo.java");
        std::fs::write(&cached_language_file, stripped_content).unwrap();

        // Simulate a run whose in-memory tracking never saw this file at all
        // (e.g. its language hit the per-language cache and was dropped from
        // `bindings` before `current_gen_paths` was built).
        let explicit_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        let roots = vec![dir.path().to_path_buf()];
        let sources_hash = "sources";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"java\"]\n";

        let updated =
            finalize_hashes_sweeping(&explicit_paths, &roots, sources_hash, alef_toml_bytes).expect("sweep ok");
        assert_eq!(updated, 1, "the swept, previously-unstamped file must be finalized");

        let after = std::fs::read_to_string(&cached_language_file).expect("read after sweep");
        assert!(
            crate::core::hash::extract_hash(&after).is_some(),
            "file discovered only via the roots sweep must carry an alef:hash: line, got:\n{after}"
        );
    }

    /// `finalize_hashes_sweeping` must not double-count a file present in
    /// both the explicit `paths` set and the `roots` sweep -- the union is a
    /// `HashSet`, so it is stamped exactly once either way.
    #[test]
    fn test_finalize_hashes_sweeping_does_not_duplicate_explicitly_tracked_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("python");
        std::fs::create_dir_all(&root).expect("mkdir");

        let content = "# This file is auto-generated by alef — DO NOT EDIT.\nprint('hi')\n";
        let file_path = root.join("mod.py");
        std::fs::write(&file_path, content).unwrap();

        let explicit_paths: std::collections::HashSet<std::path::PathBuf> =
            std::iter::once(file_path.clone()).collect();
        let roots = vec![dir.path().to_path_buf()];

        let updated = finalize_hashes_sweeping(&explicit_paths, &roots, "sources", b"[workspace]\n").expect("sweep ok");
        assert_eq!(updated, 1, "one physical file must produce exactly one update");
    }

    /// End-to-end regression for #524's documented two-pass design: a file
    /// written by `write_files_report` (headered, no hash yet by design) must
    /// carry an `alef:hash:` line once `finalize_hashes` runs over its path,
    /// matching what every real pipeline caller does.
    #[test]
    fn test_write_then_finalize_stamps_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let files = vec![(
            crate::core::config::Language::Rust,
            vec![GeneratedFile {
                path: PathBuf::from("lib.rs"),
                content: "pub fn hello() {}\n".to_string(),
                generated_header: true,
            }],
        )];

        let report = write_files_report(&files, base).expect("write ok");
        let written_path = base.join("lib.rs");
        assert!(report.changed_paths.contains(&written_path));

        let after_write = std::fs::read_to_string(&written_path).expect("read after write");
        assert!(
            crate::core::hash::extract_hash(&after_write).is_none(),
            "write_files_report must not embed a hash before finalize_hashes runs"
        );

        let mut paths = std::collections::HashSet::new();
        paths.insert(written_path.clone());
        finalize_hashes(&paths, "sources", b"[workspace]\nlanguages = [\"rust\"]\n").expect("finalize ok");

        let after_finalize = std::fs::read_to_string(&written_path).expect("read after finalize");
        assert!(
            crate::core::hash::extract_hash(&after_finalize).is_some(),
            "a written file must carry an alef:hash: line after finalize_hashes, got:\n{after_finalize}"
        );
    }

    /// Invariant: after `write` + simulated format-pass + `finalize_hashes`, the
    /// embedded `alef:hash:` must cover the finalized file body.
    #[test]
    fn test_finalize_hashes_embeds_per_file_content_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content_before_format = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n";
        let file_path = base.join("lib.rs");
        std::fs::write(&file_path, content_before_format).expect("write pre-format content");

        let content_after_format = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n\n";
        std::fs::write(&file_path, content_after_format).expect("write post-format content");

        let sources_hash = "deadbeef";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"rust\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("finalize ok");

        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");
        let embedded = crate::core::hash::extract_hash(&finalised).expect("hash must be present");
        let expected = crate::core::hash::compute_file_hash(content_after_format);
        assert_eq!(embedded, expected, "embedded hash must cover the finalized file body");

        let reformatted = format!("{content_after_format}\n// formatter added this line\n");
        std::fs::write(&file_path, &reformatted).expect("simulate post-finalize formatter rewrite");
        let after_reformat = std::fs::read_to_string(&file_path).expect("read after reformat");
        let _still_embedded = crate::core::hash::extract_hash(&after_reformat);
        assert_ne!(
            crate::core::hash::compute_file_hash(&after_reformat),
            expected,
            "editing generated output must invalidate its embedded hash"
        );
    }

    /// Regression: `finalize_hashes` must be idempotent when run twice on the
    /// same file — the second pass must detect the existing hash is already
    /// correct and skip the write.
    #[test]
    fn test_finalize_hashes_is_idempotent_with_inputs_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let content = "// This file is auto-generated by alef — DO NOT EDIT.\nfn hello() {}\n";
        let file_path = base.join("lib.rs");
        std::fs::write(&file_path, content).expect("write initial content");

        let sources_hash = "sources";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"rust\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());

        let n1 = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("first finalize");
        assert_eq!(n1, 1, "first finalize must write the hash line");

        let n2 = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("second finalize");
        assert_eq!(n2, 0, "second finalize must be a no-op (same inputs hash)");
    }

    /// `finalize_hashes` must skip files without the alef header marker, even
    /// when a non-Rust file has content that would otherwise match. Go files
    /// (gofmt emitting blank lines) are preserved unchanged.
    #[test]
    fn test_finalize_hashes_non_rust_file_gets_inputs_hash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let gofmt_output = concat!(
            "// This file is auto-generated by alef — DO NOT EDIT.\n",
            "package foo\n",
            "\n",
            "\n",
            "func Hello() {}\n",
        );
        let file_path = base.join("binding.go");
        std::fs::write(&file_path, gofmt_output).expect("write gofmt output");

        let sources_hash = "deadbeef";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"go\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("finalize ok");

        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");

        let embedded = crate::core::hash::extract_hash(&finalised).expect("hash must be present");
        let expected = crate::core::hash::compute_file_hash(gofmt_output);
        assert_eq!(embedded, expected, "embedded hash must cover Go file content");

        let stripped = crate::core::hash::strip_hash_line(&finalised);
        assert!(
            stripped.contains("\n\n\n"),
            "two consecutive blank lines must survive finalize_hashes: got:\n{stripped:?}"
        );
    }

    /// Regression: `finalize_hashes` must recognize both "auto-generated by alef"
    /// (standard header) and "Generated by alef" (custom headers in Swift, Kotlin,
    /// Dart, Gleam, Zig, JNI). Without this, renamed files like SwiftPluginHelpers.swift
    /// would not get the `alef:hash:` marker, preventing the cleanup system from
    /// identifying them as alef-owned and deleting stale renamed files.
    #[test]
    fn test_finalize_hashes_recognizes_generated_by_alef_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let swift_content =
            "// Generated by alef. Do not edit by hand.\n// swift-format-ignore-file\n\nimport Foundation\n";
        let file_path = base.join("Helpers.swift");
        std::fs::write(&file_path, swift_content).expect("write swift content");

        let sources_hash = "deadbeef";
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"swift\"]\n";
        let mut paths = std::collections::HashSet::new();
        paths.insert(file_path.clone());
        let updated = finalize_hashes(&paths, sources_hash, alef_toml_bytes).expect("finalize ok");

        assert_eq!(
            updated, 1,
            "finalize_hashes must process files with 'Generated by alef' header"
        );

        let finalised = std::fs::read_to_string(&file_path).expect("read finalised");

        let embedded = crate::core::hash::extract_hash(&finalised).expect("hash must be present");
        let expected = crate::core::hash::compute_file_hash(swift_content);
        assert_eq!(embedded, expected, "embedded hash must cover Swift file content");
    }

    /// Regression: `write_scaffold_files_with_overwrite(overwrite=false)` must
    /// skip files that already exist on disk, leaving the existing content
    /// unchanged.  This is the invariant relied on by scaffold-once files
    /// (Cargo.toml, package.json, gemspec) — user customisations are preserved.
    ///
    /// README files are NOT scaffold-once: they are always regenerated from
    /// templates, and `generated_header: true` content is always attempted
    /// regardless of the `overwrite` flag once alef can prove ownership of the
    /// path (see `write_scaffold_files_report`'s guard doc) — `overwrite` only
    /// ever gates `generated_header: false` seeds. This closes the original
    /// `alef generate`/`alef readme` divergence this test used to document as a
    /// bug state: before the ownership guard existed, `overwrite: false` used
    /// to silently preserve externally-reformatted content (e.g. `rumdl-fmt`
    /// padding table columns) while `overwrite: true` replaced it with compact
    /// bytes, so the two commands could produce different bytes for the same
    /// README depending on which flag they happened to pass. That is no longer
    /// possible: both flags now produce byte-identical output whenever alef can
    /// prove ownership.
    ///
    /// README.md is `generated_header: true` in production (see
    /// `readme/template.rs`) but `.md` is an unmarkable extension, so the
    /// initial write below is what establishes alef's ownership record for it
    /// ([`crate::cli::cache::is_scaffold_owned_path`]); real README output also
    /// self-embeds an HTML-comment marker (`readme/template.rs`'s `~keep` note)
    /// so a *committed* README proves ownership from content alone even on a
    /// fresh clone, but this test constructs its `GeneratedFile` directly
    /// without going through that real generator, so it still relies on the
    /// local record here.
    #[test]
    fn readme_overwrite_flag_no_longer_produces_divergent_bytes_once_owned() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let compact_content = "# My README\n\n| Document | Size |\n|----------|------|\n| Lists (Timeline) | 129KB |\n";
        let generated = GeneratedFile {
            path: PathBuf::from("README.md"),
            content: compact_content.to_owned(),
            generated_header: true,
        };
        write_scaffold_files_with_overwrite(std::slice::from_ref(&generated), base, true)
            .expect("initial write establishes ownership");

        let padded_content = "# My README\n\n| Document            | Size  |\n| ------------------- | ----- |\n| Lists (Timeline)    | 129KB |\n";

        std::fs::write(base.join("README.md"), padded_content).expect("simulate rumdl-fmt padding");
        write_scaffold_files_with_overwrite(std::slice::from_ref(&generated), base, false)
            .expect("write ok (overwrite=false)");
        let after_false = std::fs::read_to_string(base.join("README.md")).expect("read");
        assert!(
            after_false.contains("|----------|") && !after_false.contains("| ------------------- |"),
            "overwrite=false must replace externally-reformatted content once ownership is \
             established -- it is no longer a create-only flag for generated_header: true content, \
             got:\n{after_false}"
        );

        std::fs::write(base.join("README.md"), padded_content).expect("simulate rumdl-fmt padding again");
        write_scaffold_files_with_overwrite(&[generated], base, true).expect("write ok (overwrite=true)");
        let after_true = std::fs::read_to_string(base.join("README.md")).expect("read");

        assert_eq!(
            after_false, after_true,
            "overwrite=false and overwrite=true must produce byte-identical output for an \
             alef-owned generated_header: true file -- the divergence this test used to document \
             is closed"
        );
        assert_eq!(
            after_true,
            normalize_content(base, &std::path::PathBuf::from("README.md"), compact_content),
            "alef readme and alef all must produce identical on-disk bytes for README files"
        );
    }

    /// A `.gitattributes` (or any seed file with `generated_header: false`) written
    /// by `write_scaffold_files(overwrite=false)` must not be overwritten when the
    /// file already exists on disk. This preserves hand-added entries such as
    /// `* text=auto eol=lf` that the user may have added alongside alef's entries.
    #[test]
    fn seed_file_with_generated_header_false_is_preserved_on_overwrite_false() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let original = "# hand-crafted\n* text=auto eol=lf\n";
        std::fs::write(base.join(".gitattributes"), original).expect("write original");

        let generated = GeneratedFile {
            path: std::path::PathBuf::from(".gitattributes"),
            content: "# Generated by alef scaffold.\ne2e/** linguist-generated=true\n".to_owned(),
            generated_header: false,
        };

        let count = write_scaffold_files_with_overwrite(&[generated], base, false).expect("write ok");
        assert_eq!(
            count, 0,
            "overwrite=false must not write any file when seed already exists"
        );

        let after = std::fs::read_to_string(base.join(".gitattributes")).expect("read");
        assert_eq!(
            after, original,
            "overwrite=false must not touch an existing seed file (generated_header: false)"
        );
    }

    /// `detect_crate_edition` must return the edition declared in the nearest
    /// `Cargo.toml` when one is present, and fall back to `"2024"` when absent.
    #[test]
    fn test_detect_crate_edition_reads_from_cargo_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let cargo_toml = "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        std::fs::write(base.join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");

        let src = base.join("src").join("lib.rs");
        std::fs::create_dir_all(src.parent().unwrap()).expect("mkdir src");

        let edition = detect_crate_edition(base, &src);
        assert_eq!(edition, "2021", "should detect edition 2021 from Cargo.toml");
    }

    #[test]
    fn test_detect_crate_edition_defaults_to_2024_when_no_cargo_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let orphan = base.join("orphan.rs");

        let edition = detect_crate_edition(base, &orphan);
        assert_eq!(edition, "2024", "should default to 2024 when no Cargo.toml found");
    }

    #[test]
    fn test_detect_crate_edition_defaults_to_2024_when_edition_absent_from_cargo_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        std::fs::write(
            base.join("Cargo.toml"),
            "[package]\nname = \"no-edition-crate\"\nversion = \"0.1.0\"\n",
        )
        .expect("write Cargo.toml");

        let src = base.join("lib.rs");
        let edition = detect_crate_edition(base, &src);
        assert_eq!(edition, "2024", "should default to 2024 when edition field absent");
    }

    #[test]
    fn test_parse_package_edition_extracts_value() {
        let toml = "[package]\nname = \"x\"\nedition = \"2021\"\n";
        assert_eq!(parse_package_edition(toml).as_deref(), Some("2021"));
    }

    #[test]
    fn test_parse_package_edition_ignores_other_sections() {
        let toml = "[workspace]\nedition = \"2021\"\n[package]\nname = \"x\"\n";
        assert_eq!(parse_package_edition(toml), None);
    }

    /// `write_scaffold_files_with_overwrite` must set the executable bit on files
    /// whose content begins with a shebang line, matching the behaviour of
    /// `write_files`. Previously the scaffold writer lacked the chmod call, so
    /// generated shell scripts (e.g. `download_ffi.sh`, `run_tests.sh`) landed
    /// as `-rw-r--r--` and consumers could not execute them.
    #[cfg(unix)]
    #[test]
    fn test_scaffold_write_sets_executable_bit_for_shebang_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let shebang_content = "#!/usr/bin/env bash\nset -euo pipefail\necho hello\n";
        let file = GeneratedFile {
            path: std::path::PathBuf::from("run_tests.sh"),
            content: shebang_content.to_owned(),
            generated_header: false,
        };

        write_scaffold_files_with_overwrite(&[file], base, true).expect("write ok");

        let path = base.join("run_tests.sh");
        let metadata = std::fs::metadata(&path).expect("metadata");
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o100 != 0,
            "shebang file must have owner-executable bit set, got mode {mode:#o}"
        );
    }

    /// Non-shebang files must NOT receive the executable bit.
    #[cfg(unix)]
    #[test]
    fn test_scaffold_write_does_not_set_executable_bit_for_non_shebang_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let plain_content = "# not a shebang\nsome content\n";
        let file = GeneratedFile {
            path: std::path::PathBuf::from("plain.sh"),
            content: plain_content.to_owned(),
            generated_header: false,
        };

        write_scaffold_files_with_overwrite(&[file], base, true).expect("write ok");

        let path = base.join("plain.sh");
        let metadata = std::fs::metadata(&path).expect("metadata");
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o111 == 0,
            "non-shebang file must not have any executable bit set, got mode {mode:#o}"
        );
    }
}

/// Regression coverage for the crawlberg incident: a plain `alef all --clean` run
/// silently claimed and stamped hand-written `e2e/go/helpers_test.go` /
/// `e2e/go/main_test.go` (added an `alef:hash:` header they never had), and
/// separately clobbered `e2e/elixir/test/test_helper.exs` (deleted a hand-written
/// FFI environment-propagation workaround), because `write_scaffold_files_report`
/// wrote every `generated_header: true` file unconditionally, without ever
/// checking whether the pre-existing content on disk had ever been alef's.
#[cfg(test)]
mod scaffold_ownership_guard_tests {
    use super::*;
    use crate::core::backend::GeneratedFile;
    use crate::core::hash::{CommentStyle, content_has_alef_marker, extract_hash, header, inject_hash_line};
    use std::path::PathBuf;

    fn marked_content(body: &str) -> String {
        let with_header = format!("{}{body}", header(CommentStyle::DoubleSlash));
        inject_hash_line(&with_header, &"0".repeat(64))
    }

    /// THE core regression test: a pre-existing, unmarked file at a path alef has
    /// never emitted before must survive a `generated_header: true` write
    /// byte-for-byte, and must NOT gain an `alef:hash:` marker. Without the
    /// ownership guard in `write_scaffold_files_report`, this fails — the write
    /// path stamps the alef header onto the hand-written body unconditionally
    /// (reproducing the Go half of the crawlberg incident exactly: alef's
    /// generated content for `helpers_test.go` happened to be byte-identical to
    /// the hand-written file, so the only visible change was the new header).
    #[test]
    fn pre_existing_unmarked_file_at_unrecorded_path_survives_untouched_and_unstamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("e2e/go/helpers_test.go");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let hand_written =
            "package e2e_test\n\n// jsonString is hand-rolled here.\nfunc jsonString(v any) string { return \"\" }\n";
        std::fs::write(&target, hand_written).expect("seed hand-written file");

        let generated = GeneratedFile {
            path: target_relative,
            content: hand_written.to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(&target).expect("read after");
        assert_eq!(
            after, hand_written,
            "a path alef has never recorded as its own output must be left byte-for-byte untouched"
        );
        assert!(
            extract_hash(&after).is_none(),
            "a file alef never authored must never gain an alef:hash: marker, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
    }

    #[test]
    fn pre_existing_unmarked_manifest_is_not_claimed_or_merged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target_relative = PathBuf::from("crates/sample-ffi/Cargo.toml");
        let target = dir.path().join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
        let hand_written = "[package]\nname = \"hand-written\"\n";
        std::fs::write(&target, hand_written).expect("seed hand-written manifest");
        let generated = GeneratedFile {
            path: target_relative,
            content: "[package]\nname = \"generated\"\n".to_string(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], dir.path(), true).expect("write report");

        assert_eq!(std::fs::read_to_string(target).expect("read manifest"), hand_written);
        assert_eq!(report.changed_count(), 0);
    }

    /// The ownership guard must not fire on a path alef is physically unable to stamp,
    /// *provided* alef has a durable committed record of having owned it before
    /// ([`crate::cli::cache::is_scaffold_owned_path`]).
    ///
    /// `ensure_generated_header` only knows a comment syntax for a fixed extension set;
    /// everything else (`.md` above all) is returned unchanged, so an alef-authored
    /// README never carries a marker no matter how many times it is regenerated. Keying
    /// the guard on the marker alone would therefore read every generated README as
    /// foreign content and freeze it permanently on the first run after this guard ships
    /// -- the ownership-manifest record (populated by this write path itself the first
    /// time it creates or confirms a path) is what lets a genuinely alef-owned unmarkable
    /// file keep regenerating instead.
    #[test]
    fn unstampable_generated_file_is_still_regenerated_when_alef_owns_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("README.md");
        let target = base.join(&target_relative);
        std::fs::write(&target, "# Stale generated README\n").expect("seed previous output");
        crate::cli::cache::record_scaffold_owned_path(base, &target).expect("seed ownership record");

        let regenerated = "# Regenerated README\n";
        let generated = GeneratedFile {
            path: target_relative,
            content: regenerated.to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            regenerated,
            "a markdown file alef cannot stamp must still be regenerated once alef has a durable \
             ownership record for it -- a missing marker there is not evidence the file is foreign"
        );
        assert_eq!(report.changed_count(), 1);
    }

    /// `.R` is the conventional extension for an R script, and alef emits `install.R`,
    /// `run_tests.R` and every `packages/r/R/*.R` with `generated_header: true`. The emit predicate
    /// matched a lowercase `"r"` only, so `ensure_generated_header` returned all of them unstamped
    /// and the write guard then froze them for want of a marker nothing had ever been emitting. ~keep
    #[test]
    fn an_uppercase_r_script_extension_still_receives_the_generated_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("test_apps/r/install.R");
        let generated = GeneratedFile {
            path: target_relative.clone(),
            content: "install.packages(\"htmltomarkdown\")\n".to_owned(),
            generated_header: true,
        };

        write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let written = std::fs::read_to_string(base.join(&target_relative)).expect("read after");
        assert!(
            crate::core::hash::content_has_alef_marker(&written),
            "an uppercase-.R script must be stamped like its lowercase spelling: {written:?}"
        );
        assert!(
            written.starts_with('#'),
            "the marker must use R's own `#` line-comment syntax: {written:?}"
        );
    }

    /// A content-embedded marker proves ownership on an unmarkable extension even with
    /// NO ownership record at all -- e.g. a fresh clone with `docs/render.rs`-style
    /// pages, which self-mark with an HTML-comment header (`<!-- ... auto-generated by
    /// alef ... -->`) outside `ensure_generated_header`/`marker_comment_style`'s
    /// extension-keyed mechanism entirely. Gating the marker check on "is this a
    /// markable extension" would misread this exact case as foreign and refuse to
    /// regenerate a stale reference page forever on a cache-less checkout.
    #[test]
    fn unmarkable_extension_with_self_embedded_marker_regenerates_without_local_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("docs-site/api-c.md");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let stale = "<!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\
                     <!-- alef:hash:0000000000000000000000000000000000000000000000000000000000000000 -->\n\n\
                     ## C API Reference v1.16.0\n";
        std::fs::write(&target, stale).expect("seed stale, self-marked page");
        assert!(
            !crate::cli::cache::is_scaffold_owned_path(base, &target),
            "sanity: no local ownership record must exist for this path"
        );

        let regenerated = "<!-- This file is auto-generated by alef — DO NOT EDIT. -->\n\n## C API Reference v1.17.1\n";
        let generated = GeneratedFile {
            path: target_relative,
            content: regenerated.to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(&target).expect("read after");
        assert!(
            after.contains("v1.17.1") && !after.contains("v1.16.0"),
            "a self-marked unmarkable file must regenerate on content-proven ownership alone, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 1);
    }

    /// Counterpart to the above: an unstampable extension with NO durable ownership
    /// record must be refused, not clobbered. This is the `packages/java/pom.xml`
    /// incident -- `generated_header: true`, but `.xml` cannot carry a marker, so the
    /// old guard exempted it from any check at all and silently overwrote hand-written
    /// content the very first time alef saw that path.
    ///
    /// This is a *live* case, not one the content-marker-first check already covers:
    /// `src/scaffold/languages/java.rs`'s `pom.xml` generator embeds no
    /// "auto-generated by alef"/"Generated by alef" text anywhere in its output
    /// (unlike `docs::render::with_html_header`'s pages or README output, which
    /// self-mark and so never need this fallback), so `has_marker` is always
    /// `false` for it and `is_scaffold_owned_path` is the *only* thing standing
    /// between a pre-existing `pom.xml` and a silent overwrite. It is not dead
    /// code; removing it would reopen this exact incident. See
    /// `cache::scaffold_owned_path_key`'s doc for the cross-`base_dir`-spelling
    /// bug that made this fallback read as inert across real command sequences
    /// even though it was always this test's guard, correctly, all along.
    #[test]
    fn unstampable_generated_file_with_no_ownership_record_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/java/pom.xml");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let hand_written = "<project><!-- hand-written, never alef's --></project>\n";
        std::fs::write(&target, hand_written).expect("seed hand-written pom.xml");

        let generated = GeneratedFile {
            path: target_relative,
            content: "<project><!-- alef-generated --></project>\n".to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            hand_written,
            "an unstampable extension with no durable ownership record must be left untouched"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
    }

    /// The narrowest tempting shape for an automatic adoption route, pinned as refused:
    /// an unmarkable extension whose on-disk bytes are *already identical* to this run's
    /// output. A content-equivalence predicate would adopt this — it is byte-for-byte the
    /// case such a predicate is built for — and it must not, because those same bytes are
    /// equally consistent with a hand-written `pom.xml` that happens to match. Ownership
    /// is a fact about who authored the file, and content alone cannot recover it.
    ///
    /// The remedy is `alef adopt <path>`, which shows a human the diff first. If this test
    /// ever fails, an automatic adoption predicate has been reintroduced into the write
    /// path; the fix is to remove it, not to relax this assertion. ~keep
    #[test]
    fn converged_unmarked_file_is_still_refused_rather_than_silently_adopted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/java/pom.xml");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let identical = "<project><!-- identical to generated output --></project>\n";
        std::fs::write(&target, identical).expect("seed converged file");

        let generated = GeneratedFile {
            path: target_relative,
            content: identical.to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(&target).expect("read after");
        assert_eq!(
            after, identical,
            "content equivalence is not proof of authorship: a converged unmarked file must stay unstamped"
        );
        assert!(
            extract_hash(&after).is_none(),
            "no automatic route may stamp a file alef cannot prove it wrote, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 0);
    }

    /// The axis the test above leaves open, and the one that was actually broken: a
    /// converged unmarked file must not acquire an **ownership record** either.
    ///
    /// Refusing to stamp and refusing to claim are different acts, and the write path used
    /// to do the first while quietly doing the opposite of the second — its
    /// "unchanged, nothing to do" branch called `record_scaffold_owned_path` for every
    /// unmarkable path it found already converged. That is the rejected `bootstrap_owned`
    /// predicate exactly, relocated from a predicate into the record: byte-equality with
    /// generated output became a permanent licence to overwrite. Nothing observable
    /// changed in that run, which is why a test asserting only on the file's contents and
    /// `changed_count` passed throughout.
    ///
    /// The consequence only shows up on the *next* run, so that is what this asserts: the
    /// second pass carries different content, and it must still be refused. Since the
    /// record is now committed to git, a claim minted here would not merely be wrong on
    /// one machine — it would be distributed to every clone.
    ///
    /// The target must be unmarkable AND `generated_header: false`, and `package.json` is
    /// both. An earlier draft used `packages/java/pom.xml` with a header and passed against
    /// the unsound code: alef stamps XML, so generated output carried a marker the
    /// hand-written file lacked, the two never compared equal, and the run took the refusal
    /// path instead of the claim path. It asserted against a case that was already safe —
    /// which is the same defect it exists to catch, one level up. ~keep
    #[test]
    fn converged_unmarked_file_does_not_acquire_an_ownership_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/node/package.json");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let hand_written = "{\n  \"name\": \"hand-written-and-coincidentally-identical\"\n}\n";
        std::fs::write(&target, hand_written).expect("seed converged hand-written file");

        let converged = GeneratedFile {
            path: target_relative.clone(),
            content: hand_written.to_owned(),
            generated_header: false,
        };
        write_scaffold_files_report(&[converged], base, true).expect("write ok");

        assert!(
            !crate::cli::cache::is_scaffold_owned_path(base, &target),
            "a file that merely coincides with generated output must not be claimed as alef's"
        );
        assert!(
            !base.join(".alef-ownership.toml").exists(),
            "no ownership record may be created at all for a file nobody adopted"
        );

        let drifted = GeneratedFile {
            path: target_relative,
            content: "{\n  \"name\": \"alef-wants-something-else-now\"\n}\n".to_owned(),
            generated_header: false,
        };
        let second = write_scaffold_files_report(&[drifted], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after second run"),
            hand_written,
            "the first run's coincidence must not license the second run to clobber the file"
        );
        assert_eq!(
            second.refused_count(),
            1,
            "the second write must be refused, not silent"
        );
    }

    /// alef #80: ownership of an unmarkable file must be determinable from the repository
    /// alone, identically on a fresh clone and on the machine that generated it.
    ///
    /// The record used to live at `.alef/scaffold-owned-paths.manifest`, under the very
    /// directory alef writes into every consumer's `.gitignore`
    /// (`cli::pipeline::extract::gitignore::ensure_gitignore`). Ownership was therefore a
    /// property of one developer's disk: CI and a fresh clone refused regenerating files
    /// that the warm machine rewrote without complaint, and no amount of committing could
    /// close the gap because the evidence was ignored by construction.
    ///
    /// Deleting the whole `.alef/` cache between the two runs is the load-bearing step —
    /// it is what a fresh clone *is*. Without it the assertion would pass on the old code
    /// too, and prove nothing. ~keep
    #[test]
    fn ownership_of_an_unmarkable_file_survives_a_cache_less_fresh_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/typescript/package.json");
        let target = base.join(&target_relative);

        let created = GeneratedFile {
            path: target_relative.clone(),
            content: "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\"\n}\n".to_owned(),
            generated_header: true,
        };
        let first = write_scaffold_files_report(&[created], base, true).expect("write ok");
        assert_eq!(first.changed_count(), 1, "sanity: alef must have created the file");

        let record = base.join(".alef-ownership.toml");
        assert!(
            record.exists(),
            "creating an unmarkable file must leave a committable record"
        );
        assert!(
            !base.join(".alef").join("scaffold-owned-paths.manifest").exists(),
            "the gitignored record must no longer be the place ownership is kept"
        );

        std::fs::remove_dir_all(base.join(".alef")).ok();
        assert!(!base.join(".alef").exists(), "sanity: the machine-local cache is gone");

        assert!(
            crate::cli::cache::is_scaffold_owned_path(base, &target),
            "a checkout carrying only committed files must still know alef owns this path"
        );

        let regenerated = GeneratedFile {
            path: target_relative,
            content: "{\n  \"name\": \"demo\",\n  \"version\": \"2.0.0\"\n}\n".to_owned(),
            generated_header: true,
        };
        let second = write_scaffold_files_report(&[regenerated], base, true).expect("write ok");

        assert_eq!(
            second.refused_count(),
            0,
            "a cache-less clone must not refuse alef's own file"
        );
        assert!(
            std::fs::read_to_string(&target)
                .expect("read after regen")
                .contains("2.0.0"),
            "the file must regenerate on the committed record alone"
        );
    }

    /// The refusal has to be *reported*, not merely performed. `refused_paths` is what
    /// `report_refused_writes` turns into an actionable "run `alef adopt <path>`" line;
    /// a guard that silently declines is the frozen-file failure mode that made a
    /// permanently-refused `Cargo.toml` invisible in a real consumer tree. ~keep
    #[test]
    fn a_refused_write_is_recorded_in_the_report_so_adopt_can_be_pointed_at_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("crates/sample-ffi/Cargo.toml");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        std::fs::write(&target, "[package]\nname = \"hand-written\"\n").expect("seed");

        let generated = GeneratedFile {
            path: target_relative,
            content: "[package]\nname = \"generated\"\n".to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        assert_eq!(
            report.refused_paths.iter().collect::<Vec<_>>(),
            vec![&target],
            "the refused path must be surfaced, not just skipped"
        );
        assert_eq!(report.refused_count(), 1);
    }

    /// Happy-path counterpart: a file alef legitimately authored on a prior run
    /// (and therefore already carries the marker on disk) is still updated
    /// normally when its content changes. This proves the ownership guard does
    /// not degrade into a blanket "never touch an existing file" rule.
    #[test]
    fn file_alef_already_owns_is_updated_normally_on_the_next_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("e2e/go/main_test.go");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let prior_run_content = marked_content("package e2e_test\n\nfunc TestMain(m *testing.M) { m.Run() }\n");
        std::fs::write(&target, &prior_run_content).expect("seed prior alef output");
        assert!(
            content_has_alef_marker(&prior_run_content),
            "sanity: seed must carry the marker"
        );

        let generated = GeneratedFile {
            path: target_relative,
            content: "package e2e_test\n\nfunc TestMain(m *testing.M) { os.Exit(m.Run()) }\n".to_owned(),
            generated_header: true,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(&target).expect("read after");
        assert!(
            after.contains("os.Exit(m.Run())"),
            "a file alef already owns must be updated with this run's content, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 1);
    }

    /// A brand-new path (nothing on disk yet) is written and stamped normally —
    /// the guard only ever engages when there is pre-existing content to protect.
    #[test]
    fn brand_new_managed_file_is_written_and_stamped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        let generated = GeneratedFile {
            path: PathBuf::from("e2e/go/robots_test.go"),
            content: "package e2e_test\n\nfunc TestRobots(t *testing.T) {}\n".to_owned(),
            generated_header: true,
        };

        write_scaffold_files_report(&[generated], base, true).expect("write ok");

        let after = std::fs::read_to_string(base.join("e2e/go/robots_test.go")).expect("read after");
        assert!(
            after.starts_with("// This file is auto-generated by alef"),
            "a genuinely new path must be stamped normally, got:\n{after}"
        );
    }

    /// The specific shape that broke in the incident: a generator target
    /// directory containing a mix of alef-authored and hand-written files. Only
    /// the alef-owned file may be touched; the hand-written sibling in the same
    /// directory must survive completely untouched.
    #[test]
    fn mixed_directory_of_owned_and_hand_written_files_touches_only_the_owned_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let e2e_go_dir = base.join("e2e/go");
        std::fs::create_dir_all(&e2e_go_dir).expect("mkdir");

        let hand_written_path = e2e_go_dir.join("helpers_test.go");
        let hand_written =
            "package e2e_test\n\n// hand rolled, never alef's\nfunc jsonString(v any) string { return \"\" }\n";
        std::fs::write(&hand_written_path, hand_written).expect("seed hand-written sibling");

        let owned_path = e2e_go_dir.join("main_test.go");
        let owned_prior = marked_content("package e2e_test\n\nfunc TestMain(m *testing.M) { m.Run() }\n");
        std::fs::write(&owned_path, &owned_prior).expect("seed owned sibling");

        let files = vec![
            GeneratedFile {
                path: PathBuf::from("e2e/go/helpers_test.go"),
                content: hand_written.to_owned(),
                generated_header: true,
            },
            GeneratedFile {
                path: PathBuf::from("e2e/go/main_test.go"),
                content: "package e2e_test\n\nfunc TestMain(m *testing.M) { os.Exit(m.Run()) }\n".to_owned(),
                generated_header: true,
            },
        ];

        let report = write_scaffold_files_report(&files, base, true).expect("write ok");

        let hand_written_after = std::fs::read_to_string(&hand_written_path).expect("read hand-written after");
        assert_eq!(
            hand_written_after, hand_written,
            "the hand-written sibling in the same directory must survive untouched"
        );
        assert!(extract_hash(&hand_written_after).is_none(), "must not gain a marker");

        let owned_after = std::fs::read_to_string(&owned_path).expect("read owned after");
        assert!(
            owned_after.contains("os.Exit(m.Run())"),
            "the alef-owned sibling must still update, got:\n{owned_after}"
        );
        assert_eq!(
            report.changed_count(),
            1,
            "only the genuinely-owned sibling counts as changed"
        );
    }

    /// The zig/dart/swift incident: a `generated_header: false` scaffold seed on a
    /// *markable* extension (`.dart`, `.swift`, `.zig`, ...) must never be silently
    /// replaced once it exists on disk, even under `overwrite: true` -- not on first
    /// hand-edit, and not on every subsequent run thereafter. Seeds never carry an
    /// `alef:hash:` marker by design (see `write_scaffold_files_report`'s doc), so
    /// "no marker" is the seed's permanent, intentional state, not a transient gap
    /// that a durable ownership record could paper over the way it does for
    /// unmarkable extensions.
    #[test]
    fn markable_seed_with_local_modifications_is_not_replaced_under_overwrite_true() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/dart/test/my_lib_test.dart");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let hand_written_test = "import 'package:test/test.dart';\n\nvoid main() {\n  test('really checks something', () {\n    expect(computeAnswer(), equals(42));\n  });\n}\n";
        std::fs::write(&target, hand_written_test).expect("seed hand-edited test file");

        let placeholder_seed = GeneratedFile {
            path: target_relative,
            content: "import 'package:test/test.dart';\n\nvoid main() {\n  test('placeholder', () {\n    expect(1 + 1, equals(2));\n  });\n}\n".to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[placeholder_seed], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            hand_written_test,
            "a generated_header: false seed's local modifications must survive overwrite: true"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
    }

    /// Counterpart to the dart preserve case above: when the target does not exist yet,
    /// the seed must still be created normally -- the guard only ever protects a file
    /// that is already there.
    #[test]
    fn dart_test_seed_is_created_when_the_target_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/dart/test/my_lib_test.dart");
        let seed_content = "import 'package:test/test.dart';\n\nvoid main() {\n  test('placeholder', () {\n    expect(1 + 1, equals(2));\n  });\n}\n";

        let placeholder_seed = GeneratedFile {
            path: target_relative.clone(),
            content: seed_content.to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[placeholder_seed], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(base.join(&target_relative)).expect("read after"),
            seed_content,
            "an absent target must be created with exactly the seed's content"
        );
        assert_eq!(report.changed_count(), 1, "creating a new seed counts as a change");
        assert!(
            report.refused_paths.is_empty(),
            "creating a brand-new path must never be refused, got: {:?}",
            report.refused_paths
        );
    }

    /// The zig half of the same incident, pinned separately from the dart case above:
    /// `packages/zig/test/{module}_test.zig` is the exact path `scaffold_zig` seeds and
    /// `build.zig`'s `test_module` compiles (see `scaffold_zig_test`'s doc), so a real,
    /// hand-written suite living there is exactly what `alef version`'s
    /// `regenerate_scaffold_after_sync` calls `write_scaffold_files_with_overwrite(..,
    /// overwrite: true)` against. This reproduces a real consumer suite shape (multiple
    /// `test` blocks asserting real values against the generated bindings, not the
    /// single-assertion placeholder `scaffold_zig_test` would emit) to prove the guard
    /// does not merely tolerate a trivially different placeholder but leaves a
    /// genuinely different, larger hand-written file untouched byte-for-byte.
    #[test]
    fn zig_test_seed_with_a_real_hand_written_suite_is_not_replaced_under_overwrite_true() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/zig/test/xberg_test.zig");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let hand_written_suite = "const std = @import(\"std\");\nconst testing = std.testing;\nconst xberg = @import(\"xberg\");\n\ntest \"xberg.version returns a non-empty string\" {\n    const version = xberg.version();\n    try testing.expect(version.len > 0);\n}\n\ntest \"xberg.add sums two integers\" {\n    try testing.expectEqual(@as(i64, 5), xberg.add(2, 3));\n}\n";
        std::fs::write(&target, hand_written_suite).expect("seed hand-written suite");

        let placeholder_seed = GeneratedFile {
            path: target_relative.clone(),
            content: "const xberg = @import(\"xberg\");\n\ntest \"xberg.version symbol resolves\" {\n    _ = &xberg.version;\n}\n".to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[placeholder_seed], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            hand_written_suite,
            "a real, hand-written zig test suite must survive overwrite: true byte-for-byte"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
        assert!(
            report.refused_paths.contains(&target),
            "the refusal (and its tracing::warn!) must name this exact path, got: {:?}",
            report.refused_paths
        );
    }

    /// Counterpart: when the target does not exist yet, the seed must still be created
    /// normally — the guard only ever protects a file that is already there.
    #[test]
    fn zig_test_seed_is_created_when_the_target_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/zig/test/xberg_test.zig");
        let seed_content = "const xberg = @import(\"xberg\");\n\ntest \"xberg.version symbol resolves\" {\n    _ = &xberg.version;\n}\n";

        let placeholder_seed = GeneratedFile {
            path: target_relative.clone(),
            content: seed_content.to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[placeholder_seed], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(base.join(&target_relative)).expect("read after"),
            seed_content,
            "an absent target must be created with exactly the seed's content"
        );
        assert_eq!(report.changed_count(), 1, "creating a new seed counts as a change");
        assert!(
            report.refused_paths.is_empty(),
            "creating a brand-new path must never be refused, got: {:?}",
            report.refused_paths
        );
    }

    /// The swift third of the same incident: `packages/swift/Tests/{module}Tests/{module}Tests.swift`
    /// is the exact path `scaffold_swift` seeds with `generated_header: false` (see
    /// `scaffold_swift`'s doc). A real, hand-written XCTest suite (multiple real
    /// assertions, not the single tautological `XCTAssertTrue(true)` placeholder
    /// `migrate_swift_placeholder_test` is scoped to) must survive `alef version`'s
    /// `overwrite: true` write byte-for-byte.
    #[test]
    fn swift_test_seed_with_a_real_hand_written_suite_is_not_replaced_under_overwrite_true() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/swift/Tests/RustLibTests/RustLibTests.swift");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");

        let hand_written_suite = "import XCTest\n@testable import RustLib\n\nfinal class RustLibTests: XCTestCase {\n    func testVersionIsNonEmpty() throws {\n        XCTAssertFalse(RustLib.version().isEmpty)\n    }\n\n    func testAddSumsTwoIntegers() throws {\n        XCTAssertEqual(RustLib.add(2, 3), 5)\n    }\n}\n";
        std::fs::write(&target, hand_written_suite).expect("seed hand-written suite");

        let placeholder_seed = GeneratedFile {
            path: target_relative.clone(),
            content: "import XCTest\n@testable import RustLib\n\nfinal class RustLibTests: XCTestCase {\n    func testModuleLoads() throws {\n        XCTAssertTrue(true)\n    }\n}\n".to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[placeholder_seed], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            hand_written_suite,
            "a real, hand-written swift test suite must survive overwrite: true byte-for-byte"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
        assert!(
            report.refused_paths.contains(&target),
            "the refusal (and its tracing::warn!) must name this exact path, got: {:?}",
            report.refused_paths
        );
    }

    /// Counterpart: when the target does not exist yet, the swift seed must still be
    /// created normally.
    #[test]
    fn swift_test_seed_is_created_when_the_target_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/swift/Tests/RustLibTests/RustLibTests.swift");
        let seed_content = "import XCTest\n@testable import RustLib\n\nfinal class RustLibTests: XCTestCase {\n    func testModuleLoads() throws {\n        XCTAssertTrue(true)\n    }\n}\n";

        let placeholder_seed = GeneratedFile {
            path: target_relative.clone(),
            content: seed_content.to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[placeholder_seed], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(base.join(&target_relative)).expect("read after"),
            seed_content,
            "an absent target must be created with exactly the seed's content"
        );
        assert_eq!(report.changed_count(), 1, "creating a new seed counts as a change");
        assert!(
            report.refused_paths.is_empty(),
            "creating a brand-new path must never be refused, got: {:?}",
            report.refused_paths
        );
    }

    /// The snippet-coverage ledger's exact failing sequence (alef bug report): a
    /// pre-existing, unrecorded copy of `.alef-snippet-coverage.json` — the shape every
    /// consumer tree is actually in, since the ledger predates write-time registration or
    /// its only prior writes happened to leave content unchanged, which records nothing by
    /// design (see `converged_unmarked_file_does_not_acquire_an_ownership_record` for that
    /// half) — must still accept a write with different content, not be refused forever like
    /// an ordinary unmarkable manifest at an unrecorded path would be.
    #[test]
    fn snippet_coverage_ledger_write_succeeds_without_a_prior_ownership_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("docs/snippets/.alef-snippet-coverage.json");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let stale = "{\n  \"expected\": [],\n  \"generated\": []\n}\n";
        std::fs::write(&target, stale).expect("seed stale, unrecorded ledger");
        assert!(
            !crate::cli::cache::is_scaffold_owned_path(base, &target),
            "sanity: no ownership record exists for this path yet"
        );

        let fresh = "{\n  \"expected\": [\"a\"],\n  \"generated\": [\"a\"]\n}\n";
        let generated = GeneratedFile {
            path: target_relative,
            content: fresh.to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            fresh,
            "the ledger must be rewritten with this run's freshly computed content"
        );
        assert_eq!(report.refused_count(), 0, "the ledger write must not be refused");
        assert_eq!(report.changed_count(), 1);
        assert!(
            crate::cli::cache::is_scaffold_owned_path(base, &target),
            "the write must durably register ownership so a future run needs no bootstrap help"
        );
    }

    /// Counterpart to the ledger test above, pinning the guard's general behaviour is
    /// unchanged: a genuinely hand-written file at a path alef has no record of ever
    /// owning is still refused, even when its extension is unmarkable exactly like the
    /// ledger's. The distinguishing signal is the ledger's own reserved name — see
    /// `e2e::snippets::is_snippet_coverage_manifest_path` — never "any unmarkable JSON at an
    /// unrecorded path", which would reopen the `composer.json`/`package.json` incident this
    /// guard exists to prevent.
    #[test]
    fn unrelated_unmarkable_json_at_unrecorded_path_is_still_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let target_relative = PathBuf::from("packages/php/composer.json");
        let target = base.join(&target_relative);
        std::fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        let hand_written = "{\n  \"name\": \"hand-written/never-alefs\"\n}\n";
        std::fs::write(&target, hand_written).expect("seed hand-written manifest");

        let generated = GeneratedFile {
            path: target_relative,
            content: "{\n  \"name\": \"alef-generated\"\n}\n".to_owned(),
            generated_header: false,
        };

        let report = write_scaffold_files_report(&[generated], base, true).expect("write ok");

        assert_eq!(
            std::fs::read_to_string(&target).expect("read after"),
            hand_written,
            "a hand-written file at a path alef does not own must survive untouched"
        );
        assert_eq!(
            report.refused_count(),
            1,
            "the write must be refused, not silently adopted"
        );
    }
}

#[cfg(test)]
mod generated_header_tests {
    use super::*;
    use crate::core::backend::GeneratedFile;

    #[test]
    fn write_files_adds_missing_headers_to_managed_java_and_csharp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            (
                crate::core::config::Language::Java,
                vec![GeneratedFile {
                    path: "packages/java/Example.java".into(),
                    content: "package example;\n\npublic final class Example {}\n".into(),
                    generated_header: true,
                }],
            ),
            (
                crate::core::config::Language::Csharp,
                vec![GeneratedFile {
                    path: "packages/csharp/Example.cs".into(),
                    content: "namespace Example;\n\npublic sealed class Example {}\n".into(),
                    generated_header: true,
                }],
            ),
        ];

        write_files(&files, dir.path()).expect("write managed files");

        for path in ["packages/java/Example.java", "packages/csharp/Example.cs"] {
            let output = std::fs::read_to_string(dir.path().join(path)).expect("read managed file");
            assert!(
                output.starts_with("// This file is auto-generated by alef"),
                "{path}: {output}"
            );
        }
    }

    #[test]
    fn write_files_places_rust_header_before_multiline_inner_attribute() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Ffi,
            vec![GeneratedFile {
                path: "src/service.rs".into(),
                content: "#![allow(\n    unused_variables,\n)]\n\nfn generated() {}\n".into(),
                generated_header: true,
            }],
        )];

        write_files(&files, dir.path()).expect("write Rust file");

        let output = std::fs::read_to_string(dir.path().join("src/service.rs")).expect("read Rust file");
        assert!(output.starts_with("// This file is auto-generated by alef"));
        let header_end = output.find("#![allow(").expect("inner attribute");
        assert!(output[..header_end].contains("To verify freshness: alef verify"));
        let allowed_lint = output.find("unused_variables").expect("allowed lint");
        assert!(header_end < allowed_lint);
        assert!(!output[header_end..allowed_lint].contains("auto-generated by alef"));
    }

    #[test]
    fn write_files_places_php_header_after_opening_tag() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Php,
            vec![GeneratedFile {
                path: "src/Service.php".into(),
                content: "<?php\ndeclare(strict_types=1);\n\nfinal class Service {}\n".into(),
                generated_header: true,
            }],
        )];

        write_files(&files, dir.path()).expect("write PHP file");

        let output = std::fs::read_to_string(dir.path().join("src/Service.php")).expect("read PHP file");
        assert!(output.starts_with("<?php\n// This file is auto-generated by alef"));
        assert!(output.contains("To verify freshness: alef verify\n\ndeclare(strict_types=1);"));
    }

    #[test]
    fn write_files_preserves_explicit_header_and_unmanaged_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let explicit = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
        let files = vec![(
            crate::core::config::Language::Java,
            vec![
                GeneratedFile {
                    path: "Managed.java".into(),
                    content: format!("{explicit}\npublic final class Managed {{}}\n"),
                    generated_header: true,
                },
                GeneratedFile {
                    path: "Unmanaged.java".into(),
                    content: "public final class Unmanaged {}\n".into(),
                    generated_header: false,
                },
            ],
        )];

        write_files(&files, dir.path()).expect("write files");

        let managed = std::fs::read_to_string(dir.path().join("Managed.java")).expect("read managed file");
        assert_eq!(managed.matches("auto-generated by alef").count(), 1);
        let unmanaged = std::fs::read_to_string(dir.path().join("Unmanaged.java")).expect("read unmanaged file");
        assert!(!unmanaged.contains("auto-generated by alef"));
    }

    #[test]
    fn unchanged_write_preserves_mtime_and_reports_no_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "Example.java".into(),
                content: "public final class Example {}\n".into(),
                generated_header: true,
            }],
        )];
        let first = write_files_report(&files, dir.path()).expect("initial write");
        let path = dir.path().join("Example.java");
        let initial_mtime = std::fs::metadata(&path).expect("metadata").modified().expect("mtime");

        let second = write_files_report(&files, dir.path()).expect("repeat write");
        let repeated_mtime = std::fs::metadata(&path).expect("metadata").modified().expect("mtime");

        assert_eq!(first.changed_count(), 1);
        assert_eq!(second.changed_count(), 0);
        assert_eq!(first.expected_count(), 1);
        assert_eq!(second.expected_count(), 1);
        assert_eq!(initial_mtime, repeated_mtime);
    }

    #[test]
    fn duplicate_outputs_are_deduplicated_or_rejected_before_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = GeneratedFile {
            path: "Shared.java".into(),
            content: "final class Shared {}\n".into(),
            generated_header: true,
        };
        let identical = vec![
            (crate::core::config::Language::Java, vec![file.clone()]),
            (crate::core::config::Language::Kotlin, vec![file.clone()]),
        ];
        assert_eq!(
            write_files_report(&identical, dir.path())
                .expect("deduplicated")
                .changed_count(),
            1
        );

        let mut conflicting = file;
        conflicting.content = "final class Different {}\n".into();
        let conflict = vec![
            (crate::core::config::Language::Java, identical[0].1.clone()),
            (crate::core::config::Language::Kotlin, vec![conflicting]),
        ];
        let error = write_files_report(&conflict, dir.path()).expect_err("conflict must fail");
        assert!(
            error
                .to_string()
                .contains("multiple generators emitted different content")
        );
    }

    #[test]
    fn duplicate_conflict_does_not_create_output_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            (
                crate::core::config::Language::Java,
                vec![GeneratedFile {
                    path: "new/Shared.java".into(),
                    content: "first\n".into(),
                    generated_header: false,
                }],
            ),
            (
                crate::core::config::Language::Kotlin,
                vec![GeneratedFile {
                    path: "new/Shared.java".into(),
                    content: "second\n".into(),
                    generated_header: false,
                }],
            ),
        ];

        write_files_report(&files, dir.path()).expect_err("conflict must fail");

        assert!(!dir.path().join("new").exists());
    }

    /// `tool.txt` is an unmarkable extension (no comment syntax `marker_comment_style`
    /// knows), so `write_files_report`'s ownership guard requires a committed
    /// `.alef-ownership.toml` record before it may replace pre-existing
    /// content there. The initial write below establishes that record the same way a
    /// real first run would, so the second write below exercises the atomic-replace
    /// path this test actually targets rather than being refused by the guard.
    #[cfg(unix)]
    #[test]
    fn changed_atomic_replacement_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tool.txt");
        let seed_files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "tool.txt".into(),
                content: "old\n".into(),
                generated_header: false,
            }],
        )];
        write_files_report(&seed_files, dir.path()).expect("initial write establishes ownership");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o744)).expect("seed mode");

        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "tool.txt".into(),
                content: "new\n".into(),
                generated_header: false,
            }],
        )];

        write_files_report(&files, dir.path()).expect("replacement");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read after replacement"),
            "new\n",
            "content must actually be replaced"
        );
        assert_eq!(
            std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o744
        );
    }

    #[test]
    fn managed_output_paths_exclude_handwritten_emissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            GeneratedFile {
                path: "managed.rs".into(),
                content: "fn managed() {}\n".into(),
                generated_header: true,
            },
            GeneratedFile {
                path: "handwritten.rs".into(),
                content: "fn handwritten() {}\n".into(),
                generated_header: false,
            },
        ];

        let managed = managed_output_paths(&files, dir.path());
        let formatter_inputs = managed_generated_files(&files);

        assert_eq!(managed, [dir.path().join("managed.rs")].into_iter().collect());
        assert!(!managed.contains(&dir.path().join("handwritten.rs")));
        assert_eq!(formatter_inputs.len(), 1);
        assert_eq!(formatter_inputs[0].path, std::path::Path::new("managed.rs"));
    }

    #[test]
    fn managed_manifest_is_hashed_while_handwritten_manifest_stays_unowned() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![
            GeneratedFile {
                path: "managed/Cargo.toml".into(),
                content: "[package]\nname = \"managed\"\n".into(),
                generated_header: true,
            },
            GeneratedFile {
                path: "handwritten/Cargo.toml".into(),
                content: "[package]\nname = \"handwritten\"\n".into(),
                generated_header: false,
            },
        ];
        write_scaffold_files_with_overwrite(&files, dir.path(), true).expect("write manifests");
        let managed_paths = managed_output_paths(&files, dir.path());
        finalize_hashes(&managed_paths, "sources", b"config").expect("finalize managed manifest");

        let managed = std::fs::read_to_string(dir.path().join("managed/Cargo.toml")).expect("managed manifest");
        let handwritten =
            std::fs::read_to_string(dir.path().join("handwritten/Cargo.toml")).expect("handwritten manifest");
        assert!(crate::core::hash::extract_hash(&managed).is_some());
        assert!(crate::core::hash::extract_hash(&handwritten).is_none());
        assert!(!handwritten.contains("Generated by alef"));
    }

    /// `write_files_report`'s narrow ownership guard: a `generated_header: true`
    /// binding file on a markable extension must not silently claim a path that
    /// already holds hand-written content the very first time alef ever writes
    /// there -- the same day-one collision class `write_scaffold_files_report`
    /// guards against, scoped down to the subset of this writer's output that can
    /// actually prove authorship (see the guard's doc comment for why the rest of
    /// this writer, which is always-regenerated by design, is deliberately left
    /// unguarded).
    #[test]
    fn write_files_report_refuses_pre_existing_unmarked_file_on_markable_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hand_written = "package example;\n\npublic final class Example {\n    // hand-rolled\n}\n";
        std::fs::write(dir.path().join("Example.java"), hand_written).expect("seed hand-written file");

        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "Example.java".into(),
                content: "package example;\n\npublic final class Example {}\n".into(),
                generated_header: true,
            }],
        )];

        let report = write_files_report(&files, dir.path()).expect("write ok");

        let after = std::fs::read_to_string(dir.path().join("Example.java")).expect("read after");
        assert_eq!(
            after, hand_written,
            "a path with no alef marker must survive a generated_header: true binding write untouched"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
        assert_eq!(
            report.refused_paths,
            std::collections::BTreeSet::from([dir.path().join("Example.java")]),
            "a refused write must be recorded so it can be surfaced -- the guard returns before the \
             path reaches expected_paths, so otherwise the refusal is invisible to every caller"
        );
    }

    /// The other half of the predicate. Without this, the assertion above would still pass if
    /// every write were recorded as refused -- which would make the report useless in the
    /// direction that matters, reporting a freeze that is not happening. ~keep
    #[test]
    fn write_files_report_records_no_refusal_when_the_write_is_authorised() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "Fresh.java".into(),
                content: "package example;\n\npublic final class Fresh {}\n".into(),
                generated_header: true,
            }],
        )];

        let report = write_files_report(&files, dir.path()).expect("write ok");

        assert!(
            report.refused_paths.is_empty(),
            "writing a path that does not yet exist is never a refusal, got {:?}",
            report.refused_paths
        );
        assert_eq!(
            report.changed_count(),
            1,
            "the authorised write must still count as a change"
        );
    }

    /// Counterpart: once a binding file already carries the marker (alef legitimately
    /// wrote it on a prior run), it must keep regenerating on every subsequent run --
    /// the guard added above must not degrade binding regeneration into create-once.
    #[test]
    fn write_files_report_still_regenerates_a_file_alef_already_owns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marked = crate::core::hash::inject_hash_line(
            &format!(
                "{}package example;\n\npublic final class Example {{}}\n",
                crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash)
            ),
            &"0".repeat(64),
        );
        std::fs::write(dir.path().join("Example.java"), &marked).expect("seed prior alef output");

        let files = vec![(
            crate::core::config::Language::Java,
            vec![GeneratedFile {
                path: "Example.java".into(),
                content: "package example;\n\npublic final class Example {\n    int added;\n}\n".into(),
                generated_header: true,
            }],
        )];

        let report = write_files_report(&files, dir.path()).expect("write ok");

        let after = std::fs::read_to_string(dir.path().join("Example.java")).expect("read after");
        assert!(
            after.contains("int added;"),
            "a file alef already owns must keep regenerating, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 1);
    }

    /// `write_files_report`'s unmarkable-extension route: a `.cmake` config file (no
    /// comment syntax `marker_comment_style` recognizes) with no local ownership
    /// record must be refused, not clobbered -- the same class of gap that let
    /// `packages/java/pom.xml` be silently reclaimed in `write_scaffold_files_report`,
    /// found in this writer's own output via cross-repo review of
    /// `crates/*-ffi/cmake/*-config.cmake`.
    #[test]
    fn write_files_report_refuses_pre_existing_unmarkable_file_with_no_ownership_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hand_written = "# hand-written, never alef's\nset(FOO_INCLUDE_DIRS \"/usr/local/include\")\n";
        std::fs::write(dir.path().join("foo-config.cmake"), hand_written).expect("seed hand-written file");

        let files = vec![(
            crate::core::config::Language::Ffi,
            vec![GeneratedFile {
                path: "foo-config.cmake".into(),
                content: "set(FOO_INCLUDE_DIRS \"${CMAKE_CURRENT_LIST_DIR}/include\")\n".into(),
                generated_header: true,
            }],
        )];

        let report = write_files_report(&files, dir.path()).expect("write ok");

        let after = std::fs::read_to_string(dir.path().join("foo-config.cmake")).expect("read after");
        assert_eq!(
            after, hand_written,
            "an unmarkable extension with no durable ownership record must survive untouched"
        );
        assert_eq!(report.changed_count(), 0, "a refused write must not count as a change");
    }

    /// Counterpart: once alef has a committed ownership record for an unmarkable path
    /// (populated by this writer's own first successful write), it must keep
    /// regenerating on every subsequent run -- the unmarkable route must not degrade
    /// into create-once either.
    #[test]
    fn write_files_report_regenerates_unmarkable_file_once_it_has_a_local_ownership_record() {
        let dir = tempfile::tempdir().expect("tempdir");
        let initial = vec![(
            crate::core::config::Language::Ffi,
            vec![GeneratedFile {
                path: "foo-config.cmake".into(),
                content: "set(FOO_INCLUDE_DIRS \"${CMAKE_CURRENT_LIST_DIR}/include\")\n".into(),
                generated_header: true,
            }],
        )];
        write_files_report(&initial, dir.path()).expect("initial write establishes ownership");

        let updated = vec![(
            crate::core::config::Language::Ffi,
            vec![GeneratedFile {
                path: "foo-config.cmake".into(),
                content: "set(FOO_INCLUDE_DIRS \"${CMAKE_CURRENT_LIST_DIR}/include2\")\n".into(),
                generated_header: true,
            }],
        )];
        let report = write_files_report(&updated, dir.path()).expect("write ok");

        let after = std::fs::read_to_string(dir.path().join("foo-config.cmake")).expect("read after");
        assert!(
            after.contains("include2"),
            "an unmarkable file alef already owns must keep regenerating, got:\n{after}"
        );
        assert_eq!(report.changed_count(), 1);
    }
}
