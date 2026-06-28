//! Runtime server source fragment for the generated standalone mock-server binary.

const RUNTIME_SERVER_SOURCE: &str = r####"// ---------------------------------------------------------------------------
// Route table
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct MockRoute {
    status: u16,
    body: Vec<u8>,
    /// Whether this route serves a streaming SSE response.
    /// True even when `stream_chunks` is empty (e.g. an empty stream still sends
    /// `data: [DONE]\n\n` with the correct `content-type: text/event-stream` header).
    is_streaming: bool,
    stream_chunks: Vec<String>,
    headers: Vec<(String, String)>,
    /// Optional artificial delay applied before the handler returns. When set,
    /// the handler `tokio::time::sleep`s for this many milliseconds before
    /// constructing the response — used by timeout-error fixtures to force
    /// client-side request timeouts.
    delay_ms: Option<u64>,
}

type RouteTable = Arc<HashMap<String, MockRoute>>;

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_request(State(routes): State<RouteTable>, req: Request<Body>) -> Response {
    let path = req.uri().path().to_owned();

    // Try exact match first
    if let Some(route) = routes.get(&path) {
        if let Some(delay_ms) = route.delay_ms {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        return serve_route(route);
    }

    // Try prefix match: find a route that is a prefix of the request path
    // This allows /fixtures/basic_chat/v1/chat/completions to match /fixtures/basic_chat
    for (route_path, route) in routes.iter() {
        if path.starts_with(route_path) && (path.len() == route_path.len() || path.as_bytes()[route_path.len()] == b'/') {
            if let Some(delay_ms) = route.delay_ms {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            return serve_route(route);
        }
    }

    // Fall back to serving a raw test document from the docs dir, so HTTP URL
    // fixtures that reference relative file paths resolve against test_documents.
    if let Some(response) = serve_test_document(&path) {
        return response;
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!("No mock route for {path}")))
        .unwrap()
        .into_response()
}

fn serve_route(route: &MockRoute) -> Response {
    let status = StatusCode::from_u16(route.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    if route.is_streaming {
        // Note: stream_chunks may be empty for an empty-stream fixture; we still emit
        // `data: [DONE]\n\n` with the correct SSE headers so clients do not hang.
        let mut sse = String::new();
        for chunk in &route.stream_chunks {
            sse.push_str("data: ");
            sse.push_str(chunk);
            sse.push_str("\n\n");
        }
        sse.push_str("data: [DONE]\n\n");

        let mut builder = Response::builder()
            .status(status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache");
        for (name, value) in &route.headers {
            builder = builder.header(name, value);
        }
        return builder.body(Body::from(sse)).unwrap().into_response();
    }

    // Only set the default content-type if the fixture does not override it.
    // Inspect the first non-whitespace byte to detect JSON vs binary vs plain text.
    let has_content_type = route.headers.iter().any(|(k, _)| k.to_lowercase() == "content-type");
    let mut builder = Response::builder().status(status);
    if !has_content_type {
        let first_nonws = route.body.iter().find(|&&b| b != b' ' && b != b'\t' && b != b'\n' && b != b'\r');
        let default_ct = match first_nonws {
            Some(&b'{') | Some(&b'[') => "application/json",
            _ => "text/plain",
        };
        builder = builder.header("content-type", default_ct);
    }
    for (name, value) in &route.headers {
        // Skip content-encoding headers — the mock server returns uncompressed bodies.
        // Sending a content-encoding without actually encoding the body would cause
        // clients to fail decompression.
        if name.to_lowercase() == "content-encoding" {
            continue;
        }
        // The <<absent>> sentinel means this header must NOT be present in the
        // real server response — do not emit it from the mock server either.
        if value == "<<absent>>" {
            continue;
        }
        // Replace the <<uuid>> sentinel with a real UUID v4 so clients can
        // assert the header value matches the UUID pattern.
        if value == "<<uuid>>" {
            let uuid = format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                rand_u32(),
                rand_u16(),
                rand_u16() & 0x0fff,
                (rand_u16() & 0x3fff) | 0x8000,
                rand_u48(),
            );
            builder = builder.header(name, uuid);
            continue;
        }
        builder = builder.header(name, value);
    }
    builder.body(Body::from(route.body.clone())).unwrap().into_response()
}

/// Generate a pseudo-random u32 using the current time nanoseconds.
fn rand_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    ns ^ (ns.wrapping_shl(13)) ^ (ns.wrapping_shr(17))
}

fn rand_u16() -> u16 {
    (rand_u32() & 0xffff) as u16
}

fn rand_u48() -> u64 {
    ((rand_u32() as u64) << 16) | (rand_u16() as u64)
}

"####;

pub(super) fn render_runtime_server_source() -> &'static str {
    RUNTIME_SERVER_SOURCE
}
