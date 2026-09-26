//! Carry `#[cfg(...)]` gates from `lib.rs` free functions into the
//! flutter_rust_bridge-generated `frb_generated.rs` wire dispatch.
//!
//! flutter_rust_bridge has no concept of Cargo features: its `#[frb]` macro
//! expansion runs exactly once, against whichever feature set was active for
//! that codegen run (the crate's `default` features), and bakes every
//! function it saw — gated or not — into `frb_generated.rs` unconditionally.
//! That includes a numeric `func_id => wire__crate__<name>_impl(...)`
//! dispatch arm and the `fn wire__crate__<name>_impl(...)` wrapper that calls
//! straight into `crate::<name>(...)`.
//!
//! `frb_generated.rs` is generated once and committed; the *same* file is
//! then compiled against every platform's feature subset (for example
//! Android's `--no-default-features --features android-target`). When that
//! subset excludes a feature that gates a function in `lib.rs`, the wire
//! wrapper still calls it unconditionally, which fails to compile with
//! `E0425: cannot find function ... in the crate root`.
//!
//! [`cfg_gated_free_functions`] scans `lib.rs` for top-level (`col 0`)
//! `#[cfg(...)]`-gated `pub fn` / `pub async fn` items and returns their
//! `(name, gate)` pairs, where `gate` is the exact, possibly multi-line
//! `#[cfg(...)]` attribute text. [`inject_frb_cfg_gates`] then adds a copy of
//! that same gate directly above both the `fn wire__crate__<name>_impl(...)`
//! wrapper and its dispatch-table arm in `frb_generated.rs`.
//!
//! ## Wire-ID stability
//!
//! `frb_generated.rs`'s dispatch table is a `match func_id { <literal> =>
//! ..., ... }` — each arm is keyed by an explicit numeric literal assigned
//! once, when the (single, committed) `frb_generated.rs` was generated, not
//! by its position in the match. Gating an arm out under a reduced feature
//! set removes that one arm; it never renumbers any sibling arm, because
//! sibling arms don't derive their ID from position. So every function that
//! *does* survive a given feature subset keeps the exact wire ID it was
//! assigned at generation time, and Dart-side code compiled against that
//! same `frb_generated.rs` — the file is never regenerated per platform —
//! always dispatches to the right function.
//!
//! This module only recognizes top-level free functions. `#[cfg(...)]` gates
//! on `impl` blocks (opaque-type methods, e.g. `#[cfg(feature = "presets")]
//! impl MetaSchema { ... }`) are not covered; extending this rewrite to that
//! shape needs a matching `wire__crate__<Type>_<method>_impl` lookup and is
//! left for when a gated method actually triggers this failure mode.

/// Scan `lib_rs_source` for top-level `#[cfg(...)]`-gated `pub fn` /
/// `pub async fn` free functions and return their `(name, gate)` pairs.
///
/// `gate` is the exact attribute text, including the `#[cfg(` / `)]`
/// delimiters, exactly as written in `lib.rs` (multi-line `cfg(any(...))`
/// predicates are preserved verbatim). Only functions declared at column 0
/// are considered — gates on fields, enum variants, or `impl` blocks (all
/// indented) are ignored, since they don't correspond to a free-function wire
/// wrapper in `frb_generated.rs`.
///
/// A `#[cfg(...)]` gate is followed by whatever
/// [`free_pub_fn_name_past_attributes`] skips over -- see its doc for why that lookahead exists:
/// the real generated facade almost never puts the `pub fn` directly on the next line. ~keep
pub fn cfg_gated_free_functions(lib_rs_source: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = lib_rs_source.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("#[cfg(")
            && let Some(end) = find_attribute_end(&lines, i)
        {
            if let Some(name) = free_pub_fn_name_past_attributes(&lines, end + 1) {
                result.push((name, lines[i..=end].join("\n")));
            }
            i = end + 1;
            continue;
        }
        i += 1;
    }

    result
}

