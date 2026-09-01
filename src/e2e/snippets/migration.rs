use crate::core::backend::GeneratedFile;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Identical,
    Different,
    NoGeneratedEquivalent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationEntry {
    pub path: PathBuf,
    pub status: MigrationStatus,
    /// True when `path` matches a `[crates.e2e.snippets].curated_snippets` glob -- always
    /// `false` from [`compare_root`] / [`compare_existing`], which know nothing about
    /// curated declarations. [`compare_root_curated`] / [`compare_existing_curated`] are the
    /// curated-aware siblings that populate it.
    ///
    /// A `NoGeneratedEquivalent` entry with `curated: true` is a file the project declared,
    /// on purpose, alef will never generate -- distinct from `curated: false`, a genuine,
    /// unaccounted migration gap. Before this field existed, the two were indistinguishable
    /// in a migration comparison: a project with hundreds of intentionally hand-authored
    /// snippets saw every one of them reported identically to a real gap.
    #[serde(default)]
    pub curated: bool,
}

pub fn compare_root(
    existing_root: &Path,
    generated_root: &Path,
    generated: &[GeneratedFile],
) -> Result<Vec<MigrationEntry>> {
    compare_root_curated(&CuratedComparison {
        project_root: existing_root,
        existing_root,
        generated_root,
        generated,
        curated_globs: &[],
    })
}

/// One curated-aware `alef e2e snippets-migrate` comparison.
///
/// `curated_globs` are `[crates.e2e.snippets].curated_snippets` patterns, relative to
/// `project_root` -- the directory holding `alef.toml`, the base `output` is written in and
/// the base [`crate::e2e::snippets::coverage::resolve_curated_snippet_paths`] resolves
/// against. Entry paths are `existing_root`-relative, so each is rebased onto
/// `project_root` before matching; without that the globs would be interpreted in a third
/// key space and a declaration written for the coverage ledger would silently match nothing
/// here. ~keep
pub struct CuratedComparison<'a> {
    pub project_root: &'a Path,
    pub existing_root: &'a Path,
    pub generated_root: &'a Path,
    pub generated: &'a [GeneratedFile],
    pub curated_globs: &'a [String],
}

