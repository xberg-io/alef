use super::super::version_registry::{render_registry_version, update_zig_package_hash};
use super::super::version_swift::compute_sha256_hex;
use super::super::version_text::*;
use super::*;
use crate::cli::pipeline::generate;
use crate::core::config::{CitationAuthor, CitationConfig};
use crate::test_support::CWD_LOCK;

#[path = "version_tests/basic.rs"]
mod basic;
#[path = "version_tests/catch_all_ownership.rs"]
mod catch_all_ownership;
#[path = "version_tests/e2e_manifests.rs"]
mod e2e_manifests;
#[path = "version_tests/e2e_rust_manifest.rs"]
mod e2e_rust_manifest;
#[path = "version_tests/git_ignored_discovery.rs"]
mod git_ignored_discovery;
#[path = "version_tests/go_const_alignment.rs"]
mod go_const_alignment;
#[path = "version_tests/go_sentinel_pairing.rs"]
mod go_sentinel_pairing;
#[path = "version_tests/lockfile_relock.rs"]
mod lockfile_relock;
#[path = "version_tests/manifests.rs"]
mod manifests;
#[path = "version_tests/native_cargo_manifest.rs"]
mod native_cargo_manifest;
#[path = "version_tests/readme_regen.rs"]
mod readme_regen;
#[path = "version_tests/registry_bump_hash_ordering.rs"]
mod registry_bump_hash_ordering;
#[path = "version_tests/registry_dep_pin.rs"]
mod registry_dep_pin;
#[path = "version_tests/swift_checksum.rs"]
mod swift_checksum;
#[path = "version_tests/swift_placeholder.rs"]
mod swift_placeholder;
#[path = "version_tests/sync_versions.rs"]
mod sync_versions;
#[path = "version_tests/write_version_idempotent.rs"]
mod write_version_idempotent;
