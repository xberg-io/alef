//! API reference documentation generator for alef polyglot bindings.
//!
//! Generates per-language `api-{lang}.md` files plus shared `configuration.md`
//! and `errors.md` files from the alef IR (`ApiSurface`).

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use heck::ToPascalCase;

mod descriptions;
pub mod doc_cleaning;
mod examples;
mod formatting;
mod language_pages;
mod naming;
mod shared_pages;
mod signatures;
mod sorting;
mod template_env;
#[cfg(test)]
mod tests;
mod type_mapping;
mod version_labels;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use doc_cleaning::clean_doc;
pub use type_mapping::doc_type;

/// Generate API reference documentation for the given languages.
///
/// Produces one `api-{lang}.md` per language, plus shared `configuration.md`,
/// `types.md`, and `errors.md` files written into `output_dir`.
pub fn generate_docs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    output_dir: &str,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();
    let ffi_prefix = &config.ffi_prefix().to_pascal_case();

    for &lang in languages {
        files.push(language_pages::generate_lang_doc(
            api, config, lang, output_dir, ffi_prefix,
        )?);
    }

    files.push(shared_pages::generate_configuration_doc(api, config, output_dir)?);
    files.push(shared_pages::generate_types_doc(api, output_dir)?);
    files.push(shared_pages::generate_errors_doc(api, output_dir)?);

    for file in &mut files {
        file.content = doc_cleaning::wrap_bare_urls(&file.content);
        if !file.content.ends_with('\n') {
            file.content.push('\n');
        }
    }

    Ok(files)
}
