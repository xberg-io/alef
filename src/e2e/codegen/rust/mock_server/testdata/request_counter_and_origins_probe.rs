#[cfg(test)]
mod request_counter_and_origins_probe {
    use super::*;

    // -----------------------------------------------------------------------
    // Deterministic, offline-safe unit tests for the loopback probe and the
    // poison-origin branch it feeds. These use IP literals (never a real DNS
    // lookup: `ToSocketAddrs` parses a literal address directly) so the test
    // does not depend on the sandbox's network access or the platform's
    // `*.localhost` wildcard support -- exactly the thing the real probe exists
    // to detect variance in.
    // -----------------------------------------------------------------------

    #[test]
    fn sub_host_resolves_to_loopback_is_true_for_a_loopback_ip_literal() {
        assert!(sub_host_resolves_to_loopback("127.0.0.1"));
    }

    #[test]
    fn sub_host_resolves_to_loopback_is_false_for_a_non_loopback_ip_literal() {
        // 203.0.113.0/24 is RFC 5737 TEST-NET-3: guaranteed non-loopback and never assigned.
        assert!(!sub_host_resolves_to_loopback("203.0.113.5"));
    }

    #[test]
    fn sub_origin_or_poison_uses_the_real_sub_host_when_it_resolves_to_loopback() {
        assert_eq!(
            sub_origin_or_poison("sub.localhost", 4242, true),
            "http://sub.localhost:4242"
        );
    }

    #[test]
    fn sub_origin_or_poison_uses_a_poison_host_when_it_does_not_resolve_to_loopback() {
        assert_eq!(
            sub_origin_or_poison("sub.localhost", 4242, false),
            "http://alef-unresolvable-alt-host.invalid:4242"
        );
    }

    // -----------------------------------------------------------------------
    // The standalone binary's server: request counter.
    // -----------------------------------------------------------------------

    fn binary_route(body: &str) -> MockRoute {
        MockRoute {
            status: 200,
            body: body.as_bytes().to_vec(),
            is_streaming: false,
            stream_chunks: Vec::new(),
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }
    }

    async fn spawn_binary_server(routes: std::collections::HashMap<String, MockRoute>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind probe");
        let address = listener.local_addr().expect("probe address");
        let state = AppState {
            routes: Arc::new(routes),
            origins: resolve_origins(address.port()),
        };
        let app = Router::new().fallback(handle_request).with_state(state);
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve probe") });
        address
    }

    async fn control_total(client: &reqwest::Client, base: &str, prefix: &str) -> String {
        client
            .get(format!("{base}/__alef/requests/total?prefix={prefix}"))
            .send()
            .await
            .expect("total request")
            .text()
            .await
            .expect("total body")
    }

    async fn control_one(client: &reqwest::Client, base: &str, key: &str) -> String {
        client
            .get(format!("{base}/__alef/requests/one?key={key}"))
            .send()
            .await
            .expect("one request")
            .text()
            .await
            .expect("one body")
    }

    /// Keying on the REQUEST path (not a shared registered route path) must keep
    /// `/fixtures/x/a` and `/fixtures/x/b` independent, and control endpoints -- checked
    /// before route lookup -- must never count themselves.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn counter_keys_on_request_path_and_excludes_control_requests() {
        let routes = std::collections::HashMap::from([
            ("/fixtures/x/a".to_string(), binary_route("a")),
            ("/fixtures/x/b".to_string(), binary_route("b")),
        ]);
        let address = spawn_binary_server(routes).await;
        let base = format!("http://{address}");
        let client = reqwest::Client::new();

        let total = control_total(&client, &base, "/fixtures/x").await;
        assert_eq!(total, "0", "a never-hit prefix must count zero");

        for _ in 0..2 {
            client.get(format!("{base}/fixtures/x/a")).send().await.expect("hit a");
        }
        client.get(format!("{base}/fixtures/x/b")).send().await.expect("hit b");

