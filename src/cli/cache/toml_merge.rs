use std::fs;
use std::path::{Path, PathBuf};

use super::render_record_assignment;

/// Repo-scoped (rooted at `base_dir`), COMMITTED record of the array
/// *values* alef's own generator proposed for a TOML merge target, per
/// dotted key path, on the most recent successful merge -- e.g. alef last
/// generated `["target/**", "docs/snippets/**"]` for `poly.toml`'s
/// `discovery.exclude`.
///
/// This is the provenance data `merge_managed_toml`'s prune step needs to
/// answer "did alef itself, in a past run, propose this exact value" without
/// guessing from the value's text alone: a value present in `existing` that
/// merely *equals* something alef's current template happens to emit is not
/// evidence of authorship (a consumer's own `[workspace.poly] exclude` entry
/// can coincide), but a value that was captured here -- straight from alef's
/// own generated output, before any merge with consumer content -- genuinely
/// was alef's proposal. A value the consumer configures via
/// `[workspace.poly] exclude` (or `file_safety_exclude`) is echoed back into
/// the generator's own output on every run for as long as it stays
/// configured, so it keeps reappearing here too and is never a prune
/// candidate; it only becomes one if the consumer removes it from their own
/// config, at which point pruning it matches their own subsequent intent.
///
/// Deliberately keyed by the merge target's *relative* path (`"poly.toml"`),
/// not the `base_dir`-joined absolute one, so the record does not depend on
/// how a given invocation happened to express `base_dir`.
///
/// **Committed to git on purpose**, same rationale and same failure mode as
/// [`super::OWNERSHIP_MANIFEST`]: this used to live at gitignored
/// `.alef/toml-merge-provenance.json`, so a fresh clone or a CI checkout
/// never had a baseline and the prune step could never fire there, no matter
/// how long a value had been gone from alef's own template. Concretely, this
/// is why a consumer's `docs/assets/**` / `docs/snippets/**` `poly.toml`
/// excludes, for a `docs/` tree that had been deleted, had to be removed BY
/// HAND downstream instead of pruning themselves. Sitting at the repo
/// root, this file travels with every checkout of the commit that describes
/// it, so pruning behaves identically on a fresh clone and a warm machine.
///
/// Unlike [`super::OWNERSHIP_MANIFEST`] this record carries no legacy-gitignored-read
/// fallback and no cross-machine migration bridge: [`super::OWNERSHIP_MANIFEST`]
/// needs one because losing a positive ownership claim flips the guard to
/// *refuse* a write it used to allow, which upgrading alef must never do to
/// every existing consumer repo at once. Losing a stale prune baseline only
/// means *not pruning* for one run -- never data loss, never a spurious
/// refusal -- so the first run after upgrading simply establishes a fresh
/// committed baseline and pruning resumes from there. ~keep
pub(super) const TOML_MERGE_PROVENANCE_MANIFEST: &str = ".alef-toml-merge-provenance.toml";

/// Preamble written above the entry list, mirroring [`super::OWNERSHIP_MANIFEST_HEADER`]:
/// addressed at a human reading a `git diff` who has no reason to know what this
/// mystery dotfile is for. ~keep
pub(super) const TOML_MERGE_PROVENANCE_HEADER: &str = "\
# alef toml-merge provenance record -- COMMIT THIS FILE, do not add it to .gitignore.
#
# Records, per merge target and key path, the array values alef itself generated on
# the most recent `alef generate` run -- the baseline the poly.toml merge's prune step
# diffs against to tell \"alef proposed this and later stopped\" from \"the consumer
# wrote this by hand.\" Without this file committed, a fresh clone has no baseline, so
# a value alef stops generating can never be pruned there -- it accumulates forever.
#
# Nothing here is inferred by comparing bytes -- an entry is only ever a copy of
# alef's own past `generated` output for the given key path, captured before merging
# with consumer content. Do not hand-edit; it is rewritten on every `alef generate`.
";

/// Deserialize-only on purpose: the record is *written* by
/// [`render_toml_merge_provenance`], because `toml::to_string_pretty` indents array elements
/// four spaces and `toml::to_string` puts the whole array on one line -- neither matches the
/// sibling ownership record or the `poly fmt` gate consumers run. Deriving `Serialize` would put
/// the discarded route back within reach of the next edit. ~keep
#[derive(serde::Deserialize)]
pub(super) struct TomlMergeProvenanceEntry {
    pub(super) relative_path: String,
    pub(super) key_path: String,
    pub(super) values: Vec<String>,
}

#[derive(Default, serde::Deserialize)]
pub(super) struct TomlMergeProvenanceFile {
    #[serde(default)]
    pub(super) entries: Vec<TomlMergeProvenanceEntry>,
}

