use alef_backend_kotlin::literal_normalizer;

#[test]
fn test_fix_float_literals_double_with_comma() {
    let content = "val minNonWhitespacePerPage: Double = 32,";
    let result = literal_normalizer::fix_float_literals(content);
    assert_eq!(result, "val minNonWhitespacePerPage: Double = 32.0,");
}

#[test]
fn test_fix_float_literals_double_eol() {
    let content = "val minAvgWordLength: Double = 2\n)";
    let result = literal_normalizer::fix_float_literals(content);
    assert_eq!(result, "val minAvgWordLength: Double = 2.0\n)");
}

#[test]
fn test_fix_float_literals_multiple() {
    let content = "val minNonWhitespacePerPage: Double = 32,\nval minAvgWordLength: Double = 2,";
    let result = literal_normalizer::fix_float_literals(content);
    assert_eq!(result, "val minNonWhitespacePerPage: Double = 32.0,\nval minAvgWordLength: Double = 2.0,");
}

#[test]
fn test_fix_float_literals_float() {
    let content = "val field: Float = 32f";
    let result = literal_normalizer::fix_float_literals(content);
    assert_eq!(result, "val field: Float = 32.0f");
}
