pub(super) fn canonicalize_rustdoc_markdown(doc: &str) -> String {
    let mut output = Vec::new();
    let mut in_fence = false;
    let mut previous_was_heading = false;

    for raw_line in doc.lines() {
        let trimmed = raw_line.trim_start();
        let is_fence = trimmed.starts_with("```");
        let is_heading = !in_fence && is_markdown_heading(trimmed);

        if is_heading && output.last().is_some_and(|line: &String| !line.is_empty()) {
            output.push(String::new());
        } else if previous_was_heading && !raw_line.trim().is_empty() {
            output.push(String::new());
        }

        let line = if !in_fence && is_fence && trimmed.trim().chars().all(|character| character == '`') {
            let indentation = &raw_line[..raw_line.len() - trimmed.len()];
            format!("{indentation}```rust")
        } else if in_fence {
            raw_line.to_string()
        } else {
            normalize_sentence_spacing(raw_line)
        };
        output.push(line);

        if is_fence {
            in_fence = !in_fence;
        }
        previous_was_heading = is_heading;
    }

    output.join("\n")
}

pub(super) fn markdown_to_table_cell_inline(doc: &str) -> String {
    let mut parts = Vec::new();
    let mut code_lines = Vec::new();
    let mut in_fence = false;

    for line in doc.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_fence {
                parts.push(render_inline_code(&code_lines.join(" ")));
                code_lines.clear();
            }
            in_fence = !in_fence;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if in_fence {
            code_lines.push(trimmed);
        } else {
            parts.push(trimmed.to_string());
        }
    }

    if in_fence && !code_lines.is_empty() {
        parts.push(render_inline_code(&code_lines.join(" ")));
    }

    parts.join(" ")
}

fn is_markdown_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|character| *character == '#').count();
    (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ')
}

fn normalize_sentence_spacing(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut output = String::with_capacity(line.len());
    let mut code_delimiter = 0usize;
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '`' {
            let start = index;
            while index < chars.len() && chars[index] == '`' {
                index += 1;
            }
            let run = index - start;
            if code_delimiter == 0 {
                code_delimiter = run;
            } else if code_delimiter == run {
                code_delimiter = 0;
            }
            output.extend(std::iter::repeat_n('`', run));
            continue;
        }

        if chars[index] == ' ' && code_delimiter == 0 {
            let start = index;
            while index < chars.len() && chars[index] == ' ' {
                index += 1;
            }
            let previous = output.chars().next_back();
            if index < chars.len() && index - start > 1 && previous.is_some_and(|ch| matches!(ch, '.' | '?' | '!')) {
                output.push(' ');
            } else {
                output.extend(std::iter::repeat_n(' ', index - start));
            }
            continue;
        }

        output.push(chars[index]);
        index += 1;
    }

    output
}

fn render_inline_code(code: &str) -> String {
    let max_run = code
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let delimiter = "`".repeat(max_run + 1);
    if code.starts_with('`') || code.ends_with('`') {
        format!("{delimiter} {code} {delimiter}")
    } else {
        format!("{delimiter}{code}{delimiter}")
    }
}
