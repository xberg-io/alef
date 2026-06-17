//! Tests for the shared same-name function dedup pass used by single-surface backends.

use super::dedup_same_name_functions;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

fn make_param(name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..Default::default()
    }
}

fn make_fn(name: &str, rust_path: &str, cfg: Option<&str>, param_names: &[&str]) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        params: param_names.iter().map(|n| make_param(n)).collect(),
        return_type: TypeRef::String,
        cfg: cfg.map(|s| s.to_string()),
        ..Default::default()
    }
}

fn normalize_cfg(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Two entries sharing a name with disjoint cfg gates collapse to one canonical entry with
/// `cfg = any(<a>, <b>)`. This mirrors the pattern where a `pub use real::fn` re-export is
/// paired with an `Err(...)` stub fallback — the case that produced duplicate Java/C#/Go
/// methods after alef 0.25.34's pub-use-clears-skip + dedup-by-(name,cfg) extractor passes.
#[test]
fn merges_disjoint_cfgs_for_same_name_functions() {
    let cfg_real = r#"all(feature = "embeddings", feature = "tokio-runtime")"#;
    let cfg_stub = r#"all(feature = "embedding-presets", not(feature = "embeddings"), feature = "tokio-runtime")"#;

    let input = vec![
        make_fn(
            "embed_texts_async",
            "my_crate::embed_texts_async",
            Some(cfg_real),
            &["texts", "config"],
        ),
        make_fn(
            "embed_texts_async",
            "my_crate::embed_texts_async",
            Some(cfg_stub),
            &["_texts", "_config"],
        ),
    ];

    let out = dedup_same_name_functions(&input);

    assert_eq!(
        out.len(),
        1,
        "disjoint-cfg group must be collapsed into a single function entry"
    );
    let merged = &out[0];
    assert_eq!(merged.name, "embed_texts_async");

    let cfg = merged.cfg.as_deref().expect("merged entry must have a cfg");
    let cfg_norm = normalize_cfg(cfg);
    assert!(
        cfg_norm.starts_with("any("),
        "merged cfg must start with `any(`, got: {cfg}"
    );
    assert!(
        cfg_norm.contains(&normalize_cfg(cfg_real)),
        "merged cfg must contain the real implementation cfg; got: {cfg}"
    );
    assert!(
        cfg_norm.contains(&normalize_cfg(cfg_stub)),
        "merged cfg must contain the stub cfg; got: {cfg}"
    );

    let param_names: Vec<&str> = merged.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        param_names,
        vec!["texts", "config"],
        "canonical entry must use the non-underscore parameter names from the real implementation"
    );
}

