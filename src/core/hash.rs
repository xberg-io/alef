//! Content hashing and generated-file headers.
//!
//! Every file produced by alef gets a standard header that identifies it as
//! generated, tells agents/developers how to fix issues, and embeds a blake3
//! hash so `alef verify` can detect staleness without external state.
//!
//! # Hash semantics
//!
//! Two *different* hashes exist, answering two different questions, and — unlike the
//! pre-split design documented below (see "Migration from v0.21.0" further down) —
//! neither is folded into the other:
//!
//! ```text
//! inputs_hash = blake3(
//!   "alef:inputs\0"
//!   || CODEGEN_FORMAT_VERSION || "\0"
//!   || sources_hash || "\0"
//!   || canonical_toml          ← parse + key-sort + re-serialize alef.toml
//! )
//!
//! alef:hash:<hex> = blake3(
//!   "alef:content\0" || file_content_without_hash_line
//! )
//! ```
//!
//! Where `sources_hash` is [`compute_sources_hash`] over the sorted Rust source
//! files alef parses to build the IR, and `canonical_toml` is the normalized
//! form of `alef.toml` (comments stripped, keys sorted, whitespace and line
//! endings normalized).
//!
//! **`alef:hash:<hex>` — [`compute_file_hash`] — is a pure function of the file's
//! own content.** It answers "has this file been hand-edited since alef last wrote
//! it?" and nothing else. A hand-edit to an emitted file (a reverted dependency
//! bump, a manually patched line) changes `file_content_without_hash_line`, so it
//! changes the embedded value and `alef verify` reports it stale.
//!
//! **`inputs_hash` — [`compute_inputs_hash`] — answers a different question, "are
//! this crate's generation inputs (sources + `alef.toml`) still what they were the
//! last time it was generated?", and is recorded exactly ONCE per crate, centrally,
//! by `cli::cache::record_inputs_hash` (the committed
//! `cli::cache::generation_record::GENERATION_RECORD` file) — never embedded in any
//! generated file.** `alef generate`/`alef all` write it after a successful run;
//! `alef verify` recomputes the current value and compares it to the recorded one.
//!
//! This split replaced a single-value design (every `alef:hash:` folded
//! `inputs_hash` directly into the per-file hash: `blake3("sources\0" ||
//! inputs_hash || "\0content\0" || content)`) that made the *embedded* hash change
//! whenever *any* source file or `alef.toml` key changed, project-wide, even for a
//! file whose own emitted bytes were unaffected. Measured across real consumer
//! repos, that meant single-key `alef.toml` edits committing thousands of files
//! whose diff was a one-line hash-only churn with no other change — 98.8% of all
//! generated-file diffs in one audit. `inputs_hash` still exists and is still
//! computed exactly the same way; it is simply no longer mixed into per-file
//! content hashing, so an unrelated input change no longer touches a file whose
//! own bytes did not change.
//!
//! One consequence of the split: recomputing `compute_file_hash` from current
//! on-disk bytes alone can no longer, by itself, prove a file is fresh with
//! respect to the *current* inputs — only that its bytes match what was last
//! written under *some* generation. That coarser question ("do this crate's
//! current inputs match what was last generated") is what the central record
//! answers instead, once per crate rather than once per file — see
//! `cli::cache::generation_record`'s module doc and `bin_cli::core_commands::verify`
//! for how the two checks combine.
//!
//! Post-generation formatter drift (rustfmt, ruff, rumdl-fmt, oxfmt, etc.) is
//! *not* a false positive, but not because content is excluded — it is
//! included. The reason is ordering: `generate::write::finalize_hashes`
//! stamps the hash *after* every formatter has already run
//! (`write_files_report` writes the header with no hash line; formatters run;
//! `finalize_hashes` then hashes the final, formatted bytes). So the embedded
//! hash reflects the post-format content from the start, and a later run of
//! the same formatter over already-formatted content is a no-op, not drift.
//! Routine alef crate releases do not change `inputs_hash` because the alef crate
//! version (`ALEF_REV`) is not an input to it. Separately, bumping the
//! `[workspace] alef_version` pin inside `alef.toml` — the standard consumer upgrade step —
//! also does not change it: that one key is stripped from `canonical_toml` before
//! hashing, because nothing in codegen branches on it (see `strip_alef_version_pin`).
//! Every other key in `alef.toml` is still a real input, so a genuine config change still
//! moves `inputs_hash` and is caught by the central-record comparison — without touching
//! any individual file's `alef:hash:` stamp.
//!
//! `alef verify` (`bin_cli::helpers::verify_walk` / `stale_among`) reads each
//! on-disk alef-owned file, strips its `alef:hash:` line, recomputes
//! [`compute_file_hash`] over the *actual on-disk bytes* alone, and compares
//! that to the embedded line — a pure content check, no `alef.toml` or source
//! read required per file. Separately, and only once per crate, it recomputes
//! `inputs_hash` and compares it to the crate's recorded value. Both are
//! read-only — no regeneration, no writes.
//!
//! What this mechanism cannot see is scoped precisely by two other things, not
//! by the hash formula: a file whose extension isn't in
//! `bin_cli::helpers::VERIFY_SCAN_EXTENSIONS`/`VERIFY_SCAN_FILENAMES` is
//! never opened at all, and a file whose format cannot carry a comment marker
//! (`.json`, `.jar`, lockfiles) is tracked only by path presence in
//! `cli::cache::OWNERSHIP_MANIFEST`, which has no content hash to compare —
//! for those, `alef verify` can confirm alef once wrote the path but not that
//! its current bytes still match. Both gaps are enumerated, not this hash
//! design; see the doc comments at the definitions above.
//!
//! # Migration from v0.10.1 — v0.20.x
//!
//! Pre-v0.21.0 alef embedded `blake3(sources_hash || file_content_without_hash_line)` —
//! already content-inclusive, just without the `inputs_hash` domain
//! separation and canonicalized-`alef.toml` mixing described above. Any file
//! regenerated with v0.21.0+ will carry a new hash value; `alef verify` from
//! v0.21.0+ rejects old-format hashes. Run `alef generate` once after
//! upgrading to stamp all files with the new format.
//!
//! # Migration from v0.21.0 — v0.71.x (the inputs-hash split)
//!
//! Every one of those releases embedded `inputs_hash` directly into `alef:hash:`, as
//! described above. Any file last stamped by one of them carries a hash value this
//! version's [`compute_file_hash`] cannot reproduce, by construction — the two
//! recipes hash different byte streams over the same content, so they collide only
//! by chance. `alef verify` therefore reports **every** previously-stamped file as
//! stale on the first run after upgrading to this version, exactly once, the same
//! way the v0.21.0 migration above did for its own recipe change. This is
//! `CODEGEN_FORMAT_VERSION`'s bump policy operating as designed (see its doc): a
//! structural change to the stamp recipe is exactly what that constant exists to
//! flag, and this module's `stamp_recipe_tests` golden vectors enforce that the bump
//! happens in the same commit as the recipe change. Run `alef generate`/`alef all` once
//! after upgrading — this both re-stamps every file with the new content-only
//! recipe (most files re-stamp with unchanged bytes, since the fix is precisely
//! that content didn't need to change) and writes the first
//! `cli::cache::generation_record` entry for each crate, after which `alef verify`
//! is clean again and future unrelated input changes no longer cause any per-file
//! churn.

