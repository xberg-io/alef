use super::super::version_core::*;
use super::super::version_registry::{render_registry_version, update_zig_package_hash};
use super::super::version_swift::compute_sha256_hex;
use super::super::version_text::*;
use super::*;
use crate::cli::pipeline::generate;
use crate::core::config::{CitationAuthor, CitationConfig};

/// Serialize tests that mutate process-global CWD. `std::env::set_current_dir`
/// is shared across the test binary, so concurrent tempdir-based `sync_versions`
/// tests would race without this guard.
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[path = "version_tests/basic.rs"]
mod basic;
#[path = "version_tests/e2e_manifests.rs"]
mod e2e_manifests;
#[path = "version_tests/manifests.rs"]
mod manifests;
#[path = "version_tests/swift_checksum.rs"]
mod swift_checksum;
#[path = "version_tests/sync_versions.rs"]
mod sync_versions;
