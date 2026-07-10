//! Test that cbindgen.toml template uses the current cbindgen type style.
//!
//! This is a template-level test that verifies the jinja template for
//! cbindgen.toml includes the fix for Go cgo vtable type mismatch.

#[test]
fn cbindgen_toml_template_uses_global_style_both() {
    let template_content = include_str!("../src/backends/ffi/templates/cbindgen_toml.jinja");

    assert!(
        template_content.contains("style = \"both\""),
        "cbindgen_toml.jinja template missing global style = \"both\""
    );

    assert!(
        template_content.contains("[export]"),
        "cbindgen_toml.jinja template missing [export] section"
    );

    let export_start = template_content.find("[export]").unwrap();
    let after_export = &template_content[export_start..];
    let next_bracket = after_export[1..].find('[').map(|i| i + 1).unwrap_or(after_export.len());
    let export_block = &after_export[..next_bracket];

    assert!(
        !export_block.contains("style ="),
        "cbindgen_toml.jinja [export] section should not override global style. Export block:\n{}",
        export_block
    );
}