const HASH_PREFIX: &str = "alef:hash:";
const DEFAULT_REGENERATE_COMMAND: &str = "alef generate";
const DEFAULT_VERIFY_COMMAND: &str = "alef verify";

/// The first line of the standard alef header, emitted by every [`header`]/[`header_for_config`]
/// call — the single source for the `"auto-generated by alef"` spelling, so a marker-spelling test
/// can assert against the real value instead of a hand-copied duplicate. ~keep
pub(crate) const STANDARD_HEADER_LINE: &str = "This file is auto-generated by alef — DO NOT EDIT.";

/// Marker line for the umbrella `RUST_BRIDGE_C_H` header Swift's binding generation writes.
///
/// Shared between `backends::swift::gen_bindings::bridge_artifacts::emit_swift_bridge_files` and
/// `scaffold::languages::swift::render_rust_bridge_c_header`, which independently build the same
/// header text on two different code paths (fresh generation vs. reconciling an existing file) —
/// hoisted here so the two can never spell it differently. ~keep
pub(crate) const SWIFT_C_UMBRELLA_HEADER_MARKER: &str = "// Auto-generated by alef — do not edit by hand.";

/// Marker line for the generated `CITATION.cff` header, written by
/// `cli::pipeline::version_text::render_citation_cff`. ~keep
pub(crate) const CITATION_CFF_HEADER_LINE: &str =
    "# This file is generated by alef sync-versions; do not edit by hand.";

/// Marker line shared by every backend that self-marks instead of prepending [`header`]/
/// [`header_for_config`] — Dart, Gleam, Kotlin (JVM, Multiplatform, and Android), Swift, and the
/// Gleam/Kotlin-Android e2e project generators each write this exact text at one or more call
/// sites; hoisted here so none of them can drift to a spelling `content_has_alef_marker` would
/// still accept but this constant no longer names. ~keep
pub(crate) const SELF_MARKING_HEADER_LINE: &str = "// Generated by alef. Do not edit by hand.";

/// [`inject_stamp_line`]/[`extract_stamp`] key for the opaque-handle ABI
/// generation a file was produced against (pointer vs. `u64` registry key,
/// or any future representation). Backends that emit an FFI-consuming
/// artifact encoding that representation (the FFI header/glue itself, and
/// any binding backend whose template hardcodes the handle's native type
/// instead of deriving it live from the FFI header at build time — zig's
/// `_handle: u64` fields and C#'s `IntPtr`-based `SafeHandle` wrapper are the
/// known cases) should stamp their generated files with this key so a
/// verify-time consistency check can catch two different ABI generations
/// coexisting in one tree — the hazard is silent because pointer and `u64`
/// are both 8 bytes on 64-bit targets, so a straddle compiles and then
/// misinterprets a pointer as a registry index at runtime. No backend calls
/// this yet; the constant exists so the first one to do so and the verify
/// side (`crate::bin_cli::helpers::find_stamp_disagreement`) agree on the
/// key without a second source of truth. ~keep
pub const HANDLE_ABI_STAMP_KEY: &str = "handle-abi";

