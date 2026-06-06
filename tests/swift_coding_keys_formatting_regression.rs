// Regression test for Swift CodingKeys formatting bug where multiple case
// statements were crammed on one line due to trim_blocks eating newlines.
// Symptom: "consecutive declarations on a line must be separated by newline or ';'"
// Root cause: minijinja trim_blocks=true consumed newline after {% endif %} tag
// Fix: moved newline before case statement so it's static text, not after template tag

#[test]
fn test_swift_coding_key_case_template_each_case_on_own_line() {
    // The swift_tagged_coding_key_case.swift.jinja template must produce
    // one case per line when rendered. This test verifies the template
    // has the newline positioned correctly.

    let template_content = include_str!(
        "../src/backends/swift/templates/swift_tagged_coding_key_case.swift.jinja"
    );

    // Split by lines
    let lines: Vec<&str> = template_content.lines().collect();

    // The template should have 2 lines:
    // Line 0: empty (the leading newline becomes an empty line)
    // Line 1: the "case" statement
    assert!(lines.len() >= 2, "Template must have at least 2 lines");

    // Line 0 should be empty (from the leading newline)
    assert_eq!(lines[0], "", "First line should be empty (leading newline)");

    // Line 1 should be the case statement
    let case_line = lines[1].trim();
    assert!(
        case_line.starts_with("case "),
        "Second line should start with 'case': {}",
        case_line
    );

    // Verify there's no jinja block tag on the same line as multiple cases
    // The pattern "case ... {% if ... %} = ... {% endif %}" should have exactly
    // one "case" keyword because the if block is for conditional content only
    let case_count = lines[1].matches("case ").count();
    assert_eq!(case_count, 1, "Line should have exactly one case keyword");

    // Verify the template file itself ends with a newline (helps with formatting)
    assert!(
        template_content.ends_with('\n'),
        "Template should end with newline"
    );
}
