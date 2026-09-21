//! `--strict` must mean the same thing on `alef generate` as it already does on `alef all`'s
//! e2e stage: a formatter whose executable is not installed fails the run.
//!
//! Before this, `alef all --strict`'s own help text promised exactly that, and only the e2e
//! formatter honoured it. `cli::pipeline::format::format_generated` -- the pass that formats
//! `packages/<lang>`, i.e. the SHIPPED bindings -- never received `strict` at all, and every
//! missing-tool skip inside it was a fire-and-forget `warn!`: invisible to the caller and
//! never fatal. The more important surface was the unguarded one. ~keep

use crate::bin_cli::args::{Cli, Commands};
use clap::Parser as _;

/// The `--strict` flag has to exist on `alef generate` before anything downstream can honour
/// it. This is the CLI half of the contract; the behavioural half (a missing formatter
/// actually failing the run) lives in `cli::pipeline::format`'s strict tests.
#[test]
fn generate_accepts_strict_and_defaults_to_lenient() {
    let lenient = Cli::try_parse_from(["alef", "generate"]).expect("plain generate command");
    let Commands::Generate { strict, .. } = lenient.command else {
        panic!("expected generate command");
    };
    assert!(
        !strict,
        "a missing formatter must NOT be fatal by default -- poly, rustfmt, cargo-sort and mix \
         are host toolchains a fresh clone may legitimately lack"
    );

    let strict_run =
        Cli::try_parse_from(["alef", "generate", "--strict"]).expect("`alef generate --strict` must parse");
    let Commands::Generate { strict, .. } = strict_run.command else {
        panic!("expected generate command");
    };
    assert!(strict, "`alef generate --strict` must set the strict flag");
}

/// The flag existing is worthless if the command arm drops it on the floor, which is the
/// precise shape of the original defect: the policy existed, one caller never passed it in.
/// Pinning the call keeps `alef generate` and `alef all` from drifting into two different
/// answers to the same question again. ~keep
#[test]
fn generate_threads_strict_into_the_package_formatting_pass() {
    let dispatch_source = include_str!("../core_commands.rs");
    // ~keep Locate the arm and assert `strict` is among the bindings, rather than pinning the
    // whole destructuring literal. The literal spelling names every OTHER field too, so adding
    // an unrelated flag to the arm -- `skip_compile` did exactly this -- broke a test that has
    // no opinion about that flag, and the repair is always "paste the new literal", which
    // re-arms the same trip wire without ever having caught the thing this test exists to catch.
    let arm = dispatch_source
        .find("Commands::Generate {")
        .expect("dispatch must have a Generate arm");
    let bindings_end = dispatch_source[arm..]
        .find('}')
        .expect("the Generate arm's destructuring must be terminated");
    let bindings = &dispatch_source[arm..arm + bindings_end];
    assert!(
        bindings.contains("strict,"),
        "the Generate arm must destructure `strict` from the parsed command. Bindings were: \
         {bindings}"
    );
    // The arm's body -- including the formatting-pass call this test pins -- lives in
    // `core_commands/generate.rs`, split out of `core_commands.rs` for the file-modularization
    // cap. ~keep
    let source = include_str!("generate.rs");
    // Matches both `format_generated_reporting(` and its `_with_extra_paths` sibling (the one
    // `alef generate` actually calls, so a workspace-root scaffold file no language owns still
    // reaches the formatter -- see `unowned_changed_paths`) -- either is "the reporting entry
    // point", as opposed to the discarding `format_generated` the assertion below rules out.
    let call = source
        .find("pipeline::format_generated_reporting")
        .expect("`alef generate` must format through the reporting entry point, not the discarding one");
    let call_site = &source[call..];
    let end = call_site.find(");").expect("terminated call");
    let call_site = &call_site[..end];
    assert!(
        call_site.contains("strict"),
        "`alef generate` must pass its own `strict` flag into the package formatting pass, not a \
         hard-coded `false` -- a formatter skip that reaches no caller is the whole bug. Call was: \
         {call_site:?}"
    );
    assert!(
        !source.contains("pipeline::format_generated(&files_to_format"),
        "the discarding `format_generated` entry point must not survive on the generate path"
    );
}