/// Comment style for the generated header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `// line comment`  (Rust, Go, Java, C#, TypeScript, C, PHP)
    DoubleSlash,
    /// `# line comment`   (Python, Ruby, Elixir, R, TOML, Shell, Makefile)
    Hash,
    /// `/* block comment */` (C headers)
    Block,
    /// `; line comment`   (INI-family formats -- `.npmrc`, which pnpm/npm's `ini` parser reads
    /// with `;` as the conventional comment prefix, alongside `#`.)
    Semicolon,
}

/// Return the standard alef header as a comment block.
///
/// ```text
/// // This file is auto-generated by alef — DO NOT EDIT.
/// // To regenerate: alef generate
/// // To verify freshness: alef verify
/// ```
pub fn header(style: CommentStyle) -> String {
    render_header(style, &default_header_body())
}

/// Return the standard alef header using metadata from a resolved crate config.
pub fn header_for_config(style: CommentStyle, config: &crate::core::config::ResolvedCrateConfig) -> String {
    let header_config = config.scaffold.as_ref().and_then(|s| s.generated_header.as_ref());
    let body = match header_config {
        Some(header) => {
            let regenerate = header
                .regenerate_command
                .as_deref()
                .unwrap_or(DEFAULT_REGENERATE_COMMAND);
            let verify = header.verify_command.as_deref().unwrap_or(DEFAULT_VERIFY_COMMAND);
            let issues_url = header.issues_url.as_deref().or_else(|| configured_header_url(config));
            header_body(regenerate, verify, issues_url)
        }
        None => header_body(
            DEFAULT_REGENERATE_COMMAND,
            DEFAULT_VERIFY_COMMAND,
            configured_header_url(config),
        ),
    };
    render_header(style, &body)
}

fn header_body(regenerate: &str, verify: &str, issues_url: Option<&str>) -> String {
    let mut body = format!(
        "{STANDARD_HEADER_LINE}\n\
To regenerate: {regenerate}\n\
To verify freshness: {verify}"
    );
    if let Some(url) = issues_url {
        body.push_str(&format!("\nIssues & docs: {url}"));
    }
    body
}

fn configured_header_url(config: &crate::core::config::ResolvedCrateConfig) -> Option<&str> {
    config
        .package_metadata
        .as_ref()
        .and_then(|m| m.issues.as_deref().or(m.documentation.as_deref()))
}

fn default_header_body() -> String {
    header_body(DEFAULT_REGENERATE_COMMAND, DEFAULT_VERIFY_COMMAND, None)
}

fn render_header(style: CommentStyle, body: &str) -> String {
    match style {
        CommentStyle::DoubleSlash => body.lines().map(|l| format!("// {l}\n")).collect(),
        CommentStyle::Hash => body.lines().map(|l| format!("# {l}\n")).collect(),
        CommentStyle::Semicolon => body.lines().map(|l| format!("; {l}\n")).collect(),
        CommentStyle::Block => {
            let mut out = String::from("/*\n");
            for line in body.lines() {
                out.push_str(&format!(" * {line}\n"));
            }
            out.push_str(" */\n");
            out
        }
    }
}

/// The substring shared by every marker spelling alef emits or must recognize on read-back,
/// matched case-insensitively (see [`line_has_marker`]). A single substring subsumes every
/// capitalization variant produced across backends -- `"auto-generated by alef"`, `"Auto-generated
/// by alef"`, `"Generated by alef"`, `"generated by alef"` -- and also Go's own `"Code generated
/// ... by alef"` convention, which alef should honour rather than fight. Previously this was two
/// case-sensitive literals (`"auto-generated by alef"` / `"Generated by alef"`); that check missed
/// every other capitalization a backend or a hand-adopted file might use, silently refusing to ever
/// stamp or verify the file again. ~keep
const MARKER_SUBSTRING: &str = "generated by alef";

/// Additional contiguous substrings that are unambiguous alef ownership claims but do not contain
/// [`MARKER_SUBSTRING`].
///
/// A consumer census of every marker alef actually emits found 18 distinct forms; 16 contain
/// `generated by alef` outright and differ only in capitalisation and trailing punctuation. The two
/// below do not, and both land in shipped output rather than throwaway files -- a Homebrew
/// `Brewfile` that says "managed by" instead of "generated by", and an R entrypoint whose header
/// names alef and says "do not edit" without ever putting the two words adjacent. A human reading
/// either sees an obvious ownership marker; a contiguous substring test does not.
///
/// These are recognised rather than merely re-spelled at the emitter because a file already on disk
/// in a consumer tree carries the old spelling, and a predicate that refuses it also refuses to
/// rewrite it -- the file would be permanently unadoptable by the very fix meant to free it. ~keep
const ALTERNATE_MARKER_SUBSTRINGS: &[&str] = &["managed by alef", "regenerate with `alef"];

