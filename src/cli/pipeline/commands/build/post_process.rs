use crate::core::backend::PostProcessor;
use crate::core::config::ResolvedCrateConfig;
use anyhow::Context as _;
use std::path::Path;
use tracing::{debug, info};

pub(super) fn run(
    file_path: &Path,
    crate_dir: &Path,
    processor: &PostProcessor,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<()> {
    if !file_path.exists() {
        debug!("PostProcessFile target not found: {}", file_path.display());
        return Ok(());
    }
    let content = std::fs::read_to_string(file_path)
        .with_context(|| format!("failed to read post-process target {}", file_path.display()))?;
    let processed = apply(&content, processor, crate_dir, config)?;
    if processed != content {
        std::fs::write(file_path, &processed)
            .with_context(|| format!("failed to write post-processed file {}", file_path.display()))?;
        info!("PostProcessed {}: {:?}", file_path.display(), processor);
    } else {
        debug!(
            "PostProcessFile {}: no changes (already rewritten or absent variants)",
            file_path.display()
        );
    }
    Ok(())
}

fn apply(
    content: &str,
    processor: &PostProcessor,
    crate_dir: &Path,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<String> {
    Ok(match processor {
        PostProcessor::FrbDartSealedVariants => {
            crate::backends::dart::rewrite_frb_sealed_variants(content, &config.dart_pubspec_name())
        }
        PostProcessor::FrbDartSingleVariantCopyWith => crate::backends::dart::repair_single_variant_copy_with(content),
        PostProcessor::FrbDartExcludeFunctions(excluded) => {
            let exclude_set: std::collections::HashSet<&str> = excluded.iter().map(|s| s.as_str()).collect();
            crate::backends::dart::filter_excluded_functions(content, &exclude_set)
        }
        PostProcessor::FrbDartOptionalFieldsWithDefaults => {
            super::frb_constructor_defaults::apply(content, crate_dir, config)?
        }
        PostProcessor::FrbDartFixHandlerExecutorCalls => crate::backends::dart::fix_handler_executor_calls(content),
        PostProcessor::FrbDartInjectTextMethods(type_names) => {
            crate::backends::dart::inject_display_as_text_methods(content, type_names)
        }
        PostProcessor::DartStripTrailingWhitespace => crate::backends::dart::strip_trailing_whitespace(content),
    })
}
