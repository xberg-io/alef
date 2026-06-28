use crate::snippets::error::Result;
use crate::snippets::parser;
use crate::snippets::types::{Language, Snippet, SnippetAnnotation, SnippetAnnotationKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Discover snippets beneath the provided directories.
///
/// # Errors
///
/// Returns an error when a source file cannot be parsed into snippet blocks.
pub fn discover_snippets(dirs: &[PathBuf], language_filter: Option<&[Language]>) -> Result<Vec<Snippet>> {
    let mut snippets = Vec::new();

    for dir in dirs {
        if !dir.exists() {
            continue;
        }

        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let file_snippets = extract_snippets_from_file(path, dir)?;

            for snippet in file_snippets {
                if let Some(filter) = language_filter
                    && !filter.contains(&snippet.language)
                {
                    continue;
                }

                snippets.push(snippet);
            }
        }
    }

    snippets.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.block_index.cmp(&right.block_index))
    });
    Ok(snippets)
}

fn extract_snippets_from_file(path: &Path, base_dir: &Path) -> Result<Vec<Snippet>> {
    let blocks = parser::parse_code_blocks(path)?;
    let dir_language = infer_language_from_path(path, base_dir);
    let mut snippets = Vec::new();

    for (index, block) in blocks.into_iter().enumerate() {
        let language = {
            let from_fence = Language::from_fence_tag(&block.lang);
            if from_fence == Language::Unknown {
                dir_language.unwrap_or(Language::Unknown)
            } else {
                from_fence
            }
        };

        if language == Language::Unknown {
            continue;
        }

        let annotation = block.preceding_comment.as_deref().and_then(parse_annotation);
        let metadata = block.metadata;
        let language = metadata.language.unwrap_or(language);
        let title = metadata.title.clone().or(block.title);
        let annotation = if metadata.skip {
            Some(SnippetAnnotation {
                kind: SnippetAnnotationKind::Skip,
                reason: metadata.reason.clone(),
            })
        } else if let Some(level) = metadata.level {
            level_annotation(level, metadata.reason.clone()).or(annotation)
        } else {
            annotation
        };

        snippets.push(Snippet {
            id: metadata.id.clone(),
            path: path.to_path_buf(),
            language,
            title,
            code: block.code,
            start_line: block.start_line,
            block_index: index,
            annotation,
            metadata,
        });
    }

    Ok(snippets)
}

fn infer_language_from_path(path: &Path, base_dir: &Path) -> Option<Language> {
    let relative = path.strip_prefix(base_dir).ok()?;
    for component in relative.components() {
        let dir_name = component.as_os_str().to_str()?;
        let language = Language::from_dir_name(dir_name);
        if language != Language::Unknown {
            return Some(language);
        }
    }

    None
}

pub fn parse_annotation(comment: &str) -> Option<SnippetAnnotation> {
    let inner = comment.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let mut parts = inner.split_whitespace();
    let kind = match parts.next()? {
        "snippet:skip" => SnippetAnnotationKind::Skip,
        "snippet:compile-only" => SnippetAnnotationKind::CompileOnly,
        "snippet:syntax-only" => SnippetAnnotationKind::SyntaxOnly,
        "snippet:typecheck-only" => SnippetAnnotationKind::TypeCheckOnly,
        _ => return None,
    };
    let reason = parse_reason_attr(inner);

    Some(SnippetAnnotation { kind, reason })
}

fn level_annotation(
    level: crate::snippets::types::ValidationLevel,
    reason: Option<String>,
) -> Option<SnippetAnnotation> {
    let kind = match level {
        crate::snippets::types::ValidationLevel::Syntax => SnippetAnnotationKind::SyntaxOnly,
        crate::snippets::types::ValidationLevel::Compile => SnippetAnnotationKind::CompileOnly,
        crate::snippets::types::ValidationLevel::TypeCheck => SnippetAnnotationKind::TypeCheckOnly,
        crate::snippets::types::ValidationLevel::Run => return None,
    };
    Some(SnippetAnnotation { kind, reason })
}

fn parse_reason_attr(inner: &str) -> Option<String> {
    let marker = "reason=";
    let start = inner.find(marker)? + marker.len();
    let rest = inner[start..].trim_start();

    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(stripped[..end].to_string());
    }

    if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        return Some(stripped[..end].to_string());
    }

    let value: String = rest.chars().take_while(|ch| !ch.is_whitespace()).collect();
    if value.is_empty() { None } else { Some(value) }
}

#[must_use]
pub fn count_by_language(snippets: &[Snippet]) -> Vec<(Language, usize)> {
    let mut counts: HashMap<Language, usize> = HashMap::new();
    for snippet in snippets {
        *counts.entry(snippet.language).or_default() += 1;
    }

    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by_key(|(language, _)| language.to_string());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_annotations() {
        assert_eq!(
            parse_annotation("<!-- snippet:skip -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::Skip)
        );
        assert_eq!(
            parse_annotation("<!-- snippet:compile-only -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::CompileOnly)
        );
        assert_eq!(
            parse_annotation("<!-- snippet:syntax-only -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::SyntaxOnly)
        );
        assert_eq!(
            parse_annotation("<!-- snippet:typecheck-only -->").map(|annotation| annotation.kind),
            Some(SnippetAnnotationKind::TypeCheckOnly)
        );
    }

    #[test]
    fn parses_annotation_reason_attribute() {
        let annotation = parse_annotation(r#"<!-- snippet:skip reason="requires service" -->"#).unwrap();
        assert_eq!(annotation.kind, SnippetAnnotationKind::Skip);
        assert_eq!(annotation.reason.as_deref(), Some("requires service"));
    }

    #[test]
    fn infers_language_from_nested_snippet_path() {
        let base = Path::new("/repo/docs");
        let path = Path::new("/repo/docs/snippets/python/example.md");
        assert_eq!(infer_language_from_path(path, base), Some(Language::Python));
    }

    #[test]
    fn does_not_infer_language_from_non_language_directories() {
        let base = Path::new("/repo/docs");
        let path = Path::new("/repo/docs/cli/usage.md");
        assert_eq!(infer_language_from_path(path, base), None);
    }
}
