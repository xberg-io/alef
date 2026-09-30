#[derive(Clone, Copy)]
pub(super) enum FenceEvent {
    Open(usize),
    Close,
}

pub(super) fn fence_event(line: &str, opening_ticks: Option<usize>) -> Option<FenceEvent> {
    let trimmed = line.trim_start();
    let tick_count = trimmed.chars().take_while(|character| *character == '`').count();
    match opening_ticks {
        Some(opening_ticks) if tick_count >= opening_ticks && trimmed[tick_count..].trim().is_empty() => {
            Some(FenceEvent::Close)
        }
        None if tick_count >= 3 => Some(FenceEvent::Open(tick_count)),
        _ => None,
    }
}

pub(super) fn apply_fence_event(opening_ticks: &mut Option<usize>, event: FenceEvent) {
    *opening_ticks = match event {
        FenceEvent::Open(ticks) => Some(ticks),
        FenceEvent::Close => None,
    };
}

pub(super) fn canonicalize_rustdoc_markdown(doc: &str) -> String {
    let mut output = Vec::new();
    let mut fence_ticks = None;
    let mut previous_was_heading = false;

    for raw_line in doc.lines() {
        let trimmed = raw_line.trim_start();
        let tick_count = trimmed.chars().take_while(|character| *character == '`').count();
        let is_fence = tick_count >= 3;
        let closes_fence = fence_ticks
            .is_some_and(|opening_ticks| tick_count >= opening_ticks && trimmed[tick_count..].trim().is_empty());
        let is_heading = fence_ticks.is_none() && is_markdown_heading(trimmed);

        if is_heading && output.last().is_some_and(|line: &String| !line.is_empty()) {
            output.push(String::new());
        } else if previous_was_heading && !raw_line.trim().is_empty() {
            output.push(String::new());
        }

        let line = if fence_ticks.is_none() && is_fence && trimmed[tick_count..].trim().is_empty() {
            let indentation = &raw_line[..raw_line.len() - trimmed.len()];
            format!("{indentation}{}rust", "`".repeat(tick_count))
        } else if fence_ticks.is_some() || is_fence {
            raw_line.to_string()
        } else {
            normalize_sentence_spacing(raw_line)
        };
        output.push(line);

        if closes_fence {
            fence_ticks = None;
        } else if fence_ticks.is_none() && is_fence {
            fence_ticks = Some(tick_count);
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
