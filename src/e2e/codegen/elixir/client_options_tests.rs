#[test]
fn json_only_client_declares_its_encoder_without_network_test_dependencies() {
    let output = super::project::render_mix_exs(
        "sample",
        "",
        crate::e2e::config::DependencyMode::Local,
        super::project::MixTestSurface {
            has_json_clients: true,
            has_http_tests: false,
            has_mock_server_tests: false,
            has_nif_tests: false,
            uses_harness: false,
        },
    );
    assert!(output.contains(":jason"), "{output}");
    assert!(!output.contains(":finch"), "{output}");
    assert!(!output.contains(":req"), "{output}");
}
