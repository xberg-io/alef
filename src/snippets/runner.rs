use crate::snippets::error::Result;
use crate::snippets::types::{
    RunSummary, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel, ValidationResult,
};
use crate::snippets::validators::ValidatorRegistry;
use rayon::prelude::*;
use std::time::Instant;

pub struct RunnerConfig {
    pub level: ValidationLevel,
    pub parallelism: usize,
    pub timeout_secs: u64,
    pub fail_fast: bool,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            level: ValidationLevel::Syntax,
            parallelism: available_parallelism(),
            timeout_secs: 120,
            fail_fast: false,
        }
    }
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get)
}

/// Run validation over the provided snippets.
///
/// # Errors
///
/// Returns an error when the validation thread pool cannot be created.
pub fn run_validation(snippets: &[Snippet], registry: &ValidatorRegistry, config: &RunnerConfig) -> Result<RunSummary> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.parallelism)
        .build()
        .map_err(|err| crate::snippets::error::Error::Other(format!("failed to build thread pool: {err}")))?;

    let fail_fast = config.fail_fast;
    let results: Vec<ValidationResult> = pool.install(|| {
        if fail_fast {
            let mut results = Vec::with_capacity(snippets.len());
            for snippet in snippets {
                let result = validate_one(snippet, registry, config);
                let should_stop = matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
                results.push(result);
                if should_stop {
                    break;
                }
            }
            results
        } else {
            snippets
                .par_iter()
                .map(|snippet| validate_one(snippet, registry, config))
                .collect()
        }
    });

    Ok(RunSummary::from_results(results))
}

fn validate_one(snippet: &Snippet, registry: &ValidatorRegistry, config: &RunnerConfig) -> ValidationResult {
    if let Some(annotation) = &snippet.annotation {
        match annotation.kind {
            SnippetAnnotationKind::Skip => {
                return ValidationResult {
                    snippet: snippet.clone(),
                    status: SnippetStatus::Skip,
                    level: config.level,
                    message: Some(skip_message("skipped via annotation", annotation.reason.as_deref())),
                    duration_ms: 0,
                };
            }
            SnippetAnnotationKind::SyntaxOnly if config.level > ValidationLevel::Syntax => {
                return ValidationResult {
                    snippet: snippet.clone(),
                    status: SnippetStatus::Skip,
                    level: config.level,
                    message: Some("annotation limits to syntax-only".to_string()),
                    duration_ms: 0,
                };
            }
            SnippetAnnotationKind::CompileOnly if config.level > ValidationLevel::Compile => {
                return ValidationResult {
                    snippet: snippet.clone(),
                    status: SnippetStatus::Skip,
                    level: config.level,
                    message: Some("annotation limits to compile-only".to_string()),
                    duration_ms: 0,
                };
            }
            _ => {}
        }
    }

    let Some(validator) = registry.get(snippet.language) else {
        return ValidationResult {
            snippet: snippet.clone(),
            status: SnippetStatus::Unavailable,
            level: config.level,
            message: Some(format!("no validator for {}", snippet.language)),
            duration_ms: 0,
        };
    };

    if !validator.is_available() {
        return ValidationResult {
            snippet: snippet.clone(),
            status: SnippetStatus::Unavailable,
            level: config.level,
            message: Some(format!("{} toolchain not found", snippet.language)),
            duration_ms: 0,
        };
    }

    let effective_level = config.level.min(validator.max_level());
    let start = Instant::now();
    let (mut status, message) = match validator.validate(snippet, effective_level, config.timeout_secs) {
        Ok((status, message)) => (status, message),
        Err(err) => (SnippetStatus::Error, Some(err.to_string())),
    };
    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    if status == SnippetStatus::Fail
        && effective_level == ValidationLevel::Syntax
        && let Some(error_output) = &message
        && validator.is_dependency_error(error_output)
    {
        status = SnippetStatus::Pass;
    }

    ValidationResult {
        snippet: snippet.clone(),
        status,
        level: effective_level,
        message,
        duration_ms,
    }
}

fn skip_message(message: &str, reason: Option<&str>) -> String {
    match reason {
        Some(reason) if !reason.is_empty() => format!("{message}: {reason}"),
        _ => message.to_string(),
    }
}
