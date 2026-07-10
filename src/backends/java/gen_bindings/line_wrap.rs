//! Post-processor that wraps Java source lines exceeding the project Checkstyle
//! limit. Splits at logical points so the generated code stays valid Java.
//!
//! The downstream Checkstyle configuration enforces a 140-char `LineLength`
//! rule. The codegen-time templates are intentionally simple and emit some
//! statements on one line; the splits done here are purely cosmetic and do
//! not change semantics.

/// Maximum line length allowed by downstream Checkstyle. Lines longer than
/// this threshold are eligible for splitting.
const MAX_LINE_LEN: usize = 140;

/// Tighter threshold for record-field annotation lines. Eclipse spotless
/// reflows our 4-space-indented record components to 8-space indent, adding
/// 4 chars to every component line. Wrap codegen-side at 136 so the
/// reflowed output still fits in `MAX_LINE_LEN`.
const RECORD_FIELD_WRAP_THRESHOLD: usize = 136;

/// Wrap any line in `src` that exceeds `MAX_LINE_LEN` chars, using a few
/// recipes that recognise patterns the Java codegen emits. The output is
/// reassembled with `\n` line terminators (matching what the generator
/// produces).
pub(crate) fn wrap_long_java_lines(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 256);
    for line in src.split_inclusive('\n') {
        let (content, terminator) = match line.strip_suffix('\n') {
            Some(rest) => (rest, "\n"),
            None => (line, ""),
        };

        let len = visible_len(content);
        let is_record_field_annotation =
            content.trim_start().starts_with('@') && (content.ends_with(',') || content.contains(") {"));
        let threshold = if is_record_field_annotation {
            RECORD_FIELD_WRAP_THRESHOLD
        } else {
            MAX_LINE_LEN
        };

        if len <= threshold {
            out.push_str(content);
            out.push_str(terminator);
            continue;
        }

        let wrapped = wrap_line(content);
        out.push_str(&wrapped);
        out.push_str(terminator);
    }
    out
}

/// Visible length used for the threshold check. Java sources here are ASCII
/// (annotations, identifiers, JSON string literals) so byte length matches
/// character count.
fn visible_len(s: &str) -> usize {
    s.len()
}