/// Starting at `start` -- the line immediately after a `#[cfg(...)]` gate (or any other
/// attribute [`find_attribute_end`] has just skipped) -- skip past any further attribute lines
/// (`#[...]`, single- or multi-line, closed the same balanced-paren way `#[cfg(...)]` itself is)
/// and doc comments (`///`, `//!`), then return the free function name on the line the scan
/// lands on.
///
/// The real generated FRB facade puts at least one more attribute -- `#[frb]`, `#[frb(opaque)]`,
/// ... -- between a `#[cfg(...)]` gate and the `pub fn` it gates (see
/// `backends::dart::templates::rust_from_json_bridge_fn.rs.jinja`, which always emits
/// `#[cfg(...)]` immediately followed by `#[frb]`). Looking only at the line directly after the
/// gate -- an earlier version of this function did exactly that -- finds the association for the
/// one facade shape that has no intervening attribute and silently drops it for nearly every
/// other gated free function alef actually generates: the gate is never recorded, so
/// [`super::bridge_coverage::active_free_function_names`] treats the function as ungated and
/// unconditionally expects it in the bridge, even when its feature is off and frb correctly
/// never emitted it. ~keep
///
/// Returns `None` -- the gate does not attach to a free function -- as soon as the scan lands on
/// anything that is not itself a skippable attribute/doc-comment line or a `pub fn`/`pub async
/// fn` line: a blank line, a private `fn`, an `impl`/`struct`/`enum`/`mod` item, or the source
/// simply ending. This preserves the pre-existing impl-level-gate exclusion: `#[cfg(...)] impl
/// Foo { ... }` must never be mistaken for a gated free function.
///
/// Deliberately does NOT skip a further `#[cfg(...)]` line the way it skips `#[frb]` and other
/// plain attributes: two stacked `#[cfg(...)]` attributes directly on one function is not a shape
/// this repo's generators produce (`codegen::cfg::combine_gates` always collapses an owner and a
/// member gate into one `all(...)` predicate instead), so treating the second gate as "just
/// another attribute to skip past" would let the outer gate's own lookahead reach the function
/// too and record a second, duplicate `(name, gate)` entry for it alongside the one the caller's
/// own top-level scan already records for the inner gate. Returning `None` here instead -- the
/// same answer this function already gives any other non-attribute, non-fn line -- keeps that
/// pre-existing single-entry behavior unchanged. ~keep
fn free_pub_fn_name_past_attributes(lines: &[&str], start: usize) -> Option<String> {
    let mut i = start;
    while let Some(line) = lines.get(i) {
        if line.starts_with("///") || line.starts_with("//!") {
            i += 1;
            continue;
        }
        if line.starts_with("#[") && !line.starts_with("#[cfg(") {
            i = find_attribute_end(lines, i)? + 1;
            continue;
        }
        return free_pub_fn_name(line);
    }
    None
}

/// Starting at `start` (a line beginning with `#[`), find the index of the line where the
/// attribute closes: its parens balance out (an attribute with no `(...)` argument list, such as
/// `#[frb]`, balances at zero immediately) and the line ends with `]`. Returns `None` if the
/// parens never balance before the source ends.
///
/// Shared by [`cfg_gated_free_functions`] (always called on a `#[cfg(`-prefixed line) and
/// [`free_pub_fn_name_past_attributes`] (called on any attribute line between a gate and the
/// function it gates) -- one balanced-paren scan for every attribute shape, not two
/// independently maintained ones that could disagree about what "closed" means. ~keep
fn find_attribute_end(lines: &[&str], start: usize) -> Option<usize> {
    let mut depth = paren_delta(lines[start]);
    let mut end = start;
    while depth > 0 {
        end += 1;
        let line = *lines.get(end)?;
        depth += paren_delta(line);
    }
    (depth == 0 && lines[end].trim_end().ends_with(']')).then_some(end)
}

