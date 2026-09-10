//! Shared response-body source for both generated Rust mock servers.

pub(super) const RESPONSE_BODY_SOURCE: &str = r#"fn response_body_with_headers(headers: &[(String, String)], bytes: Vec<u8>) -> Body {
    let declared_length = headers.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<u64>().ok());
    if !declared_length.is_some_and(|length| length > bytes.len() as u64) {
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

"#;
