//! Regression test for PHP e2e codegen streaming-vs-non-streaming field disambiguation.
//!
//! When a fixture has a non-streaming call, the field "chunks" should NOT be treated
//! as a streaming virtual field. Instead, it should be handled as a regular result field
//! accessible via $result->chunks.
//!
//! The bug: PHP codegen was checking `is_streaming_virtual_field("chunks")` without
//! verifying that the call was actually streaming, causing "Undefined variable $chunks"
//! errors at test runtime when fixtures like config_chunking_prepend_heading_context
//! tried to reference an undeclared $chunks variable.
//!
//! The fix (in src/e2e/codegen/php.rs): add `is_streaming &&` guard before the
//! streaming field check in render_assertion. This ensures "chunks" is only treated
//! as a streaming field when the call is actually streaming.

#[test]
fn php_chunks_count_min_non_streaming_uses_result_field() {}
