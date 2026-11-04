use super::{SnippetGenerationReport, generate_snippet_report_with_views};
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{E2eConfig, SnippetConfig};
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::fixture::Fixture;
use anyhow::Result;

pub(super) struct SnippetRenderContext<'a> {
    pub(super) e2e: &'a E2eConfig,
    pub(super) crate_config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [TypeDef],
    pub(super) enums: &'a [EnumDef],
    pub(super) functions: &'a [crate::core::ir::FunctionDef],
    pub(super) errors: &'a [crate::core::ir::ErrorDef],
}

/// ~keep `errors` is `ApiSurface::errors`. Only a caller that holds it can let a snippet name the
/// exception class a specific error variant maps to (see `codegen::snippet_error_branch`); the
/// coverage-only paths, which never publish a rendered body, pass an empty slice and get the
/// generic catch-all, which is exactly what they rendered before.
#[expect(clippy::too_many_arguments, reason = "preserves the public snippet generation API")]
pub fn generate_snippet_report(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) -> Result<SnippetGenerationReport> {
    crate::with_extensions(|extensions| {
        let context = SnippetRenderContext {
            e2e,
            crate_config,
            type_defs,
            enums,
            functions,
            errors,
        };
        generate_snippet_report_with_extensions(fixtures, languages, snippets, &context, extensions)
    })
}

pub(super) fn generate_snippet_report_with_extensions(
    fixtures: &[Fixture],
    languages: &[String],
    snippets: &SnippetConfig,
    context: &SnippetRenderContext<'_>,
    extensions: &[Box<dyn crate::Extension>],
) -> Result<SnippetGenerationReport> {
    generate_snippet_report_with_views(fixtures, languages, snippets, context, None, extensions)
}

pub(in crate::e2e) fn generate_fixture_snippet_report(
    fixtures: &[Fixture],
    languages: &[String],
    e2e: &E2eConfig,
    snippets: &SnippetConfig,
    crate_config: &ResolvedCrateConfig,
    api: crate::e2e::fixture_api::FixtureApi<'_>,
) -> Result<SnippetGenerationReport> {
    let canonical = SnippetRenderContext {
        e2e,
        crate_config,
        type_defs: api.type_defs,
        enums: api.enums,
        functions: api.functions,
        errors: api.errors,
    };
    let projected = api.projected().map(|api| SnippetRenderContext {
        type_defs: api.type_defs,
        enums: api.enums,
        ..canonical
    });
    crate::with_extensions(|extensions| {
        generate_snippet_report_with_views(
            fixtures,
            languages,
            snippets,
            &canonical,
            projected.as_ref(),
            extensions,
        )
    })
}
