//! Test that cbindgen.toml template includes [export] style = "type".
//!
//! This is a template-level test that verifies the jinja template for
//! cbindgen.toml includes the fix for Go cgo vtable type mismatch.
//!
//! When cbindgen uses style = "both" globally but [export] style = "type",
//! function parameters use typedef names instead of struct tags, which
//! prevents cgo type mismatch errors when passing vtable pointers.

#[test]
fn cbindgen_toml_template_includes_export_style_type() {
    // The template is in src/backends/ffi/templates/cbindgen_toml.jinja
    // We test that it's been properly fixed by checking the source directly.
    let template_content = include_str!("../src/backends/ffi/templates/cbindgen_toml.jinja");

    // Verify [export] section exists
    assert!(
        template_content.contains("[export]"),
        "cbindgen_toml.jinja template missing [export] section"
    );

    // Verify style = "type" is in the [export] section
    let export_start = template_content.find("[export]").unwrap();
    let after_export = &template_content[export_start..];
    let next_bracket = after_export[1..]
        .find('[')
        .map(|i| i + 1)
        .unwrap_or(after_export.len());
    let export_block = &after_export[..next_bracket];

    assert!(
        export_block.contains("style = \"type\""),
        "cbindgen_toml.jinja [export] section missing style = \"type\". \
         This is required for Go cgo to use typedef names instead of struct tags \
         in function parameters, avoiding vtable type mismatch. Export block:\n{}", export_block
    );
}
