#[test]
fn test_async_body_tokio_template_braces_are_balanced() {
    let template_source = include_str!("../src/codegen/templates/binding_helpers/async_body_tokio.jinja");

    let mut depth = 0;
    let chars: Vec<char> = template_source.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

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