#[test]
fn no_op_when_single_function_per_name() {
    let input = vec![make_fn(
        "embed_texts_async",
        "my_crate::embed_texts_async",
        Some(r#"feature = "embeddings""#),
        &["texts"],
    )];
    let out = dedup_same_name_functions(&input);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].cfg.as_deref(), Some(r#"feature = "embeddings""#));
}

/// Three-way groups (e.g. `download_model` / `known_models` / `default_model_name`, which the
/// kreuzberg surface emits under real + two stub cfgs) must collapse to a single entry.
#[test]
fn merges_three_way_cfg_group() {
    let input = vec![
        make_fn(
            "download_model",
            "my_crate::download_model",
            Some(r#"feature = "real""#),
            &["name", "cache_dir"],
        ),
        make_fn(
            "download_model",
            "my_crate::download_model",
            Some(r#"all(feature = "stub-a", not(feature = "real"))"#),
            &["_name", "_cache_dir"],
        ),
        make_fn(
            "download_model",
            "my_crate::download_model",
            Some(r#"all(feature = "stub-b", not(feature = "real"))"#),
            &["_name", "_cache_dir"],
        ),
    ];
    let out = dedup_same_name_functions(&input);
    assert_eq!(out.len(), 1, "three-way group must collapse to one entry");
    assert_eq!(
        out[0].params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        vec!["name", "cache_dir"],
        "canonical entry must keep the real (non-underscore) param names"
    );
}

#[test]
fn unconditional_entry_produces_no_cfg() {
    let input = vec![
        make_fn("embed_texts_async", "my_crate::embed_texts_async", None, &["texts"]),
        make_fn(
            "embed_texts_async",
            "my_crate::embed_texts_async",
            Some(r#"all(feature = "embedding-presets", not(feature = "embeddings"))"#),
            &["_texts"],
        ),
    ];
    let out = dedup_same_name_functions(&input);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].cfg.is_none(),
        "when one branch is unconditional, the merged entry must also be unconditional"
    );
}

#[test]
fn preserves_relative_order_of_other_functions() {
    let input = vec![
        make_fn("before", "my_crate::before", None, &[]),
        make_fn(
            "embed_texts_async",
            "my_crate::embed_texts_async",
            Some(r#"feature = "a""#),
            &["texts"],
        ),
        make_fn(
            "embed_texts_async",
            "my_crate::embed_texts_async",
            Some(r#"feature = "b""#),
            &["_texts"],
        ),
        make_fn("after", "my_crate::after", None, &[]),
    ];
    let out = dedup_same_name_functions(&input);
    let names: Vec<&str> = out.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["before", "embed_texts_async", "after"]);
}

/// Same-named entries that share an identical cfg are NOT a cfg-variant pair (they are genuine
/// arity overloads emitted from convenience defaults) and must be left untouched.
#[test]
fn does_not_merge_identical_cfg_overloads() {
    let input = vec![
        make_fn("render", "my_crate::render", None, &["a", "b"]),
        make_fn("render", "my_crate::render", None, &["a"]),
    ];
    let out = dedup_same_name_functions(&input);
    assert_eq!(
        out.len(),
        2,
        "identical-cfg same-name entries are arity overloads, not cfg variants, and must be preserved"
    );
}

#[test]
fn merges_multiple_function_groups_without_index_drift() {
    let input = vec![
        make_fn(
            "first_async",
            "my_crate::first_async",
            Some(r#"feature = "alpha""#),
            &["value"],
        ),
        make_fn(
            "first_async",
            "my_crate::first_async",
            Some(r#"feature = "alpha-fallback""#),
            &["_value"],
        ),
        make_fn("middle", "my_crate::middle", None, &[]),
        make_fn(
            "second_async",
            "my_crate::second_async",
            Some(r#"feature = "beta""#),
            &["value"],
        ),
        make_fn(
            "second_async",
            "my_crate::second_async",
            Some(r#"feature = "beta-fallback""#),
            &["_value"],
        ),
        make_fn("after", "my_crate::after", None, &[]),
    ];
    let out = dedup_same_name_functions(&input);
    let names: Vec<&str> = out.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["first_async", "middle", "second_async", "after"]);
    assert_eq!(out[0].params[0].name, "value");
    assert_eq!(out[2].params[0].name, "value");
    assert!(
        out[0].cfg.as_deref().is_some_and(|cfg| cfg.starts_with("any(")),
        "first merged cfg should be an any(...) expression"
    );
    assert!(
        out[2].cfg.as_deref().is_some_and(|cfg| cfg.starts_with("any(")),
        "second merged cfg should be an any(...) expression"
    );
}

/// Regression guard: the dedup helper is a pure transformation on the input slice — it must
/// not mutate the source. This is the contract Rust-cfg-gated backends rely on so that they
/// (and the e2e call-export validator) continue to see the multi-entry surface untouched.
#[test]
fn does_not_mutate_input_function_list() {
    let cfg_real = r#"all(feature = "reranker", feature = "tokio-runtime")"#;
    let cfg_stub = r#"all(feature = "reranker-presets", not(feature = "reranker"), feature = "tokio-runtime")"#;

    let input = vec![
        make_fn(
            "rerank_async",
            "my_crate::rerank_async",
            Some(cfg_real),
            &["query", "documents", "config"],
        ),
        make_fn(
            "rerank_async",
            "my_crate::rerank_async",
            Some(cfg_stub),
            &["_query", "_documents", "_config"],
        ),
    ];

    let input_snapshot = input.clone();
    let _ = dedup_same_name_functions(&input);

    assert_eq!(
        input.len(),
        input_snapshot.len(),
        "dedup must not modify the input slice"
    );
    for (before, after) in input_snapshot.iter().zip(input.iter()) {
        assert_eq!(before.name, after.name);
        assert_eq!(before.rust_path, after.rust_path);
        assert_eq!(before.cfg, after.cfg);
        assert_eq!(
            before.params.iter().map(|p| &p.name).collect::<Vec<_>>(),
            after.params.iter().map(|p| &p.name).collect::<Vec<_>>(),
        );
    }
}
