mod cleanup;
mod commands;
mod extract;
mod format;
pub(crate) mod generate;
mod helpers;
mod lock_freshness;
mod toolchains;
mod version;
mod version_core;
mod version_csharp;
mod version_lockfiles;
mod version_python;
mod version_regen;
mod version_registry;
mod version_swift;
mod version_text;
mod version_workspace;
mod workspace_lints;

pub use cleanup::cleanup_orphaned_files;
pub use commands::{
    StagingProfile, build, clean, fmt, fmt_post_generate, lint, run_post_build, setup, test, test_apps_run, update,
};
pub(crate) use commands::{build_with_environment, canonical_frb_generated};
pub use extract::extract;
pub use format::{format_generated, format_generated_reporting, unstamp_before_formatting, warn_missing_formatters};
pub(crate) use format::{
    generated_tree_needs_formatting, install_poly_hooks, is_tool_available, languages_owning_changed_paths,
    poly_format, poly_format_strict,
};
pub use generate::{
    WriteReport, collect_alef_headered_paths, diff_files, finalize_hashes, finalize_hashes_after_tree_format,
    finalize_hashes_sweeping, find_create_once_template_drift, generate, generate_public_api, generate_service_api,
    generate_stubs, generate_sweep_roots, managed_generated_files, managed_output_paths, normalize_content, readme,
    reconcile_managed_scaffold_manifests, report_refused_writes, report_user_owned_skips, scaffold,
    stampable_output_paths, sweep_manifest_orphans, sweep_orphans, targeted_e2e_sweep_roots, write_files,
    write_files_report, write_scaffold_files, write_scaffold_files_report, write_scaffold_files_with_overwrite,
};
pub(crate) use generate::{
    apply_shebang_chmod, atomic_write, check_ffi_header_freshness, declared_user_owned, decode_base64_binary,
    ensure_ffi_header_freshness, ensure_generated_header, is_base64_binary_output, is_markable_path,
    is_owned_by_ownership_record, marker_comment_style, matches_alef_output, provenance_header_for_path,
    stamp_for_adoption,
};
pub use helpers::{init, run_optional};
pub(crate) use lock_freshness::{
    check_generated_composer_lock_freshness_tolerating_pending_publish,
    check_generated_dart_lock_freshness_tolerating_pending_publish,
    check_generated_gemfile_lock_freshness_tolerating_pending_publish,
    check_generated_go_sum_freshness_tolerating_pending_publish,
    check_generated_lock_freshness_tolerating_pending_publish,
    check_generated_node_lock_freshness_tolerating_pending_publish,
    check_generated_uv_lock_freshness_tolerating_pending_publish,
};
pub use toolchains::{enforce_required_toolchains, enforce_required_toolchains_for_all};
pub use version::sync_versions;
pub use version_core::{set_version, verify_versions};
pub(crate) use version_lockfiles::check_release_lock_freshness;
pub(crate) use version_registry::sync_registry_package_versions;
pub use workspace_lints::ensure_workspace_alef_meta_check_cfg;
