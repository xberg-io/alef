//! `FieldResolver::with_result_is_byte_payload` is the ONE place every backend's
//! `is_valid_for_result` / `result_field_oracle_knows` call now consults for "does a field path
//! even make sense against this call's result" when the call's own declared Rust return type is
//! a raw byte payload (`bytes::Bytes`, `Vec<u8>`, `[u8]`, `[u8; N]` — all collapsed to
//! `TypeRef::Bytes` by `extract::type_resolver`).
//!
//! ~keep Before this flag existed, a byte-returning call's anchored root type
//! (`ir_result_field_map.root_type` / `ir_collection_map.root_type`) was `None` for exactly the
//! same reason it is `None` for a call with no IR wired in at all: `call_ir::named_type` has no
//! `Named` leaf to report for `TypeRef::Bytes`. Both oracles' permissive "the IR has never heard
//! of this name" default then silently accepted a fixture's declared response-struct field path
//! (e.g. `audio`, `content`) against a value that is not a struct at all — `result.audio` on a
//! `bytes::Bytes` result is `E0609: no field 'audio' on type 'Bytes'` in Rust, and the Go
//! equivalent (`result.Content` on a `[]byte`) is a `go vet` failure. Some backends had
//! independently learned to guard this via the config-level `result_is_bytes` flag
//! (java/csharp/c/zig/swift/r); rust and go's assertion gating checked only `result_is_simple`
//! and missed the byte-payload case — two components reading the same fact and disagreeing.

use super::FieldResolver;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn resolver_with_result_fields(result_fields: &[&str]) -> FieldResolver {
    let result_fields: HashSet<String> = result_fields.iter().map(|s| s.to_string()).collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
}

/// The bare-minimum reproduction of both reported victims: no IR wired in at all (the state
/// every config-only fixture resolver is built in), just the byte-payload flag. Both the
/// `audio` name from the Rust leg and the `Content` name from the Go leg must be rejected.
#[test]
fn byte_payload_flag_rejects_any_field_path_with_no_ir_wired_in() {
    let resolver = resolver_with_result_fields(&[]).with_result_is_byte_payload(true);
    assert!(!resolver.is_valid_for_result("audio"));
    assert!(!resolver.is_valid_for_result("Content"));
    assert_eq!(resolver.result_field_oracle_knows("audio"), Some(false));
    assert_eq!(resolver.result_field_oracle_knows("Content"), Some(false));
}

/// The exact shape of the reported defect: the consumer's fixture config lists the field under
/// `result_fields` (e.g. `result_fields = ["audio"]`), which is precisely what made
/// `is_valid_for_result` permissive before this fix — `result_fields` is a hand-maintained
/// allowlist consulted specifically for names the IR has not anchored, and a byte-returning call
/// anchors nothing. The byte-payload flag must override that allowlist, not lose to it.
#[test]
fn byte_payload_flag_rejects_a_field_even_when_result_fields_declares_it() {
    let resolver = resolver_with_result_fields(&["audio"]).with_result_is_byte_payload(true);
    assert!(
        !resolver.is_valid_for_result("audio"),
        "result_fields declaring 'audio' must not rescue a byte-payload result"
    );
    assert_eq!(resolver.result_field_oracle_knows("audio"), Some(false));
}

/// `Envelope { audio: Bytes }` — a genuine struct whose field happens to be byte-typed. This is
/// NOT the top-level byte-payload case (the call still returns a struct), so the byte-payload
/// flag must stay unset here and every field on `Envelope` must resolve normally.
fn struct_type_defs_with_audio_field() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Envelope".to_string(),
        fields: vec![
            FieldDef {
                name: "audio".to_string(),
                ty: TypeRef::Bytes,
                ..FieldDef::default()
            },
            FieldDef {
                name: "title".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    }]
}

fn resolver_anchored_at_envelope() -> FieldResolver {
    let type_defs = struct_type_defs_with_audio_field();
    let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(map, Some("Envelope".to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

/// The critical negative control: a call whose result genuinely IS a struct (even one with a
/// byte-typed field on it) must keep getting real field access when the byte-payload flag is not
/// set. A fix that suppresses field access too broadly — e.g. keying off `TypeRef::Bytes`
/// appearing ANYWHERE in the type graph instead of the call's own top-level return type — would
/// silently gut every assertion on a struct that merely contains a byte field.
#[test]
fn negative_control_a_genuine_struct_result_still_gets_field_access() {
    let resolver = resolver_anchored_at_envelope();
    assert!(resolver.is_valid_for_result("audio"));
    assert!(resolver.is_valid_for_result("title"));
    assert_eq!(resolver.result_field_oracle_knows("audio"), Some(true));
    assert_eq!(resolver.result_field_oracle_knows("title"), Some(true));
}

/// The flag must win even against a fully anchored, genuinely-declaring struct result — proving
/// this is a positive, call-specific override rather than something that only happens to work
/// because an unresolved root type already defaulted the same way. A caller wires this flag in
/// exactly when the call's OWN declared return type (not a field somewhere in its graph) is
/// `TypeRef::Bytes`, so this scenario should never occur together with a real anchored struct in
/// practice — but the oracle's precedence must still be unambiguous.
#[test]
fn byte_payload_flag_overrides_even_a_fully_anchored_struct_result() {
    let resolver = resolver_anchored_at_envelope().with_result_is_byte_payload(true);
    assert!(!resolver.is_valid_for_result("audio"));
    assert!(!resolver.is_valid_for_result("title"));
    assert_eq!(resolver.result_field_oracle_knows("audio"), Some(false));
    assert_eq!(resolver.result_field_oracle_knows("title"), Some(false));
}

/// Every existing constructor (`new`, `new_with_error_aliases`) and builder
/// (`with_php_getter_map`, `with_swift_first_class_map`, `with_dart_first_class_map`) must
/// default the flag to `false`, so every resolver built before this flag existed keeps its
/// exact prior behaviour unless a call site opts in explicitly.
#[test]
fn the_flag_defaults_to_false_and_is_purely_additive() {
    let resolver = resolver_anchored_at_envelope();
    assert!(
        resolver.is_valid_for_result("audio"),
        "constructing a resolver must never implicitly set the byte-payload flag"
    );
}
