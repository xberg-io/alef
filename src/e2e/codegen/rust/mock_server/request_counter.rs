//! Request-counter source fragment shared by both generated Rust mock servers
//! (the in-process `MockServer` in `server_module.rs` and the standalone binary's
//! own server loop in `runtime_server.rs`/`binary.rs`).
//!
//! Every type here is fully qualified (`std::sync::Mutex`, not `Mutex`) so this
//! fragment can be spliced into either generated file without also having to keep
//! that file's `use` list in sync.

const REQUEST_COUNTER_SOURCE: &str = r####"// ---------------------------------------------------------------------------
// Request counter
// ---------------------------------------------------------------------------
//
// Exposes three control endpoints so e2e suites can assert how many requests a
// fixture actually received, without pulling a JSON parser into 21 language
// harnesses:
//   GET  /__alef/requests/total?prefix=<p>       -> decimal count, text/plain
//   GET  /__alef/requests/one?key=<METHOD>%20<p> -> decimal count, text/plain
//   POST /__alef/requests/reset?prefix=<p>       -> 204, no body
//
// Control requests are matched and answered BEFORE route lookup, and are never
// counted themselves.

/// Counts are keyed on the REQUEST path as received, not the matched route's
/// registered (possibly-prefix) path -- both servers here prefix-match routes, so
/// keying on the route path would collapse e.g. `/fixtures/x/a` and `/fixtures/x/b`
/// into a single bucket and make per-path counts meaningless.
static REQUEST_COUNTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, String), u64>>> =
    std::sync::OnceLock::new();

fn request_counts() -> &'static std::sync::Mutex<std::collections::HashMap<(String, String), u64>> {
    REQUEST_COUNTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Record one request for `(method, path)`. Callers must invoke this for every
/// non-control request, before route lookup, so redirects and 404s are counted too.
fn record_request(method: &str, path: &str) {
    let mut counts = request_counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *counts.entry((method.to_string(), path.to_string())).or_insert(0) += 1;
}

/// Sum of counts across every `(method, path)` key whose path starts with `prefix`.
/// An empty prefix (the parameter was absent) matches every key.
fn total_requests_for_prefix(prefix: &str) -> u64 {
    let counts = request_counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    counts.iter().filter(|((_, path), _)| path.starts_with(prefix)).map(|(_, count)| *count).sum()
}

/// Count for one exact `"<METHOD> <path>"` key. Returns 0 for a malformed or unseen key.
fn count_for_key(key: &str) -> u64 {
    let Some((method, path)) = key.split_once(' ') else {
        return 0;
    };
    let counts = request_counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    counts.get(&(method.to_string(), path.to_string())).copied().unwrap_or(0)
}

/// Remove every counted key whose path starts with `prefix`. An empty prefix clears
/// every key, matching `total_requests_for_prefix`'s empty-prefix behavior.
fn reset_requests_for_prefix(prefix: &str) {
    let mut counts = request_counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    counts.retain(|(_, path), _| !path.starts_with(prefix));
}

/// Minimal `application/x-www-form-urlencoded`-style percent-decoder for query values.
/// Query strings here only ever carry a plain path (`prefix=/fixtures/x`) or a
/// `"<METHOD> <path>"` pair (`key=GET%20/fixtures/x`), so this only needs to handle
/// `%XX` escapes and `+` for space -- not full RFC 3986 generality.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(value) => {
                        out.push(value);
                        index += 3;
                    }
                    None => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Look up a single query-string parameter by name, percent-decoding its value.
fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next()?;
        if key == name {
            return Some(percent_decode(parts.next().unwrap_or("")));
        }
    }
    None
}

/// Build a `text/plain` response carrying a bare decimal integer -- deliberately not
/// JSON, so every one of 21 language e2e harnesses can read a counter with "GET a URL,
/// trim, parse an int" instead of pulling in a JSON parser for one number.
fn plain_text_response(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Body::from(body))
        .unwrap()
        .into_response()
}

/// Handle a control-plane request under `/__alef/requests/*`. Returns `None` for any
/// other method/path so the caller falls through to normal route handling.
fn handle_request_counter_control(method: &str, path: &str, query: Option<&str>) -> Option<Response> {
    match (method, path) {
        ("GET", "/__alef/requests/total") => {
            let prefix = query_param(query, "prefix").unwrap_or_default();
            Some(plain_text_response(StatusCode::OK, total_requests_for_prefix(&prefix).to_string()))
        }
        ("GET", "/__alef/requests/one") => {
            let key = query_param(query, "key").unwrap_or_default();
            Some(plain_text_response(StatusCode::OK, count_for_key(&key).to_string()))
        }
        ("POST", "/__alef/requests/reset") => {
            let prefix = query_param(query, "prefix").unwrap_or_default();
            reset_requests_for_prefix(&prefix);
            Some(Response::builder().status(StatusCode::NO_CONTENT).body(Body::empty()).unwrap().into_response())
        }
        _ => None,
    }
}

"####;

pub(super) fn render_request_counter_source() -> &'static str {
    REQUEST_COUNTER_SOURCE
}
