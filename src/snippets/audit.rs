use crate::snippets::gaps::{discover_includes, parse_include_target};
use crate::snippets::parser::{self, FrontmatterStatus};
use crate::snippets::types::Language;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default)]
pub struct AuditConfig {
    pub docs_dirs: Vec<PathBuf>,
    pub snippet_dirs: Vec<PathBuf>,
    pub require_frontmatter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditIssueKind {
    BrokenFrontmatter,
    MissingFrontmatter,
    BrokenFence,
    MissingInclude,
    InvalidInclude,
    UnknownLanguage,
    UnreadableFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditIssue {
    pub kind: AuditIssueKind,
    pub severity: AuditSeverity,
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    pub issues: Vec<AuditIssue>,
}

impl AuditReport {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|issue| issue.severity == AuditSeverity::Error)
    }
}

/// Audit documentation snippets and include references for structural errors.
///
/// # Errors
///
/// This function reports unreadable files as audit issues rather than returning
/// an error, so callers can see every problem found in one run.
#[must_use]
pub fn audit(config: &AuditConfig) -> AuditReport {
    let mut issues = Vec::new();
    for snippet_dir in &config.snippet_dirs {
        issues.extend(audit_snippets(snippet_dir, config.require_frontmatter));
    }
    for docs_dir in &config.docs_dirs {
        issues.extend(audit_docs(docs_dir));
    }
    issues.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.message.cmp(&right.message))
    });
    AuditReport { issues }
}

fn audit_snippets(snippet_dir: &Path, require_frontmatter: bool) -> Vec<AuditIssue> {
    markdown_files(snippet_dir)
        .into_iter()
        .flat_map(|path| audit_snippet_file(&path, require_frontmatter))
        .collect()
}

fn audit_snippet_file(path: &Path, require_frontmatter: bool) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            issues.push(issue(
                AuditIssueKind::UnreadableFile,
                path,
                1,
                format!("failed to read snippet file: {err}"),
            ));
            return issues;
        }
    };

    match parser::frontmatter_status(&content) {
        FrontmatterStatus::Missing if require_frontmatter => issues.push(issue(
            AuditIssueKind::MissingFrontmatter,
            path,
            1,
            "snippet markdown is missing YAML frontmatter".to_string(),
        )),
        FrontmatterStatus::Malformed(message) => {
            issues.push(issue(AuditIssueKind::BrokenFrontmatter, path, 1, message))
        }
        FrontmatterStatus::Present => {}
        FrontmatterStatus::Missing => {}
    }

    issues.extend(audit_fences(path, &content));
    issues
}

fn audit_docs(docs_dir: &Path) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    for path in markdown_files(docs_dir) {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                issues.push(issue(
                    AuditIssueKind::UnreadableFile,
                    &path,
                    1,
                    format!("failed to read documentation file: {err}"),
                ));
                continue;
            }
        };

        issues.extend(audit_fences(&path, &content));
        issues.extend(audit_includes(&path, &content));
    }

    match discover_includes(&[docs_dir.to_path_buf()], &[]) {
        Ok(references) => {
            for reference in references {
                if !reference.target.exists() {
                    issues.push(issue(
                        AuditIssueKind::MissingInclude,
                        &reference.source,
                        reference.line,
                        format!("included snippet does not exist: {}", reference.target.display()),
                    ));
                }
            }
        }
        Err(err) => issues.push(issue(
            AuditIssueKind::UnreadableFile,
            docs_dir,
            1,
            format!("failed to discover include references: {err}"),
        )),
    }

    issues
}

fn audit_includes(path: &Path, content: &str) -> Vec<AuditIssue> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("--8<--") && parse_include_target(line).is_none())
        .map(|(index, _)| {
            issue(
                AuditIssueKind::InvalidInclude,
                path,
                index + 1,
                "invalid MkDocs include syntax, expected --8<-- \"path\"".to_string(),
            )
        })
        .collect()
}