/// [`compare_root`]'s curated-aware sibling: matches [`CuratedComparison::curated_globs`]
/// against every `NoGeneratedEquivalent` path to populate [`MigrationEntry::curated`].
///
/// A pattern is validated the same way [`crate::e2e::snippets::coverage::resolve_curated_snippet_paths`]
/// validates it for the coverage ledger -- invalid glob syntax fails the comparison rather
/// than silently matching nothing -- but does NOT repeat that function's "must match at
/// least one file" anti-vacuity check: a migration comparison walks `existing_root` itself,
/// so a pattern matching nothing here already shows up as a visibly empty count in the
/// caller's own report, unlike the coverage ledger, which has no equivalent per-pattern
/// visibility.
///
/// # Errors
///
/// Returns an error when `existing_root` cannot be walked, a generated path lies outside
/// `generated_root`, or a curated glob does not parse.
pub fn compare_root_curated(comparison: &CuratedComparison<'_>) -> Result<Vec<MigrationEntry>> {
    let &CuratedComparison {
        project_root,
        existing_root,
        generated_root,
        generated,
        curated_globs,
    } = comparison;
    let existing = read_existing(existing_root)?;
    let generated_prefix = nested_prefix(existing_root, generated_root);
    let curated_base = curated_base(project_root, existing_root, curated_globs)?;
    let relative_generated = generated
        .iter()
        .map(|file| {
            let path = file.path.strip_prefix(generated_root).with_context(|| {
                format!(
                    "generated snippet {} is outside configured output {}",
                    file.path.display(),
                    generated_root.display()
                )
            })?;
            let path = match &generated_prefix {
                Some(prefix) => prefix.join(path),
                None => path.to_path_buf(),
            };
            Ok(GeneratedFile {
                path,
                content: file.content.clone(),
                generated_header: file.generated_header,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    compare_existing_curated(
        existing
            .iter()
            .map(|(path, content)| (path.as_path(), content.as_str())),
        &relative_generated,
        curated_globs,
        &curated_base,
    )
}

/// The prefix an `existing_root`-relative entry path needs to become a project-root-relative
/// key a curated glob can be matched against.
///
/// Refusing an unrelatable pair is the anti-vacuity half: if `existing_root` cannot be placed
/// under `project_root` -- an absolute argument pointing outside the project, say -- then every
/// project-root-relative glob would match nothing, and a comparison that reported zero curated
/// files would be indistinguishable from one where the declaration genuinely covered nothing.
/// Silently defaulting to an empty base would reinterpret the globs in a different key space.
/// ~keep
fn curated_base(project_root: &Path, existing_root: &Path, curated_globs: &[String]) -> Result<PathBuf> {
    let project_root = if project_root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        project_root
    };
    if let Some(prefix) = nested_prefix(project_root, existing_root) {
        return Ok(prefix);
    }
    let same_root = match (std::path::absolute(project_root), std::path::absolute(existing_root)) {
        (Ok(project), Ok(existing)) => project == existing,
        _ => false,
    };
    if same_root || curated_globs.is_empty() {
        return Ok(PathBuf::new());
    }
    bail!(
        "curated snippet globs are relative to the project root `{}`, but the migrated root `{}` does not \
         lie beneath it; pass a migrated root inside the project",
        project_root.display(),
        existing_root.display()
    )
}

/// Where `inner` sits relative to `outer`, when one configured tree lives INSIDE another.
///
/// Both sides of the comparison have to be keyed off one base. For parallel trees -- a
/// handwritten `docs/handwritten` against `output = "docs/generated"` -- each side keys off its
/// own root and this is `None`, which is the original behaviour. When `output` is a subdirectory
/// of `existing_root` (`alef e2e snippets-migrate docs/snippets` against
/// `output = "docs/snippets/generated"`) the walk of `existing_root` enumerates alef's own output
/// under `generated/...`, so the generated side must carry the same prefix or the two key spaces
/// are disjoint by construction and every file alef just wrote reports as a migration gap. ~keep
///
/// The same rebasing answers the curated question in the other direction: curated globs are
/// project-root-relative, so `nested_prefix(project_root, existing_root)` is the prefix an
/// `existing_root`-relative entry path needs before a curated pattern can be matched against it.
///
/// The lexical `strip_prefix` answers the CLI's own shape, where both paths are project-relative;
/// the absolute retry covers a caller mixing an absolute root with a relative configured output.
/// Identical roots yield an empty prefix, which is `None` rather than a no-op join.
fn nested_prefix(outer: &Path, inner: &Path) -> Option<PathBuf> {
    let non_empty = |prefix: PathBuf| (!prefix.as_os_str().is_empty()).then_some(prefix);
    // An absolute remainder means the lexical strip matched nothing real (`strip_prefix("")`
    // succeeds against any path), so fall through to the absolute comparison instead of
    // returning a "prefix" that is really the whole path. ~keep
    if let Ok(prefix) = inner.strip_prefix(outer)
        && !prefix.is_absolute()
    {
        return non_empty(prefix.to_path_buf());
    }
    let outer = std::path::absolute(outer).ok()?;
    let inner = std::path::absolute(inner).ok()?;
    non_empty(inner.strip_prefix(&outer).ok()?.to_path_buf())
}

fn read_existing(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    if !root.is_dir() {
        bail!("existing snippet root is not a directory: {}", root.display());
    }
    let mut files = Vec::new();
    read_directory(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn read_directory(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let entries =
        fs::read_dir(directory).with_context(|| format!("failed to read snippet directory {}", directory.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read entry in {}", directory.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if file_type.is_dir() {
            read_directory(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let path = entry.path();
            // alef's own coverage ledger is bookkeeping, not a snippet, and it is never in the
            // generated file list -- so leaving it in reported it as `no_generated_equivalent`,
            // i.e. as a migration gap the project was expected to close by hand. ~keep
            if crate::e2e::snippets::is_snippet_coverage_manifest_path(&path) {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("failed to relativize {}", path.display()))?
                .to_path_buf();
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read snippet {} as UTF-8", path.display()))?;
            files.push((relative, content));
        }
    }
    Ok(())
}

pub fn compare_existing<'a>(
    existing: impl IntoIterator<Item = (&'a Path, &'a str)>,
    generated: &[GeneratedFile],
) -> Vec<MigrationEntry> {
    compare_existing_curated(existing, generated, &[], Path::new(""))
        .expect("no curated globs to compile means this can never fail")
}

/// [`compare_existing`]'s curated-aware sibling: see [`compare_root_curated`] for the field
/// this populates and why it exists.
///
/// `curated_base` is prepended to each entry path before matching, rebasing an
/// `existing_root`-relative key into the project-root-relative key space curated globs are
/// written in. An empty base means the two already coincide.
///
/// # Errors
///
/// Returns an error when a curated glob does not parse.
pub fn compare_existing_curated<'a>(
    existing: impl IntoIterator<Item = (&'a Path, &'a str)>,
    generated: &[GeneratedFile],
    curated_globs: &[String],
    curated_base: &Path,
) -> Result<Vec<MigrationEntry>> {
    let compiled_globs = curated_globs
        .iter()
        .map(|pattern| glob::Pattern::new(pattern).with_context(|| format!("invalid curated snippet glob `{pattern}`")))
        .collect::<Result<Vec<_>>>()?;
    let expected: BTreeMap<_, _> = generated
        .iter()
        .map(|file| (file.path.as_path(), file.content.as_str()))
        .collect();
    Ok(existing
        .into_iter()
        .map(|(path, content)| {
            let status = match expected.get(path) {
                Some(expected) if *expected == content => MigrationStatus::Identical,
                Some(_) => MigrationStatus::Different,
                None => MigrationStatus::NoGeneratedEquivalent,
            };
            // Curated is only meaningful alongside `NoGeneratedEquivalent`: it answers "is
            // this the *declared* absence of a generated equivalent, or a genuine gap" --
            // a path that DOES have a generated equivalent is Identical/Different regardless
            // of what any curated glob says about it.
            let curated_key = curated_base.join(path);
            let curated = status == MigrationStatus::NoGeneratedEquivalent
                && compiled_globs.iter().any(|pattern| pattern.matches_path(&curated_key));
            MigrationEntry {
                path: path.to_path_buf(),
                status,
                curated,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_root_recurses_and_reports_stable_relative_paths() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("python/topic")).expect("create nested directory");
        fs::write(directory.path().join("python/topic/a.md"), "same").expect("write identical snippet");
        fs::write(directory.path().join("python/topic/b.md"), "old").expect("write different snippet");
        fs::write(directory.path().join("orphan.md"), "manual").expect("write orphan snippet");
        let generated = vec![
            generated("docs/generated/python/topic/a.md", "same"),
            generated("docs/generated/python/topic/b.md", "new"),
        ];

        let entries =
            compare_root(directory.path(), Path::new("docs/generated"), &generated).expect("compare snippets");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("orphan.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("python/topic/a.md"),
                    status: MigrationStatus::Identical,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("python/topic/b.md"),
                    status: MigrationStatus::Different,
                    curated: false,
                },
            ]
        );
    }

    /// The curated-declaration side of the migration comparison: a hand-authored file with
    /// no generated equivalent, matching a `curated_snippets` glob, must classify as
    /// `NoGeneratedEquivalent` with `curated: true` -- distinct from an unrelated
    /// hand-authored file with no glob match, which stays `curated: false`. Both remain
    /// `NoGeneratedEquivalent`; the flag is what a caller filters a real migration gap on.
    #[test]
    fn compare_root_curated_flags_paths_a_curated_glob_claims() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("docker")).expect("create curated directory");
        fs::write(directory.path().join("docker/quick-start.md"), "curated by hand").expect("write curated snippet");
        fs::write(directory.path().join("orphan.md"), "manual").expect("write uncurated orphan snippet");
        let generated: Vec<GeneratedFile> = Vec::new();

        let entries = compare_root_curated(&CuratedComparison {
            project_root: directory.path(),
            existing_root: directory.path(),
            generated_root: Path::new("docs/generated"),
            generated: &generated,
            curated_globs: &["docker/*.md".to_string()],
        })
        .expect("curated comparison succeeds");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("docker/quick-start.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: true,
                },
                MigrationEntry {
                    path: PathBuf::from("orphan.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: false,
                },
            ]
        );
    }

    /// A curated glob must never retroactively make a real gap disappear: it only annotates
    /// `NoGeneratedEquivalent` entries, so a path that DOES have a generated equivalent stays
    /// `Identical`/`Different` regardless of whether some curated pattern also happens to
    /// match its name.
    #[test]
    fn a_curated_glob_matching_a_path_with_a_real_generated_equivalent_leaves_its_status_untouched() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(directory.path().join("python")).expect("create directory");
        fs::write(directory.path().join("python/example.md"), "old").expect("write stale snippet");
        let generated = vec![generated("docs/generated/python/example.md", "new")];

        let entries = compare_root_curated(&CuratedComparison {
            project_root: directory.path(),
            existing_root: directory.path(),
            generated_root: Path::new("docs/generated"),
            generated: &generated,
            curated_globs: &["python/*.md".to_string()],
        })
        .expect("curated comparison succeeds");

        assert_eq!(
            entries,
            vec![MigrationEntry {
                path: PathBuf::from("python/example.md"),
                status: MigrationStatus::Different,
                curated: false,
            }]
        );
    }

    #[test]
    fn an_invalid_curated_glob_fails_the_comparison_rather_than_silently_matching_nothing() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::write(directory.path().join("orphan.md"), "manual").expect("write orphan snippet");
        let generated: Vec<GeneratedFile> = Vec::new();

        let error = compare_root_curated(&CuratedComparison {
            project_root: directory.path(),
            existing_root: directory.path(),
            generated_root: Path::new("docs/generated"),
            generated: &generated,
            curated_globs: &["[unterminated".to_string()],
        })
        .expect_err("an invalid glob pattern must fail rather than silently match nothing");

        assert!(error.to_string().contains("invalid curated snippet glob"), "{error}");
    }

    /// The nested-root defect: `alef e2e snippets-migrate docs/snippets` against a project
    /// whose `[crates.e2e.snippets].output` is `docs/snippets/generated` -- the generated tree
    /// lives INSIDE the tree being migrated.
    ///
    /// The walk of `existing_root` enumerates alef's own output under an `existing_root`-relative
    /// key (`generated/python/a.md`) while the generated list was keyed against `output`
    /// (`python/a.md`). The two key spaces are disjoint by construction, so every file alef had
    /// just written reported as `NoGeneratedEquivalent`. One consumer saw 7796 files it had
    /// generated itself reported as migration gaps this way.
    #[test]
    fn a_generated_tree_nested_inside_the_migrated_root_is_matched_not_reported_as_a_gap() {
        let directory = tempfile::tempdir().expect("tempdir");
        let existing_root = directory.path();
        let generated_root = existing_root.join("generated");
        fs::create_dir_all(generated_root.join("python")).expect("create generated tree");
        fs::create_dir_all(existing_root.join("cli")).expect("create hand-authored tree");
        fs::write(generated_root.join("python/a.md"), "alef:hash:abc\nsame").expect("write fresh generated snippet");
        fs::write(generated_root.join("python/b.md"), "alef:hash:def\nstale").expect("write stale generated snippet");
        fs::write(existing_root.join("cli/quickstart.md"), "by hand").expect("write hand-authored snippet");
        let generated = vec![
            generated(
                &generated_root.join("python/a.md").to_string_lossy(),
                "alef:hash:abc\nsame",
            ),
            generated(
                &generated_root.join("python/b.md").to_string_lossy(),
                "alef:hash:def\nfresh",
            ),
        ];

        let entries = compare_root(existing_root, &generated_root, &generated).expect("compare snippets");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("cli/quickstart.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("generated/python/a.md"),
                    status: MigrationStatus::Identical,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("generated/python/b.md"),
                    status: MigrationStatus::Different,
                    curated: false,
                },
            ],
            "a file alef itself generates must never be reported as having no generated equivalent"
        );
    }

    /// alef's own coverage ledger is not a snippet and alef never lists it among the files it
    /// generates, so a comparison that walked it reported it as `no_generated_equivalent` --
    /// telling the project to hand-author a replacement for alef's own bookkeeping.
    #[test]
    fn the_coverage_ledger_is_not_reported_as_a_migration_gap() {
        let directory = tempfile::tempdir().expect("tempdir");
        let existing_root = directory.path();
        fs::create_dir_all(existing_root.join("generated")).expect("create generated tree");
        fs::write(existing_root.join("generated/.alef-snippet-coverage.json"), "{}").expect("write ledger");
        fs::write(existing_root.join("orphan.md"), "by hand").expect("write orphan snippet");

        let entries = compare_root(existing_root, &existing_root.join("generated"), &[]).expect("compare snippets");

        assert_eq!(
            entries,
            vec![MigrationEntry {
                path: PathBuf::from("orphan.md"),
                status: MigrationStatus::NoGeneratedEquivalent,
                curated: false,
            }],
            "alef's own coverage ledger must never be reported as a file the project should author"
        );
    }

    /// The equal-roots case the 0.67.6 nested-tree fix (`nested_prefix`) did not itself add a
    /// test for: `alef e2e snippets-migrate docs-site/src/snippets` where `docs-site/src/snippets`
    /// IS the configured `[crates.e2e.snippets].output`, not merely a directory containing it.
    ///
    /// `nested_prefix(existing_root, generated_root)` strips an equal pair down to an empty
    /// remainder, which `non_empty` collapses to `None` -- the same "no rebasing needed" answer
    /// it gives for two genuinely unrelated (parallel) trees. That is deliberate, not
    /// coincidental: an existing-root walk and a generated-file list keyed off the identical root
    /// already share one key space without any prefix, so forcing a non-empty rebase here would
    /// double the prefix (`generated/generated/...`) and break the exact case this test pins.
    /// A migrated root equal to `output` is a real, useful invocation -- it answers "is this
    /// tree's content still fresh", the same question `alef verify` answers via hash comparison
    /// -- so the right contract is a correct comparison, not a refusal: refusing here would take
    /// away the one shape of this command that needs no separate hand-authored directory at all.
    #[test]
    fn equal_existing_and_generated_roots_compare_correctly_without_double_prefixing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = directory.path().join("docs-site/src/snippets");
        fs::create_dir_all(root.join("rust/topic")).expect("create rust tree");
        fs::create_dir_all(root.join("python/topic")).expect("create python tree");
        fs::write(root.join("rust/topic/a.md"), "same content").expect("write fresh rust snippet");
        fs::write(root.join("python/topic/a.md"), "stale content").expect("write stale python snippet");

        let generated = vec![
            generated(&root.join("rust/topic/a.md").to_string_lossy(), "same content"),
            generated(&root.join("python/topic/a.md").to_string_lossy(), "fresh content"),
        ];

        let entries = compare_root(&root, &root, &generated).expect("compare snippets against an equal root");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("python/topic/a.md"),
                    status: MigrationStatus::Different,
                    curated: false,
                },
                MigrationEntry {
                    path: PathBuf::from("rust/topic/a.md"),
                    status: MigrationStatus::Identical,
                    curated: false,
                },
            ],
            "a migrated root equal to the configured output must match every file alef itself \
             generated, never report it as having no generated equivalent"
        );
    }

    /// The curated-aware sibling of the equal-roots case above, shaped like the real CLI wiring
    /// (`snippet_migration::compare`): `project_root` is the directory holding `alef.toml`,
    /// `existing_root` is nested under it and equals `generated_root`, and a curated glob names a
    /// hand-authored file that sits beside the generated tree. Equal roots must not disturb
    /// `curated_base`, which rebases off `project_root` independently of the `existing_root` /
    /// `generated_root` relationship this test targets.
    #[test]
    fn equal_roots_compare_correctly_through_the_curated_aware_entry_point_too() {
        let directory = tempfile::tempdir().expect("tempdir");
        let project_root = directory.path();
        let existing_root = project_root.join("docs-site/src/snippets");
        fs::create_dir_all(existing_root.join("rust/topic")).expect("create rust tree");
        fs::create_dir_all(existing_root.join("cli")).expect("create curated directory");
        fs::write(existing_root.join("rust/topic/a.md"), "same content").expect("write fresh rust snippet");
        fs::write(existing_root.join("cli/quickstart.md"), "by hand").expect("write curated snippet");

        let generated = vec![generated(
            &existing_root.join("rust/topic/a.md").to_string_lossy(),
            "same content",
        )];

        let entries = compare_root_curated(&CuratedComparison {
            project_root,
            existing_root: &existing_root,
            generated_root: &existing_root,
            generated: &generated,
            curated_globs: &["docs-site/src/snippets/cli/*.md".to_string()],
        })
        .expect("curated comparison over an equal root succeeds");

        assert_eq!(
            entries,
            vec![
                MigrationEntry {
                    path: PathBuf::from("cli/quickstart.md"),
                    status: MigrationStatus::NoGeneratedEquivalent,
                    curated: true,
                },
                MigrationEntry {
                    path: PathBuf::from("rust/topic/a.md"),
                    status: MigrationStatus::Identical,
                    curated: false,
                },
            ]
        );
    }

    fn generated(path: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(path),
            content: content.into(),
            generated_header: false,
        }
    }
}
