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
}
