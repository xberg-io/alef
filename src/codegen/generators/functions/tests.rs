use super::collect_trait_imports;
use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn method_with_trait_source(name: &str, trait_source: Option<&str>) -> MethodDef {
    MethodDef {
        name: name.into(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: trait_source.map(String::from),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn type_with_methods(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.into(),
        rust_path: format!("my_crate::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// Two distinct trait paths sharing a final segment (`Codec`) and of equal length must resolve
/// to the same path on every run, not to whichever the internal `AHashSet` happened to iterate
/// first. Runs the collection many times over freshly-constructed, differently-ordered input to
/// make an accidental pass (the two paths landing in the same bucket order by luck) unlikely --
/// mirrors the repetition-count precedent used by the Swift and PHP determinism tests. ~keep
#[test]
fn collect_trait_imports_breaks_equal_length_ties_deterministically() {
    // "acme_core::a::Codec" and "acme_core::b::Codec" are both 20 characters, so the old
    // `path.len() < entry.len()` comparison never fires for this pair and the winner depended
    // entirely on `AHashSet` iteration order.
    let path_a = "acme_core::a::Codec";
    let path_b = "acme_core::b::Codec";
    assert_eq!(
        path_a.len(),
        path_b.len(),
        "fixture paths must be equal length to exercise the tie"
    );

    let mut results = std::collections::BTreeSet::new();
    for _ in 0..32 {
        let typ = type_with_methods(
            "Widget",
            vec![
                method_with_trait_source("encode", Some(path_a)),
                method_with_trait_source("decode", Some(path_b)),
            ],
        );
        let api = ApiSurface {
            types: vec![typ],
            ..ApiSurface::default()
        };
        results.insert(collect_trait_imports(&api));
    }

    assert_eq!(
        results.len(),
        1,
        "collect_trait_imports must pick the same trait path on every run; observed {} distinct \
         results across 32 runs: {results:?}",
        results.len()
    );
    let winner = results.into_iter().next().expect("one result recorded");
    assert_eq!(
        winner,
        vec![path_a.to_string()],
        "equal-length ties must resolve to the lexicographically smaller path"
    );
}

/// The existing shortest-path-wins behavior for an *unequal*-length collision must survive the
/// tie-break addition unchanged.
#[test]
fn collect_trait_imports_still_prefers_the_shorter_path_when_lengths_differ() {
    let typ = type_with_methods(
        "Widget",
        vec![
            method_with_trait_source("encode", Some("acme_core::deeply::nested::module::Codec")),
            method_with_trait_source("decode", Some("acme_core::Codec")),
        ],
    );
    let api = ApiSurface {
        types: vec![typ],
        ..ApiSurface::default()
    };

    assert_eq!(collect_trait_imports(&api), vec!["acme_core::Codec".to_string()]);
}