fn paren_delta(line: &str) -> i32 {
    line.matches('(').count() as i32 - line.matches(')').count() as i32
}

/// Returns the function name if `line` is a top-level (`col 0`) `pub fn` or
/// `pub async fn` declaration.
///
/// `pub(super)`: also used by [`super::bridge_coverage::free_function_names`] to enumerate
/// every free function the FRB facade declares, not just the cfg-gated subset this module
/// itself cares about.
pub(super) fn free_pub_fn_name(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("pub async fn ")
        .or_else(|| line.strip_prefix("pub fn "))?;
    let name_end = rest.find('(')?;
    Some(rest[..name_end].to_string())
}

/// Add a copy of each `(name, gate)` pair's `#[cfg(...)]` gate directly above
/// the matching `fn wire__crate__<name>_impl(...)` definition and its
/// `<id> => wire__crate__<name>_impl(...)` dispatch-table arm in
/// `frb_generated_source`.
///
/// Idempotent: running this twice on the same input does not duplicate a gate
/// that is already present immediately above a matching line.
pub fn inject_frb_cfg_gates(frb_generated_source: &str, cfg_gated_fns: &[(String, String)]) -> String {
    if cfg_gated_fns.is_empty() {
        return frb_generated_source.to_string();
    }

    let lines: Vec<&str> = frb_generated_source.lines().collect();
    let mut result = String::with_capacity(frb_generated_source.len());

    for (idx, line) in lines.iter().enumerate() {
        if let Some(gate) = gate_for_wire_line(line, cfg_gated_fns)
            && !already_gated(&lines, idx, gate)
        {
            let indent = leading_whitespace(line);
            for gate_line in gate.lines() {
                result.push_str(&indent);
                result.push_str(gate_line);
                result.push('\n');
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    if !frb_generated_source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Combines [`cfg_gated_free_functions`] and [`inject_frb_cfg_gates`]: read
/// the gates straight out of the already-generated `lib.rs` and carry them
/// into `frb_generated.rs`.
pub fn carry_lib_rs_cfg_gates_into_frb_generated(lib_rs_source: &str, frb_generated_source: &str) -> String {
    let gated = cfg_gated_free_functions(lib_rs_source);
    inject_frb_cfg_gates(frb_generated_source, &gated)
}

fn leading_whitespace(line: &str) -> String {
    line.chars().take_while(|c| c.is_whitespace()).collect()
}

fn gate_for_wire_line<'a>(line: &str, cfg_gated_fns: &'a [(String, String)]) -> Option<&'a str> {
    let trimmed = line.trim_start();
    cfg_gated_fns.iter().find_map(|(name, gate)| {
        let def = format!("fn wire__crate__{name}_impl(");
        let arm = format!("=> wire__crate__{name}_impl(");
        (trimmed.starts_with(&def) || trimmed.contains(&arm)).then_some(gate.as_str())
    })
}

fn already_gated(lines: &[&str], idx: usize, gate: &str) -> bool {
    let gate_last_line = gate.lines().last().unwrap_or(gate).trim();
    idx > 0 && lines[idx - 1].trim() == gate_last_line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_gated_free_functions_finds_single_line_gate() {
        let lib_rs =
            "#[cfg(feature = \"transcription\")]\npub fn build_decoder_prompt_tokens(x: i64) -> i64 {\n    x\n}\n";
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(
            found,
            vec![(
                "build_decoder_prompt_tokens".to_string(),
                "#[cfg(feature = \"transcription\")]".to_string()
            )]
        );
    }

    #[test]
    fn cfg_gated_free_functions_finds_async_fn() {
        let lib_rs = "#[cfg(feature = \"url-ingestion\")]\npub async fn map_url(uri: String) -> String {\n    uri\n}\n";
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(
            found,
            vec![("map_url".to_string(), "#[cfg(feature = \"url-ingestion\")]".to_string())]
        );
    }

    #[test]
    fn cfg_gated_free_functions_preserves_multiline_predicate() {
        let lib_rs = concat!(
            "#[cfg(any(\n",
            "    any(feature = \"scoring-presets\", feature = \"scoring\"),\n",
            "    feature = \"scoring-presets\"\n",
            "))]\n",
            "pub fn score_pair(a: i64, b: i64) -> f64 {\n",
            "    0.0\n",
            "}\n",
        );
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "score_pair");
        assert!(found[0].1.starts_with("#[cfg(any(\n"));
        assert!(found[0].1.ends_with("))]"));
    }

    #[test]
    fn cfg_gated_free_functions_ignores_indented_gates() {
        let lib_rs = concat!(
            "#[cfg(feature = \"presets\")]\n",
            "impl MetaSchema {\n",
            "    #[cfg(feature = \"presets\")]\n",
            "    pub fn compile(json: String) -> MetaSchema {\n",
            "        todo!()\n",
            "    }\n",
            "}\n",
        );
        // The impl-level gate at column 0 is followed by `impl MetaSchema {`, not a
        // `pub fn`, so it must not be picked up; the indented method-level gate is
        // not at column 0 at all.
        assert!(cfg_gated_free_functions(lib_rs).is_empty());
    }

    #[test]
    fn cfg_gated_free_functions_ignores_ungated_functions() {
        let lib_rs = "pub fn always_present() -> i32 {\n    1\n}\n";
        assert!(cfg_gated_free_functions(lib_rs).is_empty());
    }

    /// The production shape: `backends::dart::templates::rust_from_json_bridge_fn.rs.jinja`
    /// always emits `#[cfg(...)]` immediately followed by `#[frb]`, never a bare `pub fn` right
    /// after the gate. A version of this scanner that only looked one line past the gate found
    /// nothing here and treated the function as ungated. Positive cases (gate must attach) and
    /// negative controls (gate must not attach to anything) are tabulated together so the two
    /// kinds of assertion stay next to each other. ~keep
    struct Case {
        name: &'static str,
        lib_rs: &'static str,
        expected: Option<(&'static str, &'static str)>,
    }

    fn assert_cfg_gated_case(case: &Case) {
        let found = cfg_gated_free_functions(case.lib_rs);
        let actual = found.first().map(|(name, gate)| (name.as_str(), gate.as_str()));
        assert_eq!(actual, case.expected, "case failed: {}", case.name);
        assert_eq!(
            found.len(),
            usize::from(case.expected.is_some()),
            "case found extra/unexpected entries: {}: {found:?}",
            case.name
        );
    }

    #[test]
    fn cfg_gated_free_functions_table() {
        let cases = [
            Case {
                name: "single #[frb] between gate and pub fn -- the real facade shape",
                lib_rs: "#[cfg(feature = \"alpha\")]\n#[frb]\npub fn convert_payload(json: String) -> String {\n    json\n}\n",
                expected: Some(("convert_payload", "#[cfg(feature = \"alpha\")]")),
            },
            Case {
                name: "#[frb(opaque)] between gate and pub async fn",
                lib_rs: "#[cfg(feature = \"alpha\")]\n#[frb(opaque)]\npub async fn convert_payload_async(json: String) -> String {\n    json\n}\n",
                expected: Some(("convert_payload_async", "#[cfg(feature = \"alpha\")]")),
            },
            Case {
                name: "several attributes and doc comments between gate and pub fn",
                lib_rs: concat!(
                    "#[cfg(feature = \"alpha\")]\n",
                    "#[frb(sync)]\n",
                    "#[allow(dead_code)]\n",
                    "/// Convert a payload.\n",
                    "/// Second doc line.\n",
                    "pub fn convert_payload(json: String) -> String {\n",
                    "    json\n",
                    "}\n",
                ),
                expected: Some(("convert_payload", "#[cfg(feature = \"alpha\")]")),
            },
            Case {
                name: "multi-line predicate with #[frb] still attaches, full predicate preserved",
                lib_rs: concat!(
                    "#[cfg(any(\n",
                    "    feature = \"alpha\",\n",
                    "    feature = \"beta\"\n",
                    "))]\n",
                    "#[frb]\n",
                    "pub fn convert_payload(json: String) -> String {\n",
                    "    json\n",
                    "}\n",
                ),
                expected: Some((
                    "convert_payload",
                    "#[cfg(any(\n    feature = \"alpha\",\n    feature = \"beta\"\n))]",
                )),
            },
            Case {
                name: "negative control: gate directly above impl must not attach",
                lib_rs: "#[cfg(feature = \"alpha\")]\n#[frb]\nimpl Payload {\n}\n",
                expected: None,
            },
            Case {
                name: "negative control: blank line before pub fn must not attach",
                lib_rs: "#[cfg(feature = \"alpha\")]\n#[frb]\n\npub fn convert_payload(json: String) -> String {\n    json\n}\n",
                expected: None,
            },
            Case {
                name: "negative control: a private fn must not attach",
                lib_rs: "#[cfg(feature = \"alpha\")]\n#[frb]\nfn convert_payload(json: String) -> String {\n    json\n}\n",
                expected: None,
            },
            Case {
                name: "negative control: gate directly above struct must not attach",
                lib_rs: "#[cfg(feature = \"alpha\")]\n#[frb(opaque)]\nstruct Payload {\n}\n",
                expected: None,
            },
        ];

        for case in &cases {
            assert_cfg_gated_case(case);
        }
    }

    /// Two `#[cfg(...)]` attributes stacked directly on one function must still yield exactly
    /// one `(name, gate)` entry -- the inner gate's, the same single entry the pre-fix scanner
    /// produced -- not two. The lookahead that now skips a `#[frb]`/`#[allow(...)]`/etc.
    /// attribute between a gate and its function must not also treat a second `#[cfg(...)]` as
    /// "just another attribute to skip past": doing so would let the outer gate's own lookahead
    /// reach the function too and double-record it, once per gate, alongside the entry the
    /// scanner's own top-level loop already records when it reaches the inner gate on its next
    /// iteration.
    #[test]
    fn cfg_gated_free_functions_does_not_double_count_stacked_cfg_attributes() {
        let lib_rs = "#[cfg(feature = \"alpha\")]\n#[cfg(feature = \"beta\")]\npub fn convert_payload(json: String) -> String {\n    json\n}\n";
        let found = cfg_gated_free_functions(lib_rs);
        assert_eq!(
            found,
            vec![("convert_payload".to_string(), "#[cfg(feature = \"beta\")]".to_string())],
            "stacked cfg attributes must yield exactly one entry, keyed on the inner gate: {found:?}"
        );
    }

    #[test]
    fn inject_frb_cfg_gates_gates_definition_and_dispatch_arm() {
        let frb_generated = concat!(
            "fn wire__crate__build_decoder_prompt_tokens_impl(\n",
            "    port_: i32,\n",
            ") {\n",
            "}\n",
            "fn dispatch(func_id: i32) {\n",
            "    match func_id {\n",
            "        28 => wire__crate__build_decoder_prompt_tokens_impl(port, ptr, rust_vec_len, data_len),\n",
            "        29 => wire__crate__classify_chunks_impl(port, ptr, rust_vec_len, data_len),\n",
            "    }\n",
            "}\n",
        );
        let gated = vec![(
            "build_decoder_prompt_tokens".to_string(),
            "#[cfg(feature = \"transcription\")]".to_string(),
        )];

        let result = inject_frb_cfg_gates(frb_generated, &gated);

        assert!(
            result.contains("#[cfg(feature = \"transcription\")]\nfn wire__crate__build_decoder_prompt_tokens_impl("),
            "missing gate above the wire wrapper definition: {result}"
        );
        let expected_arm_gate = "        #[cfg(feature = \"transcription\")]\n        \
            28 => wire__crate__build_decoder_prompt_tokens_impl(";
        assert!(
            result.contains(expected_arm_gate),
            "missing gate above the dispatch arm, or wrong indentation: {result}"
        );
        assert!(
            !result.contains("#[cfg(feature = \"transcription\")]\n        29 =>"),
            "unrelated dispatch arm must not be gated: {result}"
        );
    }

    #[test]
    fn inject_frb_cfg_gates_is_idempotent() {
        let frb_generated = concat!(
            "fn wire__crate__timestamp_token_to_ms_impl(\n",
            "    port_: i32,\n",
            ") {\n",
            "}\n",
        );
        let gated = vec![(
            "timestamp_token_to_ms".to_string(),
            "#[cfg(feature = \"transcription\")]".to_string(),
        )];

        let once = inject_frb_cfg_gates(frb_generated, &gated);
        let twice = inject_frb_cfg_gates(&once, &gated);

        assert_eq!(once, twice);
        assert_eq!(once.matches("#[cfg(feature = \"transcription\")]").count(), 1);
    }

    #[test]
    fn inject_frb_cfg_gates_noop_when_no_gated_functions() {
        let frb_generated = "fn wire__crate__always_present_impl() {}\n";
        assert_eq!(inject_frb_cfg_gates(frb_generated, &[]), frb_generated);
    }

    #[test]
    fn carry_lib_rs_cfg_gates_into_frb_generated_end_to_end() {
        let lib_rs = "#[cfg(feature = \"transcription\")]\npub fn timestamp_token_to_ms(id: i64) -> i64 {\n    id\n}\n";
        let frb_generated = concat!(
            "fn wire__crate__timestamp_token_to_ms_impl(port_: i32) {}\n",
            "fn dispatch(func_id: i32) {\n",
            "    match func_id {\n",
            "        287 => wire__crate__timestamp_token_to_ms_impl(port, ptr, rust_vec_len, data_len),\n",
            "    }\n",
            "}\n",
        );

        let result = carry_lib_rs_cfg_gates_into_frb_generated(lib_rs, frb_generated);

        assert!(result.contains("#[cfg(feature = \"transcription\")]\nfn wire__crate__timestamp_token_to_ms_impl("));
        assert!(result.contains("        #[cfg(feature = \"transcription\")]\n        287 =>"));
    }

    /// The second consequence of the same parsing gap: `carry_lib_rs_cfg_gates_into_frb_generated`
    /// (backing the `CarryFrbCfgGates` post-build step) is built entirely on
    /// `cfg_gated_free_functions`, so the real facade shape -- `#[cfg(...)]` immediately followed
    /// by `#[frb]` -- must carry its gate into `frb_generated.rs` exactly like the no-intervening-
    /// attribute case above does. Before the lookahead fix, this gate was never recorded, so
    /// neither the wire wrapper nor its dispatch arm ever got gated in `frb_generated.rs` either.
    #[test]
    fn carry_lib_rs_cfg_gates_into_frb_generated_end_to_end_with_intervening_frb_attribute() {
        let lib_rs =
            "#[cfg(feature = \"transcription\")]\n#[frb]\npub fn timestamp_token_to_ms(id: i64) -> i64 {\n    id\n}\n";
        let frb_generated = concat!(
            "fn wire__crate__timestamp_token_to_ms_impl(port_: i32) {}\n",
            "fn dispatch(func_id: i32) {\n",
            "    match func_id {\n",
            "        287 => wire__crate__timestamp_token_to_ms_impl(port, ptr, rust_vec_len, data_len),\n",
            "    }\n",
            "}\n",
        );

        let result = carry_lib_rs_cfg_gates_into_frb_generated(lib_rs, frb_generated);

        assert!(result.contains("#[cfg(feature = \"transcription\")]\nfn wire__crate__timestamp_token_to_ms_impl("));
        assert!(result.contains("        #[cfg(feature = \"transcription\")]\n        287 =>"));
    }
}
