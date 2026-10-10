//! Cargo `[lints]` rendering: the workspace-lints-opt-out rationale banner and the
//! `[lints.clippy]`/`[lints.rust]` block emission that carries it.

use crate::core::config::ResolvedCrateConfig;

/// Rationale comment stamped immediately above every generated `[lints.clippy]` block
/// this module or its `scaffold::languages::*` callers emit, explaining why the crate
/// does not simply carry `[lints]\nworkspace = true` instead.
///
/// Baked into the generator rather than left for a consumer to hand-add, because these
/// binding-crate manifests (`crates/*-ffi`, `*-jni`, `*-node`, `*-php`, `*-py`,
/// `packages/r/**`, the Elixir NIF, the Ruby native extension) are `generated_header:
/// true` and rewritten in full whenever their content differs from what is on disk —
/// there is no comment-preserving merge for them the way [`merge_managed_toml`] exists
/// for `poly.toml`. A `~keep` marker only protects a comment from poly's own uncomment
/// pass; it does nothing against this full-file regeneration. The only comment that
/// reliably survives here is one alef itself emits on every run, which is what this
/// constant is for. Workspace lint inheritance is all-or-nothing, and consumer policies
/// can reject the `unsafe` required by C-ABI / PyO3 / napi / ext-php-rs / NIF boundaries.
/// Local binding policies avoid assuming which lints a consumer configures. ~keep
///
/// The emitted text carries its own `~keep` for the same reason any hand-authored
/// rationale in a consumer's tree does. Regeneration replaces this comment on every run,
/// but poly's uncomment pass runs *between* regenerations and strips any comment that is
/// not marked — so an unmarked rationale is deleted by the next `poly fmt`, and the
/// deletion lands in a commit that looks like unrelated formatting. Marking it also means
/// that where alef overwrites a consumer's own marked rationale here, what replaces it is
/// at least as durable as what it displaced. `strip_internal_doc_markers` does not reach
/// this text: it runs only inside `normalize_rustdoc`, on doc comments harvested from a
/// consumer's Rust source, never on scaffold-emitted TOML. ~keep
const CLIPPY_WORKSPACE_LINTS_RATIONALE: &str = "\
# This crate deliberately does not use `[lints]` / `workspace = true`: its C-ABI /\n\
# PyO3 / napi / ext-php-rs / NIF boundary requires `unsafe` code. Local lint tables\n\
# avoid inheriting workspace policies that could reject these boundaries, and select\n\
# the lints this binding can actually satisfy. ~keep";

/// Insert [`CLIPPY_WORKSPACE_LINTS_RATIONALE`] immediately above the first
/// `[lints.clippy]` header in `rendered` (which may also carry a preceding
/// `[lints.rust]` table). A no-op if `rendered` carries no `[lints.clippy]` header at
/// all, which [`CargoLintsConfig::render`]/[`CargoLintsConfig::clippy_block`] never
/// actually produce (the builtin deny defaults guarantee one), but this function does
/// not assume that invariant on its caller's behalf.
fn with_clippy_rationale(rendered: &str) -> String {
    match rendered.find("[lints.clippy]") {
        Some(index) => {
            let (before, from_header) = rendered.split_at(index);
            format!("{before}{CLIPPY_WORKSPACE_LINTS_RATIONALE}\n{from_header}")
        }
        None => rendered.to_string(),
    }
}

/// Like [`CargoLintsConfig::clippy_block`] but with [`CLIPPY_WORKSPACE_LINTS_RATIONALE`]
/// spliced in immediately above the `[lints.clippy]` header, for callers (e.g. the
/// Elixir NIF template) that build their own `[lints.rust]` table by hand and only pull
/// the clippy table from [`CargoLintsConfig`] directly rather than going through
/// [`cargo_lints_section`].
pub(crate) fn cargo_lints_clippy_block_with_rationale(config: &ResolvedCrateConfig) -> String {
    with_clippy_rationale(&config.cargo_lints.clippy_block())
}

/// Render `config.cargo_lints` for appending at the very END of a generated
/// Cargo.toml, after every dependency table.
///
/// `lints` is absent from cargo-sort's `DEF_TABLE_ORDER` (`package`, `workspace`,
/// `lib`, `bin`, `features`, `dependencies`, `build-dependencies`,
/// `dev-dependencies`), and cargo-sort appends every unlisted table after the
/// listed ones. Emitting `[lints.*]` between `[package]` and `[dependencies]`
/// therefore makes `cargo sort --check` reorder the manifest and fail it — under
/// the misleading message "Dependencies for <crate> are not sorted", even though
/// the dependency KEYS are already alphabetical. ~keep
///
/// The caller's template must already end with a newline; this returns
/// `\n{block}\n`, i.e. one blank separator line, the block, and the file's
/// trailing newline. Returns an empty string when no lints are configured, so
/// `...last-line\n{lints_section}"#` stays correct either way.
pub(crate) fn cargo_lints_section(config: &ResolvedCrateConfig) -> String {
    let rendered = with_clippy_rationale(&config.cargo_lints.render());
    if rendered.is_empty() {
        String::new()
    } else {
        format!("\n{rendered}\n")
    }
}
