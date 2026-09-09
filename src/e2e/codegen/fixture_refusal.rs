//! Ledger for fixture-vs-IR refusals a generator recognises too deep in its call tree to
//! return a `Result` from.
//!
//! A refusal is a consumer-fixable configuration or fixture mistake -- today, a fixture object
//! keyed by a field the type it is being built as does not declare. It used to `panic!` at the
//! point of recognition, which took down the whole `alef all` process at exit 101: every other
//! backend's e2e codegen, every later crate, and every later stage (README, docs, snippet
//! validation) never ran, and the operator got a Rust panic instead of a diagnostic. That is
//! strictly worse than the mistake being reported -- the same argument `e2e::run_generators`
//! already makes for a backend's `bail!`, and the same one `bin_cli::all_commands::stage_failures`
//! makes for a crate's post-build failure:
//! isolate the failure to the unit that produced it, let every sibling finish, and report
//! everything at the end with a non-zero exit.
//!
//! Recognition happens inside `String`-returning expression builders nested five or more frames
//! below the backend's `generate`, so the refusal is recorded here and drained at the two
//! boundaries that do own a `Result`: [`super::E2eCodegen::generate_gated`] (which turns it into
//! the per-backend `Err` `run_generators` already isolates) and `snippets::render_body`.
//! Threading `Result<String>` through the builders instead
//! would be the more direct encoding, but it rewrites every one of their call sites and every
//! test that exercises them for no change in behaviour at either boundary. ~keep
//!
//! Thread-local rather than a `Mutex` global, for the same reason as this module's neighbour
//! `codegen::SKIP_LEDGER`: a backend's whole `generate` runs on one thread, `bin_cli`'s
//! rayon parallelism is across crates (so each worker gets its own ledger and drains it itself),
//! and `#[test]` isolation comes free because cargo gives every test its own thread. ~keep

use std::cell::RefCell;

/// Which configuration level supplied the type a refused fixture value was validated against.
///
/// The distinction is the whole point of this diagnostic: a *file-level* `options_type` for a
/// language applies to every call that does not override it, so adding one to fix call A
/// silently re-types call B. That is the shape that misled a real operator for an hour, because
/// the message only ever said "fix the fixture or the Rust struct" -- and in that incident both
/// were already correct. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionsTypeSource {
    /// `[e2e.calls.<call>.overrides.<lang>].options_type` -- declared for this call specifically.
    PerCall,
    /// `[e2e.call.overrides.<lang>].options_type` -- the file-level default for this language,
    /// inherited because this call declares none of its own.
    LanguageDefault,
    /// `[e2e.calls.<call>].options_type` -- the call's own language-agnostic setting. Only the
    /// documentation-snippet path falls back to this; the test-file path falls back to the
    /// file-level per-language default instead. ~keep
    CallLevel,
    /// No `options_type` at any level; the type came from the argument's `element_type` or
    /// from the IR.
    Unset,
}

/// Classify the test-file path's resolution: this call's own per-language override, else the
/// file-level `[e2e.call.overrides.<lang>]` default it inherits.
pub(crate) fn language_default_source(per_call: Option<&str>, file_level: Option<&str>) -> OptionsTypeSource {
    match (per_call, file_level) {
        (Some(_), _) => OptionsTypeSource::PerCall,
        (None, Some(_)) => OptionsTypeSource::LanguageDefault,
        (None, None) => OptionsTypeSource::Unset,
    }
}

/// Classify the documentation-snippet path's resolution: this call's own per-language override,
/// else the call's language-agnostic `options_type`.
///
/// ~keep Deliberately a second constructor rather than a shared one: the two paths genuinely
/// fall back to different config keys (`ResolvedE2eCallRecipe::resolve` versus the file-level
/// `options_type` threaded into `render_test_file`), and a diagnostic that names the wrong key
/// is the defect this module exists to stop repeating.
pub(crate) fn call_level_source(per_call: Option<&str>, call_level: Option<&str>) -> OptionsTypeSource {
    match (per_call, call_level) {
        (Some(_), _) => OptionsTypeSource::PerCall,
        (None, Some(_)) => OptionsTypeSource::CallLevel,
        (None, None) => OptionsTypeSource::Unset,
    }
}

