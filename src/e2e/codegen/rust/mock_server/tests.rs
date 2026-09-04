use super::*;

#[test]
fn render_mock_server_module_contains_struct_definition() {
    let out = render_mock_server_module();
    assert!(out.contains("pub struct MockRoute"));
    assert!(out.contains("pub struct MockServer"));
}

#[test]
fn render_mock_server_binary_contains_main() {
    let out = render_mock_server_binary();
    assert!(out.contains("async fn main()"));
    assert!(out.contains("MOCK_SERVER_URL=http://"));
}

#[test]
fn render_mock_server_binary_is_valid_rust() {
    let out = render_mock_server_binary();
    syn::parse_file(&out).expect("generated mock server must parse as Rust");
    assert!(
        out.contains("if let Some(input) = &self.input\n            && let Some(arr)"),
        "mock route loading must use a let-chain accepted by clippy"
    );
}

#[test]
fn render_mock_server_binary_spawn_before_print() {
    let out = render_mock_server_binary();
    // The shared server must be spawned (and every listener probed for
    // readiness) BEFORE the MOCK_SERVER_URL line is printed, so consumers
    // that connect immediately after reading the line do not get ECONNREFUSED.
    let spawn_pos = out
        .find("axum::serve(shared_listener, shared_app)")
        .expect("shared spawn missing");
    let probe_pos = out.find("TcpStream::connect").expect("readiness probe missing");
    let print_pos = out.find("println!(\"MOCK_SERVER_URL=http://").expect("print missing");
    assert!(
        spawn_pos < print_pos,
        "shared server spawn must appear before MOCK_SERVER_URL print"
    );
    assert!(
        probe_pos < print_pos,
        "readiness probe must appear before MOCK_SERVER_URL print"
    );
}

#[test]
fn render_mock_server_binary_uses_generic_fixture_schema_terms() {
    let out = render_mock_server_binary();
    assert!(
        out.contains("Route-array fixture schema"),
        "missing generic route-array schema docs"
    );
    assert!(
        out.contains("ORIGIN_ROOT_ROUTE_PREFIXES"),
        "missing named origin-root route prefixes"
    );
    assert!(
        !out.contains("sample-"),
        "must not mention project-specific fixture names"
    );
}

#[test]
fn render_mock_server_binary_keeps_route_loading_paths() {
    let out = render_mock_server_binary();
    assert!(out.contains("fn load_routes("), "missing route loader");
    assert!(
        out.contains("fn load_routes_recursive("),
        "missing recursive fixture directory loading"
    );
    assert!(
        out.contains("fixtures_dir.join(\"responses\").join(file)"),
        "missing responses/body_file fallback path"
    );
    assert!(
        out.contains("per_fixture"),
        "missing per-fixture origin-root route table"
    );
    assert!(
        out.contains("is_host_root_path(&resolved.original_path)"),
        "missing host-root route split"
    );
}

#[test]
fn render_common_module_has_expected_symbols() {
    let src = render_common_module();
    assert!(src.contains("pub fn mock_server_url"), "missing mock_server_url");
    assert!(src.contains("OnceLock"), "missing OnceLock");
    assert!(src.contains("MOCK_SERVER_URL"), "missing MOCK_SERVER_URL");
    assert!(src.contains("MOCK_SERVERS"), "missing MOCK_SERVERS");
    assert!(src.contains("serde_json"), "missing serde_json parsing");
}

/// A fixture that declares a `body_file` the mock server cannot read is a broken fixture, not a
/// zero-length document. Serving an empty 200 for it made a provisioning gap reach the consumer
/// as whatever error its HTTP client invented for an empty body -- in one investigation, an
/// intermittent `data_loss: error decoding response body` that named neither the fixture nor the
/// file. The generated binary must abort with the path instead.
#[test]
fn an_unreadable_body_file_aborts_instead_of_serving_an_empty_body() {
    let out = render_mock_server_binary();

    assert!(
        out.contains("std::process::exit(1)"),
        "an unreadable body_file must abort the mock server, got:\n{out}"
    );
    assert!(
        !out.contains("            Vec::new()\n                            }\n                        }"),
        "the silent empty-body fallback for an unreadable body_file must be gone, got:\n{out}"
    );
}

/// CONTROL: a route that legitimately declares no body at all still resolves to an empty body.
/// A fix that aborted on every empty body would take these down with it.
#[test]
fn a_route_declaring_no_body_still_resolves_to_an_empty_body() {
    let out = render_mock_server_binary();

    assert!(
        out.contains("} else {\n                        Vec::new()\n                    };"),
        "a route with neither body nor body_file must still yield an empty body, got:\n{out}"
    );
}
