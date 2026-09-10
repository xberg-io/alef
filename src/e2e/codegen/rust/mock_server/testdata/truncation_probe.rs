#[cfg(test)]
mod truncation_probe {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const BODY: &[u8] = b"partial response";
    const DECLARED_LENGTH: usize = 1000;

    fn route(length: Option<usize>) -> MockRoute {
        MockRoute {
            status: 200,
            body: BODY.to_vec(),
            is_streaming: false,
            stream_chunks: Vec::new(),
            headers: length
                .map(|value| vec![("Content-Length".to_string(), value.to_string())])
                .unwrap_or_default(),
            delay_ms: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn partial_body_is_sent_before_connection_closes() {
        let routes = Arc::new(HashMap::from([
            ("/truncated".to_string(), route(Some(DECLARED_LENGTH))),
            ("/exact".to_string(), route(Some(BODY.len()))),
            ("/implicit".to_string(), route(None)),
        ]));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind probe");
        let address = listener.local_addr().expect("probe address");
        let app = Router::new().fallback(handle_request).with_state(routes);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve probe") });
        verify_responses(address).await;
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn per_test_server_sends_partial_body_before_connection_closes() {
        let routes = [
            ("/truncated", Some(DECLARED_LENGTH)),
            ("/exact", Some(BODY.len())),
            ("/implicit", None),
        ]
        .into_iter()
        .map(|(path, length)| per_test::MockRoute {
            path,
            method: "GET",
            status: 200,
            body: String::from_utf8(BODY.to_vec()).expect("text probe"),
            is_streaming: false,
            stream_chunks: Vec::new(),
            headers: route(length).headers,
            delay_ms: None,
        })
        .collect();
        let server = per_test::MockServer::start(routes).await;
        let address = server
            .url
            .strip_prefix("http://")
            .expect("HTTP probe")
            .parse()
            .expect("socket address");
        verify_responses(address).await;
    }

    async fn verify_responses(address: SocketAddr) {
        let client = reqwest::Client::new();
        for path in ["exact", "implicit"] {
            let response = client
                .get(format!("http://{address}/{path}"))
                .send()
                .await
                .expect("normal response");
            assert_eq!(response.bytes().await.expect("normal body").as_ref(), BODY);
        }
        verify_wire(address).await;
        let response = client
            .get(format!("http://{address}/truncated"))
            .send()
            .await
            .expect("headers arrive");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.content_length(), Some(DECLARED_LENGTH as u64));
        assert!(response.bytes().await.expect_err("body must be truncated").is_decode());
        verify_node(address).await;
    }

    async fn verify_wire(address: SocketAddr) {
        let mut connection = TcpStream::connect(address).await.expect("connect wire probe");
        connection
            .write_all(b"GET /truncated HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("send wire request");
        let mut wire = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), connection.read_to_end(&mut wire))
            .await
            .expect("response closes")
            .expect("read wire bytes");
        let delimiter = wire
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .expect("HTTP headers must reach the client");
        let headers = String::from_utf8_lossy(&wire[..delimiter]).to_ascii_lowercase();
        assert!(headers.starts_with("http/1.1 200"), "{headers}");
        assert!(
            headers.contains(&format!("content-length: {DECLARED_LENGTH}")),
            "{headers}"
        );
        assert!(!headers.contains("transfer-encoding:"), "{headers}");
        assert_eq!(&wire[delimiter + 4..], BODY);
    }

    async fn verify_node(address: SocketAddr) {
        if node_is_missing().await {
            eprintln!("Node body probe skipped: Node runtime unavailable");
            return;
        }
        let script = r#"
const assert = require('node:assert/strict');
(async () => {
  const response = await fetch(process.argv[1]);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('content-length'), '1000');
  let partial = ''; let failure;
  try { for await (const bytes of response.body) partial += Buffer.from(bytes).toString(); }
  catch (error) { failure = error; }
  assert.equal(partial, 'partial response');
  assert.ok(failure, 'body reader must reject a truncated response');
})().catch(error => { console.error(error); process.exitCode = 1; });
"#;
        let output = tokio::process::Command::new("node")
            .args(["-e", script, &format!("http://{address}/truncated")])
            .output()
            .await
            .expect("run Node body probe");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    async fn node_is_missing() -> bool {
        match tokio::process::Command::new("node").arg("--version").output().await {
            Ok(output) => !output.status.success(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => panic!("check Node runtime: {error}"),
        }
    }
}