        let one_a = control_one(&client, &base, "GET%20/fixtures/x/a").await;
        assert_eq!(one_a, "2", "got:\n{one_a}");
        let one_b = control_one(&client, &base, "GET%20/fixtures/x/b").await;
        assert_eq!(one_b, "1", "got:\n{one_b}");
        let total_prefix = control_total(&client, &base, "/fixtures/x").await;
        assert_eq!(total_prefix, "3", "got:\n{total_prefix}");

        // Hit the control endpoint several times, then assert its own key never accumulated.
        for _ in 0..5 {
            control_total(&client, &base, "/fixtures/x").await;
        }
        let control_self_count = control_one(&client, &base, "GET%20/__alef/requests/total").await;
        assert_eq!(control_self_count, "0", "control endpoints must never count themselves");

        // Reset a single fixture's prefix and confirm only that prefix cleared.
        let reset_status = client
            .post(format!("{base}/__alef/requests/reset?prefix=/fixtures/x/a"))
            .send()
            .await
            .expect("reset request")
            .status();
        assert_eq!(reset_status.as_u16(), 204);

        let total_after_reset = control_total(&client, &base, "/fixtures/x").await;
        assert_eq!(
            total_after_reset, "1",
            "only /fixtures/x/a's count should have cleared, got:\n{total_after_reset}"
        );
    }

    // -----------------------------------------------------------------------
    // The standalone binary's server: origin substitution reaches both the body
    // and header values.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn origin_tokens_are_substituted_in_body_and_headers() {
        let mut route = binary_route("origin={{mock_origin}} alt={{mock_alt_origin}} foreign={{mock_foreign_origin}}");
        route.headers = vec![("location".to_string(), "{{mock_alt_origin}}/next".to_string())];
        let routes = std::collections::HashMap::from([("/fixtures/tokens".to_string(), route)]);
        let address = spawn_binary_server(routes).await;

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client");
        let response = client
            .get(format!("http://{address}/fixtures/tokens"))
            .send()
            .await
            .expect("token substitution request");

        let expected_location = format!("http://localhost:{}/next", address.port());
        assert_eq!(
            response
                .headers()
                .get("location")
                .expect("location header present")
                .to_str()
                .expect("ascii header"),
            expected_location,
            "header-value substitution must reach Location, not only the body"
        );

        let body = response.text().await.expect("token substitution body");
        assert_eq!(
            body,
            format!(
                "origin=http://127.0.0.1:{port} alt=http://localhost:{port} foreign=http://127.0.0.2:{port}",
                port = address.port()
            ),
            "got:\n{body}"
        );
    }

    // -----------------------------------------------------------------------
    // #442: a non-default `[crates.e2e] alt_host` (threaded to the server as
    // `ALEF_MOCK_ALT_HOST` by every harness spawn site) must actually change
    // `{{mock_alt_origin}}`/`{{mock_sub_origin}}` substitution, not just validate
    // cleanly and get silently ignored. The outer harness invocation runs this
    // probe binary with `--test-threads=1`, so mutating the process-global env
    // var here is not racy against other tests.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_configured_alef_mock_alt_host_reaches_the_substituted_origin() {
        // SAFETY: `--test-threads=1` (see the outer harness invocation) means no other
        // test in this probe binary observes this mutation concurrently.
        unsafe {
            std::env::set_var("ALEF_MOCK_ALT_HOST", "alef-alt-host-probe.example");
        }
        let route = binary_route("alt={{mock_alt_origin}} sub={{mock_sub_origin}}");
        let routes = std::collections::HashMap::from([("/fixtures/alt-host".to_string(), route)]);
        let address = spawn_binary_server(routes).await;
        // SAFETY: see above.
        unsafe {
            std::env::remove_var("ALEF_MOCK_ALT_HOST");
        }

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{address}/fixtures/alt-host"))
            .send()
            .await
            .expect("alt-host substitution request");
        let body = response.text().await.expect("alt-host substitution body");
        assert_eq!(
            body,
            format!(
                "alt=http://alef-alt-host-probe.example:{port} sub=http://alef-unresolvable-alt-host.invalid:{port}",
                port = address.port()
            ),
            "a configured ALEF_MOCK_ALT_HOST must reach both mock_alt_origin and \
             mock_sub_origin substitution, got:\n{body}"
        );
    }

    // -----------------------------------------------------------------------
    // The in-process `MockServer` (used by Rust's own e2e tests) exercises the
    // same two mechanisms through its own, independently-written `handle_request`.
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn per_test_server_counts_by_path_and_substitutes_origin_tokens() {
        let route_a = per_test::MockRoute {
            path: "/fixtures/y/a",
            method: "GET",
            status: 200,
            body: "a".to_string(),
            is_streaming: false,
            stream_chunks: Vec::new(),
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        };
        let route_b = per_test::MockRoute {
            path: "/fixtures/y/b",
            method: "GET",
            status: 200,
            body: "{{mock_origin}}/b".to_string(),
            is_streaming: false,
            stream_chunks: Vec::new(),
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        };
        let server = per_test::MockServer::start(vec![route_a, route_b]).await;
        let base = server.url.clone();
        let client = reqwest::Client::new();

        client.get(format!("{base}/fixtures/y/a")).send().await.expect("hit a");
        client
            .get(format!("{base}/fixtures/y/a"))
            .send()
            .await
            .expect("hit a again");
        let response_b = client.get(format!("{base}/fixtures/y/b")).send().await.expect("hit b");

        let expected_origin = format!("{base}/b");
        assert_eq!(
            response_b.text().await.expect("body b"),
            expected_origin,
            "in-process server must substitute body tokens too"
        );

        let one_a = control_one(&client, &base, "GET%20/fixtures/y/a").await;
        assert_eq!(one_a, "2", "got:\n{one_a}");
        let one_b = control_one(&client, &base, "GET%20/fixtures/y/b").await;
        assert_eq!(one_b, "1", "got:\n{one_b}");
    }
    // -----------------------------------------------------------------------
    // Request bodies: the body endpoint answers whether the recorded JSON bodies
    // hold a value at a dotted path.
    // -----------------------------------------------------------------------

    async fn control_body(client: &reqwest::Client, base: &str, key: &str, field: &str, op: &str, value: &str) -> String {
        client
            .get(format!(
                "{base}/__alef/requests/body?key={key}&field={field}&op={op}&value={value}"
            ))
            .send()
            .await
            .expect("body request")
            .text()
            .await
            .expect("body answer")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn body_endpoint_compares_a_dotted_body_path_for_the_matching_requests() {
        let routes = std::collections::HashMap::from([("/fixtures/b/api/chat".to_string(), binary_route("ok"))]);
        let address = spawn_binary_server(routes).await;
        let base = format!("http://{address}");
        let client = reqwest::Client::new();
        let key = "POST%20%2Ffixtures%2Fb%2Fapi%2Fchat";

        assert_eq!(
            control_body(&client, &base, key, "options.num_ctx", "equals", "8192").await,
            "0",
            "no request recorded yet, so equals must not hold"
        );
        assert_eq!(
            control_body(&client, &base, key, "options.num_ctx", "not_equals", "1").await,
            "0",
            "no request recorded yet, so not_equals must not hold either"
        );

        client
            .post(format!("{base}/fixtures/b/api/chat"))
            .body(r#"{"model":"m","options":{"num_ctx":8192},"messages":[{"role":"user"}]}"#)
            .send()
            .await
            .expect("post chat");

        assert_eq!(control_body(&client, &base, key, "options.num_ctx", "equals", "8192").await, "1");
        assert_eq!(
            control_body(&client, &base, key, "options.num_ctx", "equals", "8192.0").await,
            "1",
            "8192 and 8192.0 are the same number"
        );
        assert_eq!(control_body(&client, &base, key, "options.num_ctx", "equals", "4096").await, "0");
        assert_eq!(control_body(&client, &base, key, "options.num_ctx", "not_equals", "4096").await, "1");
        assert_eq!(control_body(&client, &base, key, "options.num_ctx", "not_equals", "8192").await, "0");
        assert_eq!(control_body(&client, &base, key, "model", "equals", "%22m%22").await, "1");
        assert_eq!(control_body(&client, &base, key, "messages.0.role", "equals", "%22user%22").await, "1");
        assert_eq!(
            control_body(&client, &base, key, "options.missing", "equals", "null").await,
            "0",
            "an absent field does not equal null"
        );
        assert_eq!(
            control_body(&client, &base, key, "options.missing", "not_equals", "1").await,
            "1",
            "an absent field is not equal to a value"
        );
        assert_eq!(
            control_body(&client, &base, "%2Ffixtures%2Fb%2Fapi%2Fchat", "options.num_ctx", "equals", "8192").await,
            "1",
            "a bare path key matches any method"
        );
        assert_eq!(
            control_body(&client, &base, "GET%20%2Ffixtures%2Fb%2Fapi%2Fchat", "options.num_ctx", "equals", "8192").await,
            "0",
            "a different method matches no request"
        );

        client
            .post(format!("{base}/fixtures/b/api/chat"))
            .body(r#"{"options":{"num_ctx":4096}}"#)
            .send()
            .await
            .expect("post chat again");
        assert_eq!(
            control_body(&client, &base, key, "options.num_ctx", "equals", "8192").await,
            "0",
            "equals needs every matching request to hold the value"
        );
        assert_eq!(
            control_body(&client, &base, key, "options.num_ctx", "not_equals", "8192").await,
            "0",
            "not_equals needs none of the matching requests to hold the value"
        );

        client
            .post(format!("{base}/__alef/requests/reset?prefix=%2Ffixtures%2Fb"))
            .send()
            .await
            .expect("reset");
        assert_eq!(
            control_body(&client, &base, key, "options.num_ctx", "not_equals", "1").await,
            "0",
            "reset must drop the recorded bodies"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn body_endpoint_treats_a_non_json_body_as_not_holding_the_value() {
        let routes = std::collections::HashMap::from([("/fixtures/c".to_string(), binary_route("ok"))]);
        let address = spawn_binary_server(routes).await;
        let base = format!("http://{address}");
        let client = reqwest::Client::new();
        client
            .post(format!("{base}/fixtures/c"))
            .body("not json")
            .send()
            .await
            .expect("post text");
        let key = "POST%20%2Ffixtures%2Fc";
        assert_eq!(control_body(&client, &base, key, "a", "equals", "1").await, "0");
        assert_eq!(control_body(&client, &base, key, "a", "not_equals", "1").await, "1");
    }

    // -----------------------------------------------------------------------
    // Paced streams: a stream chunk delay spreads the events over time in both servers.
    // -----------------------------------------------------------------------

    /// Read a paced stream to its end and return the time each body chunk arrived.
    async fn chunk_arrival_times(url: &str) -> (Vec<std::time::Duration>, String) {
        let started = std::time::Instant::now();
        let mut response = reqwest::Client::new().get(url).send().await.expect("stream request");
        let mut arrivals = Vec::new();
        let mut text = String::new();
        while let Some(chunk) = response.chunk().await.expect("stream chunk") {
            arrivals.push(started.elapsed());
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
        (arrivals, text)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_server_waits_before_each_stream_chunk() {
        let route = MockRoute {
            status: 200,
            body: Vec::new(),
            is_streaming: true,
            stream_chunks: vec!["one".to_string(), "two".to_string(), "three".to_string()],
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: Some(200),
        };
        let address = spawn_binary_server(std::collections::HashMap::from([("/fixtures/s".to_string(), route)])).await;
        let (arrivals, text) = chunk_arrival_times(&format!("http://{address}/fixtures/s")).await;
        assert_eq!(text, "data: one\n\ndata: two\n\ndata: three\n\ndata: [DONE]\n\n");
        let first = arrivals.first().expect("a first chunk");
        let last = arrivals.last().expect("a last chunk");
        assert!(*first >= std::time::Duration::from_millis(190), "first chunk came after {first:?}");
        assert!(*last >= std::time::Duration::from_millis(590), "stream ended after {last:?}");
        assert!(arrivals.len() >= 3, "the events must arrive in separate writes, got {arrivals:?}");
        assert!(
            arrivals[1] - arrivals[0] >= std::time::Duration::from_millis(150),
            "the second event must wait after the first, got {arrivals:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_server_streams_at_once_without_a_chunk_delay() {
        let route = MockRoute {
            status: 200,
            body: Vec::new(),
            is_streaming: true,
            stream_chunks: vec!["one".to_string(), "two".to_string()],
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        };
        let address = spawn_binary_server(std::collections::HashMap::from([("/fixtures/s".to_string(), route)])).await;
        let (arrivals, text) = chunk_arrival_times(&format!("http://{address}/fixtures/s")).await;
        assert_eq!(text, "data: one\n\ndata: two\n\ndata: [DONE]\n\n");
        assert!(
            *arrivals.last().expect("a chunk") < std::time::Duration::from_millis(150),
            "an unpaced stream must not wait, got {arrivals:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn per_test_server_waits_before_each_stream_chunk() {
        let route = per_test::MockRoute {
            path: "/fixtures/p",
            method: "GET",
            status: 200,
            body: String::new(),
            is_streaming: true,
            stream_chunks: vec!["one".to_string(), "two".to_string()],
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: Some(200),
        };
        let server = per_test::MockServer::start(vec![route]).await;
        let (arrivals, text) = chunk_arrival_times(&format!("{}/fixtures/p", server.url)).await;
        assert_eq!(text, "data: one\n\ndata: two\n\ndata: [DONE]\n\n");
        assert!(
            *arrivals.last().expect("a chunk") >= std::time::Duration::from_millis(390),
            "stream ended after {arrivals:?}"
        );
        assert!(arrivals.len() >= 2, "the events must arrive in separate writes, got {arrivals:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn per_test_server_records_request_bodies() {
        let route = per_test::MockRoute {
            path: "/fixtures/q",
            method: "POST",
            status: 200,
            body: "{}".to_string(),
            is_streaming: false,
            stream_chunks: Vec::new(),
            headers: Vec::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        };
        let server = per_test::MockServer::start(vec![route]).await;
        let client = reqwest::Client::new();
        client
            .post(format!("{}/fixtures/q", server.url))
            .body(r#"{"options":{"num_ctx":8192}}"#)
            .send()
            .await
            .expect("post");
        assert_eq!(
            control_body(&client, &server.url, "POST%20%2Ffixtures%2Fq", "options.num_ctx", "equals", "8192").await,
            "1"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_server_streams_ndjson_when_the_content_type_says_so() {
        let route = MockRoute {
            status: 200,
            body: Vec::new(),
            is_streaming: true,
            stream_chunks: vec![r#"{"a":1}"#.to_string(), r#"{"done":true}"#.to_string()],
            headers: vec![("content-type".to_string(), "application/x-ndjson".to_string())],
            delay_ms: None,
            stream_chunk_delay_ms: None,
        };
        let address = spawn_binary_server(std::collections::HashMap::from([("/fixtures/n".to_string(), route)])).await;
        let response = reqwest::Client::new()
            .get(format!("http://{address}/fixtures/n"))
            .send()
            .await
            .expect("ndjson request");
        let types: Vec<_> = response.headers().get_all("content-type").iter().collect();
        assert_eq!(types.len(), 1, "one content-type header, got {types:?}");
        assert_eq!(types[0], "application/x-ndjson");
        assert_eq!(response.text().await.expect("ndjson body"), "{\"a\":1}\n{\"done\":true}\n");
    }
}
