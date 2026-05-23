#[test]
fn test_visitor_substitution_pattern() {
    let content = r#"            visitor: Default::default(),
            ..Default::default()"#;
    let pattern = format!(
        "            {}: Default::default(),\n            ..Default::default()",
        "visitor"
    );
    assert!(
        content.contains(&pattern),
        "Pattern not found.\nExpected: {:?}\nIn: {:?}",
        pattern,
        content
    );
}

#[test]
fn test_full_impl_contains_visitor_default() {
    let impl_content = r#"impl From<WasmConversionOptions> for html_to_markdown_rs::options::ConversionOptions {
    fn from(val: WasmConversionOptions) -> Self {
        Self {
            heading_style: val.heading_style.into(),
            list_indent_type: val.list_indent_type.into(),
            max_depth: val.max_depth,
            exclude_selectors: val.exclude_selectors,
            visitor: Default::default(),
            ..Default::default()
        }
    }
}"#;
    let pattern = "            visitor: Default::default(),\n            ..Default::default()";
    assert!(impl_content.contains(pattern), "Pattern not found in impl");
}