/// Case-insensitive ASCII substring search for `needle` within `haystack`.
///
/// Used instead of `haystack.to_lowercase().contains(needle)` to avoid allocating a lowercased
/// copy of every scanned line: this only ever runs against the first [`MARKER_SCAN_LINES`] lines
/// of a file, so the cost is already bounded, but a byte-window scan with
/// [`u8::eq_ignore_ascii_case`] gets the same correctness for free. Marker text is always plain
/// ASCII, so ASCII-only case-folding is sufficient here. ~keep
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Negation cues that, found as the single word immediately before a matched marker substring,
/// mean the phrase is being denied rather than claimed as an ownership marker (`"This file is
/// NOT generated by alef"`). Deliberately narrow -- see the module doc on
/// [`immediately_preceded_by_negation`] and `tests::marker_negation` for exactly which negated
/// forms this catches and which it does not. Over-widening this list risks the opposite,
/// strictly worse failure: a false NEGATIVE that makes alef refuse to ever manage a file it
/// genuinely authored, versus this false POSITIVE's cost of one avoidable write refusal a human
/// reviews. ~keep
const NEGATION_CUES: &[&str] = &[
    "not", "never", "isn't", "wasn't", "aren't", "weren't", "doesn't", "didn't",
];

/// Whether the single word immediately before byte offset `match_start` in `line` is one of
/// [`NEGATION_CUES`], case-insensitively.
///
/// Deliberately scoped to exactly one adjacent word, with no tolerance for intervening
/// punctuation or words -- `"is not generated by alef"` is caught, `"was not, in any real
/// sense, generated by alef"` is not (documented gap; see `tests::marker_negation`). A wider
/// window (scanning the whole line for a negation word anywhere) would defeat alef's own
/// standard header, which pairs the marker with "DO NOT EDIT" on the same line. ~keep
fn immediately_preceded_by_negation(line: &str, match_start: usize) -> bool {
    let prefix = line[..match_start].trim_end_matches(|character: char| character.is_ascii_whitespace());
    let word_start = prefix
        .rfind(|character: char| character.is_ascii_whitespace())
        .map_or(0, |index| index + 1);
    let word = &prefix[word_start..];
    NEGATION_CUES.iter().any(|cue| word.eq_ignore_ascii_case(cue))
}

/// Case-insensitive ASCII substring search for `needle` within `haystack`, true only when at
/// least one match is not [`immediately_preceded_by_negation`].
///
/// [`contains_ignore_ascii_case`]'s marker-detection call sites route through this instead: a
/// bare substring match accepted `"This file is NOT generated by alef"` as an ownership claim,
/// which is a data-loss path -- that claim decides whether alef's write guard is willing to
/// overwrite the file (`write_files_report`, `crate::cli::pipeline::generate::write`). ~keep
fn contains_unnegated_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() {
        return true;
    }
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }
    haystack_bytes
        .windows(needle_bytes.len())
        .enumerate()
        .filter(|(_, window)| window.eq_ignore_ascii_case(needle_bytes))
        .any(|(start, _)| !immediately_preceded_by_negation(haystack, start))
}

/// Whether a single line carries [`MARKER_SUBSTRING`], case-insensitively, and not merely as a
/// negated denial (see [`contains_unnegated_ignore_ascii_case`]).
///
/// The one predicate every marker-scanning function in this module shares --
/// [`content_has_alef_marker`], [`inject_hash_line`], [`inject_stamp_line`], and
/// [`generated_hash_line`] (which backs [`extract_hash`]/[`strip_hash_line`]) -- so a line that one
/// of them treats as a marker is treated as one by all of them. They must agree: a marker
/// `content_has_alef_marker` accepts but `inject_hash_line` cannot find is a file claimed by `alef
/// verify` and never stamped, permanently stale. ~keep
fn line_has_marker(line: &str) -> bool {
    contains_unnegated_ignore_ascii_case(line, MARKER_SUBSTRING)
        || ALTERNATE_MARKER_SUBSTRINGS
            .iter()
            .any(|alternate| contains_unnegated_ignore_ascii_case(line, alternate))
}

/// Whether `content` carries an alef header marker in its leading lines.
///
/// This is the single definition of "alef owns this file's provenance". Both
/// the stamping pass and the callers that assemble its input set must agree on
/// it, otherwise a marked file can be emitted, claimed by `alef verify`, and
/// never stamped — leaving it permanently stale. ~keep
pub fn content_has_alef_marker(content: &str) -> bool {
    content.lines().take(MARKER_SCAN_LINES).any(line_has_marker)
}

/// A leading line that looks like a failed attempt at an alef marker -- it mentions both "alef"
/// and "generated" (independently, case-insensitively) but doesn't satisfy [`line_has_marker`] --
/// without actually being one. Surfaced so a write-time refusal can name what's wrong instead of
/// looking identical to a plain hand-written file that never tried. Returns `None` when nothing in
/// the scan window is even close (including when a real marker is already present: a matching line
/// is never a near miss of itself). ~keep
pub(crate) fn near_miss_marker(content: &str) -> Option<&str> {
    content.lines().take(MARKER_SCAN_LINES).find(|line| {
        !line_has_marker(line)
            && contains_ignore_ascii_case(line, "alef")
            && contains_ignore_ascii_case(line, "generated")
    })
}

