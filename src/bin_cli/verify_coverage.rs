//! What a passing `alef verify` actually proves -- stated in the report, every run.
//!
//! `alef verify`'s findings are all *negative* claims (nothing stale, nothing missing,
//! nothing orphaned), and a report made only of negative claims is indistinguishable from a
//! report that examined nothing. Consumer CI runs this command under job names like
//! "Alef-generated bindings freshness" and reads a green result as a whole-tree guarantee,
//! while the actual claim is much narrower: only files carrying an alef marker on disk are
//! held to a hash. Everything else is proven by PATH PRESENCE at best -- a present-but-wrong
//! file passes -- and files outside the ownership walk's scan set are never opened at all.
//!
//! This module states the gap in numbers alongside the verdict. It is the same fix
//! `alef snippets audit` needed when a snippets-only invocation printed a bare
//! "Audit clean: no issues found." for a run in which the documentation-page checks had
//! never executed: the floor is not "check more", it is "never let the report read as a
//! bigger claim than the check made". ~keep

/// The measured scope of one `alef verify` run.
///
/// Built by [`Self::measure`] from facts the run already has in hand, never from a second
/// walk of its own: a coverage report derived independently of the checks it describes can
/// disagree with them, which would make it worse than no report at all. ~keep
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerifyCoverage {
    /// Paths this run's configuration would produce, across every selected crate.
    pub(crate) managed_total: usize,
    /// Managed paths that exist on disk carrying an alef marker: the only ones whose CONTENTS
    /// were checked, by comparing the embedded `alef:hash:` against the current generation
    /// inputs.
    pub(crate) managed_content_verified: usize,
    /// Managed paths that exist but carry no marker the walk could read -- create-once seeds,
    /// formats with no comment syntax (`.json`, `.jar`, lockfiles) whose ownership lives in
    /// `.alef-ownership.toml`, and anything the walk did not open. Their presence was checked;
    /// nothing else about them was. ~keep
    pub(crate) managed_present_only: usize,
    /// Managed paths absent from disk. Already reported under the missing-file headings; kept
    /// here so the three managed numbers add up to [`Self::managed_total`] and a reader can
    /// see that they do.
    pub(crate) managed_absent: usize,
    /// Alef-marked files found on disk that this run's managed surface does not claim. The
    /// orphan check reports these; counted here so a reader can tell "verify found nothing"
    /// from "verify's surface and the disk disagree about this many files".
    pub(crate) marked_outside_surface: usize,
    /// Files the ownership walk opened and read anywhere under the tree.
    pub(crate) files_opened: usize,
    /// Files the walk reached and did not examine at all -- see
    /// [`super::verify_scan::ScanCoverage::unexamined`].
    pub(crate) files_unexamined: usize,
    /// The subset of [`Self::managed_present_only`] that are create-once seeds carrying no
    /// provenance marker.
    ///
    /// Reported here rather than under a frozen-file heading, and that placement IS the fix:
    /// alef writes a create-once seed only when its path is absent, so an existing one is a
    /// user-owned file alef will never rewrite -- not drift, and not something any command can
    /// or should change. See [`super::helpers::frozen::report_lines`] for the measured incident.
    /// Stating the number as coverage keeps it visible on a clean run, which the old heading
    /// (printed only when some other check had already failed) did not. ~keep
    pub(crate) create_once_unmarked: usize,
    /// The subset of [`Self::create_once_unmarked`] that alef re-renders on every run, would
    /// write different bytes for, and is refused on -- see
    /// [`super::helpers::frozen::drifted_frozen_seeds`].
    ///
    /// Split out because the line above it makes a positive claim -- "the missing marker is not
    /// drift" -- that this run does not check and that is FALSE for exactly these paths. A
    /// coverage report exists to stop a green result being read as more than it is; asserting
    /// "not drift" about files nobody compared is the same
    /// over-claim one level down, and it is the one that let three consumer repositories ship a
    /// version-pinned installer frozen at a stale release. ~keep
    pub(crate) create_once_drifted: usize,
    /// The subset of [`Self::managed_absent`] that `[crates.verify].ignore_ephemeral`
    /// (`crate::core::config::VerifyConfig`) matched and dropped from the "missing generated
    /// files" / "missing generated files that are also gitignored" headings -- output the
    /// consumer declared intentionally ephemeral (e.g. registry-mode `test_apps/`) and
    /// deliberately never commits.
    ///
    /// Reported here, unconditionally, for the same reason [`Self::create_once_unmarked`] is:
    /// an opt-out that narrows what one check may report must never be free to shrink silently.
    /// A run that excluded 316 paths must say so on every run, including a clean one -- not
    /// only when a reader happens to look at the missing-file headings that no longer name
    /// them. See `crate::core::config::verify`'s module doc for the incident this closes. ~keep
    pub(crate) ephemeral_excluded: usize,
    /// Managed paths that `[workspace.ownership] user_owned`
    /// ([`crate::core::config::OwnershipConfig`]) declares owned by the consuming repository
    /// rather than by alef, and that this run therefore did not write, stamp, hash or hold to
    /// any content claim.
    ///
    /// Counted here, unconditionally, for the reason this whole module exists: the declaration
    /// shrinks what a green `alef verify` proves, and an opt-out that narrows a check must
    /// never be free to shrink it silently. A reader who sees a clean run must be able to see
    /// that 17 of the paths in the managed surface were exempted by config, not checked and
    /// found correct. ~keep
    pub(crate) declared_user_owned: usize,
}

