//! Shared Rust e2e test-file helpers.

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

pub(super) fn resolve_function_name_for_call(call_config: &crate::e2e::config::CallConfig) -> String {
    call_config
        .overrides
        .get("rust")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| call_config.function.clone())
}

pub(super) fn resolve_module(e2e_config: &E2eConfig, dep_name: &str) -> String {
    resolve_module_for_call(&e2e_config.call, dep_name)
}

pub(super) fn resolve_module_for_call(call_config: &crate::e2e::config::CallConfig, dep_name: &str) -> String {
    // For Rust, the module name is the crate identifier (underscores).
    // Priority: override.crate_name > override.module > dep_name
    let overrides = call_config.overrides.get("rust");
    overrides
        .and_then(|o| o.crate_name.clone())
        .or_else(|| overrides.and_then(|o| o.module.clone()))
        .unwrap_or_else(|| dep_name.to_string())
}

pub(in crate::e2e::codegen::rust) fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture.skip.as_ref().is_some_and(|s| s.should_skip(language))
}

pub(super) fn client_trait_imports(
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
) -> std::collections::BTreeSet<String> {
    let mut imports = std::collections::BTreeSet::new();
    for fixture in fixtures {
        if fixture.http.is_some() {
            continue;
        }
        let call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        if fixture.mock_response.is_none() && resolve_function_name_for_call(call).is_empty()
            || crate::e2e::codegen::client_factory::is_free_function_call(e2e_config, call, "rust")
        {
            continue;
        }
        let traits = call
            .overrides
            .get("rust")
            .map(|overrides| &overrides.trait_imports)
            .filter(|traits| !traits.is_empty())
            .or_else(|| {
                e2e_config
                    .call
                    .overrides
                    .get("rust")
                    .map(|overrides| &overrides.trait_imports)
            });
        if let Some(traits) = traits {
            imports.extend(traits.iter().cloned());
        }
    }
    imports
}

/// Returns true when the rendered test body contains a word-boundary reference to `symbol`.
/// Used to decide whether a `use ...::Symbol;` import is needed; emitting it unconditionally
/// trips `-D unused_imports` for fixtures whose bodies never reference the symbol.
pub(super) fn body_references_symbol(body: &str, symbol: &str) -> bool {
    let bytes = body.as_bytes();
    let sym = symbol.as_bytes();
    let n = bytes.len();
    let m = sym.len();
    if m == 0 || m > n {
        return false;
    }
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i + m <= n {
        if bytes[i..i + m] == *sym {
            let before_ok = i == 0 || !is_word(bytes[i - 1]);
            let after_ok = i + m == n || !is_word(bytes[i + m]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}