/// How far into a file the header marker is searched for.
///
/// Deliberately 10 and not 11, even though `poly`'s built-in generated-file skip scans the first
/// **11** lines for a `<tool>:hash:<40-or-64 hex>` line. The two bounds measure different things:
/// this one bounds where the *prose* marker may sit, and [`inject_hash_line`] always writes the
/// `alef:hash:` line on the line *immediately after* it. A marker on lines 1..=10 therefore puts
/// its hash on lines 2..=11 — exactly poly's window, with nothing to spare. Widening this to 11
/// would let a marker on line 11 push its hash to line 12, where poly no longer sees it, so alef
/// would claim a file poly still reformats: the ping-pong this bound exists to prevent,
/// re-introduced at the boundary. If poly's window ever widens, this may follow it — never lead
/// it. ~keep
const MARKER_SCAN_LINES: usize = 10;

/// How many leading lines `poly`'s built-in generated-file skip reads when looking for a
/// `<tool>:hash:<40-or-64 hex>` line.
///
/// Measured against poly 0.21.6 by bisection, with `[discovery] exclude = []` and nothing else
/// configured: a marker on line 11 is skipped, one on line 12 is not. The skip is compiled into
/// the poly binary — no `poly.toml` key drives it, and the `<tool>` segment is not alef-specific —
/// so alef cannot negotiate this bound, only stay inside it. Held as a constant rather than
/// described in prose so [`MARKER_SCAN_LINES`]'s relationship to it is checkable by a test. ~keep
pub const POLY_GENERATED_SCAN_LINES: usize = 11;

/// The last line number (1-based) on which [`inject_hash_line`] can place an `alef:hash:` line,
/// given a marker anywhere in [`MARKER_SCAN_LINES`]. Must stay `<= POLY_GENERATED_SCAN_LINES` or
/// alef claims files poly still reformats.
#[must_use]
pub const fn deepest_hash_line() -> usize {
    MARKER_SCAN_LINES + 1
}

/// Blake3 hash of a content string, returned as hex.
///
/// Used by the IR / language caches and any caller that needs a hash of an
/// in-memory string. **Not used for the embedded `alef:hash:` header** — that
/// is computed by [`compute_file_hash`].
pub fn hash_content(content: &str) -> String {
    hash_bytes(content.as_bytes())
}

