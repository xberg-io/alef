//! Resolves where a hardcoded synthetic-handler leaf field (e.g. `chunks`) actually lives when
//! the call's own result type is an envelope wrapping the type that declares it.
//!
//! [`FieldResolver::result_field_oracle_knows`] already answers "does the call's own root type
//! declare this path" by walking the IR struct graph from the anchored root. That is exactly
//! right for a synthetic handler whose root genuinely lacks the field — refusing is correct,
//! since `{result_var}.chunks` would not compile. It is too narrow for a crate whose result type
//! is an envelope: `result_fields` already names the envelope projection (e.g. `results`), and
//! the IR can confirm whether THAT path, not the bare leaf, reaches a declaring type. Neither
//! `result_field_oracle_knows` nor `root_declares_path` tries any prefix but the bare name, so an
//! envelope shape reads identically to a genuinely unreachable field: both answer `Some(false)`.
//! [`FieldResolver::anchor_leaf`] tries every `result_fields` entry as a candidate prefix before
//! agreeing with that refusal.
//!
//! The candidate search must be exhaustive and order-independent: `result_fields` is a
//! [`HashSet`], whose iteration order is reseeded per process, so a search that stops at the
//! first accepting prefix (`Iterator::find_map`) returns whichever prefix the hash happens to
//! yield first. When more than one prefix reaches a declaring type — a genuine prefix and a
//! decoy alike — that made the rescue's answer a coin flip run to run. The search below visits
//! every `result_fields` entry, confirms each with the STRICT [`super::ir_result_fields::root_declares_path`]
//! (the same check [`FieldResolver::result_relative_path`]'s `envelope_projected_path` applies
//! when accepting the result), and requires EXACTLY ONE confirming prefix. Zero confirms is the
//! pre-existing honest refusal; more than one is a newly-recognized ambiguity that must also
//! refuse rather than silently pick one — a scalar `result_fields` entry that only looks
//! plausible under the permissive `result_field_oracle_knows` (see that oracle's flat,
//! name-keyed fallback) must never be able to shadow the one real answer.

use std::collections::HashSet;

use super::types::FieldResolver;

/// Where a synthetic handler's hardcoded leaf field name should be accessed from, once the
/// call's own root type has had a chance to answer directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafAnchor {
    /// The root declares the leaf itself, or no IR root was ever anchored (the permissive
    /// default every oracle in this module already applies to an unresolved root) — access it
    /// straight off the result variable.
    Direct,
    /// The root does not declare the leaf, but this `result_fields`-declared path — confirmed by
    /// the IR to reach a type that does — should be accessed first. Already carries an index hop
    /// (`[0]`) when the IR shows the prefix field is a `Vec<T>`.
    Prefixed(String),
}

impl FieldResolver {
    /// Resolve where `leaf` lives relative to the call's own result type.
    ///
    /// `None` means every candidate was positively refused: `leaf` is unreachable both directly
    /// on the root and through every `result_fields` prefix, so the caller must refuse to render
    /// an accessor for it — there is no compiling path. This is purely additive over
    /// [`FieldResolver::result_field_oracle_knows`]: it can only turn that oracle's `Some(false)`
    /// into a rescue by finding a `result_fields` prefix that reaches `leaf`, never turn an
    /// existing `Some(true)` or `None` answer into a refusal. ~keep
    pub fn anchor_leaf(&self, leaf: &str) -> Option<LeafAnchor> {
        match self.result_field_oracle_knows(leaf) {
            None | Some(true) => Some(LeafAnchor::Direct),
            Some(false) => self.anchor_leaf_via_result_fields(leaf, &self.result_fields),
        }
    }

