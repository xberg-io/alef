//! Per-category test file generation for Rust e2e tests.

mod collection;
mod file_rendering;
mod helpers;
mod test_function;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod trait_import_tests;

pub use collection::collect_test_filenames;
pub use file_rendering::render_test_file;
pub(super) use helpers::is_skipped;
pub(super) use test_function::STREAMING_COLLECTED_VAR;
pub use test_function::render_test_function;
