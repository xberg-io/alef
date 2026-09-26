//! Route-loading source fragment for the generated standalone mock-server binary.

const ROUTE_LOADING_SOURCE: &str = r####"// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

/// The four origin-substitution placeholders `Origins::token_pairs` resolves at serve time (see
/// origins.rs). A fixture whose body or header values carry one needs a dedicated per-fixture
/// listener for the same reason as the origin-root triggers below: the client is told to follow a
/// full URL, and the shared `/fixtures/<id>`-namespaced table has no route for the path resolved
/// against it. ~keep
const ORIGIN_SUBSTITUTION_TOKENS: [&str; 4] = [
    "{{mock_origin}}",
    "{{mock_alt_origin}}",
    "{{mock_sub_origin}}",
    "{{mock_foreign_origin}}",
];

/// True if `value` contains one of [`ORIGIN_SUBSTITUTION_TOKENS`] verbatim.
fn contains_origin_substitution_token(value: &str) -> bool {
    ORIGIN_SUBSTITUTION_TOKENS.iter().any(|token| value.contains(token))
}

/// Intermediate fixture-loading result: shared route table plus per-fixture origin-root data.
struct LoadedRoutes {
    /// Routes namespaced under /fixtures/<id> for the shared listener.
    shared: HashMap<String, MockRoute>,
    /// For each fixture that has origin-root routes: fixture_id → route table at origin root.
    per_fixture: HashMap<String, HashMap<String, MockRoute>>,
}

fn load_routes(fixtures_dir: &Path) -> LoadedRoutes {
    let mut shared = HashMap::new();
    let mut per_fixture: HashMap<String, HashMap<String, MockRoute>> = HashMap::new();
    load_routes_recursive(fixtures_dir, fixtures_dir, &mut shared, &mut per_fixture);
    LoadedRoutes { shared, per_fixture }
}

fn load_routes_recursive(
    dir: &Path,
    fixtures_root: &Path,
    shared: &mut HashMap<String, MockRoute>,
    per_fixture: &mut HashMap<String, HashMap<String, MockRoute>>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("warning: cannot read directory {}: {err}", dir.display());
            return;
        }
    };

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            load_routes_recursive(&path, fixtures_root, shared, per_fixture);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("warning: cannot read {}: {err}", path.display());
                    continue;
                }
            };
            let fixtures: Vec<Fixture> = if content.trim_start().starts_with('[') {
                match serde_json::from_str(&content) {
                    Ok(v) => v,
                    Err(err) => {
                        eprintln!("warning: cannot parse {}: {err}", path.display());
                        continue;
                    }
                }
            } else {
                match serde_json::from_str::<Fixture>(&content) {
                    Ok(f) => vec![f],
                    Err(err) => {
                        eprintln!("warning: cannot parse {}: {err}", path.display());
                        continue;
                    }
                }
            };

            for fixture in fixtures {
                let resolved_routes = fixture.as_routes(fixtures_root);
                // A fixture needs origin-root routing if it serves an origin-root discovery path,
                // or if it returns content that makes the client under test request another
                // origin-root path from the same fixture.
                let has_intra_fixture_redirect = resolved_routes.iter().any(|r| {
                    // 3xx with relative Location header
                    let location_redirect = (300..400).contains(&r.response.status)
                        && r.response.headers.iter().any(|(name, value)| {
                            name.eq_ignore_ascii_case("location") && value.starts_with('/')
                        });
                    // Refresh header with url=/...
                    let refresh_redirect = r.response.headers.iter().any(|(name, value)| {
                        if !name.eq_ignore_ascii_case("refresh") {
                            return false;
                        }
                        let lower = value.to_ascii_lowercase();
                        lower
                            .find("url=")
                            .map(|idx| value[idx + 4..].trim_start().starts_with('/'))
                            .unwrap_or(false)
                    });
                    // HTML meta-refresh tag pointing to /...
                    let body_lower_lossy = String::from_utf8_lossy(&r.body_bytes).to_ascii_lowercase();
                    let meta_refresh = body_lower_lossy
                        .split("http-equiv=\"refresh\"")
                        .nth(1)
                        .and_then(|s| s.split("content=").nth(1))
                        .map(|s| {
                            let trimmed = s.trim_start_matches(['"', '\'']);
                            trimmed.contains("url=/")
                        })
                        .unwrap_or(false);
                    location_redirect || refresh_redirect || meta_refresh
                });
                // Inline HTML anchors that target host-absolute paths (`<a href="/page1">`)
                // also require a dedicated listener because clients resolve these against the
                // URL host, not the `/fixtures/<id>/` namespace.
                let has_inline_host_link = resolved_routes.iter().any(|r| {
                    let body_lossy = String::from_utf8_lossy(&r.body_bytes);
                    body_lossy.contains("href=\"/") || body_lossy.contains("href='/")
                });
                // A body or header value carrying an origin-substitution token (see
                // ORIGIN_SUBSTITUTION_TOKENS above) resolves to a host-absolute URL, so it needs
                // the same dedicated listener as an intra-fixture redirect or inline host link.
                let has_origin_substitution_token = resolved_routes.iter().any(|r| {
                    let body_lossy = String::from_utf8_lossy(&r.body_bytes);
                    contains_origin_substitution_token(&body_lossy)
                        || r.response
                            .headers
                            .iter()
                            .any(|(_, value)| contains_origin_substitution_token(value))
                });
                let has_host_root = has_intra_fixture_redirect
                    || has_inline_host_link
                    || has_origin_substitution_token
                    || resolved_routes.iter().any(|r| is_host_root_path(&r.original_path));

                for resolved in resolved_routes {
                    let is_streaming = resolved.response.stream_chunks.is_some();
                    let stream_chunks = resolved.response
                        .stream_chunks
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| match c {
                            serde_json::Value::String(s) => s,
                            other => serde_json::to_string(&other).unwrap_or_default(),
                        })
                        .collect();
                    let mut headers: Vec<(String, String)> = resolved.response.headers.into_iter().collect();
                    headers.sort_by(|a, b| a.0.cmp(&b.0));

                    let mock_route = MockRoute {
                        status: resolved.response.status,
                        body: resolved.body_bytes,
                        is_streaming,
                        stream_chunks,
                        headers,
                        delay_ms: resolved.response.delay_ms,
                    };

                    // Insert into the shared namespaced table, but skip origin-root paths
                    // (`/robots*`, `/sitemap*`) because those collide across fixtures and the
                    // last-write-wins behavior makes test results depend on fixture-load
                    // order. Origin-root routes are served only by the dedicated per-fixture
                    // listener spawned below for fixtures that declare them.
                    if !is_host_root_path(&resolved.original_path) {
                        shared.insert(resolved.path.clone(), mock_route.clone());
                    }

                    // For fixtures with origin-root routes, also build a per-fixture table
                    // where routes are mounted at their original (un-namespaced) paths.
                    if has_host_root {
                        per_fixture
                            .entry(fixture.id.clone())
                            .or_default()
                            .insert(resolved.original_path.clone(), mock_route);
                    }
                }
            }
        }
    }
}

"####;

pub(super) fn render_route_loading_source() -> &'static str {
    ROUTE_LOADING_SOURCE
}
