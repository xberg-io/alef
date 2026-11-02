/// Escape literal pipe characters and square brackets outside code spans so a value can be
/// embedded inside a Markdown table cell without being misinterpreted.
///
/// This is required because some renderers (notably rumdl's MD056 check) treat
/// every `|` on a row as a separator, even when the pipe is inside a backtick
/// span — e.g. union types like `` `string | null` `` or `` `String.t() | nil` ``
/// would otherwise be parsed as two cells.
///
/// Square brackets inside backticks (e.g., `` `list[Table]` ``) are parsed as
/// link references by Zensical's CommonMark parser, causing "unresolved reference"
/// warnings in strict mode. We escape them to prevent this.
pub(crate) fn escape_table_cell(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let mut code_delimiter = 0;
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut run = 1;
            while chars.peek() == Some(&'`') {
                chars.next();
                run += 1;
            }
            if code_delimiter == 0 {
                code_delimiter = run;
            } else if code_delimiter == run {
                code_delimiter = 0;
            }
            escaped.extend(std::iter::repeat_n('`', run));
        } else if ch == '|' {
            escaped.push_str("\\|");
        } else if code_delimiter == 0 && matches!(ch, '[' | ']') {
            escaped.push('\\');
            escaped.push(ch);
        } else {
            escaped.push(ch);
        }
    }

    escaped
}

#[cfg(test)]
mod table_escape_tests {
    use super::escape_table_cell;

    #[test]
    fn preserves_brackets_in_code_spans_and_escapes_table_syntax() {
        assert_eq!(
            escape_table_cell("Use `[Item]` or [Item] | none"),
            "Use `[Item]` or \\[Item\\] \\| none"
        );
    }

    #[test]
    fn preserves_brackets_in_multi_backtick_code_spans() {
        assert_eq!(escape_table_cell("``value[`key`]``"), "``value[`key`]``");
    }
}
