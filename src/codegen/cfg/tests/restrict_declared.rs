//! Coverage for [`restrict_cfg_gate_to_declared`].
//!
//! Split out of the parent test module for the same reason `feature_forwarding.rs` is: this is
//! a self-contained concern (narrowing a copied `#[cfg(...)]` gate to a binding crate's own
//! declared Cargo features) with its own full matrix of shapes worth naming individually.

use crate::codegen::cfg::{DeclaredCfgGate, restrict_cfg_gate_to_declared};
use std::collections::HashSet;

fn declared<'a>(names: &'a [&'a str]) -> HashSet<&'a str> {
    names.iter().copied().collect()
}

/// CONTROL: a gate naming only declared features must pass through unchanged, byte for byte.
/// Without this, a fix that narrows or drops indiscriminately would still pass every other test
/// in this file.
#[test]
fn gate_with_every_feature_declared_passes_through_unchanged() {
    let gate = r#"any(feature = "url-ingestion", feature = "url-config-types")"#;
    let declared = declared(&["url-ingestion", "url-config-types"]);
    assert_eq!(
        restrict_cfg_gate_to_declared(gate, &declared),
        DeclaredCfgGate::Gate(gate.to_string())
    );
}

/// A single declared feature name, the common case, is also returned unchanged.
#[test]
fn single_declared_feature_passes_through_unchanged() {
    let gate = r#"feature = "url-config-types""#;
    let declared = declared(&["url-config-types"]);
    assert_eq!(
        restrict_cfg_gate_to_declared(gate, &declared),
        DeclaredCfgGate::Gate(gate.to_string())
    );
}

/// The reported defect: `any(a, b)` with `b` undeclared narrows to `a` alone, in single-term
/// form -- not `any(a)` -- since `b` can never contribute a `true` in a crate that never
/// declares it.
#[test]
fn any_with_one_undeclared_feature_narrows_to_the_declared_term_alone() {
    let gate = r#"any(feature = "url-ingestion", feature = "url-config-types")"#;
    let declared = declared(&["url-config-types"]);
    assert_eq!(
        restrict_cfg_gate_to_declared(gate, &declared),
        DeclaredCfgGate::Gate(r#"feature = "url-config-types""#.to_string())
    );
}

/// `any(a, b)` with BOTH undeclared: neither disjunct can ever hold in this crate, so the item
/// this gate would guard is unreachable.
#[test]
fn any_with_every_feature_undeclared_is_unreachable() {
    let gate = r#"any(feature = "url-ingestion", feature = "crawl-enterprise")"#;
    let declared = declared(&["url-config-types"]);
    assert_eq!(restrict_cfg_gate_to_declared(gate, &declared), DeclaredCfgGate::Unreachable);
}

/// A bare single-term gate naming an undeclared feature is unreachable the same way a
/// fully-undeclared `any(...)` is -- there is no declared alternative to fall back to.
#[test]
fn single_undeclared_feature_is_unreachable() {
    let gate = r#"feature = "url-ingestion""#;
    let declared = declared(&["url-config-types"]);
    assert_eq!(restrict_cfg_gate_to_declared(gate, &declared), DeclaredCfgGate::Unreachable);
}

/// `all(a, b)` with `b` undeclared: `b` is never on in this crate, so the conjunction can never
/// hold regardless of `a` -- fail-closed, not narrowed to `all(a)`.
#[test]
fn all_with_one_undeclared_feature_is_unreachable_even_though_the_other_is_declared() {
    let gate = r#"all(feature = "url-config-types", feature = "url-ingestion")"#;
    let declared = declared(&["url-config-types"]);
    assert_eq!(restrict_cfg_gate_to_declared(gate, &declared), DeclaredCfgGate::Unreachable);
}

/// `all(a, b)` with every term declared passes through unchanged, same as any other
/// all-declared gate.
#[test]
fn all_with_every_feature_declared_passes_through_unchanged() {
    let gate = r#"all(feature = "a", feature = "b")"#;
    let declared = declared(&["a", "b"]);
    assert_eq!(
        restrict_cfg_gate_to_declared(gate, &declared),
        DeclaredCfgGate::Gate(gate.to_string())
    );
}

/// A three-way `any(...)` with exactly one undeclared term keeps the remaining two as a
/// narrower `any(...)`, not a single term -- the single-term collapse only applies once exactly
/// one alternative survives.
#[test]
fn any_with_two_of_three_features_declared_narrows_to_a_two_term_any() {
    let gate = r#"any(feature = "a", feature = "b", feature = "c")"#;
    let declared = declared(&["a", "c"]);
    assert_eq!(
        restrict_cfg_gate_to_declared(gate, &declared),
        DeclaredCfgGate::Gate(r#"any(feature = "a", feature = "c")"#.to_string())
    );
}

/// A predicate naming no Cargo feature at all (`target_os`, ...) can never be missing a
/// declaration, so it passes through unchanged regardless of what is declared.
#[test]
fn non_feature_predicate_passes_through_unchanged() {
    let gate = r#"target_os = "macos""#;
    let declared = declared(&[]);
    assert_eq!(
        restrict_cfg_gate_to_declared(gate, &declared),
        DeclaredCfgGate::Gate(gate.to_string())
    );
}
