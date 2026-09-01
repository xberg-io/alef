use std::fs;
use std::path::{Path, PathBuf};

/// Eager legacy-to-committed ownership migration -- see [`ownership::is_scaffold_owned_path`],
/// re-exported below at this module's own path so every existing caller
/// (`crate::cli::cache::is_scaffold_owned_path`) keeps compiling unchanged. Split out of this
/// file on its own, rather than folded into the surrounding ownership-manifest code, purely to
/// keep this already-oversized file from growing further -- see `file-modularization`. ~keep
mod ownership;
pub use ownership::is_scaffold_owned_path;

/// The central generation-inputs record -- see [`generation_record`]'s module doc for what it
/// replaces and why. Split into its own file for the same `file-modularization` reason
/// [`ownership`] was; `pub(crate)`, not `pub` -- see that module's marker-fn doc. ~keep
pub(crate) mod generation_record;
pub use generation_record::{record_inputs_hash, recorded_inputs_hash, stale_crate_names};

pub(super) const CACHE_DIR: &str = ".alef";
const PER_FILE_CACHE_NAME: &str = "sources_hash.cache";

/// Read the raw bytes of the alef config file for use in [`crate::core::hash::compute_inputs_hash`].
///
/// Returns an empty `Vec` when the file is absent or unreadable — callers
/// treat missing bytes as "empty config", which still produces a stable hash
/// when combined with `sources_hash`.
pub fn read_alef_toml_bytes(config_path: &Path) -> Vec<u8> {
    fs::read(config_path).unwrap_or_default()
}

/// Compute the per-run sources hash that drives both the IR cache and the
/// embedded `alef:hash:` value. Pure function of the rust source files
/// (paths + content); independent of `alef.toml` and the alef CLI version, so
/// that `alef verify` is idempotent across alef upgrades.
///
/// Warm-run optimisation: stat every source and check `(mtime_nanos, size)`
/// against an on-disk memo (`.alef/sources_hash.cache`). When **every** file's
/// stat is unchanged we return the cached aggregate hash directly — no file
/// reads, no blake3 work. Any change to any file falls back to the canonical
/// [`crate::core::hash::compute_sources_hash`] (which reads + hashes everything)
/// and refreshes the memo. The output is always equivalent to the canonical
/// function; the memo only elides redundant reads on no-change runs.
pub fn sources_hash(sources: &[PathBuf]) -> anyhow::Result<String> {
    let mut sorted: Vec<&PathBuf> = sources.iter().collect();
    sorted.sort();

    let memo = read_per_file_memo();
    let mut current: Vec<(String, u64, u64)> = Vec::with_capacity(sorted.len());
    let mut all_match = !memo.entries.is_empty() && memo.aggregate.is_some();
    for source in &sorted {
        let metadata =
            fs::metadata(source).map_err(|e| anyhow::anyhow!("failed to stat source {}: {e}", source.display()))?;
        let mtime_nanos = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let size = metadata.len();
        let path_str = source.to_string_lossy().to_string();
        if all_match {
            match memo.entries.get(&path_str) {
                Some((m, s)) if *m == mtime_nanos && *s == size => {}
                _ => all_match = false,
            }
        }
        current.push((path_str, mtime_nanos, size));
    }

    if all_match
        && current.len() == memo.entries.len()
        && let Some(agg) = memo.aggregate
    {
        return Ok(agg);
    }

    let aggregate = crate::core::hash::compute_sources_hash(sources)?;
    let _ = write_per_file_memo(&current, &aggregate);
    Ok(aggregate)
}

struct PerFileMemo {
    aggregate: Option<String>,
    entries: std::collections::HashMap<String, (u64, u64)>,
}

fn read_per_file_memo() -> PerFileMemo {
    let path = Path::new(CACHE_DIR).join(PER_FILE_CACHE_NAME);
    let Ok(content) = fs::read_to_string(&path) else {
        return PerFileMemo {
            aggregate: None,
            entries: std::collections::HashMap::new(),
        };
    };
    let mut aggregate: Option<String> = None;
    let mut entries = std::collections::HashMap::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("aggregate\t") {
            aggregate = Some(rest.to_string());
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let mtime_nanos = parts[1].parse::<u64>().unwrap_or(0);
        let size = parts[2].parse::<u64>().unwrap_or(0);
        entries.insert(parts[0].to_string(), (mtime_nanos, size));
    }
    PerFileMemo { aggregate, entries }
}

fn write_per_file_memo(entries: &[(String, u64, u64)], aggregate: &str) -> anyhow::Result<()> {
    let dir = Path::new(CACHE_DIR);
    crate::core::cache_dir::ensure_cache_dir(dir)?;
    let mut content = format!("aggregate\t{aggregate}\n");
    for (path, mtime, size) in entries {
        content.push_str(&format!("{path}\t{mtime}\t{size}\n"));
    }
    fs::write(dir.join(PER_FILE_CACHE_NAME), content)?;
    Ok(())
}

/// Validate a crate name before using it as a filesystem path component.
///
/// Returns an error if the name contains path separators, NUL bytes, `..`,
/// or is a bare `.` — any of which could be used to escape the cache directory.
pub fn validate_cache_crate_name(crate_name: &str) -> anyhow::Result<()> {
    if crate_name.contains('\0') {
        anyhow::bail!("invalid crate name for cache: NUL byte not allowed in {crate_name:?}");
    }
    if crate_name.contains('/') || crate_name.contains('\\') {
        anyhow::bail!("invalid crate name for cache: path separator not allowed in {crate_name:?}");
    }
    if crate_name == ".." || crate_name == "." {
        anyhow::bail!("invalid crate name for cache: {crate_name:?} is not a valid crate name");
    }
    Ok(())
}

/// Return the per-crate IR cache directory, e.g. `.alef/<crate_name>/`.
fn ir_cache_dir(crate_name: &str) -> PathBuf {
    Path::new(CACHE_DIR).join(crate_name)
}

