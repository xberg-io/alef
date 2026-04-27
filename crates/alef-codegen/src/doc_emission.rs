//! Language-native documentation comment emission.
//! Provides standardized functions for emitting doc comments in different languages.

/// Emit PHPDoc-style comments (/** ... */)
/// Used for PHP classes, methods, and properties.
pub fn emit_phpdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str(" * ");
        out.push_str(&escape_phpdoc_line(line));
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Escape PHPDoc line: handle */ sequences that could close the comment early.
fn escape_phpdoc_line(s: &str) -> String {
    s.replace("*/", "* /")
}

/// Emit C# XML documentation comments (/// <summary> ... </summary>)
/// Used for C# classes, structs, methods, and properties.
pub fn emit_csharp_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/// <summary>\n");
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(&escape_csharp_doc_line(line));
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str("/// </summary>\n");
}

/// Escape C# XML doc line: handle XML special characters.
fn escape_csharp_doc_line(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Emit Elixir documentation comments (@doc)
/// Used for Elixir modules and functions.
pub fn emit_elixir_doc(out: &mut String, doc: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str("@doc \"\"\"\n");
    for line in doc.lines() {
        out.push_str(&escape_elixir_doc_line(line));
        out.push('\n');
    }
    out.push_str("\"\"\"\n");
}

/// Emit Rust `///` documentation comments.
///
/// Used by alef backends that emit Rust source (e.g., the Rustler NIF crate,
/// the swift-bridge wrapper crate, the FRB Dart bridge crate). Distinct from
/// `emit_swift_doc` only by intent — the syntax is identical (`/// ` per line).
pub fn emit_rustdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Escape Elixir doc line: handle triple-quote sequences that could close the heredoc early.
fn escape_elixir_doc_line(s: &str) -> String {
    s.replace("\"\"\"", "\"\" \"")
}

/// Emit R roxygen2-style documentation comments (#')
/// Used for R functions.
pub fn emit_roxygen(out: &mut String, doc: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str("#' ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Swift-style documentation comments (///)
/// Used for Swift structs, enums, and functions.
pub fn emit_swift_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Javadoc-style documentation comments (/** ... */)
/// Used for Java classes, methods, and fields.
/// Handles XML escaping and Javadoc tag formatting.
pub fn emit_javadoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    for line in doc.lines() {
        if line.is_empty() {
            out.push_str(indent);
            out.push_str(" *\n");
        } else {
            let escaped = escape_javadoc_line(line);
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(&escaped);
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Emit KDoc-style documentation comments (/** ... */)
/// Used for Kotlin classes, methods, and properties.
pub fn emit_kdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    for line in doc.lines() {
        if line.is_empty() {
            out.push_str(indent);
            out.push_str(" *\n");
        } else {
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Emit Dartdoc-style documentation comments (///)
/// Used for Dart classes, methods, and properties.
pub fn emit_dartdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Gleam documentation comments (///)
/// Used for Gleam functions and types.
pub fn emit_gleam_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Zig documentation comments (///)
/// Used for Zig functions, types, and declarations.
pub fn emit_zig_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Escape Javadoc line: handle XML special chars and backtick code blocks.
fn escape_javadoc_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut code = String::new();
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                code.push(c);
            }
            result.push_str("{@code ");
            result.push_str(&code);
            result.push('}');
        } else if ch == '<' {
            result.push_str("&lt;");
        } else if ch == '>' {
            result.push_str("&gt;");
        } else if ch == '&' {
            result.push_str("&amp;");
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_phpdoc() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "Simple documentation", "    ");
        assert!(out.contains("/**"));
        assert!(out.contains("Simple documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_phpdoc_escaping() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "Handle */ sequences", "");
        assert!(out.contains("Handle * / sequences"));
    }

    #[test]
    fn test_emit_csharp_doc() {
        let mut out = String::new();
        emit_csharp_doc(&mut out, "C# documentation", "    ");
        assert!(out.contains("<summary>"));
        assert!(out.contains("C# documentation"));
        assert!(out.contains("</summary>"));
    }

    #[test]
    fn test_csharp_xml_escaping() {
        let mut out = String::new();
        emit_csharp_doc(&mut out, "foo < bar & baz > qux", "");
        assert!(out.contains("foo &lt; bar &amp; baz &gt; qux"));
    }

    #[test]
    fn test_emit_elixir_doc() {
        let mut out = String::new();
        emit_elixir_doc(&mut out, "Elixir documentation");
        assert!(out.contains("@doc \"\"\""));
        assert!(out.contains("Elixir documentation"));
        assert!(out.contains("\"\"\""));
    }

    #[test]
    fn test_elixir_heredoc_escaping() {
        let mut out = String::new();
        emit_elixir_doc(&mut out, "Handle \"\"\" sequences");
        assert!(out.contains("Handle \"\" \" sequences"));
    }

    #[test]
    fn test_emit_roxygen() {
        let mut out = String::new();
        emit_roxygen(&mut out, "R documentation");
        assert!(out.contains("#' R documentation"));
    }

    #[test]
    fn test_emit_swift_doc() {
        let mut out = String::new();
        emit_swift_doc(&mut out, "Swift documentation", "    ");
        assert!(out.contains("/// Swift documentation"));
    }

    #[test]
    fn test_emit_javadoc() {
        let mut out = String::new();
        emit_javadoc(&mut out, "Java documentation", "    ");
        assert!(out.contains("/**"));
        assert!(out.contains("Java documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_emit_kdoc() {
        let mut out = String::new();
        emit_kdoc(&mut out, "Kotlin documentation", "    ");
        assert!(out.contains("/**"));
        assert!(out.contains("Kotlin documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_emit_dartdoc() {
        let mut out = String::new();
        emit_dartdoc(&mut out, "Dart documentation", "    ");
        assert!(out.contains("/// Dart documentation"));
    }

    #[test]
    fn test_emit_gleam_doc() {
        let mut out = String::new();
        emit_gleam_doc(&mut out, "Gleam documentation", "    ");
        assert!(out.contains("/// Gleam documentation"));
    }

    #[test]
    fn test_emit_zig_doc() {
        let mut out = String::new();
        emit_zig_doc(&mut out, "Zig documentation", "    ");
        assert!(out.contains("/// Zig documentation"));
    }

    #[test]
    fn test_empty_doc_skipped() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "", "");
        emit_csharp_doc(&mut out, "", "");
        emit_elixir_doc(&mut out, "");
        emit_roxygen(&mut out, "");
        emit_kdoc(&mut out, "", "");
        emit_dartdoc(&mut out, "", "");
        emit_gleam_doc(&mut out, "", "");
        emit_zig_doc(&mut out, "", "");
        assert!(out.is_empty());
    }
}
