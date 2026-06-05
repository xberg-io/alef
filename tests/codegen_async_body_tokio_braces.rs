#[test]
fn test_async_body_tokio_template_braces_are_balanced() {
    // This test validates that the async_body_tokio.jinja template
    // produces Rust code with balanced braces.

    let template_source = include_str!("../src/codegen/templates/binding_helpers/async_body_tokio.jinja");

    // Count literal braces in the generated Rust code (strip Jinja syntax)
    // We look for rt.block_on(async { ... }) patterns which should have balanced braces
    let mut depth = 0;
    let chars: Vec<char> = template_source.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Skip Jinja interpolation {{ ... }}
        if ch == '{' && i + 1 < chars.len() && chars[i + 1] == '{' {
            i += 2;
            while i + 1 < chars.len() {
                if chars[i] == '}' && chars[i + 1] == '}' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Skip Jinja tags {% ... %}
        if ch == '{' && i + 1 < chars.len() && chars[i + 1] == '%' {
            i += 2;
            while i + 1 < chars.len() {
                if chars[i] == '%' && chars[i + 1] == '}' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Count literal braces (Rust code)
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            assert!(
                depth >= 0,
                "Unmatched closing brace in async_body_tokio.jinja: {}",
                template_source
                    .chars()
                    .skip(i.saturating_sub(40))
                    .take(80)
                    .collect::<String>()
            );
        }

        i += 1;
    }

    assert_eq!(
        depth, 0,
        "Braces not balanced in async_body_tokio.jinja: final depth = {}",
        depth
    );
}
