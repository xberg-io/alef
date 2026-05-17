//! `alef snippets` subcommand — discover, validate, audit, and gap-check documentation snippets.

use alef_snippets::audit::{AuditConfig, AuditSeverity, audit};
use alef_snippets::discovery;
use alef_snippets::gaps::{GapConfig, detect_gaps};
use alef_snippets::output;
use alef_snippets::runner::{RunnerConfig, run_validation};
use alef_snippets::types::{Language, ValidationLevel};
use alef_snippets::validators::ValidatorRegistry;
use clap::Subcommand;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Subcommand)]
pub enum SnippetsAction {
    /// List discovered snippets and a per-language count summary.
    List {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, value_delimiter = ',')]
        languages: Option<Vec<String>>,
    },

    /// Validate snippet syntax (and optionally compilation / execution).
    Validate {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short = 'L', long, default_value = "syntax")]
        level: ValidationLevel,

        #[arg(short, long, value_delimiter = ',')]
        languages: Option<Vec<String>>,

        #[arg(short, long)]
        output: Option<PathBuf>,

        #[arg(short = 'j', long, default_value = "4")]
        jobs: usize,

        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,

        #[arg(long)]
        fail_fast: bool,

        #[arg(long)]
        include: Option<String>,

        #[arg(long)]
        show_code: bool,
    },

    /// Parse a single file and print its code blocks.
    Parse {
        file: PathBuf,
    },

    /// Structural integrity audit (frontmatter, fences, include targets).
    Audit {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, num_args = 0..)]
        docs: Vec<PathBuf>,

        #[arg(long)]
        require_frontmatter: bool,
    },

    /// Coverage gap report (unreferenced snippets, missing language variants).
    Gaps {
        #[arg(short, long, required = true, num_args = 1..)]
        snippets: Vec<PathBuf>,

        #[arg(short, long, num_args = 0..)]
        docs: Vec<PathBuf>,

        #[arg(short = 'L', long, value_delimiter = ',')]
        required_languages: Option<Vec<String>>,
    },
}

pub fn run(action: SnippetsAction) -> ExitCode {
    match action {
        SnippetsAction::List { snippets, languages } => run_list(&snippets, languages.as_ref()),
        SnippetsAction::Validate {
            snippets,
            level,
            languages,
            output: output_path,
            jobs,
            timeout,
            fail_fast,
            include,
            show_code,
        } => run_validate(
            &snippets,
            level,
            languages.as_ref(),
            output_path,
            jobs,
            timeout,
            fail_fast,
            include.as_ref(),
            show_code,
        ),
        SnippetsAction::Parse { file } => run_parse(&file),
        SnippetsAction::Audit {
            snippets,
            docs,
            require_frontmatter,
        } => run_audit(&snippets, &docs, require_frontmatter),
        SnippetsAction::Gaps {
            snippets,
            docs,
            required_languages,
        } => run_gaps(&snippets, &docs, required_languages.as_ref()),
    }
}

fn parse_language_filter(languages: Option<&[String]>) -> Option<Vec<Language>> {
    let languages = languages?;
    Some(
        languages
            .iter()
            .map(|language| Language::from_fence_tag(language))
            .filter(|language| *language != Language::Unknown)
            .collect(),
    )
}

