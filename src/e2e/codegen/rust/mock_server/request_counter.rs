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
//   GET  /__alef/requests/body?key=<k>&field=<path>&op=<equals|not_equals>&value=<json>
//                                                -> "1" or "0", text/plain; or a sentence
//                                                   naming why the bodies cannot be checked
//   POST /__alef/requests/reset?prefix=<p>       -> 204, no body
//
// The body endpoint answers whether the JSON bodies of the requests recorded for `<k>`
// (`<METHOD> <path>`, or a bare `<path>` for any method) hold `<json>` at the dotted
// `<path>`. It answers 1 only when at least one request matched `<k>`. When any matching
// request has a body that is not valid JSON, or one the server did not store, no value can
// be checked: the answer is a sentence that says so, never 1 and never 0. Every language
// reads the answer as an integer, so the sentence fails the assertion with that reason.

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

/// Largest request body the server records, in bytes. A larger body (an upload) is counted
/// but not stored, so a suite that sends big files does not hold them in memory.
const MAX_RECORDED_BODY_BYTES: usize = 1024 * 1024;

/// A request body as the server stored it, or the reason it stored none.
type RecordedBody = Result<Vec<u8>, &'static str>;

/// Request bodies in arrival order, as `(method, path, body)`.
static REQUEST_BODIES: std::sync::OnceLock<std::sync::Mutex<Vec<(String, String, RecordedBody)>>> =
    std::sync::OnceLock::new();

fn request_bodies() -> &'static std::sync::Mutex<Vec<(String, String, RecordedBody)>> {
    REQUEST_BODIES.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Read a request body up to `MAX_RECORDED_BODY_BYTES`. A longer or unreadable body is not
/// stored: the result holds the reason instead.
async fn read_request_body(body: Body) -> RecordedBody {
    axum::body::to_bytes(body, MAX_RECORDED_BODY_BYTES)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|_| "request body was not checked: it is larger than 1048576 bytes or could not be read")
}

/// Record one request for `(method, path)`. Callers must invoke this for every
/// non-control request, before route lookup, so redirects and 404s are counted too.
fn record_request(method: &str, path: &str, body: RecordedBody) {
    let mut counts = request_counts().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *counts.entry((method.to_string(), path.to_string())).or_insert(0) += 1;
    drop(counts);
    request_bodies()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((method.to_string(), path.to_string(), body));
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
    drop(counts);
    request_bodies()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|(_, path, _)| !path.starts_with(prefix));
}

/// Follow a dotted path through a JSON value. A segment names an object key, or an array
/// index when the value is an array and the segment is a number.
fn json_at_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(segment)?,
            serde_json::Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// The value of an integer JSON number, or `None` for a number written with a fraction or exponent.
fn exact_integer(number: &serde_json::Number) -> Option<i128> {
    number.as_i64().map(i128::from).or_else(|| number.as_u64().map(i128::from))
}

/// Number equality that treats `8192` and `8192.0` as the same number. Two integers compare as
/// integers, so `9007199254740992` and `9007199254740993` stay different.
fn json_numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    match (exact_integer(a), exact_integer(b)) {
        (Some(x), Some(y)) => x == y,
        (None, None) => a.as_f64() == b.as_f64(),
        (Some(integer), None) | (None, Some(integer)) => {
            let float = if exact_integer(a).is_some() { b } else { a };
            float
                .as_f64()
                .is_some_and(|f| f.fract() == 0.0 && f.abs() < 1e18 && f as i128 == integer)
        }
    }
}

/// JSON equality that treats `8192` and `8192.0` as the same number.
fn json_values_equal(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match (actual, expected) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => json_numbers_equal(a, b),
        _ => actual == expected,
    }
}

/// Whether the bodies of the requests recorded for `key` hold `expected_json` at `field`.
///
/// `equals` needs at least one matching request and every one of them to hold the value.
/// `not_equals` needs at least one matching request and none of them to hold it; a body that
/// lacks the field does not hold it. An unknown `op` or unparsable `expected_json` answers
/// false.
///
/// A matching request whose body is not valid JSON (plain text, empty, compressed), or whose
/// body the server did not store, cannot be checked. The answer is then `Err` with the reason,
/// for `equals` and `not_equals` alike: a body that was not read never satisfies an assertion.
fn request_bodies_match(key: &str, field: &str, op: &str, expected_json: &str) -> Result<bool, String> {
    let Ok(expected) = serde_json::from_str::<serde_json::Value>(expected_json) else {
        return Ok(false);
    };
    let (method, path) = match key.split_once(' ') {
        Some((method, path)) => (Some(method), path),
        None => (None, key),
    };
    let bodies = request_bodies().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut matched = 0usize;
    let mut holding = 0usize;
    for (request_method, request_path, body) in bodies.iter() {
        if request_path != path || method.is_some_and(|m| m != request_method) {
            continue;
        }
        matched += 1;
        let bytes = body.as_ref().map_err(|reason| (*reason).to_string())?;
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            return Err(format!(
                "request body was not valid JSON: {request_method} {request_path} sent {} bytes that do not parse",
                bytes.len()
            ));
        };
        if json_at_path(&json, field).is_some_and(|actual| json_values_equal(actual, &expected)) {
            holding += 1;
        }
    }
    Ok(match op {
        "equals" => matched > 0 && holding == matched,
        "not_equals" => matched > 0 && holding == 0,
        _ => false,
    })
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
        ("GET", "/__alef/requests/body") => {
            let key = query_param(query, "key").unwrap_or_default();
            let field = query_param(query, "field").unwrap_or_default();
            let op = query_param(query, "op").unwrap_or_default();
            let value = query_param(query, "value").unwrap_or_default();
            let answer = match request_bodies_match(&key, &field, &op, &value) {
                Ok(holds) => u8::from(holds).to_string(),
                Err(reason) => reason,
            };
            Some(plain_text_response(StatusCode::OK, answer))
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