/// Try each recipe in turn. The first that yields a multi-line result with
/// every emitted line under the limit wins. If none apply, the original line
/// is returned unchanged — Checkstyle will then flag it, which is a
/// deliberate signal that another recipe is needed.
fn wrap_line(line: &str) -> String {
    let indent = leading_whitespace(line);

    if let Some(wrapped) = wrap_javadoc_line(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    if let Some(wrapped) = wrap_record_field_annotations(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    if let Some(wrapped) = wrap_function_descriptor_call(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    if let Some(wrapped) = wrap_method_type_call(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    if let Some(wrapped) = wrap_method_signature(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    if let Some(wrapped) = wrap_ternary_string_msg(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    if let Some(wrapped) = wrap_invoke_call(line, indent) {
        if all_lines_fit(&wrapped) {
            return wrapped;
        }
    }
    line.to_string()
}

fn all_lines_fit(text: &str) -> bool {
    text.lines().all(|l| l.len() <= MAX_LINE_LEN)
}

fn leading_whitespace(line: &str) -> &str {
    let end = line.find(|c: char| !c.is_whitespace()).unwrap_or(line.len());
    &line[..end]
}

/// Javadoc continuation lines (` * text...`) — break at the last space that
/// keeps the line under the limit and emit the remainder as a new
/// continuation line with the same ` * ` prefix.
fn wrap_javadoc_line(line: &str, indent: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("* ") && trimmed != "*" {
        return None;
    }
    let prefix = format!("{indent}* ");
    let rest = &trimmed[2..];
    let budget = MAX_LINE_LEN.saturating_sub(prefix.len());
    if rest.len() <= budget {
        return None;
    }

    let mut wrapped = String::new();
    let mut remaining = rest;
    loop {
        if remaining.len() <= budget {
            wrapped.push_str(&prefix);
            wrapped.push_str(remaining);
            break;
        }
        let slice = &remaining[..budget.min(remaining.len())];
        let break_at = slice.rfind(' ');
        let (head, tail) = match break_at {
            Some(idx) if idx > 0 => (&remaining[..idx], remaining[idx + 1..].trim_start()),
            _ => {
                let after_budget_space = remaining[budget..].find(' ');
                match after_budget_space {
                    Some(off) => {
                        let idx = budget + off;
                        (&remaining[..idx], remaining[idx + 1..].trim_start())
                    }
                    None => {
                        wrapped.push_str(&prefix);
                        wrapped.push_str(remaining);
                        break;
                    }
                }
            }
        };
        wrapped.push_str(&prefix);
        wrapped.push_str(head);
        wrapped.push('\n');
        remaining = tail;
        if remaining.is_empty() {
            break;
        }
    }
    Some(wrapped)
}

/// Record DTO field annotations: emit each annotation on its own indented
/// line above the declaration. Recognised by a leading indent containing only
/// `@`-annotations and a trailing `Type name[,]?` declaration.
fn wrap_record_field_annotations(line: &str, indent: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('@') {
        return None;
    }
    let (body, trailing) = match trimmed.strip_suffix(',') {
        Some(rest) => (rest, ","),
        None => (trimmed, ""),
    };

    let annotations = split_annotations(body)?;
    if annotations.len() < 2 {
        return None;
    }
    let declaration = annotations.last()?.clone();
    let annotation_lines = &annotations[..annotations.len() - 1];
    if !declaration.contains(' ') {
        return None;
    }

    let mut wrapped = String::new();
    for ann in annotation_lines {
        wrapped.push_str(indent);
        wrapped.push_str(ann);
        wrapped.push('\n');
    }
    wrapped.push_str(indent);
    wrapped.push_str(&declaration);
    wrapped.push_str(trailing);
    Some(wrapped)
}

/// Split a string of the form `@Ann1(...) @Ann2(...) Type name` into the
/// individual annotation tokens followed by the trailing declaration. Returns
/// `None` if the string doesn't lead with an annotation.
fn split_annotations(body: &str) -> Option<Vec<String>> {
    let bytes = body.as_bytes();
    if bytes.first() != Some(&b'@') {
        return None;
    }
    let mut tokens: Vec<String> = Vec::new();
    let mut idx = 0;
    while idx < bytes.len() {
        while idx < bytes.len() && bytes[idx] == b' ' {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        if bytes[idx] != b'@' {
            tokens.push(body[idx..].trim().to_string());
            break;
        }
        let start = idx;
        idx += 1;
        while idx < bytes.len() && (bytes[idx].is_ascii_alphanumeric() || bytes[idx] == b'_' || bytes[idx] == b'.') {
            idx += 1;
        }
        if idx < bytes.len() && bytes[idx] == b'(' {
            let mut depth = 1;
            let mut in_str = false;
            idx += 1;
            while idx < bytes.len() && depth > 0 {
                let b = bytes[idx];
                if in_str {
                    if b == b'\\' && idx + 1 < bytes.len() {
                        idx += 2;
                        continue;
                    }
                    if b == b'"' {
                        in_str = false;
                    }
                } else {
                    match b {
                        b'"' => in_str = true,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                }
                idx += 1;
            }
        }
        tokens.push(body[start..idx].to_string());
    }
    Some(tokens)
}

/// `FunctionDescriptor.of(ValueLayout.X, ValueLayout.Y, ...)` or
/// `FunctionDescriptor.ofVoid(ValueLayout.X, ...)`. Args are split one per
/// line, keeping any trailing punctuation (e.g. `),`).
fn wrap_function_descriptor_call(line: &str, indent: &str) -> Option<String> {
    wrap_call_args(line, indent, &["FunctionDescriptor.of(", "FunctionDescriptor.ofVoid("])
}

/// `MethodType.methodType(int.class, MemorySegment.class, ...)`. Same shape.
fn wrap_method_type_call(line: &str, indent: &str) -> Option<String> {
    wrap_call_args(line, indent, &["MethodType.methodType("])
}

/// `int rc = (int) NativeLib.X.invoke(a, b, c);` — wrap the arg list.
fn wrap_invoke_call(line: &str, indent: &str) -> Option<String> {
    wrap_call_args(line, indent, &[".invoke("])
}

/// Common helper: locate `prefix` in `line`, find the matching closing paren,
/// split the inner argument list on top-level commas, and re-emit one arg
/// per line. Continuation lines use `indent` + 4 spaces.
fn wrap_call_args(line: &str, indent: &str, prefixes: &[&str]) -> Option<String> {
    let (prefix_pos, prefix_str) = prefixes
        .iter()
        .filter_map(|p| line.find(p).map(|pos| (pos, *p)))
        .min_by_key(|(pos, _)| *pos)?;

    let open_paren = prefix_pos + prefix_str.len() - 1;
    let close_paren = matching_paren(line, open_paren)?;
    let inner = &line[open_paren + 1..close_paren];
    if inner.trim().is_empty() {
        return None;
    }
    let args = split_top_level_commas(inner);
    if args.len() < 2 {
        return None;
    }

    let head = &line[..=open_paren];
    let tail = &line[close_paren..];
    let cont_indent = format!("{indent}    ");

    let mut wrapped = String::new();
    wrapped.push_str(head);
    wrapped.push('\n');
    for (i, arg) in args.iter().enumerate() {
        wrapped.push_str(&cont_indent);
        wrapped.push_str(arg.trim());
        if i + 1 < args.len() {
            wrapped.push(',');
        }
        wrapped.push('\n');
    }
    wrapped.push_str(indent);
    wrapped.push_str(tail);
    Some(wrapped)
}

/// Find the index of the `)` that matches the `(` at `open_idx`. Respects
/// string literals so quoted parens don't confuse the depth count.
fn matching_paren(line: &str, open_idx: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.get(open_idx) != Some(&b'(') {
        return None;
    }
    let mut depth = 1;
    let mut i = open_idx + 1;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Split a comma-delimited list at the top level only — commas inside nested
/// `()`, `<>`, or string literals are preserved verbatim.
fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut depth_paren = 0i32;
    let mut depth_angle = 0i32;
    let mut in_str = false;
    let mut current = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let c = b as char;
        if in_str {
            current.push(c);
            if b == b'\\' && i + 1 < bytes.len() {
                current.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_str = true;
                current.push(c);
            }
            b'(' => {
                depth_paren += 1;
                current.push(c);
            }
            b')' => {
                depth_paren -= 1;
                current.push(c);
            }
            b'<' => {
                depth_angle += 1;
                current.push(c);
            }
            b'>' => {
                depth_angle -= 1;
                current.push(c);
            }
            b',' if depth_paren == 0 && depth_angle == 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Method signatures (`public static X foo(final A a, final B b) throws Y {`)
/// — wrap one param per line and dedent the trailing `throws ... {` onto the
/// closing-paren line.
fn wrap_method_signature(line: &str, indent: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let is_signature = (trimmed.starts_with("public ")
        || trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("static "))
        && trimmed.ends_with('{')
        && line.contains('(')
        && line.contains(')');
    if !is_signature {
        return None;
    }

    let open_idx = line.find('(')?;
    let close_idx = matching_paren(line, open_idx)?;
    let inner = &line[open_idx + 1..close_idx];
    if inner.trim().is_empty() {
        return None;
    }
    let params = split_top_level_commas(inner);
    if params.len() < 2 {
        return None;
    }

    let head = &line[..=open_idx];
    let tail = &line[close_idx..];
    let cont_indent = format!("{indent}    ");

    let mut wrapped = String::new();
    wrapped.push_str(head);
    wrapped.push('\n');
    for (i, p) in params.iter().enumerate() {
        wrapped.push_str(&cont_indent);
        wrapped.push_str(p.trim());
        if i + 1 < params.len() {
            wrapped.push(',');
        }
        wrapped.push('\n');
    }
    wrapped.push_str(indent);
    wrapped.push_str(tail);
    Some(wrapped)
}

/// Patterns like
///   `String msg = errPtr.equals(MemorySegment.NULL) ? "..." : errPtr.reinterpret(Long.MAX_VALUE).getString(0);`
/// — wrap the ternary onto two continuation lines.
fn wrap_ternary_string_msg(line: &str, indent: &str) -> Option<String> {
    let q_idx = line.find(" ? ")?;
    let colon_idx = find_top_level(line, q_idx + 3, ':')?;
    let head = &line[..q_idx];
    let then_branch = &line[q_idx + 3..colon_idx];
    let else_branch = &line[colon_idx + 1..];
    let cont = format!("{indent}    ");
    let mut wrapped = String::new();
    wrapped.push_str(head);
    wrapped.push('\n');
    wrapped.push_str(&cont);
    wrapped.push_str("? ");
    wrapped.push_str(then_branch.trim());
    wrapped.push('\n');
    wrapped.push_str(&cont);
    wrapped.push_str(": ");
    wrapped.push_str(else_branch.trim());
    Some(wrapped)
}

/// Scan `s` from `start` for the first occurrence of `target` that sits at
/// the top nesting level (outside string literals, `()`, `<>`).
fn find_top_level(s: &str, start: usize, target: char) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth_paren = 0i32;
    let mut depth_angle = 0i32;
    let mut in_str = false;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'<' => depth_angle += 1,
            b'>' => depth_angle -= 1,
            _ => {}
        }
        if b == target as u8 && depth_paren == 0 && depth_angle == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_untouched() {
        let input = "package dev.sample_crate;\nimport java.util.List;\n";
        assert_eq!(wrap_long_java_lines(input), input);
    }

    #[test]
    fn function_descriptor_split() {
        let long = "                FunctionDescriptor.of(ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS),\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long: {line}");
        }
        assert!(out.contains("FunctionDescriptor.of(\n"));
        assert!(out.contains("ValueLayout.JAVA_LONG,"));
        assert!(out.trim_end().ends_with("),"));
    }

    #[test]
    fn method_type_split() {
        let long = "                MethodType.methodType(int.class, MemorySegment.class, MemorySegment.class, MemorySegment.class, MemorySegment.class, MemorySegment.class, MemorySegment.class)),\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long ({}): {line}", line.len());
        }
    }

    #[test]
    fn method_signature_split() {
        let long = "    public static ParseResult parseFile(final java.nio.file.Path path, final String mimeType, final ParseConfig config) throws DemoRsException {\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long ({}): {line}", line.len());
        }
        assert!(out.contains("parseFile(\n"));
        assert!(out.contains("throws DemoRsException"));
    }

    #[test]
    fn record_field_annotations_split() {
        let long = "    @JsonInclude(JsonInclude.Include.NON_NULL) @JsonProperty(\"processingWarnings\") @JsonAlias(\"processing_warnings\") List<ProcessingWarning> processingWarnings,\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long ({}): {line}", line.len());
        }
        assert!(out.contains("    @JsonInclude(JsonInclude.Include.NON_NULL)\n"));
        assert!(out.contains("    @JsonProperty(\"processingWarnings\")\n"));
        assert!(out.trim_end().ends_with(","));
    }

    #[test]
    fn ternary_msg_split() {
        let long = "                    String msg = errPtr.equals(MemorySegment.NULL) ? \"registration failed (rc=\" + rc + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long ({}): {line}", line.len());
        }
    }

    #[test]
    fn javadoc_continuation_split() {
        let long = "     * {@literal @}param items Vector of {@code BatchBytesItem} structs, each containing content bytes, MIME type, and optional per-item configuration overrides.\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long ({}): {line}", line.len());
        }
        for line in out.lines() {
            assert!(
                line.trim_start().starts_with("* "),
                "javadoc continuation lost its prefix: {line}"
            );
        }
    }

    #[test]
    fn invoke_call_split() {
        let long = "            int rc = (int) NativeLib.SAMPLE_CRATE_RENDER_PDF_PAGE_TO_PNG.invoke(cpdfBytes, pageIndex, cdpi, cpassword, outPtrHolder, outLenHolder, outCapHolder, somethingExtraToForceWrap);\n";
        let out = wrap_long_java_lines(long);
        for line in out.lines() {
            assert!(line.len() <= MAX_LINE_LEN, "line too long ({}): {line}", line.len());
        }
    }
}