    /// Scans every `result_fields` entry — never short-circuits on the first hit — and accepts
    /// only when EXACTLY ONE is confirmed by the strict [`super::ir_result_fields::root_declares_path`].
    /// Zero confirmations is the honest refusal `anchor_leaf`'s caller already expects; more than
    /// one is an ambiguity this must also refuse, since picking either would be an unproven guess.
    /// Visiting the whole set rather than stopping early is what makes the answer independent of
    /// `HashSet`'s per-process iteration order. ~keep
    fn anchor_leaf_via_result_fields(&self, leaf: &str, result_fields: &HashSet<String>) -> Option<LeafAnchor> {
        let mut sorted_prefixes: Vec<&String> = result_fields.iter().collect();
        sorted_prefixes.sort();

        let mut confirmed: Option<String> = None;
        for prefix in sorted_prefixes {
            let prefix_path = if self.is_collection_root(prefix) {
                format!("{prefix}[0]")
            } else {
                prefix.clone()
            };
            let candidate = format!("{prefix_path}.{leaf}");
            let declares =
                super::ir_result_fields::root_declares_path(&self.ir_result_field_map, &candidate) == Some(true);
            if declares {
                if confirmed.is_some() {
                    // A second confirming prefix -- ambiguous, decline rather than guess.
                    return None;
                }
                confirmed = Some(prefix_path);
            }
        }
        confirmed.map(LeafAnchor::Prefixed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
    use std::collections::HashMap;

    /// `Envelope { results: Vec<Document> }`, `Document { chunks: Vec<Chunk>, metadata: Metadata }`,
    /// `Metadata { output_format: String }` — the shape a consumer's envelope-wrapped result takes.
    fn envelope_document_metadata_type_defs() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "Envelope".to_string(),
                fields: vec![FieldDef {
                    name: "results".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Document".to_string(),
                fields: vec![
                    FieldDef {
                        name: "chunks".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "metadata".to_string(),
                        ty: TypeRef::Named("Metadata".to_string()),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Metadata".to_string(),
                fields: vec![FieldDef {
                    name: "output_format".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ]
    }

    fn envelope_resolver(result_fields: &[&str]) -> FieldResolver {
        let type_defs = envelope_document_metadata_type_defs();
        let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
        let collection_map = FieldResolver::ir_collection_fields(&type_defs);
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        let result_fields: HashSet<String> = result_fields.iter().map(|s| s.to_string()).collect();
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(result_field_map, Some("Envelope".to_string()))
        .with_ir_collection_map(collection_map, Some("Envelope".to_string()))
        .with_ir_fields(reachable, excluded, optional)
    }

    /// The confirmed defect: `chunks` is unreachable on `Envelope` directly, but IS reachable
    /// through the `results` prefix `result_fields` names — the anchor must find it, with the
    /// index hop `results` genuinely needs as a `Vec<Document>`.
    #[test]
    fn envelope_prefix_reaches_a_leaf_declared_on_the_nested_type() {
        let resolver = envelope_resolver(&["results"]);
        assert_eq!(
            resolver.anchor_leaf("chunks"),
            Some(LeafAnchor::Prefixed("results[0]".to_string()))
        );
    }

    /// The deeper, nested-and-indexed shape: a two-hop path (`results[0].metadata`) must resolve
    /// exactly like the one-hop `chunks` case — an anchoring fix that only handles a bare
    /// projected root without an index hop, or only a single hop, is too narrow.
    #[test]
    fn envelope_prefix_reaches_a_leaf_nested_two_hops_deep() {
        let resolver = envelope_resolver(&["results"]);
        assert_eq!(
            resolver.anchor_leaf("metadata.output_format"),
            Some(LeafAnchor::Prefixed("results[0]".to_string()))
        );
    }

    /// The control: a call whose root genuinely declares the leaf directly must still resolve to
    /// `Direct` — the fix must not turn into "always prefix."
    #[test]
    fn root_declaring_the_leaf_directly_stays_direct() {
        let resolver = envelope_resolver(&["results"]);
        assert_eq!(resolver.anchor_leaf("results"), Some(LeafAnchor::Direct));
    }

    /// Sanctioned refusal: when `chunks` is genuinely unreachable — no `result_fields` prefix
    /// leads to it either — the anchor must still refuse. A permissive anchor that always finds
    /// SOME prefix would silently emit a non-compiling accessor instead of the honest skip.
    #[test]
    fn genuinely_unreachable_leaf_is_still_refused() {
        let resolver = envelope_resolver(&["results"]);
        assert_eq!(resolver.anchor_leaf("not_a_real_field"), None);
    }

    /// A `result_fields` entry that itself does not lead anywhere useful must not be mistaken
    /// for the right prefix — only a prefix whose nested type positively declares the leaf wins.
    #[test]
    fn a_result_fields_entry_that_does_not_reach_the_leaf_is_not_used() {
        let resolver = envelope_resolver(&["results", "unrelated"]);
        assert_eq!(
            resolver.anchor_leaf("chunks"),
            Some(LeafAnchor::Prefixed("results[0]".to_string()))
        );
    }

    /// `Envelope { results: Vec<Document>, archived: Vec<Document> }` — two DIFFERENT
    /// `result_fields` entries that both genuinely reach `Document`, so both confirm `chunks`.
    /// Unlike `unrelated` in [`a_result_fields_entry_that_does_not_reach_the_leaf_is_not_used`],
    /// which is a decoy that reaches nothing, `archived` is a decoy that reaches the SAME leaf as
    /// the real prefix — the shape a hash-order-dependent `find_map` could not tell apart from an
    /// unambiguous single-prefix case, because it never looked past its first hit.
    fn envelope_two_reaching_prefixes_type_defs() -> Vec<TypeDef> {
        let mut type_defs = envelope_document_metadata_type_defs();
        let envelope = type_defs
            .iter_mut()
            .find(|t| t.name == "Envelope")
            .expect("Envelope type present");
        envelope.fields.push(FieldDef {
            name: "archived".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
            ..FieldDef::default()
        });
        type_defs
    }

    /// The new rule this fix adds: when MORE THAN ONE `result_fields` prefix is confirmed by the
    /// strict IR walk to reach the same leaf, the anchor must decline rather than pick either one
    /// — picking would be exactly the unproven, hash-order-dependent guess this fix removes. Before
    /// this fix, `find_map` would have returned whichever of `results[0]`/`archived[0]` its
    /// `HashSet` iteration produced first, silently and differently across runs.
    #[test]
    fn two_result_fields_prefixes_both_reaching_the_leaf_is_declined_as_ambiguous() {
        let type_defs = envelope_two_reaching_prefixes_type_defs();
        let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
        let collection_map = FieldResolver::ir_collection_fields(&type_defs);
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        let result_fields: HashSet<String> = ["results", "archived"].iter().map(|s| s.to_string()).collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(result_field_map, Some("Envelope".to_string()))
        .with_ir_collection_map(collection_map, Some("Envelope".to_string()))
        .with_ir_fields(reachable, excluded, optional);
        assert_eq!(resolver.anchor_leaf("chunks"), None);
    }

    /// No anchored root type at all (the state of every call site before this fix, and every
    /// config-only fixture with no IR wired in) must keep the pre-existing permissive default:
    /// `Direct`, never a refusal.
    #[test]
    fn no_anchored_root_type_keeps_the_permissive_default() {
        let type_defs = envelope_document_metadata_type_defs();
        let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_result_fields(result_field_map, None)
        .with_ir_fields(reachable, excluded, optional);
        assert_eq!(resolver.anchor_leaf("chunks"), Some(LeafAnchor::Direct));
    }
}
