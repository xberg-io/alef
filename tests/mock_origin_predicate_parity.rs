//! Parity gate between the two hand-maintained copies of alef's origin-root routing predicate
//! (alef issue #433).
//!
//! `Fixture::has_host_root_route` (`src/e2e/fixture.rs`) and `has_host_root` inside the
//! generated standalone mock-server binary (`src/e2e/codegen/rust/mock_server/route_loading.rs`,
//! spliced from a raw string literal via `render_mock_server_binary()`) both decide whether a
//! route-array fixture needs a dedicated per-fixture listener. They must agree, or a fixture
//! that alef's own generation-time checks say is fine ends up 404ing at test runtime for a
//! reason that has nothing to do with what it's testing -- this repo's `two-generators-disagree`
//! defect shape.
//!
//! # Why this is not a plain text `.contains(...)` check
//!
//! `route_loading.rs`'s copy lives inside a raw string literal that becomes real Rust source in
//! the generated binary, and this test cannot execute that binary without pulling axum/tokio/
//! flate2/brotli into `alef`'s own build (they are deliberately NOT `alef`'s dependencies --
//! they exist only as text this crate emits). So this test instead **parses** the real,
//! assembled generated source with `syn` and inspects the actual AST of the
//! `load_routes_recursive` function: only real code, never a comment (comments are stripped as
//! trivia before `syn` ever sees them, so they cannot satisfy any check here) and never a
//! literal that merely happens to sit somewhere else in the ~1000-line generated file. Origin
//! substitution's four token strings already appear elsewhere in that file today (in
//! `Origins::token_pairs`, which exists to *substitute* them into response bodies, not to
//! *route* based on their presence) -- a plain `out.contains("{{mock_alt_origin}}")` over the
//! whole rendered binary would therefore already be true before the routing fix ever lands, and
//! would prove nothing. [`origin_tokens_reachable_from_load_routes_recursive`] instead follows
//! only the identifiers `load_routes_recursive`'s own body actually references (transitively,
//! through top-level fn/const/associated-fn definitions in the same file), so it can only see a
//! token literal that the routing function truly reads.

use alef::e2e::codegen::rust::render_mock_server_binary;
use alef::e2e::fixture::Fixture;
use alef::e2e::fixture::origin_tokens::ORIGIN_SUBSTITUTION_TOKENS;
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;

// ---------------------------------------------------------------------------
// AST plumbing: what does `load_routes_recursive` actually reach?
// ---------------------------------------------------------------------------

/// String literals and single-segment identifier references found while visiting a syntax tree.
/// The identifier references are how the walk follows a call like `is_host_root_path(...)` (or
/// whatever name a new origin-token helper uses) to that function's own body.
#[derive(Default)]
struct LiteralsAndRefs {
    literals: Vec<String>,
    refs: Vec<String>,
}

impl<'ast> Visit<'ast> for LiteralsAndRefs {
    fn visit_expr_lit(&mut self, node: &'ast syn::ExprLit) {
        if let syn::Lit::Str(text) = &node.lit {
            self.literals.push(text.value());
        }
        syn::visit::visit_expr_lit(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if let Some(ident) = node.path.get_ident() {
            self.refs.push(ident.to_string());
        }
        syn::visit::visit_expr_path(self, node);
    }
}

fn literals_and_refs_in_block(block: &syn::Block) -> LiteralsAndRefs {
    let mut collector = LiteralsAndRefs::default();
    collector.visit_block(block);
    collector
}

fn literals_and_refs_in_expr(expr: &syn::Expr) -> LiteralsAndRefs {
    let mut collector = LiteralsAndRefs::default();
    collector.visit_expr(expr);
    collector
}

/// Every top-level fn, const, static, and associated fn in the file, keyed by name, with its own
/// (non-transitive) literals and references already collected. `load_routes_recursive` reaching
/// any of these by name is what lets the walk below follow a call into it.
fn build_top_level_item_map(file: &syn::File) -> HashMap<String, LiteralsAndRefs> {
    let mut map = HashMap::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(item_fn) => {
                map.insert(
                    item_fn.sig.ident.to_string(),
                    literals_and_refs_in_block(&item_fn.block),
                );
            }
            syn::Item::Const(item_const) => {
                map.insert(
                    item_const.ident.to_string(),
                    literals_and_refs_in_expr(&item_const.expr),
                );
            }
            syn::Item::Static(item_static) => {
                map.insert(
                    item_static.ident.to_string(),
                    literals_and_refs_in_expr(&item_static.expr),
                );
            }
            syn::Item::Impl(item_impl) => {
                for impl_item in &item_impl.items {
                    if let syn::ImplItem::Fn(method) = impl_item {
                        map.entry(method.sig.ident.to_string())
                            .or_insert_with(|| literals_and_refs_in_block(&method.block));
                    }
                }
            }
            _ => {}
        }
    }
    map
}

