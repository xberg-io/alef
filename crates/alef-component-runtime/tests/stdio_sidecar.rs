#![cfg(unix)]

use alef_component_abi::AlefContract;
use alef_component_runtime::stdio::{SidecarSession, SidecarSessionSetup};
use alef_component_runtime::{ComponentManager, ComponentStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[repr(C)]
#[derive(Debug)]
struct TypedTable {
    method: unsafe extern "C" fn(),
}

// SAFETY: a repr(C) function table has a stable plain C layout for this test hash. ~keep
unsafe impl AlefContract for TypedTable {
    const CONTRACT_HASH: [u8; 32] = [0x11; 32];
}

#[path = "stdio_sidecar/support.rs"]
mod support;

use support::is_alive;

fn manager(scenario: &str) -> (tempfile::TempDir, ComponentManager) {
    support::manager(include_str!("stdio_sidecar/fixture.py"), scenario)
}

/// A session setup that invokes `count` on each fresh connection and records every response, so a
/// test can observe how many times (and on which children) the setup ran.
fn session_recorder(calls: Arc<Mutex<Vec<Vec<u8>>>>) -> SidecarSessionSetup {
    Arc::new(move |session: &SidecarSession| {
        let response = session.invoke("engine", "count", b"")?;
        calls.lock().unwrap().push(response);
        Ok(())
    })
}

#[test]
fn should_load_signed_sidecar_and_reuse_one_process_across_contracts() {
    let (_dir, manager) = manager("valid");
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::NotDownloaded);
    manager.prefetch(&["demo"]).unwrap();
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Cached);
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let second = manager.ensure_contract("demo", "second", b"unused\0").unwrap();
    assert_eq!(
        engine.contract_table::<TypedTable>().unwrap_err().code(),
        "unsupported_transport"
    );
    assert_eq!(
        engine.instantiate::<TypedTable>().err().unwrap().code(),
        "unsupported_transport"
    );
    assert!(engine.contract().is_null());
    assert_eq!(engine.contract_size(), 0);
    assert_eq!(engine.component_id(), "demo");
    assert_eq!(engine.component_version(), "1.0.0");
    assert_eq!(engine.descriptor().contract_hash, [0x11; 32]);
    assert_eq!(second.descriptor().contract_hash, [0x22; 32]);
    assert_eq!(engine.invoke_bytes("echo", b"\0hello\xff").unwrap(), b"\0hello\xff");
    assert_eq!(second.invoke_bytes("count", b"").unwrap(), b"2");
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Ready);
}

#[test]
fn should_reject_mismatching_handshake_before_becoming_ready() {
    for scenario in ["bad_hash", "bad_version", "bad_identity", "bad_feature"] {
        let (_dir, manager) = manager(scenario);
        assert!(
            manager.ensure_contract("demo", "engine", b"unused\0").is_err(),
            "{scenario}"
        );
        assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Cached);
    }
}

#[test]
fn should_return_status_error_without_losing_process() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let error = engine.invoke_bytes("failure", b"").unwrap_err().to_string();
    assert!(error.contains("3"), "{error}");
    assert!(error.contains("producer failed"), "{error}");
    let oversized = vec![0; alef_component_runtime::stdio::MAX_FRAME_SIZE + 1];
    assert_eq!(
        engine.invoke_bytes("echo", &oversized).unwrap_err().code(),
        "sidecar_protocol"
    );
    assert_eq!(engine.invoke_bytes("echo", b"still alive").unwrap(), b"still alive");
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Ready);
    assert!(manager.ensure_contract("demo", "engine", b"unused\0").is_ok());
}

#[test]
fn should_reject_a_request_whose_json_encoding_outgrows_the_frame_without_stopping() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let pid = engine.invoke_bytes("pid", b"").unwrap();
    // Within the payload limit, but "255," per byte inflates it past the 16 MiB protocol-1 frame.
    let inflated = vec![0xFF; 5 * 1024 * 1024];
    let error = engine.invoke_bytes("echo", &inflated).unwrap_err();
    assert_eq!(error.code(), "sidecar_protocol", "{error}");
    assert_eq!(
        engine.invoke_bytes("pid", b"").unwrap(),
        pid,
        "a request rejected before sending must not stop the child"
    );
    assert_eq!(manager.sidecar_restart_count("demo"), Some(0));
}

#[test]
fn should_bound_hanging_calls_and_reject_broken_frames() {
    for method in ["hang", "exit", "truncated", "oversized", "malformed"] {
        let (_dir, manager) = manager("valid");
        // Restart budget 0 pins the pre-respawn behavior this test asserts: one fatal failure
        // stops the connection for good. The 2 s deadline is what proves calls are bounded
        // (the fixture's hang sleeps 60 s; only `hang` waits out the deadline) while leaving
        // room for the describe handshake under a fully parallel python-spawning test run.
        let manager = manager
            .with_sidecar_restart_budget(0)
            .with_sidecar_timeout(Duration::from_secs(2));
        let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
        let start = Instant::now();
        assert!(engine.invoke_bytes(method, b"").is_err(), "{method}");
        assert!(start.elapsed() < Duration::from_secs(3), "{method}");
        assert!(engine.invoke_bytes("echo", b"after failure").is_err(), "{method}");
        assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Cached, "{method}");
        for contract in ["engine", "second"] {
            let error = manager
                .ensure_contract("demo", contract, b"unused\0")
                .err()
                .expect("a stopped process must not activate any contract");
            assert_eq!(error.code(), "sidecar_stopped", "{method}: {contract}");
        }
    }
}

