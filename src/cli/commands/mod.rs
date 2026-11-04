//! Top-level CLI subcommand implementations.

pub mod adopt;
pub mod check_registry;
pub(crate) mod component_version_checks;
pub mod go_tag;
pub mod migrate;
pub mod release_metadata;
pub(crate) mod ruby_swift_versions;
pub mod snippets;
pub mod validate_versions;
pub(crate) mod version_manifests;
