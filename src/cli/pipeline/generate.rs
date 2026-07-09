mod diff;
mod generation;
mod normalization;
mod orphans;
mod scaffold;
#[cfg(test)]
mod tests;
mod validation;
mod write;

pub use diff::diff_files;
pub use generation::{generate, generate_public_api, generate_service_api, generate_stubs};
pub use normalization::normalize_content;
pub use orphans::{collect_alef_headered_paths, sweep_orphans};
pub use scaffold::{readme, scaffold, write_scaffold_files, write_scaffold_files_with_overwrite};
pub(crate) use write::apply_shebang_chmod;
pub use write::{finalize_hashes, write_files};

#[cfg(test)]
use normalization::{detect_crate_edition, parse_package_edition};