/// Where inside the argument value the refused object sat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefusalSite {
    /// The argument value itself, built as the type the argument resolves to.
    Argument,
    /// An object nested inside the argument, built as the type the IR declares for `via`.
    /// A wrong `options_type` at the root re-resolves every nested type under it, so the
    /// per-call lever is still worth naming here.
    Nested { via: String },
}

/// One refused fixture key, plus the call context a later frame attributes to it.
#[derive(Debug, Clone)]
pub(crate) struct FixtureKeyRefusal {
    type_name: String,
    key: String,
    site: RefusalSite,
    attribution: Option<Attribution>,
    kind: RefusalKind,
}

#[derive(Debug, Clone, Copy)]
enum RefusalKind {
    Field,
    EnumValue,
}

#[derive(Debug, Clone)]
struct Attribution {
    language: String,
    fixture_id: String,
    /// The `[e2e.calls.<key>]` this fixture routed to, or `None` for the default `[e2e.call]`.
    call_key: Option<String>,
    options_type_source: OptionsTypeSource,
}

thread_local! {
    static LEDGER: RefCell<Vec<FixtureKeyRefusal>> = const { RefCell::new(Vec::new()) };
}

/// Record one refused fixture key. The caller is inside an expression builder and knows nothing
/// about which call or language it is serving; [`attribute`] fills that in from the frame that
/// does.
pub(crate) fn record(type_name: &str, key: &str, site: RefusalSite) {
    record_value(type_name, key, site, RefusalKind::Field);
}

pub(crate) fn record_enum_value(type_name: &str, value: &str) {
    record_value(type_name, value, RefusalSite::Argument, RefusalKind::EnumValue);
}

fn record_value(type_name: &str, key: &str, site: RefusalSite, kind: RefusalKind) {
    LEDGER.with(|ledger| {
        ledger.borrow_mut().push(FixtureKeyRefusal {
            type_name: type_name.to_owned(),
            key: key.to_owned(),
            site,
            attribution: None,
            kind,
        });
    });
}

/// Attach call context to every refusal recorded since the last attribution.
///
/// Called by the per-fixture render frame once its builders have run. Only un-attributed
/// entries are touched, so a refusal from an earlier fixture in the same file keeps the context
/// it was already given. ~keep
pub(crate) fn attribute(
    language: &str,
    fixture_id: &str,
    call_key: Option<&str>,
    options_type_source: OptionsTypeSource,
) {
    LEDGER.with(|ledger| {
        for refusal in ledger.borrow_mut().iter_mut() {
            if refusal.attribution.is_none() {
                refusal.attribution = Some(Attribution {
                    language: language.to_owned(),
                    fixture_id: fixture_id.to_owned(),
                    call_key: call_key.map(str::to_owned),
                    options_type_source,
                });
            }
        }
    });
}

/// Drain every refusal recorded on this thread since the last drain.
pub(crate) fn take() -> Vec<FixtureKeyRefusal> {
    LEDGER.with(|ledger| std::mem::take(&mut *ledger.borrow_mut()))
}

/// Drain the ledger and turn whatever it held into one error naming every refusal.
///
/// `language` names the draining boundary and is used only for refusals no frame attributed --
/// an attributed refusal reports the language its own render frame was serving.
pub(crate) fn take_error(language: &str) -> Option<anyhow::Error> {
    let refusals = take();
    if refusals.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(
        "{language} e2e generator refused {} fixture value(s).{}",
        refusals.len(),
        refusals
            .iter()
            .map(|refusal| format!("\n  - {}", refusal.message(language)))
            .collect::<String>()
    ))
}

impl FixtureKeyRefusal {
    /// The full diagnostic for one refusal.
    ///
    /// It must name the call and the language, because the resolved type depends on both, and it
    /// must name the `options_type` level the type came from, because that is the lever -- and
    /// the level a caller has to change is not the level the wrong value was read from when a
    /// file-level default is being inherited. ~keep
    fn message(&self, fallback_language: &str) -> String {
        let language = self
            .attribution
            .as_ref()
            .map(|a| a.language.as_str())
            .unwrap_or(fallback_language);
        let type_name = &self.type_name;
        let key = &self.key;
        let problem = match self.kind {
            RefusalKind::Field => format!("key `{key}` is not declared as a field by `{type_name}`"),
            RefusalKind::EnumValue => format!("value `{key}` is not a declared wire variant of enum `{type_name}`"),
        };
        let mut message = match self.attribution.as_ref() {
            Some(attribution) => format!(
                "fixture `{}` (call {}, language `{language}`): {problem}",
                attribution.fixture_id,
                call_reference(attribution.call_key.as_deref()),
            ),
            None => format!("language `{language}`: {problem}"),
        };
        if let RefusalSite::Nested { via } = &self.site {
            message.push_str(&format!(" (reached through {via})"));
        }
        message.push('.');
        if let Some(attribution) = self.attribution.as_ref()
            && matches!(self.kind, RefusalKind::Field)
        {
            message.push(' ');
            message.push_str(&self.lever(attribution, language));
        }
        message
    }

