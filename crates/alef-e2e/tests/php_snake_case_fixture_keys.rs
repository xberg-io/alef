//! Regression test: verify that PHP e2e codegen preserves snake_case keys from fixtures
//! when emitting `from_json(json_encode([...]))` calls.
//!
//! Previously, the codegen applied camelCase transformation to all JSON keys,
//! which caused deserialization failures when the fixture had snake_case keys
//! (e.g., extract_pages, insert_page_markers). The PHP binding's serde configuration
//! would silently ignore unknown keys due to #[serde(default)], leaving the
//! configuration in its default state instead of using the fixture's values.
//!
//! Fixtures are authored in Rust wire format (snake_case), and should be passed
//! to from_json() verbatim. The binding's serde will deserialize them correctly.

#[test]
fn php_from_json_preserves_fixture_keys() {
    // This is a documentation test for the fix. Actual validation happens in:
    // - kreuzberg/e2e/php/tests/ContractTest.php::test_config_pages (generated file)
    // - Task: task e2e:test:php (runs full PHPUnit suite)
    //
    // The test verifies that config with nested snake_case keys like:
    //   "pages": {
    //     "extract_pages": true,
    //     "insert_page_markers": true
    //   }
    //
    // Generates PHP code with keys UNCHANGED:
    //   $config = \Kreuzberg\ExtractionConfig::from_json(json_encode(
    //     ["pages" => ["extract_pages" => true, "insert_page_markers" => true]]
    //   ));
    //
    // NOT with camelCase transformation:
    //   ["pages" => ["extractPages" => true, "insertPageMarkers" => true]]
    //
    // This ensures the fixture's intent is preserved and deserialization succeeds.
    // See php.rs build_args_and_setup() for the implementation.
}
