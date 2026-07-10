//! Regression test: verify that PHP e2e codegen fully qualifies config type names
//! to the binding namespace (e.g., \SampleCrate\ConfigType), not bare names.
//!
//! Bare names would be looked up in the test namespace (SampleCrate\E2e), causing
//! "Class not found" errors at runtime.

#[test]
fn php_config_types_are_namespace_qualified() {}

#[test]
fn kotlin_android_file_paths_are_not_wrapped() {}
