//! Fail a generation run whose generated manifest is vouched for beside a committed lockfile
//! that no longer resolves against it.
//!
//! ~keep alef: a consumer regenerated cleanly (`alef all --clean`, exit 0) and was then unable
//! to build the generated e2e crate at all: its committed `e2e/rust/Cargo.lock` pinned a
//! transitive registry dependency one minor behind what the crate's *path* dependency now
//! required, so `cargo metadata --locked` in that directory failed outright. Alef reported
//! nothing, because both mechanisms it had were keyed on the wrong fact:
//!
//! 1. [`super::version_lockfiles::relock_lockfiles_beside_changed_manifests`] relocks only when
//!    *alef's own manifest bytes changed in this run*. The requirement that moved lived in a
//!    hand-written path dependency alef neither generates nor watches, so the generated manifest
//!    was byte-identical and the hook never fired. No amount of fixing the relock hook closes
//!    this: it is watching a file that did not change.
//! 2. That relock is best-effort anyway (`cargo update --offline -w`, warn-only), so even when
//!    it does fire it can leave the lock stale and still exit 0.
//!
//! This module family adds the missing observation rather than a third write path: after
//! generation completes, every directory holding a manifest this run generated is checked for a
//! committed lock that contradicts it, and a contradiction is recorded as a stage failure. Alef
//! still never authors a lockfile — it only refuses to keep claiming a manifest is good when the
//! lock beside it says otherwise.
//!
//! One file per ecosystem, plus [`shared`] for the two helpers more than one of them needs
//! ([`shared::registry_self_dependency`] and [`shared::registered_unmarkable_manifest_dirs`]) --
//! split out when this used to be a single file so each ecosystem's gate, fixtures, and doc
//! comments stay under the `file-modularization` cap on their own, and so a change to one
//! ecosystem's lock-reading quirks cannot accidentally brush against another's. See `cargo.rs`'s
//! and `node.rs`'s own doc comments for why the ecosystems are deliberately NOT unified behind one
//! shared "check a lock" abstraction: each one's actual comparison (semver-range resolution vs.
//! text equality vs. exact-version-in-checksum-file) is a different problem wearing a similar name.

mod cargo;
mod dart;
mod go;
mod node;
mod php;
mod ruby;
mod shared;
mod uv;

pub(crate) use cargo::{
    StaleLockFinding, check_generated_lock_freshness_tolerating_pending_publish, stale_lock_findings,
};
pub(crate) use dart::check_generated_dart_lock_freshness_tolerating_pending_publish;
pub(crate) use go::check_generated_go_sum_freshness_tolerating_pending_publish;
pub(crate) use node::check_generated_node_lock_freshness_tolerating_pending_publish;
pub(crate) use php::check_generated_composer_lock_freshness_tolerating_pending_publish;
pub(crate) use ruby::check_generated_gemfile_lock_freshness_tolerating_pending_publish;
pub(crate) use uv::check_generated_uv_lock_freshness_tolerating_pending_publish;