    fn lever(&self, attribution: &Attribution, language: &str) -> String {
        let type_name = &self.type_name;
        let per_call = override_table(attribution.call_key.as_deref(), language);
        match attribution.options_type_source {
            // The misconfiguration shape worth spelling out: the type was inherited, not chosen
            // for this call, so the fix belongs at the per-call level and NOT at the level the
            // value was read from -- editing the file-level default re-types every other call.
            OptionsTypeSource::LanguageDefault if attribution.call_key.is_some() => format!(
                "`{type_name}` is the options type this call resolves to for `{language}`, inherited from the \
                 file-level `[e2e.call.overrides.{language}].options_type` default because {per_call} declares \
                 no `options_type` of its own -- a file-level default applies to every call that does not \
                 override it. If this call takes a different type, declare `options_type` under {per_call} \
                 rather than changing the file-level default. If `{type_name}` is correct, remove or rename \
                 the fixture key, or add the field to `{type_name}`."
            ),
            OptionsTypeSource::LanguageDefault | OptionsTypeSource::PerCall => format!(
                "`{type_name}` is the options type this call resolves to for `{language}`, from \
                 {per_call}.options_type. Either that names the wrong type for this call, or the fixture \
                 key is wrong, or `{type_name}` is missing the field."
            ),
            OptionsTypeSource::CallLevel => format!(
                "`{type_name}` is the options type this call resolves to for `{language}`, from the \
                 call-level `options_type` on {call}. Either that names the wrong type, or the fixture key \
                 is wrong, or `{type_name}` is missing the field. A per-language `options_type` under \
                 {per_call} overrides it for `{language}` alone.",
                call = call_reference(attribution.call_key.as_deref()),
            ),
            OptionsTypeSource::Unset if attribution.call_key.is_some() => format!(
                "No `options_type` is configured for `{language}` -- neither at {per_call} nor at the \
                 file-level `[e2e.call.overrides.{language}]` -- so `{type_name}` came from the argument's \
                 `element_type` or from the IR. Declare `options_type` under {per_call} to pin the type this \
                 call takes, or fix the fixture key or `{type_name}`."
            ),
            OptionsTypeSource::Unset => format!(
                "No `options_type` is configured for `{language}` at {per_call}, so `{type_name}` came from \
                 the argument's `element_type` or from the IR. Declare `options_type` under {per_call} to \
                 pin the type this call takes, or fix the fixture key or `{type_name}`."
            ),
        }
    }
}

/// How the operator refers to the call in `alef.toml`.
fn call_reference(call_key: Option<&str>) -> String {
    match call_key {
        Some(key) => format!("`[e2e.calls.{key}]`"),
        None => "`[e2e.call]` (the default call)".to_string(),
    }
}

/// The per-language override table for one call.
fn override_table(call_key: Option<&str>, language: &str) -> String {
    match call_key {
        Some(key) => format!("`[e2e.calls.{key}.overrides.{language}]`"),
        None => format!("`[e2e.call.overrides.{language}]`"),
    }
}

/// The `[e2e.calls.<key>]` name `call_config` was resolved from, or `None` when it is the
/// default `[e2e.call]`.
///
/// Resolved by identity, not by comparing field values: `E2eConfig::resolve_call_for_fixture`
/// returns a borrow of one of the configs the `E2eConfig` owns, and two named calls may be
/// field-for-field identical while only one of them is the one this fixture routed to. ~keep
pub(crate) fn resolved_call_key<'a>(
    e2e_config: &'a crate::e2e::config::E2eConfig,
    call_config: &crate::core::config::e2e::CallConfig,
) -> Option<&'a str> {
    e2e_config
        .calls
        .iter()
        .find(|(_, candidate)| std::ptr::eq(*candidate, call_config))
        .map(|(name, _)| name.as_str())
}

#[cfg(test)]
mod tests;
