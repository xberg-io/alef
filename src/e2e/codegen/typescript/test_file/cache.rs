use super::*;

/// Detect if cache isolation is needed: checks if any fixture calls `cleanCache`
/// and if a `configure` function is available.
/// Returns (has_clean_cache, has_configure).
pub(in crate::e2e::codegen::typescript::test_file) fn detect_cache_isolation_needs(
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
) -> (bool, bool) {
    let has_clean_cache = fixtures.iter().any(|fixture| {
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        resolve_node_function_name(call_config) == "cleanCache"
    });

    let has_configure = e2e_config
        .calls
        .iter()
        .any(|(_, call_config)| resolve_node_function_name(call_config) == "configure")
        || resolve_node_function_name(&e2e_config.call) == "configure";

    (has_clean_cache, has_configure)
}

/// Emit the cache isolation setup code (beforeAll/afterAll blocks).
pub(in crate::e2e::codegen::typescript::test_file) fn emit_cache_isolation_setup(out: &mut String) {
    let rendered = crate::e2e::template_env::render("typescript/cache_isolation_setup.jinja", crate::alef_context! {});
    out.push_str(&rendered);
}