/// Breadth-first closure over every literal reachable from `start` by following references into
/// `item_map`, entry names already visited are never re-expanded so a recursive/mutual call
/// pair cannot loop forever.
fn transitive_literals(start: LiteralsAndRefs, item_map: &HashMap<String, LiteralsAndRefs>) -> HashSet<String> {
    let mut collected: HashSet<String> = start.literals.into_iter().collect();
    let mut queue: Vec<String> = start.refs;
    let mut visited: HashSet<String> = HashSet::new();
    while let Some(name) = queue.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        if let Some(item) = item_map.get(&name) {
            collected.extend(item.literals.iter().cloned());
            queue.extend(item.refs.iter().cloned());
        }
    }
    collected
}

/// Parses `render_mock_server_binary()` and returns the set of string literals transitively
/// reachable from `load_routes_recursive`'s body -- the function whose `has_host_root` local
/// decides per-fixture routing (see the sibling `is_host_root_path` doc comment on
/// `Fixture::has_host_root_route` in `src/e2e/fixture.rs`).
fn literals_reachable_from_route_loading() -> HashSet<String> {
    let source = render_mock_server_binary();
    let file = syn::parse_file(&source).expect("generated mock-server binary must parse as Rust");
    let item_map = build_top_level_item_map(&file);
    let route_loading_fn = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(item_fn) if item_fn.sig.ident == "load_routes_recursive" => Some(item_fn),
            _ => None,
        })
        .expect(
            "generated mock-server binary must define `load_routes_recursive` -- this test's \
             extraction target was renamed or removed; update this test to match",
        );
    let start = literals_and_refs_in_block(&route_loading_fn.block);
    transitive_literals(start, &item_map)
}

/// True once `route_loading.rs`'s `has_host_root` decision (inside `load_routes_recursive`)
/// actually reads all four origin-substitution tokens -- i.e., once the sibling half of this
/// predicate has been extended to match [`Fixture::has_host_root_route`]'s new clause.
fn real_predicate_covers_origin_tokens() -> bool {
    let reachable = literals_reachable_from_route_loading();
    ORIGIN_SUBSTITUTION_TOKENS
        .iter()
        .all(|token| reachable.contains(*token))
}

/// Guards the pre-existing (already-shipped, not part of this issue) trigger mirror below
/// against a silent rename: if `load_routes_recursive` stops referencing these identifiers, the
/// hand-mirrored logic in [`pre_existing_shape_present`] is stale and must be updated, not
/// silently trusted.
#[test]
fn load_routes_recursive_still_names_its_pre_existing_triggers() {
    let source = render_mock_server_binary();
    let file = syn::parse_file(&source).expect("generated mock-server binary must parse as Rust");
    let route_loading_fn = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(item_fn) if item_fn.sig.ident == "load_routes_recursive" => Some(item_fn),
            _ => None,
        })
        .expect("generated mock-server binary must define `load_routes_recursive`");
    let refs: HashSet<String> = literals_and_refs_in_block(&route_loading_fn.block)
        .refs
        .into_iter()
        .collect();
    for expected in [
        "has_intra_fixture_redirect",
        "has_inline_host_link",
        "is_host_root_path",
    ] {
        assert!(
            refs.contains(expected),
            "load_routes_recursive no longer references `{expected}` -- the pre-existing trigger \
             mirror in this test (`pre_existing_shape_present`) was written against that name and \
             must be updated to match whatever it was renamed to"
        );
    }
}

// ---------------------------------------------------------------------------
// Corpus and the two sides being compared
// ---------------------------------------------------------------------------

/// A single `mock_responses` route-array entry, matching the JSON shape both
/// `Fixture::has_host_root_route` and the generated binary's route loader consume.
fn entry(path: &str, status_code: u16, headers: serde_json::Value, body_inline: &str) -> serde_json::Value {
    serde_json::json!({"path": path, "status_code": status_code, "headers": headers, "body_inline": body_inline})
}

fn fixture_with_entries(id: &str, entries: Vec<serde_json::Value>) -> Fixture {
    let json = serde_json::json!({
        "id": id,
        "description": "corpus case",
        "input": {"mock_responses": entries},
        "assertions": []
    });
    serde_json::from_value(json).expect("corpus fixture must deserialize")
}