/// Blake3 hash of arbitrary bytes, returned as hex.
///
/// The binary twin of [`hash_content`], for output alef emits base64-encoded and writes
/// as bytes (`cli::pipeline::generate::binary`). Such a file can hold no `alef:hash:`
/// marker and has no line diff, so a digest of both sides is the only reviewable
/// statement that can be made about it — see `cli::commands::adopt::BinaryFacts`. ~keep
pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Compute a stable hash over the Rust source files that alef extracts.
///
/// This is the "source side" of the per-file verify hash. Sources are sorted
/// by path so the hash is stable regardless of ordering in
/// `alef.toml`'s `[crate].sources`. The path is mixed in alongside the
/// content because the same byte-content at a different path produces
/// different IR (the `rust_path` on extracted types differs).
///
/// Used by [`compute_file_hash`]; not by itself the value embedded in any
/// file header.
///
/// # Errors
/// Returns an error if any source file is missing or unreadable.
pub fn compute_sources_hash(sources: &[std::path::PathBuf]) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut sorted: Vec<&std::path::PathBuf> = sources.iter().collect();
    sorted.sort();
    for source in sorted {
        let content = std::fs::read(source)?;
        hasher.update(b"src\0");
        hasher.update(normalize_source_path(source).as_bytes());
        hasher.update(b"\0");
        hasher.update(&content);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Compute a stable hex-encoded Blake3 hash over all Rust source files
/// belonging to a [`crate::core::config::resolved::ResolvedCrateConfig`].
///
/// Returns a hex string so callers can feed the result directly to
/// [`compute_file_hash`], matching [`compute_sources_hash`]'s return type.
///
/// The hash covers the union of:
/// - `crate_cfg.sources` (direct sources on the crate)
/// - every `source_crates[*].sources` entry
///
/// All paths are sorted before hashing so the result is independent of the
/// order they appear in `alef.toml`.  The path string is mixed in alongside
/// the file content because the same byte-content at a different path produces
/// different IR (the `rust_path` on extracted types differs).
///
/// # Phase 3 migration note
///
/// Phase 3 callers should migrate from the per-file `compute_sources_hash` to
/// this function when they have a `ResolvedCrateConfig` available, so that
/// multi-source-crate workspaces produce a single stable hash across all
/// contributing source files.
///
/// # Errors
///
/// Returns an error if any source file is missing or unreadable.
pub fn compute_crate_sources_hash(
    crate_cfg: &crate::core::config::resolved::ResolvedCrateConfig,
) -> std::io::Result<String> {
    let mut all_sources: Vec<&std::path::PathBuf> = Vec::new();

    for src in &crate_cfg.sources {
        all_sources.push(src);
    }
    for sc in &crate_cfg.source_crates {
        for src in &sc.sources {
            all_sources.push(src);
        }
    }

    all_sources.sort();
    all_sources.dedup();

    let mut hasher = blake3::Hasher::new();
    for source in all_sources {
        let content = std::fs::read(source)?;
        hasher.update(b"src\0");
        hasher.update(normalize_source_path(source).as_bytes());
        hasher.update(b"\0");
        hasher.update(&content);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Compute the generation-inputs hash that alef embeds in each generated file.
///
/// The hash covers the [`CODEGEN_FORMAT_VERSION`] constant (stable across
/// crate releases — only bumped for breaking codegen changes), the Rust
/// source fingerprint, and a **canonical normalized form** of `alef.toml`
/// (parsed and re-serialized as TOML, stripping comments, whitespace churn,
/// key-order differences, CRLF line endings, and the `[workspace] alef_version`
/// pin — see [`strip_alef_version_pin`]). It does **not** include the
/// emitted file content, so post-generation formatter rewrites (rustfmt, ruff,
/// rumdl-fmt, oxfmt, …) never invalidate the embedded hash. It also does
/// **not** include the alef crate version (`ALEF_REV`), so upgrading alef
/// between releases — including bumping the `alef_version` pin in `alef.toml`, the
/// standard way to record that upgrade — does not mass-invalidate client bindings.
///
/// - **Generate**: compute once per run, inject into every generated file header.
/// - **Verify**: re-derive from the current inputs, compare to the embedded line.
///   No file content is read or hashed — pure input comparison.
///
/// # Arguments
///
/// * `sources_hash` — output of [`compute_sources_hash`] or
///   [`compute_crate_sources_hash`] for the crate being generated.
/// * `alef_toml_bytes` — raw bytes of the `alef.toml` config file. Pass an
///   empty slice when the config path is unavailable (e.g. in tests); the hash
///   will still change when `sources_hash` changes.
///
/// [`CODEGEN_FORMAT_VERSION`]: crate::core::template_versions::precommit::CODEGEN_FORMAT_VERSION
pub fn compute_inputs_hash(sources_hash: &str, alef_toml_bytes: &[u8]) -> String {
    let version = crate::core::template_versions::precommit::CODEGEN_FORMAT_VERSION;
    let normalized_toml = normalize_toml_bytes(alef_toml_bytes);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"alef:inputs\0");
    hasher.update(version.as_bytes());
    hasher.update(b"\0");
    hasher.update(sources_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized_toml.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Normalize raw `alef.toml` bytes into a canonical string for hashing.
///
/// Parses the bytes as TOML, recursively sorts table keys, then re-serializes.
/// This strips comments, normalizes whitespace, eliminates CRLF vs LF
/// differences, and makes key ordering deterministic. Falls back to:
/// - empty string for empty / non-UTF-8 input
/// - raw UTF-8 string if the bytes are valid UTF-8 but not parseable as TOML
///   (avoids silently swallowing malformed configs while still producing a
///   deterministic hash for the data that is present)
fn normalize_toml_bytes(bytes: &[u8]) -> String {
    let Ok(s) = std::str::from_utf8(bytes) else {
        return String::new();
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match toml::from_str::<toml::Value>(trimmed) {
        Ok(value) => {
            let value = strip_alef_version_pin(value);
            let sorted = sort_toml_value(value);
            toml::to_string(&sorted).unwrap_or_default()
        }
        Err(_) => trimmed.to_string(),
    }
}

/// Remove `[workspace] alef_version` before hashing `alef.toml`.
///
/// The pin only feeds `cli::version_pin::check_alef_toml_version`, which logs a mismatch
/// warning and never branches codegen — confirmed by grepping every read of
/// `WorkspaceConfig::alef_version` in `src/`. Folding it into `inputs_hash` therefore rehashed
/// every generated file on the standard upgrade workflow (bump the pin in `alef.toml`) even
/// though emitted content never changed. Stripping only this one key, rather than the field's
/// whole containing table, keeps any other future `[workspace]` key an input. ~keep
fn strip_alef_version_pin(mut value: toml::Value) -> toml::Value {
    if let Some(workspace) = value.get_mut("workspace").and_then(toml::Value::as_table_mut) {
        workspace.remove("alef_version");
    }
    value
}

/// Recursively sort the keys of every TOML table so that key-ordering
/// differences in `alef.toml` do not produce different hashes.
fn sort_toml_value(value: toml::Value) -> toml::Value {
    match value {
        toml::Value::Table(map) => {
            let mut pairs: Vec<(String, toml::Value)> = map.into_iter().collect();
            pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
            let mut sorted = toml::map::Map::new();
            for (k, v) in pairs {
                sorted.insert(k, sort_toml_value(v));
            }
            toml::Value::Table(sorted)
        }
        toml::Value::Array(arr) => toml::Value::Array(arr.into_iter().map(sort_toml_value).collect()),
        other => other,
    }
}

/// Serialize `value` to TOML with every table's keys in a stable, sorted order.
///
/// `value` typically comes from `toml::Value::try_from(&some_serde_struct)`. Use this instead
/// of `toml::to_string` directly whenever the struct being serialized carries a `HashMap` field:
/// serde serializes a `HashMap` in that map's own randomly-seeded-per-process iteration order,
/// so two structurally identical structs can serialize to different byte strings across runs.
/// Round-tripping through `toml::Value` and sorting recursively removes that source of
/// nondeterminism from the resulting string -- a `BTreeMap` field is unaffected by this either
/// way, since it was already stable. ~keep
pub(crate) fn canonical_toml_string(value: toml::Value) -> Result<String, toml::ser::Error> {
    toml::to_string(&sort_toml_value(value))
}

/// Normalize a source-file path for stable hashing across machines and
/// operating systems.
///
/// Attempts to produce a repo-relative path by stripping the current working
/// directory prefix. Falls back to the original path if relativization fails
/// (e.g. the file lives outside the working directory, or `current_dir()`
/// is unavailable). In both cases `\\` is replaced with `/` so that hashes
/// are stable across Windows and POSIX builds of the same repo.
fn normalize_source_path(path: &std::path::Path) -> String {
    let relative = std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(&cwd).ok().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| path.to_path_buf());
    relative.to_string_lossy().replace('\\', "/")
}

/// Compute the per-file verify hash that alef embeds in each generated file.
///
/// A pure function of `content` alone — any pre-existing `alef:hash:` line is
/// stripped before hashing so the function is idempotent. Deliberately does **not**
/// take `inputs_hash` or `sources_hash`: see this module's doc for why mixing
/// generation-inputs identity into every file's own stamp caused whole-tree
/// provenance churn, and where the inputs-freshness question now lives instead
/// (`cli::cache::generation_record`).
#[doc(hidden)]
pub fn compute_file_hash(content: &str) -> String {
    let stripped = strip_hash_line(content);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"alef:content\0");
    hasher.update(stripped.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Every `(prefix, suffix)` pair alef wraps a stamp line in, one per comment syntax it emits
/// markers in.
///
/// The emit side ([`stamp_delimiters_for_marker`]) selects one *entry of this table*, and the
/// read-back side ([`parse_generated_hash_line`], [`extract_stamp`]) tries *every* entry in it,
/// so a stamp shape alef can write is by construction one it can also recognize. The two were
/// previously separate literal lists and drifted: [`CommentStyle::Semicolon`] was added for
/// `.npmrc` on the header side without a matching arm here, so an INI file got a `; ` prose
/// header and a `// ` hash line -- and `//` is not a comment in INI, it is the prefix of a
/// registry-auth key. Recognition intentionally accepts any entry regardless of which syntax the
/// file's own marker line uses, which is what lets a file stamped under an older, wrong prefix
/// keep being read back as alef-generated. ~keep
const STAMP_DELIMITERS: &[(&str, &str)] = &[("<!-- ", " -->"), ("// ", ""), ("# ", ""), ("; ", ""), (" * ", "")];

/// The delimiters a stamp line placed under `marker_line` must use, chosen from
/// [`STAMP_DELIMITERS`] by the comment syntax the marker line itself opens with.
///
/// Falls back to `// ` for a marker in a syntax this function does not recognize, matching the
/// pre-existing behavior for such lines.
fn stamp_delimiters_for_marker(marker_line: &str) -> (&'static str, &'static str) {
    let trimmed = marker_line.trim();
    let opener = if trimmed.starts_with("<!--") {
        "<!-- "
    } else if trimmed.starts_with("//") {
        "// "
    } else if trimmed.starts_with('#') {
        "# "
    } else if trimmed.starts_with(';') {
        "; "
    } else if trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.ends_with("*/") {
        " * "
    } else {
        "// "
    };
    STAMP_DELIMITERS
        .iter()
        .copied()
        .find(|&(prefix, _)| prefix == opener)
        .unwrap_or(("// ", ""))
}

/// Inject an `alef:hash:<hex>` line immediately after the first header marker
/// line found in the first [`MARKER_SCAN_LINES`] lines. The comment syntax is
/// inferred from the marker line itself, via [`stamp_delimiters_for_marker`].
///
/// The window must stay tied to [`MARKER_SCAN_LINES`] rather than repeat its value: a marker this
/// function declines to stamp is still one [`content_has_alef_marker`] claims, and a claimed but
/// unstamped file is invisible to poly's hash-keyed skip. See that constant for why 10 is the
/// value both sides carry. ~keep
///
/// If no marker line is found, the content is returned unchanged.
pub fn inject_hash_line(content: &str, hash: &str) -> String {
    let mut result = String::with_capacity(content.len() + 80);
    let mut injected = false;

    for (i, line) in content.lines().enumerate() {
        result.push_str(line);
        result.push('\n');

        if !injected && i < MARKER_SCAN_LINES && line_has_marker(line) {
            let (prefix, suffix) = stamp_delimiters_for_marker(line);
            result.push_str(&format!("{prefix}{HASH_PREFIX}{hash}{suffix}"));
            result.push('\n');
            injected = true;
        }
    }

    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Inject an arbitrary `alef:<key>:<value>` stamp line immediately after the
/// first header marker line found in the first [`MARKER_SCAN_LINES`] lines.
///
/// Generalizes the single-purpose `alef:hash:` mechanism ([`inject_hash_line`])
/// for other per-file, plaintext-extractable stamps — for example an ABI
/// generation marker that a cross-artifact consistency check (comparing an FFI
/// header against a binding backend's opaque-handle template) can read
/// directly, without re-deriving [`compute_inputs_hash`]'s generation-inputs
/// fingerprint the way `alef:hash:` requires. `key` must not itself contain a
/// colon. Call this *before* [`inject_hash_line`] when both are used on the
/// same content, so the final `alef:hash:` line reflects the stamped content
/// rather than the other way around. If no marker line is found, the content
/// is returned unchanged.
///
/// Shares [`stamp_delimiters_for_marker`] with [`inject_hash_line`] rather than repeating its
/// comment-style detection. The duplication this replaced was deliberate -- so a then-unused
/// primitive could not perturb `alef:hash:` injection -- but it is the mechanism by which the
/// two sides drifted apart for INI files, and a stamp line whose prefix is not a comment in its
/// own format is a defect in either function. ~keep
pub fn inject_stamp_line(content: &str, key: &str, value: &str) -> String {
    let mut result = String::with_capacity(content.len() + key.len() + value.len() + 16);
    let mut injected = false;

    for (i, line) in content.lines().enumerate() {
        result.push_str(line);
        result.push('\n');

        if !injected && i < MARKER_SCAN_LINES && line_has_marker(line) {
            let (prefix, suffix) = stamp_delimiters_for_marker(line);
            result.push_str(&format!("{prefix}alef:{key}:{value}{suffix}"));
            result.push('\n');
            injected = true;
        }
    }

    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Extract the value of an `alef:<key>:<value>` stamp line injected by
/// [`inject_stamp_line`], searching the first [`MARKER_SCAN_LINES`] lines.
///
/// Returns `None` when the header marker itself is absent, or when no line in
/// the scan window matches `alef:<key>:`. Unlike [`extract_hash`], the value is
/// returned as-is with no hex-digit constraint, since a generation marker need
/// not be a hash (a small integer version is the expected case).
pub fn extract_stamp(content: &str, key: &str) -> Option<String> {
    let stamp = format!("alef:{key}:");

    let mut past_marker = false;
    for (line_index, line) in content.lines().enumerate() {
        if line_index >= MARKER_SCAN_LINES {
            break;
        }
        if !past_marker {
            past_marker = line_has_marker(line);
            continue;
        }
        if let Some(value) = strip_stamp_delimiters(line, &stamp) {
            return Some(value.to_string());
        }
    }
    None
}

/// Strip any [`STAMP_DELIMITERS`] pair plus `stamp` from `line`, yielding the stamp's value.
fn strip_stamp_delimiters<'a>(line: &'a str, stamp: &str) -> Option<&'a str> {
    STAMP_DELIMITERS.iter().find_map(|&(prefix, suffix)| {
        line.strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix(stamp))
            .and_then(|value| value.strip_suffix(suffix))
    })
}

fn generated_hash_line(content: &str) -> Option<(usize, &str)> {
    let mut lines = content.lines().enumerate().peekable();
    while let Some((line_index, line)) = lines.next() {
        if line_index >= MARKER_SCAN_LINES {
            break;
        }
        if !line_has_marker(line) {
            continue;
        }

        let (hash_line_index, hash_line) = lines.peek().copied()?;
        if let Some(hash) = parse_generated_hash_line(hash_line) {
            return Some((hash_line_index, hash));
        }
    }
    None
}

/// Accepts *every* [`STAMP_DELIMITERS`] shape, not only the one the file's own marker line would
/// select today. That is deliberate and load-bearing for upgrades: a file stamped by an older
/// alef whose emit side picked a different prefix (INI files carried `// alef:hash:` under a
/// `; ` marker before the two sides were unified) must still read back as alef-generated, or the
/// whole tree would look hand-written to the ownership guard on the first run after the fix. ~keep
fn parse_generated_hash_line(line: &str) -> Option<&str> {
    let hash = strip_stamp_delimiters(line, HASH_PREFIX)?;
    (!hash.is_empty() && hash.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(hash)
}

/// Whether every non-blank line of `prefix` is provenance text rather than file body.
///
/// "Provenance" is exactly what a generator prepends above content it did not itself author:
/// a shebang, an alef ownership marker, and the `alef:hash:` stamp. A census of the marker
/// forms actually emitted (Dart/Swift/Kotlin self-marking headers included) found no other
/// line kind above the body, and descriptive comments that *follow* the stamp are body, not
/// provenance -- they are content the generator wrote and a drifted file can differ in.
///
/// Callers use this to bound a "the generated bytes are the existing bytes plus a header"
/// inference. Without the bound, a plain suffix test also matches a file whose body has been
/// truncated -- or emptied entirely, since every string ends with `""` -- and reports it as
/// converged, silently restoring deleted content under a verdict that says nothing changed. ~keep
pub(crate) fn is_provenance_only_prefix(prefix: &str) -> bool {
    prefix.lines().all(|line| {
        let trimmed = line.trim();
        trimmed.is_empty()
            || trimmed.starts_with("#!")
            || line_has_marker(line)
            || parse_generated_hash_line(trimmed).is_some()
    })
}

/// Extract the hash from the generated stamp following an alef header marker.
pub fn extract_hash(content: &str) -> Option<String> {
    generated_hash_line(content).map(|(_, hash)| hash.to_string())
}

/// Strip the generated hash stamp from content (for fallback comparison).
pub fn strip_hash_line(content: &str) -> String {
    let Some((hash_line_index, _)) = generated_hash_line(content) else {
        return content.to_string();
    };

    let mut result = String::with_capacity(content.len());
    for (line_index, line) in content.lines().enumerate() {
        if line_index == hash_line_index {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod stamp_recipe_tests;
#[cfg(test)]
mod stamp_syntax_tests;
#[cfg(test)]
mod tests;
