//! Explicit, justified exclusions from the `generated_output_downstream_gate` poly_fmt lane.
//!
//! `poly_fmt_lane` (in the parent file) runs `poly fmt --check .` over the whole emitted
//! tree. Every path it examines is expected to be alef-generated output that already
//! matches poly's own formatting, `poly fmt`'s hash-stamped-generated skip aside. A handful
//! of paths in the emitted tree are neither: they are written directly by an external build
//! tool (cbindgen, via the FFI crate's `build.rs`), never through alef's own writer, so alef
//! has no formatting to answer for at the point this gate inspects them. See
//! [`POLY_FMT_LANE_EXCLUSIONS`] for the specific paths and the full reasoning.

/// One glob the poly_fmt lane excludes from `poly fmt --check`, with why.
///
/// Every entry here is a real gap the lane would otherwise fail on that is not an alef
/// formatting defect. Mirrors `every_clippy_lane_exclusion_is_justified`'s discipline for
/// the clippy lane in the parent file: a silent exclusion here would be worse than the
/// failure it's hiding, so every entry carries a non-empty `reason`, and
/// [`every_poly_fmt_lane_exclusion_is_justified`] enforces it. ~keep
pub(crate) struct PolyFmtExclusion {
    /// Gitignore-style glob passed to `poly fmt --check --exclude`.
    pub(crate) glob: &'static str,
    /// Why alef does not own this path's formatting.
    pub(crate) reason: &'static str,
}

pub(crate) const POLY_FMT_LANE_EXCLUSIONS: &[PolyFmtExclusion] = &[PolyFmtExclusion {
    glob: "**/include/*.h",
    reason: "cbindgen writes the C FFI header directly at `cargo build` time via the FFI \
             crate's build.rs (crates/*-ffi/include/*.h), and build.rs stages a copy into \
             packages/go/include/ for cgo -- alef's writer never touches either path, so it \
             has no formatting to answer for. poly.toml deliberately does NOT exclude these \
             from a consumer's own `poly fmt` (see scaffold::languages::poly's `[tools.clang-\
             format]` comment): a consumer's `poly fmt --fix` after `cargo build` is expected \
             to reformat a freshly-cbindgen-written header via the scaffolded clang-format \
             catalog tool, the same way it would any other unformatted source it did not \
             write itself. This gate runs `generate`+`scaffold` only, never that follow-up fix \
             pass, and the header materializes anyway as a side effect of the swift lane's \
             post-build `cargo build` pulling in the FFI crate as a dependency -- so the gate \
             would be asserting cleanliness at a point in the lifecycle before the intended \
             format pass has ever run over build output the gate's own doc says is out of \
             scope (`emit_tree`: 'while `all` additionally shells out to per-language build \
             and format toolchains that no runner has installed in full').",
}];

/// `poly fmt .` (a dry run), plus one `--exclude <glob>` per [`POLY_FMT_LANE_EXCLUSIONS`] entry.
pub(crate) fn poly_fmt_check_args() -> Vec<String> {
    let mut args: Vec<String> = vec!["fmt".to_string(), ".".to_string()];
    for exclusion in POLY_FMT_LANE_EXCLUSIONS {
        args.push("--exclude".to_string());
        args.push(exclusion.glob.to_string());
    }
    args
}

/// A glob that opts out of the poly_fmt lane has to say why -- see [`POLY_FMT_LANE_EXCLUSIONS`].
///
/// Without this, the cheap way to make a poly_fmt failure go away is to add a glob with no
/// stated reason, and the gap it opens leaves no trace anywhere. ~keep
#[test]
fn every_poly_fmt_lane_exclusion_is_justified() {
    let unjustified: Vec<&str> = POLY_FMT_LANE_EXCLUSIONS
        .iter()
        .filter(|exclusion| exclusion.reason.trim().is_empty())
        .map(|exclusion| exclusion.glob)
        .collect();
    assert!(
        unjustified.is_empty(),
        "these poly_fmt exclusion globs have no stated reason: {unjustified:?}"
    );
}