#[test]
fn should_serialize_concurrent_calls_without_crossing_responses() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let threads: Vec<_> = (0..12)
        .map(|value| {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || {
                let payload = vec![value; 4096];
                assert_eq!(engine.invoke_bytes("echo", &payload).unwrap(), payload);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn should_reap_child_after_manager_and_last_component_are_dropped() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    let pid = String::from_utf8(engine.invoke_bytes("pid", b"").unwrap()).unwrap();
    drop(manager);
    assert_eq!(engine.invoke_bytes("echo", b"retained").unwrap(), b"retained");
    drop(engine);
    let status = std::process::Command::new("kill")
        .args(["-0", &pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "sidecar {pid} survived its final owner");
}

#[test]
fn should_respawn_after_a_crashed_call_and_replay_the_session_setup() {
    let (_dir, manager) = manager("valid");
    let setup_calls = Arc::new(Mutex::new(Vec::new()));
    let manager = manager.with_sidecar_session(session_recorder(Arc::clone(&setup_calls)));
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    let error = engine.invoke_bytes("exit", b"").unwrap_err();
    assert_eq!(
        error.code(),
        "sidecar_protocol",
        "the crashing call returns its own error: {error}"
    );
    assert_eq!(engine.invoke_bytes("echo", b"fresh").unwrap(), b"fresh");
    assert_eq!(
        *setup_calls.lock().unwrap(),
        vec![b"1".to_vec(), b"1".to_vec()],
        "the session setup must run on the first spawn and again on the respawn"
    );
}

#[test]
fn should_reap_a_timed_out_child_and_serve_the_next_call_from_a_fresh_one() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    let first = String::from_utf8(engine.invoke_bytes("pid", b"").unwrap()).unwrap();
    let error = engine.invoke_bytes("hang", b"").unwrap_err();
    assert_eq!(error.code(), "sidecar_timeout", "{error}");
    assert!(!is_alive(&first), "the timed-out child must be killed and reaped");
    let second = String::from_utf8(engine.invoke_bytes("pid", b"").unwrap()).unwrap();
    assert_ne!(first, second, "the next call must be served by a fresh child");
}

#[test]
fn should_stop_after_the_restart_budget_is_exhausted() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    assert!(
        engine.invoke_bytes("exit", b"").is_err(),
        "the first crash stops the connection"
    );
    assert_eq!(engine.invoke_bytes("echo", b"respawned").unwrap(), b"respawned");
    assert!(
        engine.invoke_bytes("exit", b"").is_err(),
        "the second crash stops it again"
    );
    let error = engine.invoke_bytes("echo", b"").unwrap_err();
    assert_eq!(error.code(), "sidecar_stopped", "{error}");
    assert_eq!(manager.sidecar_restart_count("demo"), Some(1));
}

#[test]
fn should_report_restarts_through_the_event_callback() {
    let (_dir, manager) = manager("valid");
    let events = Arc::new(Mutex::new(Vec::new()));
    let handler = {
        let events = Arc::clone(&events);
        Arc::new(move |component: &str, count: u32| {
            events.lock().unwrap().push((component.to_owned(), count));
        })
    };
    let manager = manager
        .with_sidecar_restart_budget(2)
        .with_sidecar_restart_event(handler);
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    assert!(engine.invoke_bytes("exit", b"").is_err());
    assert_eq!(engine.invoke_bytes("echo", b"back").unwrap(), b"back");
    assert_eq!(*events.lock().unwrap(), vec![("demo".to_owned(), 1)]);
}

#[test]
fn should_run_the_session_setup_once_on_the_first_spawn() {
    let (_dir, manager) = manager("valid");
    let setup_calls = Arc::new(Mutex::new(Vec::new()));
    let manager = manager.with_sidecar_session(session_recorder(Arc::clone(&setup_calls)));
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    assert_eq!(engine.invoke_bytes("echo", b"hello").unwrap(), b"hello");
    assert_eq!(
        *setup_calls.lock().unwrap(),
        vec![b"1".to_vec()],
        "the session setup runs once on the first spawn"
    );
}

#[test]
fn should_consume_the_budget_when_a_respawn_fails_and_fire_no_event() {
    let (_dir, manager) = manager("valid");
    let events = Arc::new(Mutex::new(Vec::new()));
    let handler = {
        let events = Arc::clone(&events);
        Arc::new(move |component: &str, count: u32| {
            events.lock().unwrap().push((component.to_owned(), count));
        })
    };
    let manager = manager.with_sidecar_restart_event(handler);
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    assert!(engine.invoke_bytes("exit", b"").is_err());

    // Corrupt the verified executable, so the respawn can start no healthy child.
    let library = manager.cache_path("demo").unwrap().join("sidecar");
    std::fs::write(&library, b"not a python script").unwrap();

    let error = engine.invoke_bytes("echo", b"").unwrap_err();
    assert_eq!(
        error.code(),
        "size_mismatch",
        "the respawn re-verifies the executable before running it: {error}"
    );
    let error = engine.invoke_bytes("echo", b"").unwrap_err();
    assert_eq!(
        error.code(),
        "sidecar_stopped",
        "the failed respawn consumed the budget: {error}"
    );
    assert_eq!(manager.sidecar_restart_count("demo"), Some(0));
    assert!(
        events.lock().unwrap().is_empty(),
        "a failed respawn must not fire the restart event"
    );
}

/// Regression: a respawn used to re-execute the cached file without verifying it again, so an
/// executable edited after the first spawn ran unverified. This edit keeps the size and leaves a
/// working script, so only re-verification can refuse it.
#[test]
fn should_refuse_to_respawn_an_executable_edited_after_the_first_spawn() {
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    assert!(engine.invoke_bytes("exit", b"").is_err());

    let library = manager.cache_path("demo").unwrap().join("sidecar");
    let original = std::fs::read(&library).unwrap();
    let edited = String::from_utf8(original.clone())
        .unwrap()
        .replacen("Read a request", "Read a REQUEST", 1);
    assert_ne!(
        edited.as_bytes(),
        original,
        "the fixture text the edit targets is missing"
    );
    assert_eq!(edited.len(), original.len());
    std::fs::write(&library, edited).unwrap();

    let error = engine.invoke_bytes("echo", b"").unwrap_err();
    assert_eq!(error.code(), "digest_mismatch", "{error}");
    assert_eq!(manager.sidecar_restart_count("demo"), Some(0));
}

/// Regression: sidecars inherited the host's whole environment (tokens, credentials) and working
/// directory. `CARGO_MANIFEST_DIR` stands in for such a variable: cargo sets it for every test
/// process, and nothing allowlists it.
#[test]
fn should_start_sidecars_with_an_allowlisted_environment_in_the_component_directory() {
    assert!(
        std::env::var_os("CARGO_MANIFEST_DIR").is_some(),
        "cargo sets this for tests"
    );
    let (_dir, manager) = manager("valid");
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    let names: Vec<String> = serde_json::from_slice(&engine.invoke_bytes("env", b"").unwrap()).unwrap();
    assert!(names.iter().any(|name| name == "PATH"), "{names:?}");
    assert!(names.iter().any(|name| name == "ALEF_STDIO_PROTOCOLS"), "{names:?}");
    assert!(!names.iter().any(|name| name.starts_with("CARGO")), "{names:?}");

    let cwd = String::from_utf8(engine.invoke_bytes("cwd", b"").unwrap()).unwrap();
    let object = manager.cache_path("demo").unwrap();
    assert_eq!(
        std::fs::canonicalize(cwd).unwrap(),
        std::fs::canonicalize(object).unwrap()
    );
}

#[test]
fn should_pass_host_allowlisted_variables_through_to_the_sidecar() {
    let (_dir, manager) = manager("valid");
    let manager = manager.with_sidecar_env_passthrough(["CARGO_MANIFEST_*"]);
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();

    let names: Vec<String> = serde_json::from_slice(&engine.invoke_bytes("env", b"").unwrap()).unwrap();
    assert!(names.iter().any(|name| name == "CARGO_MANIFEST_DIR"), "{names:?}");
    assert!(!names.iter().any(|name| name == "CARGO_PKG_NAME"), "{names:?}");
}

#[test]
fn should_report_status_without_respawning_a_stopped_connection() {
    let (_dir, manager) = manager("valid");
    let setup_calls = Arc::new(Mutex::new(Vec::new()));
    let manager = manager.with_sidecar_session(session_recorder(Arc::clone(&setup_calls)));
    let engine = manager.ensure_contract("demo", "engine", b"unused\0").unwrap();
    assert!(engine.invoke_bytes("exit", b"").is_err());

    // A status query observes, it does not heal: a stopped connection still reports `Cached`
    // (as before respawning existed) even though the restart budget would allow a respawn.
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Cached);
    assert_eq!(manager.sidecar_restart_count("demo"), Some(0));
    assert_eq!(*setup_calls.lock().unwrap(), vec![b"1".to_vec()]);

    // The budget is untouched: the next real call still respawns and runs the setup a second time.
    assert_eq!(engine.invoke_bytes("echo", b"back").unwrap(), b"back");
    assert_eq!(*setup_calls.lock().unwrap(), vec![b"1".to_vec(), b"1".to_vec()]);
    assert_eq!(manager.status("demo").unwrap(), ComponentStatus::Ready);
}