fn run_list(snippets: &[PathBuf], languages: Option<&Vec<String>>) -> ExitCode {
    let filter = parse_language_filter(languages.map(Vec::as_slice));
    match discovery::discover_snippets(snippets, filter.as_deref()) {
        Ok(found) => {
            output::print_snippet_list(&found);
            println!();
            for (language, count) in &discovery::count_by_language(&found) {
                println!("  {language:<12} {count}");
            }
            println!();
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Error discovering snippets: {err}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_validate(
    snippets: &[PathBuf],
    level: ValidationLevel,
    languages: Option<&Vec<String>>,
    output_path: Option<PathBuf>,
    jobs: usize,
    timeout: u64,
    fail_fast: bool,
    include: Option<&String>,
    show_code: bool,
) -> ExitCode {
    let filter = parse_language_filter(languages.map(Vec::as_slice));
    let mut found = match discovery::discover_snippets(snippets, filter.as_deref()) {
        Ok(found) => found,
        Err(err) => {
            eprintln!("Error discovering snippets: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(pattern) = &include {
        found.retain(|snippet| snippet.path.to_string_lossy().contains(pattern.as_str()));
    }

    if found.is_empty() {
        println!("No snippets found.");
        return ExitCode::SUCCESS;
    }

    println!("Validating {} snippets at level '{level}'...", found.len());
    let registry = ValidatorRegistry::new();
    let config = RunnerConfig {
        level,
        parallelism: jobs,
        timeout_secs: timeout,
        fail_fast,
    };

    match run_validation(&found, &registry, &config) {
        Ok(summary) => {
            output::print_summary(&summary, show_code);

            if let Some(path) = output_path {
                if let Err(err) = output::write_json(&summary.results, &path) {
                    eprintln!("Error writing JSON output: {err}");
                } else {
                    println!("Results written to {}", path.display());
                }
            }

            if summary.has_failures() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!("Error running validation: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_parse(file: &Path) -> ExitCode {
    match alef_snippets::parser::parse_code_blocks(file) {
        Ok(blocks) => {
            if blocks.is_empty() {
                println!("No code blocks found in {}", file.display());
            } else {
                for (index, block) in blocks.iter().enumerate() {
                    println!("--- Block {} (line {}) ---", index + 1, block.start_line);
                    println!("Language: {}", block.lang);
                    if let Some(title) = &block.title {
                        println!("Title: {title}");
                    }
                    if let Some(comment) = &block.preceding_comment {
                        println!("Annotation: {comment}");
                    }
                    println!("Code ({} lines):", block.code.lines().count());
                    println!("{}", block.code);
                    println!();
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Error parsing {}: {err}", file.display());
            ExitCode::FAILURE
        }
    }
}

fn run_audit(snippet_dirs: &[PathBuf], docs_dirs: &[PathBuf], require_frontmatter: bool) -> ExitCode {
    let config = AuditConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        require_frontmatter,
    };
    let report = audit(&config);
    if report.issues.is_empty() {
        println!("Audit clean: no issues found.");
        return ExitCode::SUCCESS;
    }
    println!("Audit found {} issue(s):", report.issues.len());
    for issue in &report.issues {
        let severity = match issue.severity {
            AuditSeverity::Error => "ERROR",
            AuditSeverity::Warning => "WARN",
        };
        println!(
            "  [{severity}] {}:{} ({:?}) {}",
            issue.path.display(),
            issue.line,
            issue.kind,
            issue.message
        );
    }
    if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_gaps(
    snippet_dirs: &[PathBuf],
    docs_dirs: &[PathBuf],
    required_languages: Option<&Vec<String>>,
) -> ExitCode {
    let required = required_languages
        .map(|languages| {
            languages
                .iter()
                .map(|language| Language::from_fence_tag(language))
                .filter(|language| *language != Language::Unknown)
                .collect()
        })
        .unwrap_or_default();
    let config = GapConfig {
        docs_dirs: docs_dirs.to_vec(),
        snippet_dirs: snippet_dirs.to_vec(),
        required_languages: required,
    };
    let report = match detect_gaps(&config) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("Error detecting gaps: {err}");
            return ExitCode::FAILURE;
        }
    };
    if !report.has_gaps() {
        println!("No gaps found.");
        return ExitCode::SUCCESS;
    }
    if !report.missing_references.is_empty() {
        println!("Missing include targets ({}):", report.missing_references.len());
        for reference in &report.missing_references {
            println!(
                "  {}:{} → {}",
                reference.source.display(),
                reference.line,
                reference.target.display()
            );
        }
    }
    if !report.unreferenced_snippets.is_empty() {
        println!("Unreferenced snippets ({}):", report.unreferenced_snippets.len());
        for path in &report.unreferenced_snippets {
            println!("  {}", path.display());
        }
    }
    if !report.missing_language_variants.is_empty() {
        println!(
            "Missing language variants ({}):",
            report.missing_language_variants.len()
        );
        for variant in &report.missing_language_variants {
            println!("  {} — {}", variant.group.display(), variant.language);
        }
    }
    if !report.skips_without_reason.is_empty() {
        println!("Skips without reason ({}):", report.skips_without_reason.len());
        for location in &report.skips_without_reason {
            println!(
                "  {}:{} (block {})",
                location.path.display(),
                location.line,
                location.block_index
            );
        }
    }
    if !report.unknown_languages.is_empty() {
        println!("Unknown languages ({}):", report.unknown_languages.len());
        for unknown in &report.unknown_languages {
            println!("  {}:{} tag={}", unknown.path.display(), unknown.line, unknown.tag);
        }
    }
    ExitCode::FAILURE
}
