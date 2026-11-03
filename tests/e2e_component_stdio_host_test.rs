#![cfg(all(unix, any(target_os = "linux", target_os = "macos")))]

#[path = "support/component_stdio_host/mod.rs"]
mod support;

#[test]
#[ignore = "requires supported Unix host, installed Cargo/Rust and Python 3, cached runtime dependencies; builds and runs a standalone generated host"]
fn generated_stdio_host_round_trips_signed_sidecar_and_rejects_native_control() {
    let root = support::proof_root();
    let (api, config) = support::fixture();
    let lock = support::signed_package(&root, &api, &config);
    support::write_driver(&root, &api, &config, &lock);
    let jobs = support::run_driver(&root);
    support::verify_and_record_proof(&root, &jobs);
}