/// Mirrors `route_loading.rs`'s pre-existing (pre-issue-#433) trigger shapes: an origin-root
/// path prefix, a relative 3xx `Location`, a relative `Refresh` header, an HTML meta-refresh, or
/// an `href="/"` link. Kept deliberately simple and separate from the origin-token check this
/// test exists for; [`load_routes_recursive_still_names_its_pre_existing_triggers`] guards
/// against this mirror silently going stale.
fn pre_existing_shape_present(entries: &[serde_json::Value]) -> bool {
    const ORIGIN_ROOT_PREFIXES: [&str; 2] = ["/robots", "/sitemap"];
    entries.iter().any(|entry| {
        let path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let status = entry.get("status_code").and_then(|v| v.as_u64()).unwrap_or(0);
        let headers = entry.get("headers").and_then(|v| v.as_object());
        let body = entry.get("body_inline").and_then(|v| v.as_str()).unwrap_or("");

        let origin_root_path = ORIGIN_ROOT_PREFIXES.iter().any(|prefix| path.starts_with(prefix));
        let location_redirect = (300..400).contains(&status)
            && headers
                .map(|h| {
                    h.iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case("location") && value.as_str().is_some_and(|s| s.starts_with('/'))
                    })
                })
                .unwrap_or(false);
        let refresh_redirect = headers
            .map(|h| {
                h.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("refresh")
                        && value
                            .as_str()
                            .and_then(|s| s.to_ascii_lowercase().find("url=").map(|i| (s.to_owned(), i)))
                            .map(|(s, idx)| s[idx + 4..].trim_start().starts_with('/'))
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        let meta_refresh = body
            .to_ascii_lowercase()
            .split("http-equiv=\"refresh\"")
            .nth(1)
            .and_then(|s| s.split("content=").nth(1))
            .map(|s| s.trim_start_matches(['"', '\'']).contains("url=/"))
            .unwrap_or(false);
        let inline_host_link = body.contains("href=\"/") || body.contains("href='/");

        origin_root_path || location_redirect || refresh_redirect || meta_refresh || inline_host_link
    })
}

/// Ground truth for the corpus: the pre-existing trigger mirror, OR'd with the origin-token
/// clause -- but the origin-token half only counts once the real generated source has actually
/// been extended to read those tokens ([`real_predicate_covers_origin_tokens`]). Before that,
/// truthfully reporting `false` here for an origin-token corpus case is the point: it makes the
/// parity assertion below fail for exactly the fixtures the sibling predicate does not yet
/// route correctly.
fn real_side_truth(entries: &[serde_json::Value], origin_tokens_covered: bool) -> bool {
    let origin_token_present = entries
        .iter()
        .any(alef::e2e::fixture::origin_tokens::entry_has_origin_token);
    pre_existing_shape_present(entries) || (origin_tokens_covered && origin_token_present)
}

/// `(case id, entries, whether this case is about an origin-substitution token at all)`.
/// The third field lets the summary in [`has_host_root_route_agrees_with_route_loading_source`]
/// separate "still waiting on the sibling predicate" cases from a genuine new disagreement.
fn corpus() -> Vec<(&'static str, Vec<serde_json::Value>, bool)> {
    vec![
        (
            "plain_namespaced",
            vec![entry("/data.json", 200, serde_json::json!({}), "{}")],
            false,
        ),
        (
            "robots_txt_path",
            vec![entry(
                "/robots.txt",
                200,
                serde_json::json!({}),
                "User-agent: *\nDisallow: /",
            )],
            false,
        ),
        (
            "relative_location_redirect",
            vec![
                entry("/", 302, serde_json::json!({"Location": "/final"}), ""),
                entry("/final", 200, serde_json::json!({}), "{}"),
            ],
            false,
        ),
        (
            "relative_refresh_header",
            vec![entry("/", 200, serde_json::json!({"Refresh": "0;url=/next"}), "")],
            false,
        ),
        (
            "relative_meta_refresh",
            vec![entry(
                "/",
                200,
                serde_json::json!({}),
                "<meta http-equiv=\"refresh\" content=\"0;url=/next\">",
            )],
            false,
        ),
        (
            "inline_href_root_link",
            vec![entry("/", 200, serde_json::json!({}), "<a href=\"/page\">page</a>")],
            false,
        ),
        (
            "mock_origin_in_body",
            vec![entry(
                "/",
                200,
                serde_json::json!({}),
                "<a href=\"{{mock_origin}}/page2\">same host</a>",
            )],
            true,
        ),
        (
            "mock_alt_origin_in_body",
            vec![entry(
                "/",
                200,
                serde_json::json!({}),
                "<a href=\"{{mock_alt_origin}}/page2\">alt host</a>",
            )],
            true,
        ),
        (
            "mock_sub_origin_in_header",
            vec![entry(
                "/",
                302,
                serde_json::json!({"Location": "{{mock_sub_origin}}/final"}),
                "",
            )],
            true,
        ),
        (
            "mock_foreign_origin_in_body",
            vec![entry(
                "/",
                200,
                serde_json::json!({}),
                "<a href=\"{{mock_foreign_origin}}/page2\">external</a>",
            )],
            true,
        ),
        (
            "plain_body_no_triggers",
            vec![entry(
                "/",
                200,
                serde_json::json!({}),
                "<html><body>hello</body></html>",
            )],
            false,
        ),
        ("empty_mock_responses", vec![], false),
    ]
}

