use crate::snippets::types::{Language, SnippetMetadata};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub lang: String,
    pub title: Option<String>,
    pub code: String,
    pub start_line: usize,
    pub preceding_comment: Option<String>,
    pub metadata: SnippetMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontmatterStatus {
    Present,
    Missing,
    Malformed(String),
}

#[must_use]
pub fn extract_fenced_blocks(content: &str) -> Vec<CodeBlock> {
    extract_fenced_blocks_with_metadata(content, SnippetMetadata::default())
}

#[must_use]
pub fn extract_fenced_blocks_with_metadata(content: &str, metadata: SnippetMetadata) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("```") {
            if rest.is_empty() || rest.starts_with('`') {
                index += 1;
                continue;
            }

            let (lang, title) = parse_fence_info(rest);
            if lang.is_empty() {
                index += 1;
                continue;
            }

            let preceding_comment = if index > 0 {
                let previous = lines[index - 1].trim();
                if previous.starts_with("<!--") && previous.ends_with("-->") {
                    Some(previous.to_string())
                } else {
                    None
                }
            } else {
                None
            };

            let start_line = index + 1;
            let mut code_lines = Vec::new();
            index += 1;

            while index < lines.len() {
                let closing = lines[index].trim();
                if closing == "```" || (closing.starts_with("```") && closing.chars().skip(3).all(|c| c == '`')) {
                    break;
                }

                code_lines.push(lines[index]);
                index += 1;
            }

            let code = code_lines.join("\n");
            if !code.trim().is_empty() {
                blocks.push(CodeBlock {
                    lang,
                    title,
                    code,
                    start_line,
                    preceding_comment,
                    metadata: metadata.clone(),
                });
            }
        }

        index += 1;
    }

    blocks
}

/// Parse code blocks from a markdown or source file.
///
/// # Errors
///
/// Returns an error when the file cannot be read for reasons other than invalid UTF-8.
pub fn parse_code_blocks(path: &Path) -> crate::snippets::error::Result<Vec<CodeBlock>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => return Ok(Vec::new()),
        Err(err) => {
            return Err(crate::snippets::error::Error::Parse {
                path: path.to_path_buf(),
                reason: err.to_string(),
            });
        }
    };

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();

    if extension == "md" || extension == "markdown" {
        let (metadata, body, line_offset) = parse_frontmatter(&content, path)?;
        let mut blocks = extract_fenced_blocks_with_metadata(body, metadata);
        for block in &mut blocks {
            block.start_line += line_offset;
        }
        return Ok(blocks);
    }

    let fenced = extract_fenced_blocks(&content);
    if !fenced.is_empty() {
        return Ok(fenced);
    }

    let language = Language::from_extension(&extension);
    if language == Language::Unknown {
        return Ok(Vec::new());
    }

    Ok(vec![CodeBlock {
        lang: language.to_string(),
        title: path.file_name().and_then(|name| name.to_str()).map(str::to_string),
        code: content,
        start_line: 1,
        preceding_comment: None,
        metadata: SnippetMetadata::default(),
    }])
}

fn parse_frontmatter<'a>(
    content: &'a str,
    _path: &Path,
) -> crate::snippets::error::Result<(SnippetMetadata, &'a str, usize)> {
    let Some(after_open) = content.strip_prefix("---\n") else {
        return Ok((SnippetMetadata::default(), content, 0));
    };

    let Some(close_offset) = after_open.find("\n---\n") else {
        return Ok((SnippetMetadata::default(), content, 0));
    };

    let yaml = &after_open[..close_offset];
    let Ok(metadata) = serde_yaml::from_str(yaml) else {
        return Ok((SnippetMetadata::default(), content, 0));
    };
    let body_start = "---\n".len() + close_offset + "\n---\n".len();
    let line_offset = content[..body_start].lines().count();
    Ok((metadata, &content[body_start..], line_offset))
}

#[must_use]
pub fn frontmatter_status(content: &str) -> FrontmatterStatus {
    let Some(after_open) = content.strip_prefix("---\n") else {
        return FrontmatterStatus::Missing;
    };

    let Some(close_offset) = after_open.find("\n---\n") else {
        return FrontmatterStatus::Malformed("YAML frontmatter is missing a closing --- delimiter".to_string());
    };

    let yaml = &after_open[..close_offset];
    match serde_yaml::from_str::<SnippetMetadata>(yaml) {
        Ok(_) => FrontmatterStatus::Present,
        Err(err) => FrontmatterStatus::Malformed(format!("invalid YAML frontmatter: {err}")),
    }
}

fn parse_fence_info(info: &str) -> (String, Option<String>) {
    let trimmed = info.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let lang = parts.next().unwrap_or_default().to_string();
    let rest = parts.next().unwrap_or_default();
    let title = parse_title_attr(rest);
    (lang, title)
}

fn parse_title_attr(attrs: &str) -> Option<String> {
    let trimmed = attrs.trim();

    if let Some(after) = trimmed.strip_prefix("title=") {
        let after = after.trim();

        if let Some(stripped) = after.strip_prefix('"') {
            let end = stripped.find('"')?;
            return Some(stripped[..end].to_string());
        }

        if let Some(stripped) = after.strip_prefix('\'') {
            let end = stripped.find('\'')?;
            return Some(stripped[..end].to_string());
        }

        let value: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
        if !value.is_empty() {
            return Some(value);
        }
    }

    for part in trimmed.split_whitespace() {
        if let Some(after) = part.strip_prefix("title=") {
            let value = after.trim_matches(|c| c == '"' || c == '\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_block() {
        let markdown = r#"
```rust title="example"
fn main() {}
```
"#;

        let blocks = extract_fenced_blocks(markdown);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].lang, "rust");
        assert_eq!(blocks[0].title.as_deref(), Some("example"));
    }

    #[test]
    fn preserves_annotations() {
        let markdown = r#"
<!-- snippet:skip -->
```python
print("hello")
```
"#;

        let blocks = extract_fenced_blocks(markdown);
        assert_eq!(blocks[0].preceding_comment.as_deref(), Some("<!-- snippet:skip -->"));
    }

    #[test]
    fn parses_markdown_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippet.md");
        std::fs::write(
            &path,
            r#"---
id: hello_world
language: python
title: Hello
tags:
  - smoke
---
```python
print("hello")
```
"#,
        )
        .unwrap();

        let blocks = parse_code_blocks(&path).unwrap();
        assert_eq!(blocks[0].metadata.id.as_deref(), Some("hello_world"));
        assert_eq!(blocks[0].metadata.language, Some(Language::Python));
        assert_eq!(blocks[0].metadata.tags, vec!["smoke"]);
        assert_eq!(blocks[0].start_line, 8);
    }
}