/// Check if cached IR is still valid for the given crate.
pub fn is_ir_cached(crate_name: &str, cache_key: &CacheKey) -> bool {
    let dir = ir_cache_dir(crate_name);
    let hash_path = dir.join("ir.hash");
    let ir_path = dir.join("ir.json");
    if !ir_path.exists() {
        return false;
    }
    match fs::read_to_string(&hash_path) {
        Ok(cached) => cached.trim() == cache_key.as_str(),
        Err(_) => false,
    }
}

/// Read cached IR for the given crate.
pub fn read_cached_ir(crate_name: &str) -> anyhow::Result<crate::core::ir::ApiSurface> {
    let ir_path = ir_cache_dir(crate_name).join("ir.json");
    let content = fs::read_to_string(&ir_path)?;
    Ok(serde_json::from_str(&content)?)
}

/// Write IR to cache for the given crate.
pub fn write_ir_cache(crate_name: &str, api: &crate::core::ir::ApiSurface, cache_key: &CacheKey) -> anyhow::Result<()> {
    let cache_dir = ir_cache_dir(crate_name);
    crate::core::cache_dir::ensure_cache_dir_under(Path::new(CACHE_DIR), &cache_dir)?;
    fs::write(cache_dir.join("ir.json"), serde_json::to_string_pretty(api)?)?;
    fs::write(cache_dir.join("ir.hash"), cache_key.as_str())?;
    Ok(())
}

pub use crate::cli::cache_identity::{CacheKey, compute_ir_key, compute_lang_hash, compute_stage_hash};
pub(crate) use crate::cli::cache_outputs::{outputs_exist, stamped_outputs_agree_with_disk};

/// Per-crate hashes directory: `.alef/<crate>/hashes/`.
fn hashes_dir(crate_name: &str) -> PathBuf {
    ir_cache_dir(crate_name).join("hashes")
}

/// Check if a language's output is cached for the given crate.
///
/// A hit requires the key to match, every manifested output to still be on disk, AND every
/// manifested output that carries an `alef:hash:` stamp to still agree with that stamp. The
/// last condition is what makes a hit mean anything: [`outputs_exist`]
/// tests only for *existence*, so a generated file edited in place stayed a cache hit, and
/// `alef generate` answered `Generated 0 files` while leaving the edit untouched — a skip
/// indistinguishable from a verification. The stamp comparison is the same one `alef verify`
/// runs (`hash::compute_file_hash` against the embedded value), so a tree that passes verify
/// passes here; unmarked outputs (`generated_header: false`, create-once seeds) keep the
/// existence-only rule, while a marked-but-unstamped one is a miss so stamping retries. ~keep
pub fn is_lang_cached(crate_name: &str, lang: &str, lang_hash: &CacheKey) -> bool {
    let dir = hashes_dir(crate_name);
    let hash_path = dir.join(format!("{lang}.hash"));
    let manifest_path = dir.join(format!("{lang}.manifest"));
    match fs::read_to_string(&hash_path) {
        Ok(cached) => {
            if cached.trim() != lang_hash.as_str() {
                return false;
            }
            outputs_exist(&manifest_path) && stamped_outputs_agree_with_disk(&manifest_path)
        }
        Err(_) => false,
    }
}

/// Write language hash and output file manifest for the given crate.
///
/// `output_paths` is whatever the caller passes -- this does not, by itself, cover every
/// phase a language's generation may run (service API, type stubs, public API wrappers
/// are each a separate pipeline call the caller may or may not have made yet). The count
/// is logged at `debug` because a manifest this call leaves at one or two entries for a
/// backend whose language-side output tree is much larger is otherwise silent: nothing
/// else marks the difference between "this backend genuinely emits one file" and "the
/// caller never folded a later phase's output back in" (alef#158). ~keep
pub fn write_lang_hash(crate_name: &str, lang: &str, key: &CacheKey, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    crate::core::cache_dir::ensure_cache_dir_under(Path::new(CACHE_DIR), &dir)?;
    fs::write(dir.join(format!("{lang}.hash")), key.as_str())?;
    write_manifest(&dir.join(format!("{lang}.manifest")), output_paths)?;
    tracing::debug!(
        crate_name,
        lang,
        paths = output_paths.len(),
        "wrote language manifest via write_lang_hash"
    );
    Ok(())
}

/// Replace a language manifest after every generation phase has contributed
/// its files. The language hash itself remains unchanged.
pub fn write_lang_manifest(crate_name: &str, lang: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    crate::core::cache_dir::ensure_cache_dir_under(Path::new(CACHE_DIR), &dir)?;
    write_manifest(&dir.join(format!("{lang}.manifest")), output_paths)?;
    tracing::debug!(
        crate_name,
        lang,
        paths = output_paths.len(),
        "wrote language manifest via write_lang_manifest"
    );
    Ok(())
}