fn audit_fences(path: &Path, content: &str) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    let mut open: Option<(usize, String)> = None;

    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("```") else {
            continue;
        };

        if rest.starts_with('`') {
            continue;
        }

        if open.is_some() && (rest.is_empty() || rest.chars().all(|ch| ch == '`')) {
            open = None;
            continue;
        }

        if open.is_none() {
            let tag = rest.split_whitespace().next().unwrap_or_default().to_string();
            if tag.is_empty() {
                issues.push(issue(
                    AuditIssueKind::UnknownLanguage,
                    path,
                    index + 1,
                    "fenced code block is missing a language tag".to_string(),
                ));
            } else if Language::from_fence_tag(&tag) == Language::Unknown && !is_known_display_tag(&tag) {
                issues.push(issue(
                    AuditIssueKind::UnknownLanguage,
                    path,
                    index + 1,
                    format!("unknown fenced code language: {tag}"),
                ));
            }
            open = Some((index + 1, tag));
        }
    }

    if let Some((line, _)) = open {
        issues.push(issue(
            AuditIssueKind::BrokenFence,
            path,
            line,
            "fenced code block is missing a closing fence".to_string(),
        ));
    }

    issues
}

fn markdown_files(base: &Path) -> Vec<PathBuf> {
    if !base.exists() {
        return Vec::new();
    }

    let mut files: Vec<PathBuf> = WalkDir::new(base)
        .follow_links(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| matches!(extension.to_lowercase().as_str(), "md" | "markdown"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

fn issue(kind: AuditIssueKind, path: &Path, line: usize, message: String) -> AuditIssue {
    AuditIssue {
        kind,
        severity: AuditSeverity::Error,
        path: path.to_path_buf(),
        line,
        message,
    }
}

/// Returns true for fence tags that are valid display-only markup the audit
/// should accept without flagging as `UnknownLanguage`. These tags do not map
/// to executable validators in `Language::from_fence_tag`, but they are
/// well-known in the Markdown / docs ecosystem (data formats, diagram DSLs,
/// shell session transcripts, third-party JVM build files, etc.).
fn is_known_display_tag(tag: &str) -> bool {
    matches!(
        tag.trim().to_lowercase().as_str(),
        "json"
            | "yaml"
            | "yml"
            | "xml"
            | "ini"
            | "csv"
            | "tsv"
            | "properties"
            | "env"
            | "diff"
            | "patch"
            | "html"
            | "css"
            | "scss"
            | "sass"
            | "svg"
            | "markdown"
            | "md"
            | "mdx"
            | "rst"
            | "tex"
            | "latex"
            | "mermaid"
            | "plantuml"
            | "graphviz"
            | "dot"
            | "d2"
            | "groovy"
            | "gradle"
            | "make"
            | "makefile"
            | "cmake"
            | "nginx"
            | "apache"
            | "text"
            | "txt"
            | "plain"
            | "plaintext"
            | "output"
            | "log"
            | "console"
            | "sql"
            | "graphql"
            | "gql"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_missing_frontmatter_and_broken_fence() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = dir.path().join("snippets");
        std::fs::create_dir_all(&snippets).unwrap();
        std::fs::write(snippets.join("example.md"), "```python\nprint('ok')\n").unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: Vec::new(),
            snippet_dirs: vec![snippets],
            require_frontmatter: true,
        });

        assert!(report.has_errors());
        assert_eq!(report.issues.len(), 2);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::MissingFrontmatter)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::BrokenFence)
        );
    }

    #[test]
    fn reports_invalid_and_missing_includes() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("index.md"),
            "--8<-- snippets/python/example.md\n--8<-- \"snippets/python/missing.md\"\n",
        )
        .unwrap();

        let report = audit(&AuditConfig {
            docs_dirs: vec![docs],
            snippet_dirs: Vec::new(),
            require_frontmatter: false,
        });

        assert_eq!(report.issues.len(), 2);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::InvalidInclude)
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.kind == AuditIssueKind::MissingInclude)
        );
    }
}
