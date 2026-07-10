#[test]
fn test_swift_coding_key_case_template_each_case_on_own_line() {
    let template_content = include_str!("../src/backends/swift/templates/swift_tagged_coding_key_case.swift.jinja");

    let lines: Vec<&str> = template_content.lines().collect();

    assert!(lines.len() >= 2, "Template must have at least 2 lines");

    assert_eq!(lines[0], "", "First line should be empty (leading newline)");

    let case_line = lines[1].trim();
    assert!(
        case_line.starts_with("case "),
        "Second line should start with 'case': {}",
        case_line
    );

    let case_count = lines[1].matches("case ").count();
    assert_eq!(case_count, 1, "Line should have exactly one case keyword");

    assert!(template_content.ends_with('\n'), "Template should end with newline");
}
