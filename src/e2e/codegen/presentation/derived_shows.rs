use super::*;

/// Write the field paths [`super::resolve`] would derive from `fixture`'s own assertions into the
/// fixture's `docs.shows`, so that [`Fixture::has_docs_presentation`] reports them.
///
/// ~keep That predicate is the single question the *call emitter* asks about the *snippet's*
/// intent: `rust/test_file/test_function.rs` uses it to decide both whether the call binds a
/// named `result` at all (rather than `let _ =`) and whether a `Result`-returning call is
/// unwrapped before that binding. Before this, it only ever saw hand-authored
/// `shows`/`presentation` blocks, so the operations #199 derives from assertions were invisible
/// to it: 283 generated Rust snippets in one consumer repo bound `let _ = convert(...)` and then
/// printed `result.content` (`E0425`), and any that had bound it would have field-accessed a
/// `Result` (`E0609`). Materializing the derivation into the fixture — rather than teaching the
/// call emitter to re-derive it — is what keeps the two generators reading one fact.
///
/// A fixture that already hand-authors `shows` or `presentation.operations` is left alone: its
/// operations are the authored ones, and `has_docs_presentation` already reports them. Must be
/// called before any caller clears `assertions`, which is where the derivation reads from.
pub(super) fn apply_derived_shows(
    fixture: &mut Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    factory: &ResolverFactory<'_>,
) {
    if fixture.docs.is_none() || fixture.has_docs_presentation() {
        return;
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if factory.returns_native_bytes(call, language) {
        return;
    }
    let resolver = factory.build(e2e_config, call, language);
    let paths: Vec<String> = default_operations_from_assertions(fixture, call, language, &resolver)
        .into_iter()
        .filter_map(|operation| match operation {
            FixtureDocsOperation::Show { path, .. } => Some(path),
            FixtureDocsOperation::Iterate { .. } => None,
        })
        .collect();
    if paths.is_empty() {
        return;
    }
    if let Some(docs) = fixture.docs.as_mut() {
        docs.shows = paths;
    }
}