/// The parity gate: `Fixture::has_host_root_route` must agree with the real generated
/// `has_host_root` source on every corpus entry.
#[test]
fn has_host_root_route_agrees_with_route_loading_source() {
    let origin_tokens_covered = real_predicate_covers_origin_tokens();
    let mut mismatches = Vec::new();
    for (id, entries, is_origin_token_case) in corpus() {
        let fixture = fixture_with_entries(id, entries.clone());
        let mine = fixture.has_host_root_route();
        let real = real_side_truth(&entries, origin_tokens_covered);
        if mine != real {
            mismatches.push(format!(
                "  {id}: Fixture::has_host_root_route() = {mine}, route_loading.rs = {real} \
                 (origin-token case: {is_origin_token_case}, sibling predicate currently covers \
                 origin tokens: {origin_tokens_covered})"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "fixture.rs and route_loading.rs disagree on {} of {} corpus case(s):\n{}\n\n\
         If every mismatch is an origin-token case and `sibling predicate currently covers origin \
         tokens: false`, the mock-server binary's `has_host_root` has not yet been extended to \
         read {{mock_origin}}/{{mock_alt_origin}}/{{mock_sub_origin}}/{{mock_foreign_origin}} -- \
         see alef issue #433.",
        mismatches.len(),
        corpus().len(),
        mismatches.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Deliberate-failure proof for `Fixture::has_host_root_route`'s own origin-token clause.
//
// This does not touch `route_loading.rs` or its parsed source at all -- it proves the *fixture*
// side of the gate actually looks at `entry_has_origin_token`, by evaluating the predicate two
// ways: with the real function, and with it forced to `false` (as if the clause had never been
// added). See the `prove-the-check-fired` rule: a parity test that would pass with either
// implementation of `has_host_root_route` has not tested anything.
// ---------------------------------------------------------------------------

/// Recomputes `has_host_root_route`'s answer for a `mock_responses` array, but with the
/// origin-token clause forced off -- i.e., exactly what `Fixture::has_host_root_route` would
/// return if the `|| origin_tokens::entry_has_origin_token(entry)` disjunct in `fixture.rs` were
/// deleted. Mirrors the same four pre-existing trigger shapes as
/// [`pre_existing_shape_present`] so the only behavioral difference from the real function is
/// the clause under test.
fn has_host_root_route_without_origin_token_clause(entries: &[serde_json::Value]) -> bool {
    pre_existing_shape_present(entries)
}

#[test]
fn origin_token_clause_ablation_proves_the_gate_is_live() {
    let alt_origin_entries = vec![entry(
        "/",
        200,
        serde_json::json!({}),
        "<a href=\"{{mock_alt_origin}}/page2\">alt host</a>",
    )];
    let fixture = fixture_with_entries("ablation_alt_origin", alt_origin_entries.clone());

    // With the real clause present, `Fixture::has_host_root_route` must report true.
    assert!(
        fixture.has_host_root_route(),
        "Fixture::has_host_root_route() must be true for a body carrying {{mock_alt_origin}}"
    );

    // With the clause removed (this ablated recomputation, standing in for a deleted disjunct
    // in fixture.rs), the same fixture must report false -- proving the real clause, not
    // coincidence elsewhere in the function, is what makes the true case true.
    let ablated = has_host_root_route_without_origin_token_clause(&alt_origin_entries);
    assert!(
        !ablated,
        "ablated has_host_root_route (origin-token clause removed) unexpectedly still reports \
         true for a {{mock_alt_origin}} body -- some OTHER trigger in pre_existing_shape_present \
         is accidentally matching this fixture, which would make the real clause untestable"
    );

    // The two must disagree for this fixture: that disagreement IS the parity gate's failure
    // mode when a real clause is missing on one side, demonstrated here without needing
    // `route_loading.rs` to have landed the same clause yet.
    assert_ne!(
        fixture.has_host_root_route(),
        ablated,
        "real vs. ablated has_host_root_route must disagree on a {{mock_alt_origin}} body, or \
         this ablation proves nothing about the clause it targets"
    );
}
