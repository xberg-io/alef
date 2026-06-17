use super::*;

/// Normalize a cfg string by stripping all whitespace for substring comparisons.
///
/// syn's token stream pretty-printer inserts spaces around punctuation
/// (e.g. `all (feature = "X" , feature = "Y")`), so substring checks on the
/// original string fail.  Normalizing both sides makes the comparison robust.
fn normalize_cfg(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Two public functions with the same identifier but disjoint cfg gates — the pattern that
/// causes the FFI emitter to emit the C symbol under only one branch's narrow cfg.
#[test]
fn merges_disjoint_cfgs_for_same_ffi_name_functions() {
    let cfg_real = r#"all(feature = "embeddings", feature = "tokio-runtime")"#;
    let cfg_stub = r#"all(feature = "embedding-presets", not(feature = "embeddings"), feature = "tokio-runtime")"#;

    let source = format!(
        r#"
        #[cfg({cfg_real})]
        pub fn embed_texts_async(texts: Vec<String>, config: String) -> String {{
            String::new()
        }}

        #[cfg({cfg_stub})]
        pub fn embed_texts_async(_texts: Vec<String>, _config: String) -> String {{
            String::new()
        }}
    "#
    );

    let surface = extract_from_source(&source);

    assert_eq!(
        surface.functions.len(),
        1,
        "disjoint-cfg group must be collapsed into a single function entry"
    );

    let merged = &surface.functions[0];
    assert_eq!(merged.name, "embed_texts_async");

    let cfg = merged.cfg.as_deref().expect("merged entry must have a cfg");
    // syn's pretty-printer may insert spaces, so compare whitespace-normalized forms.
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

    // Canonical entry must be the real implementation (non-underscore params).
    let param_names: Vec<&str> = merged.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        param_names,
        vec!["texts", "config"],
        "canonical entry must use the non-underscore parameter names from the real implementation"
    );
}

#[test]
fn no_op_when_single_function_per_name() {
    let source = r#"
        #[cfg(feature = "embeddings")]
        pub fn embed_texts_async(texts: Vec<String>) -> String { String::new() }
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].cfg.as_deref(), Some(r#"feature = "embeddings""#));
}

#[test]
fn unconditional_entry_produces_no_cfg() {
    // One branch unconditional, one gated: the merged entry must also be unconditional.
    //
    // Note: syn does not reject duplicate fn definitions at the parse level, so both entries
    // survive into `extract_items`.  If the parser happens to error, we skip gracefully.
    let source = r#"
        pub fn embed_texts_async(texts: Vec<String>) -> String { String::new() }

        #[cfg(all(feature = "embedding-presets", not(feature = "embeddings")))]
        pub fn embed_texts_async(_texts: Vec<String>) -> String { String::new() }
    "#;
    if syn::parse_str::<syn::File>(source).is_err() {
        return;
    }
    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);
    assert!(
        surface.functions[0].cfg.is_none(),
        "when one branch is unconditional, the merged entry must also be unconditional"
    );
}

#[test]
fn preserves_relative_order_of_other_functions() {
    // The merged entry must land at the position of its first occurrence.
    let source = r#"
        pub fn before() {}

        #[cfg(feature = "a")]
        pub fn embed_texts_async(texts: Vec<String>) -> String { String::new() }

        #[cfg(feature = "b")]
        pub fn embed_texts_async(_texts: Vec<String>) -> String { String::new() }

        pub fn after() {}
    "#;
    if syn::parse_str::<syn::File>(source).is_err() {
        return;
    }
    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 3);
    assert_eq!(surface.functions[0].name, "before");
    assert_eq!(surface.functions[1].name, "embed_texts_async");
    assert_eq!(surface.functions[2].name, "after");
}

#[test]
fn merges_multiple_function_groups_without_index_drift() {
    let source = r#"
        #[cfg(feature = "alpha")]
        pub fn first_async(value: String) -> String { value }

        #[cfg(feature = "alpha-fallback")]
        pub fn first_async(_value: String) -> String { String::new() }

        pub fn middle() {}

        #[cfg(feature = "beta")]
        pub fn second_async(value: String) -> String { value }

        #[cfg(feature = "beta-fallback")]
        pub fn second_async(_value: String) -> String { String::new() }

        pub fn after() {}
    "#;

    let surface = extract_from_source(source);
    let names: Vec<&str> = surface.functions.iter().map(|func| func.name.as_str()).collect();

    assert_eq!(names, vec!["first_async", "middle", "second_async", "after"]);
    assert_eq!(surface.functions[0].params[0].name, "value");
    assert_eq!(surface.functions[2].params[0].name, "value");
    assert!(
        surface.functions[0]
            .cfg
            .as_deref()
            .is_some_and(|cfg| cfg.starts_with("any(")),
        "first merged cfg should be an any(...) expression"
    );
    assert!(
        surface.functions[2]
            .cfg
            .as_deref()
            .is_some_and(|cfg| cfg.starts_with("any(")),
        "second merged cfg should be an any(...) expression"
    );
}