pub fn read_lang_manifest(crate_name: &str, lang: &str) -> Vec<PathBuf> {
    let manifest_path = hashes_dir(crate_name).join(format!("{lang}.manifest"));
    match fs::read_to_string(manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Replace the crate-wide scaffold-ownership manifest with every path the
/// current run's scaffold pass emitted, deliberately including
/// `generated_header: false` seeds (`composer.json`, `package.json`, ...) that
/// carry no `alef:hash:` marker and are therefore invisible to
/// [`write_lang_manifest`]'s `carries_alef_marker()` filter.
///
/// This is the sole durable record that lets `sweep_manifest_orphans`'s
/// unmarkable-manifest route (see `path_is_reclaimable` in
/// `generate/orphans.rs`) reclaim a manifest a later run stops emitting (e.g. a
/// co-located/split PHP layout toggle that drops a second `composer.json`), and
/// it doubles as the current-run "keep" evidence that stops a manifest still
/// being written from ever being mistaken for an orphan of itself.
///
/// Crate-scoped rather than per-language like [`write_lang_manifest`] because
/// `scaffold()` returns a flat, unpartitioned file list; callers that only run
/// scaffold for a `--lang` subset must not call this, or the write here would
/// clobber the recorded paths for every other language's manifests.
pub fn write_scaffold_manifest(crate_name: &str, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    crate::core::cache_dir::ensure_cache_dir_under(Path::new(CACHE_DIR), &dir)?;
    write_manifest(&dir.join("scaffold-ownership.manifest"), output_paths)
}

/// Read the previous run's scaffold-ownership manifest written by
/// [`write_scaffold_manifest`]. Empty when scaffold has never run for this
/// crate under this mechanism (including every run before this manifest was
/// introduced) -- callers must tolerate an empty result as "no known prior
/// scaffold state" rather than "nothing was ever scaffolded".
pub fn read_scaffold_manifest(crate_name: &str) -> Vec<PathBuf> {
    let manifest_path = hashes_dir(crate_name).join("scaffold-ownership.manifest");
    match fs::read_to_string(manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Repo-scoped (rooted at `base_dir`, not crate-scoped) durable record of
/// every path alef owns whose format cannot carry an `alef:hash:` marker.
///
/// **Committed to git on purpose.** For every format that can carry a comment
/// the marker is the proof of ownership and it travels in the repository; for
/// `package.json`, `*.jar` and friends there is no such place to put it, so the
/// proof has to live in a separate file — and that file has to travel too. The
/// pre-#80 record lived at `.alef/scaffold-owned-paths.manifest`, inside the
/// directory alef writes into every consumer's `.gitignore` itself
/// (`cli::pipeline::extract::gitignore::ensure_gitignore`). That made ownership
/// a property of a particular developer's disk: a fresh clone and a warm
/// machine answered differently for the same commit, so CI refused writes a
/// developer's machine permitted. Sitting at the repo root outside `.alef/`,
/// this file is picked up by an ordinary `git add` and every checkout of a
/// commit agrees about what alef owns.
///
/// Deliberately additive and never replaced wholesale, unlike
/// [`write_scaffold_manifest`]'s per-crate, per-run snapshot: the write-time
/// ownership guard in `write_scaffold_files_report` has no crate name in
/// scope (it writes plain scaffold/readme/e2e/docs output keyed only by
/// `base_dir`) and is invoked incrementally from many independent commands
/// (readme, e2e regen, version sync, ...), so each call must extend the
/// record without erasing paths a different call already proved ownership
/// of. Rooted at `base_dir` rather than the process CWD so parallel tests
/// (each with their own tempdir `base_dir`) never share, and race on, the
/// same manifest file. ~keep
pub(super) const OWNERSHIP_MANIFEST: &str = ".alef-ownership.toml";

/// The pre-#80 location of the same record, under the gitignored `.alef/` cache.
///
/// Still *read* (unioned with [`OWNERSHIP_MANIFEST`]) and never written. A
/// working copy that established ownership under an older alef keeps it, so
/// upgrading does not turn every unmarkable file in every existing consumer
/// repo into a refusal at once; the entry migrates into the committed manifest
/// the first time alef performs an authorised write of that path. Dropping the
/// read outright would be correct in the abstract and a mass outage in
/// practice. ~keep
pub(super) const LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST: &str = "scaffold-owned-paths.manifest";

/// Preamble written above the path list.
///
/// Addressed at a human reading a `git diff` who has no reason to know what the
/// file is for: without it the natural reaction to a mystery dotfile is to
/// gitignore it, which restores exactly the bug this file exists to fix. ~keep
const OWNERSHIP_MANIFEST_HEADER: &str = "\
# alef ownership record -- COMMIT THIS FILE, do not add it to .gitignore.
#
# Lists the alef-generated paths whose format cannot carry an `alef:hash:`
# provenance marker (`package.json`, `*.jar`, ...). Every other format proves
# alef's ownership from the marker in the file itself and never appears here.
# Without this list committed, a fresh clone cannot tell an alef-generated
# `package.json` from a hand-written one and refuses to regenerate it.
#
# Ownership is a fact about history, not about content: a path lands here only
# because alef created the file, or because a human ran `alef adopt` on it.
# Nothing here is inferred by comparing bytes against generated output -- a
# hand-written file that happens to match must never be claimed. Do not hand-add
# entries; run `alef adopt <path>`, read the diff it prints, and let it write.
";

/// Normalize `path` to a `base_dir`-relative key before it is used to read or
/// write the owned-paths manifest.
///
/// Production callers of [`record_scaffold_owned_path`] / [`is_scaffold_owned_path`]
/// do not agree on how they spell `base_dir`: most `bin_cli` commands pass
/// `std::env::current_dir()?` (absolute), while `version_regen.rs`'s regen
/// helpers pass `PathBuf::from(".")` (relative) -- both name the same
/// directory, but `base_dir.join(&file.path)` produces textually different
/// strings from each (`/abs/repo/packages/java/pom.xml` vs
/// `./packages/java/pom.xml`). Storing and looking up that raw joined string
/// meant a record written by one caller was invisible to a lookup from the
/// other: `is_scaffold_owned_path` read as permanently `false` for any file
/// whose write-time caller and check-time caller happened to spell `base_dir`
/// differently, which in practice is most real cross-command sequences (e.g.
/// `alef all` establishes ownership, a later `alef version` bump checks it).
/// Stripping `base_dir` back off before keying makes the record depend only
/// on `file.path`, which every caller already agrees on. Falls back to the
/// path as given if it is not actually rooted at `base_dir` (should not
/// happen in practice, since every caller builds `path` via
/// `base_dir.join(...)`, but a mismatched pair must degrade to "some key"
/// rather than panic). ~keep
pub(super) fn scaffold_owned_path_key(base_dir: &Path, path: &Path) -> String {
    path.strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[derive(serde::Deserialize)]
struct OwnershipManifest {
    #[serde(default)]
    owned_paths: Vec<String>,
}

fn ownership_manifest_path(base_dir: &Path) -> PathBuf {
    base_dir.join(OWNERSHIP_MANIFEST)
}

/// The outcome of reading the committed ownership record, keeping "there is no record yet" apart
/// from "there is a record and we could not read it".
///
/// The two are indistinguishable to a caller handed only a `Vec`, yet they license opposite
/// actions: the first is proof that alef has recorded no ownership, the second is proof of
/// nothing at all. Reading and *rewriting* the record therefore diverge -- see
/// [`read_committed_owned_paths`] and [`record_scaffold_owned_paths`] for which way each goes and
/// why. ~keep
enum OwnedPathsRecord {
    /// No manifest on disk: alef has never recorded ownership under this `base_dir`.
    Absent,
    /// Parsed cleanly; carries every path currently recorded.
    Present(Vec<String>),
    /// A manifest exists but could not be read or parsed. Carries the reason, for the operator.
    Unreadable(String),
}

fn read_owned_paths_record(base_dir: &Path) -> OwnedPathsRecord {
    match fs::read_to_string(ownership_manifest_path(base_dir)) {
        Ok(content) => match toml::from_str::<OwnershipManifest>(&content) {
            Ok(manifest) => OwnedPathsRecord::Present(manifest.owned_paths),
            Err(error) => OwnedPathsRecord::Unreadable(error.to_string()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => OwnedPathsRecord::Absent,
        Err(error) => OwnedPathsRecord::Unreadable(error.to_string()),
    }
}

/// Read the committed record for *querying* ownership, treating an unreadable or unparseable file
/// as empty.
///
/// Degrading to "alef owns nothing" is the safe direction for a query: the write-time guard then
/// refuses rather than clobbers, and nothing is silently claimed on the strength of a file we
/// could not actually parse. A hard error here would instead take down every generate in a repo
/// where someone hand-edited the manifest into invalid TOML.
///
/// This is emphatically *not* the safe direction for a caller that rewrites the record from what
/// it read back -- [`record_scaffold_owned_paths`] must refuse instead, because there the same
/// empty `Vec` would erase every recorded path. ~keep
pub(super) fn read_committed_owned_paths(base_dir: &Path) -> Vec<String> {
    match read_owned_paths_record(base_dir) {
        OwnedPathsRecord::Present(paths) => paths,
        OwnedPathsRecord::Absent => Vec::new(),
        OwnedPathsRecord::Unreadable(reason) => {
            tracing::warn!(
                manifest = %OWNERSHIP_MANIFEST,
                reason = %reason,
                "the alef ownership record could not be read; treating every path in it as unowned, \
                 so writes to unmarkable files will be refused until it is repaired"
            );
            Vec::new()
        }
    }
}

pub(super) fn read_legacy_owned_paths(base_dir: &Path) -> Vec<String> {
    let manifest_path = base_dir.join(CACHE_DIR).join(LEGACY_SCAFFOLD_OWNED_PATHS_MANIFEST);
    fs::read_to_string(manifest_path)
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The indentation every committed record alef writes uses for one array element per line.
///
/// One fact, one definition. Consumers gate their commits on `poly fmt --check`, whose TOML
/// formatter normalises array elements to two spaces, and a record that indents differently is a
/// gate failure they cannot repair: the next `alef generate` overwrites any hand-formatting. The
/// two sibling records at the repo root used to derive this separately -- the ownership record
/// hand-rendered two spaces while the merge-provenance record inherited four from
/// `toml::to_string_pretty` (whose pretty serializer writes a hard-coded `"    "` per element,
/// with nothing to configure) -- and so disagreed for as long as nothing compared them. ~keep
const RECORD_ARRAY_INDENT: &str = "  ";

/// Widest `key = [...]` line `poly fmt` leaves inline. Measured against the bundled TOML
/// formatter rather than assumed: a two-element array rendering to 120 columns is collapsed onto
/// one line, 121 is left expanded, and no repo in the polyrepo overrides the formatter's column
/// width. Both committed records are rewritten wholesale on every `alef generate`, so emitting a
/// shape the formatter disagrees with is not a one-time cosmetic diff -- alef re-expands what
/// `poly fmt` collapsed, the consumer's format gate rewrites it back, and the file ping-pongs in
/// every commit forever. ~keep
const RECORD_ARRAY_MAX_INLINE_WIDTH: usize = 120;

/// Render `key = [...]` for a committed record array, with no trailing newline: inline when the
/// result fits [`RECORD_ARRAY_MAX_INLINE_WIDTH`], otherwise one element per line at
/// [`RECORD_ARRAY_INDENT`] with a trailing comma.
///
/// Element reprs come from `toml_edit` rather than a hand-rolled escape so a value carrying a
/// quote, a backslash or a control character cannot produce a record that no longer parses. An
/// unparseable record is silent by design (it reads as "alef owns nothing" / "alef proposed
/// nothing"), so a bad escape would not announce itself. ~keep
fn render_record_assignment(key: &str, values: &[String]) -> String {
    let elements: Vec<String> = values
        .iter()
        .map(|value| toml_edit::Value::from(value.as_str()).to_string())
        .collect();

    let inline = format!("{key} = [{}]", elements.join(", "));
    if inline.chars().count() <= RECORD_ARRAY_MAX_INLINE_WIDTH {
        return inline;
    }

    let mut rendered = format!("{key} = [\n");
    for element in &elements {
        rendered.push_str(RECORD_ARRAY_INDENT);
        rendered.push_str(element);
        rendered.push_str(",\n");
    }
    rendered.push(']');
    rendered
}

/// Render the manifest by hand rather than through `toml::to_string`.
///
/// This file is read in `git diff` far more often than by a parser, and a
/// serializer is free to emit the array inline on one line. Adopting a single
/// path would then rewrite the whole line and show as a wholesale replacement,
/// which is precisely the shape that hides an unintended ownership claim from a
/// reviewer. One path per line makes every claim its own `+` line. ~keep
fn render_ownership_manifest(paths: &[String]) -> String {
    format!(
        "{OWNERSHIP_MANIFEST_HEADER}\n{}\n",
        render_record_assignment("owned_paths", paths)
    )
}

/// Record `path` (relative to `base_dir`, or already `base_dir`-joined -- see
/// [`scaffold_owned_path_key`]) as alef-owned, in the committed
/// [`OWNERSHIP_MANIFEST`].
///
/// The write-time guard in `write_scaffold_files_report` consults this for
/// extensions it cannot stamp with an `alef:hash:` marker (`.json`, `.jar`,
/// ...) to distinguish "alef legitimately wrote this before" from "this
/// pre-existed alef and must not be silently claimed." Idempotent: a path
/// already present is left alone, so a converged tree never rewrites the file
/// and never produces a spurious diff.
///
/// Callers must only reach this having established ownership *historically* --
/// alef created the file, or `alef adopt` obtained a human's consent for it.
/// Calling it because the bytes on disk happen to equal this run's output turns
/// a coincidence into a permanent, committed claim over a file nobody adopted;
/// see `cli::pipeline::generate::write::stamp_for_adoption` for the incident
/// that settles why byte-equality is not evidence. ~keep
pub fn record_scaffold_owned_path(base_dir: &Path, path: &Path) -> anyhow::Result<()> {
    record_scaffold_owned_paths(base_dir, std::slice::from_ref(&path))
}

/// Record every path in `paths` as alef-owned in one read-modify-write.
///
/// Semantically identical to calling [`record_scaffold_owned_path`] once per path,
/// which is exactly why it exists: that function reads, parses, re-renders and
/// rewrites the whole manifest per call, so adopting a batch through it costs
/// O(n) manifest parses over an O(n)-sized file — quadratic, and `alef adopt`
/// now has to clear ~12k unmarkable paths in a single consumer-repo migration.
/// One parse and one write for the whole batch makes that linear. The
/// per-path entry point delegates here rather than the reverse so there is a
/// single copy of the locking and rendering logic. ~keep
pub fn record_scaffold_owned_paths(base_dir: &Path, paths: &[&Path]) -> anyhow::Result<()> {
    // Serialised because this is a read-modify-write of one file and
    // `write_files_report` calls it from a rayon `par_iter`: two threads that both
    // observe the pre-write list and then both write it lose one entry, and a lost
    // entry is a path alef silently stops owning — a refusal on the next run, in CI,
    // for a file alef itself created. The old gitignored record had the same race and
    // could be repaired by rerunning locally; a committed one gets the wrong answer
    // captured in a commit instead. Cross-*process* concurrency in one repo is not a
    // supported mode for any of this module's caches. ~keep
    static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());

    if paths.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(base_dir)?;
    let record = read_owned_paths_record(base_dir);
    let is_new_manifest = matches!(record, OwnedPathsRecord::Absent);
    let mut recorded: std::collections::BTreeSet<String> = match record {
        OwnedPathsRecord::Present(paths) => paths.into_iter().collect(),
        OwnedPathsRecord::Absent => std::collections::BTreeSet::new(),
        // Refusing is the only non-destructive answer available here. The write below replaces the
        // manifest whole, so continuing from an unparsed read would persist this batch alone and
        // silently un-own every path already in the file -- and that record is the only thing
        // standing between `--clean`/the orphan scan and hand-written unmarkable scaffold files
        // (see `is_scaffold_owned_path`). Unlike the query direction there is no "assume nothing"
        // that preserves anything: unknown ownership survives only if nobody writes. So this
        // fails loudly and names the file, which is a state exactly one hand-edit can produce and
        // one `git checkout` can undo. ~keep
        OwnedPathsRecord::Unreadable(reason) => anyhow::bail!(
            "refusing to update the alef ownership record at {}: it exists but could not be read \
             ({reason}). Fix: repair or restore it (`git checkout -- {OWNERSHIP_MANIFEST}`), then re-run.",
            ownership_manifest_path(base_dir).display()
        ),
    };
    let mut added = false;
    for path in paths {
        added |= recorded.insert(scaffold_owned_path_key(base_dir, path));
    }
    if !added {
        return Ok(());
    }
    let ordered: Vec<String> = recorded.into_iter().collect();
    fs::write(ownership_manifest_path(base_dir), render_ownership_manifest(&ordered))?;
    if is_new_manifest {
        tracing::info!(
            manifest = %OWNERSHIP_MANIFEST,
            "created the alef ownership record: commit it, or a fresh clone cannot regenerate \
             the unmarkable files listed in it"
        );
        // The record did not exist yet when the standing check last ran for this
        // `base_dir`, so it correctly reported nothing. Re-arm it so the run that creates
        // the file is also a run that says it is untracked -- the one moment the operator
        // is unambiguously in a position to stage it. ~keep
        rearm_untracked_record_notice(base_dir);
    }
    note_untracked_required_records(base_dir);
    Ok(())
}

/// The namespace alef reserves for its own bookkeeping artifacts, and the first of the
/// two conditions [`is_alef_derived_output`] requires.
///
/// Already load-bearing elsewhere on exactly this meaning: `snippets::discovery` skips
/// every `.alef-`-prefixed entry when it walks a snippet directory, because a file in
/// this namespace is alef's own state and never documentation a consumer wrote. Naming
/// it here makes that convention checkable rather than a per-site string literal. ~keep
const ALEF_RESERVED_NAME_PREFIX: &str = ".alef-";

/// File names that are **pure derived output**: every byte is recomputed from this run's
/// inputs, nothing but alef writes them, and nothing but alef reads them.
///
/// A name earns a place here only by satisfying **all four** of these, verified against
/// the emitter, not assumed from the extension:
///
/// 1. The format structurally cannot carry an `alef:hash:` marker (strict JSON has no
///    comment syntax), so `write::marker_comment_style` is `None` for it and a missing
///    marker is not evidence of foreign authorship.
/// 2. The name sits in alef's reserved [`ALEF_RESERVED_NAME_PREFIX`] namespace, so no
///    other tool defines a file by that name and a consumer has no reason to author one.
/// 3. Alef is the only *reader* as well as the only writer. This is the condition that
///    separates this list from `orphans::UNMARKABLE_ALEF_MANIFESTS`
///    (`composer.json`, `package.json`): those are also unmarkable and also
///    alef-generated, but a human edits them and a package manager reads them, so
///    trusting their name alone would be a licence to clobber hand-written content.
/// 4. The content has no state a human could have added. Regenerating it wholesale is
///    not a loss of work, it is the *point* — the opposite of a create-once seed, whose
///    whole premise is that the copy on disk has grown past the placeholder alef emitted.
///
/// The snippet-coverage ledger (`e2e::snippets::COVERAGE_MANIFEST`) is the founding
/// member: a `generated_paths`/`generated_metadata` index of what the snippet stage
/// emitted, consumed only by alef's own coverage checks. ~keep
const ALEF_DERIVED_OUTPUT_NAMES: &[&str] = &[crate::e2e::snippets::COVERAGE_MANIFEST];

/// The single named property "this is pure derived output alef must be free to replace".
///
/// Exists because a generated artifact that cannot carry a marker has, until it is
/// answered, exactly the same signature as a hand-grown create-once seed: no marker, no
/// ownership record, `generated_header: false`. Every mechanism that reads that signature
/// — the write-time ownership guard, the write-time create-once skip, and
/// `commands::adopt`'s create-once classifier — must therefore consult *this* property
/// rather than each carving out its own exception, which is how the ledger came to be
/// unblocked at the guard and still refused by adopt (see `adopt::is_create_once_seed`).
///
/// **What this deliberately does not do.** It is not an ownership record and it never
/// widens one: [`is_scaffold_owned_path`] still answers only from what alef actually
/// wrote or a human actually adopted, so nothing here can claim a path by coincidence.
/// It is also not consulted by any *delete* gate — `orphans::path_is_reclaimable` keeps
/// its own, narrower allowlist on purpose, because "alef may overwrite this with freshly
/// computed content" and "alef may remove this file" are different licences and the
/// second one is how a consumer's public API nearly went missing.
///
/// The [`ALEF_RESERVED_NAME_PREFIX`] conjunct is a structural backstop rather than a
/// redundant test: it makes a mistaken future entry in [`ALEF_DERIVED_OUTPUT_NAMES`]
/// inert instead of dangerous. Adding `composer.json` there would grant nothing, because
/// no name a consumer's toolchain defines can live in alef's reserved namespace. ~keep
pub fn is_alef_derived_output(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(ALEF_RESERVED_NAME_PREFIX) && ALEF_DERIVED_OUTPUT_NAMES.contains(&name))
}

/// Every alef-authored record whose entire purpose depends on it being committed.
///
/// Every one is provenance alef cannot re-derive from anything else: [`OWNERSHIP_MANIFEST`]
/// is the only proof of authorship for a format that cannot carry a marker,
/// [`TOML_MERGE_PROVENANCE_MANIFEST`] is the only proof of which array values alef itself
/// once proposed, and [`generation_record::GENERATION_RECORD`] is the only record of what
/// generation inputs a crate's committed output was last generated against (see that
/// module's doc). Left out of the commit, each degrades in the *safe* direction — the
/// guard refuses, the prune declines, the stale-tree check is silently skipped — which is
/// precisely why the failure is silent: the run is green on the machine that holds the
/// untracked file and refuses/under-reports everything on a fresh clone or in CI, with no
/// signal connecting the two. ~keep
const REQUIRED_COMMITTED_RECORDS: &[&str] = &[
    OWNERSHIP_MANIFEST,
    TOML_MERGE_PROVENANCE_MANIFEST,
    generation_record::GENERATION_RECORD,
];

/// Which of [`REQUIRED_COMMITTED_RECORDS`] exist on disk under `base_dir` but are not
/// tracked by git.
///
/// A record that does not exist is not reported: alef has nothing to depend on yet, so
/// there is no hidden dependency to warn about. `None` from [`git_tracks`] — no git, not
/// a work tree, git failed — is likewise not reported: this must never cry wolf in an
/// export tarball or a container without git, where "untracked" is meaningless rather
/// than wrong.
///
/// Exposed as a pure query, separate from the logging in
/// [`note_untracked_required_records`], so a command can escalate it. The recommended
/// split: `alef all` warns (the operator can still `git add` and the run's output is
/// genuinely correct on their disk), while `alef verify` should fail — a verification
/// that passes only because of an uncommitted local file certifies a state no other
/// checkout has, which is the same defect class as a check that examines nothing. ~keep
pub fn untracked_required_records(base_dir: &Path) -> Vec<&'static str> {
    REQUIRED_COMMITTED_RECORDS
        .iter()
        .filter(|record| base_dir.join(record).is_file() && git_tracks(base_dir, record) == Some(false))
        .copied()
        .collect()
}

/// Whether git tracks `relative` under `base_dir`; `None` when git cannot answer at all.
///
/// `--error-unmatch` is what makes the exit status meaningful: without it `git ls-files`
/// exits 0 and prints nothing for an untracked path, which is indistinguishable from
/// success. A non-zero exit therefore means "git ran and does not track this", and only a
/// failure to spawn (or a repository git refuses to read) yields `None`. `git status`
/// would answer the same question far more expensively and would also fold in staged and
/// dirty state, which is not what is being asked here. ~keep
fn git_tracks(base_dir: &Path, relative: &str) -> Option<bool> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .args(["ls-files", "--error-unmatch", "--", relative])
        .output()
        .ok()?;
    if output.status.success() {
        return Some(true);
    }
    // Distinguish "git ran and said no" from "there is no repository here to ask". Git
    // reports the latter on stderr and exits non-zero for both, so the exit code alone
    // would turn every non-repo invocation into a false alarm. ~keep
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") || stderr.contains("this operation must be run in a work tree") {
        return None;
    }
    Some(false)
}

/// Base directories already reported on, so the `git` probe and the warning happen once
/// per repository per process rather than once per path consulted.
///
/// A set rather than a `OnceLock`: the check is keyed on `base_dir`, and a single
/// process legitimately visits several (a multi-crate workspace command, and every test
/// in this module, each with its own tempdir). A `OnceLock` would answer for whichever
/// directory happened to arrive first and stay silent for the rest. ~keep
static REPORTED_RECORD_TRACKING: std::sync::Mutex<Option<std::collections::BTreeSet<PathBuf>>> =
    std::sync::Mutex::new(None);

/// Warn, at most once per `base_dir` per process, about every required record that exists
/// but is untracked.
///
/// Called from the two places alef actually *depends* on such a record —
/// [`is_scaffold_owned_path`] and [`record_scaffold_owned_paths`] — rather than only from
/// the branch that creates it. That is the whole correction: the previous notice was a
/// one-shot `INFO` on the single historical run that first wrote the file, so a repository
/// that missed it once never heard about it again, and every subsequent green run was
/// green because of a file no other checkout has. A standing condition has to be re-stated
/// by every run that relies on it.
///
/// `WARN`, not `ERROR`: per this repo's level contract the run is degraded but correct on
/// this disk, and the output it produced is real. Escalation to a hard failure belongs to
/// `alef verify`, via [`untracked_required_records`]. ~keep
pub(super) fn note_untracked_required_records(base_dir: &Path) {
    {
        let mut reported = REPORTED_RECORD_TRACKING
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let seen = reported.get_or_insert_with(std::collections::BTreeSet::new);
        if !seen.insert(base_dir.to_path_buf()) {
            return;
        }
    }
    for record in untracked_required_records(base_dir) {
        tracing::warn!(
            manifest = %record,
            "alef depends on `{record}` but git does not track it. Fix: `git add {record}` and commit \
             it, or a fresh clone and CI will refuse to regenerate everything it vouches for"
        );
    }
}

/// Drop `base_dir` from the once-per-repo memo so the next
/// [`note_untracked_required_records`] re-probes it. Only for the moment a required
/// record comes into existence *after* the check already ran for this directory. ~keep
fn rearm_untracked_record_notice(base_dir: &Path) {
    let mut reported = REPORTED_RECORD_TRACKING
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(seen) = reported.as_mut() {
        seen.remove(base_dir);
    }
}

/// Repo-scoped (rooted at `base_dir`), COMMITTED record of the array
/// *values* alef's own generator proposed for a TOML merge target, per
/// dotted key path, on the most recent successful merge -- e.g. alef last
/// generated `["target/**", "docs/snippets/**"]` for `poly.toml`'s
/// `discovery.exclude`.
///
/// This is the provenance data [`merge_managed_toml`]'s prune step needs to
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
/// [`OWNERSHIP_MANIFEST`]: this used to live at gitignored
/// `.alef/toml-merge-provenance.json`, so a fresh clone or a CI checkout
/// never had a baseline and the prune step could never fire there, no matter
/// how long a value had been gone from alef's own template. Concretely, this
/// is why a consumer's `docs/assets/**` / `docs/snippets/**` `poly.toml`
/// excludes, for a `docs/` tree that had been deleted, had to be removed BY
/// HAND downstream instead of pruning themselves. Sitting at the repo
/// root, this file travels with every checkout of the commit that describes
/// it, so pruning behaves identically on a fresh clone and a warm machine.
///
/// Unlike [`OWNERSHIP_MANIFEST`] this record carries no legacy-gitignored-read
/// fallback and no cross-machine migration bridge: [`OWNERSHIP_MANIFEST`]
/// needs one because losing a positive ownership claim flips the guard to
/// *refuse* a write it used to allow, which upgrading alef must never do to
/// every existing consumer repo at once. Losing a stale prune baseline only
/// means *not pruning* for one run -- never data loss, never a spurious
/// refusal -- so the first run after upgrading simply establishes a fresh
/// committed baseline and pruning resumes from there. ~keep
const TOML_MERGE_PROVENANCE_MANIFEST: &str = ".alef-toml-merge-provenance.toml";

/// Preamble written above the entry list, mirroring [`OWNERSHIP_MANIFEST_HEADER`]:
/// addressed at a human reading a `git diff` who has no reason to know what this
/// mystery dotfile is for. ~keep
const TOML_MERGE_PROVENANCE_HEADER: &str = "\
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
struct TomlMergeProvenanceEntry {
    relative_path: String,
    key_path: String,
    values: Vec<String>,
}

#[derive(Default, serde::Deserialize)]
struct TomlMergeProvenanceFile {
    #[serde(default)]
    entries: Vec<TomlMergeProvenanceEntry>,
}

type TomlMergeProvenance = std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>;

fn toml_merge_provenance_path(base_dir: &Path) -> PathBuf {
    base_dir.join(TOML_MERGE_PROVENANCE_MANIFEST)
}

/// Read the committed record, treating an unreadable or unparseable file as
/// empty -- the same safe direction as [`read_committed_owned_paths`]: a
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
/// read-modify-write shape as [`record_scaffold_owned_paths`] (and, like it,
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

/// Check if a stage's output is cached for the given crate.
///
/// A hit requires the key to match, every manifested output to still be on disk, AND every
/// manifested output that carries an `alef:hash:` stamp to still agree with that stamp under
/// `inputs_hash` -- the same three-part check [`is_lang_cached`] runs, for the same reason: the
/// manifest-existence check alone cannot tell a hand-edited stage output (e2e suite, scaffold
/// file, README, docs page) from an untouched one, so a consumer's edit to e.g. a generated e2e
/// test survived a stage-cache hit silently. See [`is_lang_cached`]'s doc for the full incident
/// and [`stamped_outputs_agree_with_disk`]'s doc for why an unmarked output (`generated_header:
/// false`, create-once seeds) keeps the existence-only rule instead of forcing a permanent miss --
/// and why an output that carries the marker but no stamp is a miss rather than agreement. ~keep
pub fn is_stage_cached(crate_name: &str, stage: &str, stage_hash: &CacheKey) -> bool {
    let dir = hashes_dir(crate_name);
    let hash_path = dir.join(format!("{stage}.hash"));
    let manifest_path = dir.join(format!("{stage}.manifest"));
    match fs::read_to_string(&hash_path) {
        Ok(cached) => {
            if cached.trim() != stage_hash.as_str() {
                return false;
            }
            outputs_exist(&manifest_path) && stamped_outputs_agree_with_disk(&manifest_path)
        }
        Err(_) => false,
    }
}

/// Read the manifest of output paths previously written for the given stage.
///
/// Returns an empty `Vec` when the manifest does not exist (either the stage
/// has never been generated for this crate, or the cache predates the manifest
/// format introduced in 0.18.1). Callers should use this to repopulate
/// `current_gen_paths` on a cache hit so the orphan-cleanup pass does not
/// delete files that the previous run wrote but the current run skipped.
pub fn read_stage_paths(crate_name: &str, stage: &str) -> Vec<PathBuf> {
    let dir = hashes_dir(crate_name);
    let manifest_path = dir.join(format!("{stage}.manifest"));
    match fs::read_to_string(&manifest_path) {
        Ok(content) => content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Write stage hash and output file manifest for the given crate.
///
/// Takes `&str`, not [`CacheKey`]: manifest-only callers store a plain content hash here and
/// never read it back through [`is_stage_cached`]. See `cache_identity`'s module doc. ~keep
pub fn write_stage_hash(
    crate_name: &str,
    stage: &str,
    stage_hash: &str,
    output_paths: &[PathBuf],
) -> anyhow::Result<()> {
    let dir = hashes_dir(crate_name);
    crate::core::cache_dir::ensure_cache_dir_under(Path::new(CACHE_DIR), &dir)?;
    fs::write(dir.join(format!("{stage}.hash")), stage_hash)?;
    write_manifest(&dir.join(format!("{stage}.manifest")), output_paths)?;
    Ok(())
}

/// Write a manifest of output file paths (one per line).
fn write_manifest(manifest_path: &Path, output_paths: &[PathBuf]) -> anyhow::Result<()> {
    let mut paths: Vec<_> = output_paths.iter().map(|p| p.to_string_lossy()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut content = paths.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    fs::write(manifest_path, content)?;
    Ok(())
}

/// Hash all files in a directory recursively (for e2e fixture hashing).
pub fn hash_directory(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();
    if dir.exists() {
        let mut entries: Vec<_> = walkdir(dir)?;
        entries.sort();
        for path in entries {
            let content = fs::read(&path)?;
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(&content);
        }
    }
    Ok(hasher.finalize().as_bytes().to_vec())
}

fn walkdir(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

/// Blake3 hash of a content string.
pub fn hash_content(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// Store generation content hashes: Vec of (path_display, content_hash).
///
/// Call this with pre-computed hashes — use [`hash_content`] on each file's
/// content string before calling.  Stored before writing to disk so hashes
/// reflect pure codegen output, independent of any on-disk formatter.
pub fn write_generation_hashes(name: &str, hashes: &[(String, String)]) -> anyhow::Result<()> {
    let dir = Path::new(CACHE_DIR).join("hashes");
    crate::core::cache_dir::ensure_cache_dir_under(Path::new(CACHE_DIR), &dir)?;
    let lines: Vec<String> = hashes.iter().map(|(p, h)| format!("{p}\t{h}")).collect();
    fs::write(dir.join(format!("{name}.output_hashes")), lines.join("\n"))?;
    Ok(())
}

/// Load stored generation hashes as `HashMap<path, hash>`.
pub fn read_generation_hashes(name: &str) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let path = Path::new(CACHE_DIR)
        .join("hashes")
        .join(format!("{name}.output_hashes"));
    let content = fs::read_to_string(&path)?;
    Ok(content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(p, h)| (p.to_string(), h.to_string()))
        .collect())
}

/// Clear cache.
pub fn clear_cache() -> anyhow::Result<()> {
    let cache_dir = Path::new(CACHE_DIR);
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)?;
    }
    Ok(())
}

/// Show cache status information.
pub fn show_status() {
    let cache_dir = Path::new(CACHE_DIR);
    if !cache_dir.exists() {
        crate::bin_cli::output::line("No cache directory.");
        return;
    }

    crate::bin_cli::output::line("Cache directory: .alef/");

    let ir_path = cache_dir.join("ir.json");
    if ir_path.exists() {
        if let Ok(meta) = fs::metadata(&ir_path) {
            crate::bin_cli::output::line(format!("  ir.json: {} bytes", meta.len()));
        }
    } else {
        crate::bin_cli::output::line("  ir.json: not cached");
    }

    let hashes_dir = cache_dir.join("hashes");
    if hashes_dir.exists() {
        if let Ok(entries) = fs::read_dir(&hashes_dir) {
            let langs: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str().map(String::from)))
                .collect();
            if langs.is_empty() {
                crate::bin_cli::output::line("  language hashes: none");
            } else {
                crate::bin_cli::output::line(format!("  language hashes: {}", langs.join(", ")));
            }
        }
    } else {
        crate::bin_cli::output::line("  language hashes: none");
    }
}

#[cfg(test)]
#[path = "cache/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cache/committed_record_tests.rs"]
mod committed_record_tests;