type TomlMergeProvenance = std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>;

fn toml_merge_provenance_path(base_dir: &Path) -> PathBuf {
    base_dir.join(TOML_MERGE_PROVENANCE_MANIFEST)
}

/// Read the committed record, treating an unreadable or unparseable file as
/// empty -- the same safe direction as [`super::read_committed_owned_paths`]: a
/// record we could not parse must never be silently treated as "no prior
/// proposal for anything," which here is the *pruning* direction and is
/// exactly as safe as it looks (see [`TOML_MERGE_PROVENANCE_MANIFEST`]'s doc).
fn read_toml_merge_provenance_file(base_dir: &Path) -> TomlMergeProvenance {
    let Ok(content) = fs::read_to_string(toml_merge_provenance_path(base_dir)) else {
        return TomlMergeProvenance::new();
    };
    let Ok(parsed) = toml::from_str::<TomlMergeProvenanceFile>(&content) else {
        return TomlMergeProvenance::new();
    };
    let mut all = TomlMergeProvenance::new();
    for entry in parsed.entries {
        all.entry(entry.relative_path)
            .or_default()
            .insert(entry.key_path, entry.values);
    }
    all
}

/// Read the previously recorded array values for every key path in
/// `relative_path` (e.g. `"poly.toml"`). Empty when nothing was ever
/// recorded for this path -- callers must treat that as "no known prior
/// proposal," never as "alef proposed no arrays."
pub fn read_toml_merge_provenance(
    base_dir: &Path,
    relative_path: &Path,
) -> std::collections::BTreeMap<String, Vec<String>> {
    read_toml_merge_provenance_file(base_dir)
        .remove(&relative_path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Render the record body: one `[[entries]]` table per entry, blank-line separated, arrays
/// rendered by [`render_record_assignment`] like the sibling ownership record.
///
/// Hand-rendered for the indentation, which `toml::to_string_pretty` hard-codes at four spaces
/// while `poly fmt` -- the gate consumers commit through -- normalises to two, leaving this file
/// permanently "would reformat" in every repo alef generates into and unfixable by hand, since
/// the next `alef generate` rewrites it. ~keep
fn render_toml_merge_provenance(entries: &[TomlMergeProvenanceEntry]) -> String {
    let mut body = String::new();
    for entry in entries {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("[[entries]]\n");
        for (key, value) in [("relative_path", &entry.relative_path), ("key_path", &entry.key_path)] {
            body.push_str(key);
            body.push_str(" = ");
            body.push_str(&toml_edit::Value::from(value.as_str()).to_string());
            body.push('\n');
        }
        body.push_str(&render_record_assignment("values", &entry.values));
        body.push('\n');
    }
    body
}

/// Replace the recorded array values for `relative_path` with
/// `arrays_by_key_path` -- this run's freshly generated content, captured
/// before merging with consumer content -- for the next run's comparison.
/// Other merge targets' records are left untouched. Rewrites the committed
/// [`TOML_MERGE_PROVENANCE_MANIFEST`] in full every call, the same
/// read-modify-write shape as [`super::record_scaffold_owned_paths`] (and, like it,
/// not guarded against concurrent writers from other processes -- not a
/// supported mode for any cache in this module).
pub fn write_toml_merge_provenance(
    base_dir: &Path,
    relative_path: &Path,
    arrays_by_key_path: &std::collections::BTreeMap<String, Vec<String>>,
) -> anyhow::Result<()> {
    let manifest_path = toml_merge_provenance_path(base_dir);
    let is_new_manifest = !manifest_path.exists();

    let mut all = read_toml_merge_provenance_file(base_dir);
    all.insert(relative_path.to_string_lossy().into_owned(), arrays_by_key_path.clone());

    // Both maps are `BTreeMap`s, so this iterates in `(relative_path, key_path)`
    // order already -- no separate sort needed to keep the rendered file diffable.
    let entries: Vec<TomlMergeProvenanceEntry> = all
        .into_iter()
        .flat_map(|(relative_path, by_key_path)| {
            by_key_path
                .into_iter()
                .map(move |(key_path, values)| TomlMergeProvenanceEntry {
                    relative_path: relative_path.clone(),
                    key_path,
                    values,
                })
        })
        .collect();

    fs::create_dir_all(base_dir)?;
    let body = render_toml_merge_provenance(&entries);
    fs::write(&manifest_path, format!("{TOML_MERGE_PROVENANCE_HEADER}\n{body}"))?;
    if is_new_manifest {
        tracing::info!(
            manifest = %TOML_MERGE_PROVENANCE_MANIFEST,
            "created the alef toml-merge provenance record: commit it, or a fresh clone can never \
             prune a value alef stops generating"
        );
    }
    Ok(())
}
