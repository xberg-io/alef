//! Shared response-body source for both generated Rust mock servers.

pub(super) const RESPONSE_BODY_SOURCE: &str = r#"fn response_body_with_headers(headers: &[(String, String)], bytes: Vec<u8>) -> Body {
    let declared_length = headers.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<u64>().ok());
    if declared_length.is_none_or(|length| length <= bytes.len() as u64) {
        return Body::from(bytes);
    }

    // Exact-sized bodies make Hyper reject intentional Content-Length mismatches
    // before writing headers. Stream the bytes with an unknown size, then yield
    // before terminating so the client observes a truncated body, not a connection
    // failure before the response. The delay begins only after the bytes are polled.
    use tokio_stream::StreamExt;
    const TRUNCATED_BODY_CLOSE_DELAY: std::time::Duration = std::time::Duration::from_millis(50);
    let partial = tokio_stream::once(Ok::<_, std::io::Error>(bytes));
    let termination = tokio_stream::once(()).then(|()| async {
        tokio::time::sleep(TRUNCATED_BODY_CLOSE_DELAY).await;
        Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "fixture response body truncated"))
    });
    Body::from_stream(partial.chain(termination))
}

/// Whether a streaming route declares a newline-delimited JSON body through its content-type
/// header (as Ollama's `/api/chat` sends). Such a stream has no `data:` prefix and no `[DONE]`.
fn headers_declare_ndjson(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("content-type") && value.to_ascii_lowercase().contains("ndjson"))
}

/// One stream event as the wire frames it: `data: <chunk>` plus a blank line for SSE, or the
/// chunk plus one newline for NDJSON.
fn frame_stream_event(chunk: &str, ndjson: bool) -> String {
    if ndjson { format!("{chunk}\n") } else { format!("data: {chunk}\n\n") }
}

/// SSE body that waits `delay_ms` before each chunk, then ends with `data: [DONE]`.
/// The server writes each event as soon as its delay ends, so a client sees the stream
/// arrive over time instead of in one write. The task stops when the client disconnects.
fn paced_sse_body(chunks: Vec<String>, delay_ms: u64, ndjson: bool) -> Body {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::convert::Infallible>>(1);
    tokio::spawn(async move {
        for chunk in chunks {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            if sender.send(Ok(frame_stream_event(&chunk, ndjson).into_bytes())).await.is_err() {
                return;
            }
        }
        if !ndjson {
            let _ = sender.send(Ok(b"data: [DONE]\n\n".to_vec())).await;
        }
    });
    Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver))
}

"#;