impl VerifyCoverage {
    /// Measure one run from the managed surface it built and the walk it already performed.
    ///
    /// `marked_paths` must be the paths of the SAME walk `scan` describes. Passing a set from
    /// a different walk would make `managed_content_verified` describe files this run never
    /// looked at. ~keep
    pub(crate) fn measure(
        managed_paths: &std::collections::HashSet<std::path::PathBuf>,
        marked_paths: &std::collections::HashSet<std::path::PathBuf>,
        scan: super::verify_scan::ScanCoverage,
        create_once_unmarked: usize,
        create_once_drifted: usize,
        ephemeral_excluded: usize,
        declared_user_owned: usize,
    ) -> Self {
        let mut coverage = Self {
            managed_total: managed_paths.len(),
            marked_outside_surface: marked_paths.difference(managed_paths).count(),
            files_opened: scan.opened,
            files_unexamined: scan.unexamined,
            create_once_unmarked,
            create_once_drifted,
            ephemeral_excluded,
            declared_user_owned,
            ..Self::default()
        };
        for path in managed_paths {
            if marked_paths.contains(path) {
                coverage.managed_content_verified += 1;
            } else if path.exists() {
                coverage.managed_present_only += 1;
            } else {
                coverage.managed_absent += 1;
            }
        }
        coverage
    }

    /// The report, one line per element, ready for [`super::output::line`].
    ///
    /// A pure function returning lines rather than printing them, so the numbers and the
    /// wording are unit-testable. `alef verify` writes through `output::line` straight to
    /// stdout, which an in-process test cannot intercept -- an assertion on the printed text
    /// would have to be a timing argument instead of a check. ~keep
    pub(crate) fn report_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "Verify coverage (what this run examined):".to_owned(),
            format!(
                "  managed surface: {} path(s) this configuration would produce",
                self.managed_total
            ),
            format!(
                "    {} content-verified (alef marker on disk, hashed against current generation inputs)",
                self.managed_content_verified
            ),
            format!(
                "    {} present but NOT content-verified (no readable marker: create-once seeds, formats \
                 that cannot carry one, paths proven by .alef-ownership.toml -- a present-but-wrong file \
                 passes)",
                self.managed_present_only
            ),
        ];
        if self.create_once_unmarked > 0 {
            // Two numbers, not one, and the split is the point. The "not drift" claim below is
            // only true of the seeds alef never re-attempts; stating it over the whole set
            // asserted "not drift" about files this run had not compared. ~keep
            let settled = self.create_once_unmarked - self.create_once_drifted;
            lines.push(format!(
                "      of which {settled} are create-once seeds with no marker that alef never \
                 re-attempts: written only when absent, so the missing marker is not drift \
                 (-vv lists them)"
            ));
            if self.create_once_drifted > 0 {
                lines.push(format!(
                    "      and {} more that alef DOES re-render every run: on-disk content differs \
                     from what alef would write and the ownership guard refuses the write -- stale, \
                     not settled, named under the DRIFTED heading above",
                    self.create_once_drifted
                ));
            }
        }
        lines.push(format!(
            "    {} absent (reported under the missing-file headings)",
            self.managed_absent
        ));
        if self.ephemeral_excluded > 0 {
            lines.push(format!(
                "      of which {} are excluded from those headings by `[crates.verify].ignore_ephemeral` \
                 (declared ephemeral and never committed; still counted in the {} above)",
                self.ephemeral_excluded, self.managed_absent
            ));
        }
        if self.declared_user_owned > 0 {
            lines.push(format!(
                "    {} declared user-owned by `[workspace.ownership] user_owned` in alef.toml: not \
                 written, stamped or content-checked. Remove the pattern to hand a path back to alef \
                 (-vv lists them)",
                self.declared_user_owned
            ));
        }
        lines.push(format!(
            "  tree walk: {} file(s) opened, {} never examined (name and extension outside the \
             ownership walk's scan set, or not readable as text)",
            self.files_opened, self.files_unexamined
        ));
        if self.marked_outside_surface > 0 {
            // Deliberately not "see the orphan heading": the orphan check excludes known
            // create-once seed paths on top of this diff, so the two counts legitimately differ
            // and pointing at a heading that may not be printed would be the same over-claim
            // this module exists to stop. ~keep
            lines.push(format!(
                "  {} alef-marked file(s) on disk are not claimed by this run's managed surface \
                 (the orphan check reports whichever of them it can attribute to a dropped emit)",
                self.marked_outside_surface
            ));
        }
        lines
    }
}

#[cfg(test)]
mod tests;
