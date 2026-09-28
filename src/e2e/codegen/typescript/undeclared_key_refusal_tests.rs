//! End-to-end coverage for the failure MODE of an undeclared fixture key, at the boundary that
//! decides whether one backend's refusal takes the whole run down.
//!
//! The refusal itself is not new. What was new is that it arrived as a `panic!` five frames below
//! `E2eCodegen::generate`, so a single consumer `options_type` misconfiguration aborted the whole
//! `alef all` process at exit 101: every other backend's e2e codegen, every later crate and every
//! later stage (README, docs, snippet validation) silently never ran. A sibling post-build failure
//! in the same run degraded gracefully -- "continuing with the remaining stages" --
//! because it travelled as an `anyhow::Error`. This suite pins the refusal onto that same path.
//!
//! Isolation from sibling backends is already pinned by
//! `e2e::tests::run_generators_isolates_one_backend_failure_from_the_rest`, which proves an `Err`
//! from one generator leaves the others running. All this suite has to prove is that this
//! generator now produces that `Err` instead of unwinding -- the two together are the guarantee.
//!
//! Split into its own file rather than grown inline in `typescript/mod.rs`, which is already at
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md). ~keep

use super::*;
use crate::core::config::NewAlefConfig;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Assertion;

/// Neutral fixture types. `ParseOptions` is the per-call options struct a file-level default
/// names; `PackConfig` is the outer config the `pack` call actually takes. Only `PackConfig`
/// declares `cache_dir`.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ParseOptions".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "PackConfig".into(),
            fields: vec![FieldDef {
                name: "cache_dir".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

/// The exact misconfiguration shape from the incident: a FILE-LEVEL `options_type` for node
/// (`[e2e.call.overrides.node]`) and a named call that declares none of its own, so the named
/// call silently inherits the wrong type. `per_call_options_type` fills in the fix.
fn config_toml(per_call_options_type: Option<&str>) -> String {
    let per_call_override = match per_call_options_type {
        Some(type_name) => format!("[crates.e2e.calls.pack.overrides.node]\noptions_type = \"{type_name}\"\n"),
        None => String::new(),
    };
    format!(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "parse"
module = "sample-lib"
result_var = "result"

[[crates.e2e.call.args]]
name = "options"
field = "input.options"
type = "json_object"

[crates.e2e.call.overrides.node]
options_type = "ParseOptions"

[crates.e2e.calls.pack]
function = "pack"
module = "sample-lib"
result_var = "result"

[[crates.e2e.calls.pack.args]]
name = "options"
field = "input.options"
type = "json_object"

{per_call_override}"#
    )
}

/// A fixture routed to the `pack` call whose options object carries `cache_dir` -- declared by
/// `PackConfig`, not by the inherited `ParseOptions`.
fn pack_group() -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "pack".into(),
        fixtures: vec![Fixture {
            id: "pack-basic".into(),
            description: "Pack a directory".into(),
            input: serde_json::json!({"options": {"cache_dir": "/tmp/cache"}}),
            call: Some("pack".into()),
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            }],
            ..Fixture::default()
        }],
    }]
}

fn generate(per_call_options_type: Option<&str>) -> anyhow::Result<Vec<crate::core::backend::GeneratedFile>> {
    let cfg: NewAlefConfig = toml::from_str(&config_toml(per_call_options_type)).expect("valid toml");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let resolved = cfg.resolve().expect("resolve ok").remove(0);
    TypeScriptCodegen.generate_gated(&pack_group(), &e2e, &resolved, &type_defs(), &[], &[], &[])
}

/// The decisive assertion: the misconfiguration must come back as this backend's own `Err`, and
/// the call must not unwind. A `panic!` here is what took the whole run down at exit 101. ~keep
#[test]
fn an_undeclared_fixture_key_fails_this_backend_instead_of_aborting_the_process() {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| generate(None)))
        .expect("a consumer misconfiguration must never unwind the generator");

    let error = outcome.expect_err("an undeclared fixture key must fail this backend's codegen");
    let message = format!("{error:#}");

    assert!(
        message.contains("`cache_dir`"),
        "the error must name the refused key: {message}"
    );
    assert!(
        message.contains("`ParseOptions`"),
        "the error must name the type the key was checked against: {message}"
    );
}

/// The message must carry the three things the old one lacked: the call, the language, and the
/// `options_type` lever -- and, decisively, that the type was INHERITED from the file-level
/// default rather than chosen for this call. The old text ("fix the fixture ... or the Rust
/// struct") named neither the call nor the lever, and in the real incident both remedies it did
/// name were already correct. ~keep
#[test]
fn the_error_names_the_call_the_language_and_the_inherited_options_type_level() {
    let error = generate(None).expect_err("an undeclared fixture key must fail this backend's codegen");
    let message = format!("{error:#}");

    assert!(message.contains("fixture `pack-basic`"), "got: {message}");
    assert!(message.contains("`[e2e.calls.pack]`"), "must name the call: {message}");
    assert!(message.contains("language `node`"), "must name the language: {message}");
    assert!(
        message.contains("`[e2e.call.overrides.node].options_type` default"),
        "must name the level the wrong type was inherited from: {message}"
    );
    assert!(
        message.contains("declare `options_type` under `[e2e.calls.pack.overrides.node]`"),
        "must name the level the fix belongs at: {message}"
    );
}

/// The fix the diagnostic recommends must actually work. Without this, the message could name a
/// remedy that changes nothing -- which is the failure the old message shipped with. ~keep
#[test]
fn declaring_the_per_call_options_type_the_message_recommends_resolves_the_refusal() {
    let files = generate(Some("PackConfig")).expect("the per-call override the diagnostic recommends must fix it");

    assert!(
        files.iter().any(|file| file.path.ends_with("pack.test.ts")),
        "the node suite must render once the call names the type it actually takes"
    );
}

/// A drained ledger is what keeps one backend's refusal from being reported against the next
/// backend to run. Draining is done by `generate_gated` on both the `Ok` and the `Err` path. ~keep
#[test]
fn a_refusal_does_not_leak_into_the_next_backends_generation() {
    generate(None).expect_err("precondition: this configuration must refuse");

    assert_eq!(
        crate::e2e::codegen::fixture_refusal::take().len(),
        0,
        "the refusal must have been drained by `generate_gated`, not left for the next backend"
    );
    generate(Some("PackConfig")).expect("a correctly configured backend must not inherit the earlier refusal");
}
