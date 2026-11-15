#![cfg(unix)]
//! Real-process cases for stdio protocol 2: negotiation, raw payloads, pipelining, timeout
//! isolation and process death. The fixture declares protocol 2 only when the host advertises it
//! through `ALEF_STDIO_PROTOCOLS`, so every `mode == "binary"` assertion also proves the host did.

#[path = "stdio_sidecar/support.rs"]
mod support;

use alef_component_runtime::stdio::{MAX_FRAME_SIZE, SidecarSession, SidecarSessionSetup};
use alef_component_runtime::{ComponentManager, LoadedComponent};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use support::is_alive;

fn manager(scenario: &str) -> (tempfile::TempDir, ComponentManager) {
    support::manager(include_str!("stdio_sidecar/binary_fixture.py"), scenario)
}

fn engine(manager: &ComponentManager) -> Arc<LoadedComponent> {
    manager.ensure_contract("demo", "engine", b"unused\0").unwrap()
}

fn text(engine: &LoadedComponent, method: &str) -> String {
    String::from_utf8(engine.invoke_bytes(method, b"").unwrap()).unwrap()
}

fn sleep_concurrently(engine: &Arc<LoadedComponent>, calls: usize, seconds: &'static str) -> Duration {
    let start = Instant::now();
    let threads: Vec<_> = (0..calls)
        .map(|_| {
            let engine = Arc::clone(engine);
            std::thread::spawn(move || engine.invoke_bytes("sleep", seconds.as_bytes()).unwrap())
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), seconds.as_bytes());
    }
    start.elapsed()
}

#[test]
fn should_negotiate_binary_frames_and_carry_raw_payload_bytes() {
    let (_dir, manager) = manager("binary/8");
    let engine = engine(&manager);
    assert_eq!(text(&engine, "mode"), "binary");
    let every_byte: Vec<u8> = (0..=255).collect();
    assert_eq!(engine.invoke_bytes("echo", &every_byte).unwrap(), every_byte);
    assert_eq!(engine.invoke_bytes("echo", b"").unwrap(), b"");
    // The whole frame limit is available to content: protocol 1 caps this at ~4 MiB.
    let largest = vec![0xFF; MAX_FRAME_SIZE];
    assert_eq!(engine.invoke_bytes("echo", &largest).unwrap(), largest);
    let pid = text(&engine, "pid");
    let error = engine.invoke_bytes("echo", &vec![0; MAX_FRAME_SIZE + 1]).unwrap_err();
    assert_eq!(error.code(), "sidecar_protocol", "{error}");
    let error = engine.invoke_bytes("failure", b"").unwrap_err().to_string();
    assert!(error.contains("producer failed"), "{error}");
    assert_eq!(text(&engine, "pid"), pid, "rejected and failed calls keep the child");
    let second = manager.ensure_contract("demo", "second", b"unused\0").unwrap();
    assert_eq!(second.invoke_bytes("echo", b"second").unwrap(), b"second");
}

#[test]
fn should_fall_back_to_json_frames_for_a_protocol_1_sidecar() {
    let (_dir, manager) = manager("json");
    let engine = engine(&manager);
    assert_eq!(text(&engine, "mode"), "json");
    assert_eq!(engine.invoke_bytes("echo", b"\0json\xff").unwrap(), b"\0json\xff");
}

#[test]
fn should_reject_invalid_in_flight_limits_in_the_handshake() {
    for scenario in ["json-with-limit", "binary/0"] {
        let (_dir, manager) = manager(scenario);
        let error = manager.ensure_contract("demo", "engine", b"unused\0").err().unwrap();
        assert_eq!(error.code(), "sidecar_protocol", "{scenario}: {error}");
    }
}

#[test]
fn should_run_concurrent_calls_in_parallel() {
    let (_dir, manager) = manager("binary/8");
    let engine = engine(&manager);
    let elapsed = sleep_concurrently(&engine, 8, "0.5");
    // Serialized, eight half-second calls take four seconds.
    assert!(elapsed < Duration::from_secs(2), "8 concurrent calls took {elapsed:?}");
    let peak: usize = text(&engine, "peak").parse().unwrap();
    assert!(peak >= 4, "only {peak} calls ever ran at once");
}

#[test]
fn should_bound_in_flight_requests_by_the_declared_limit() {
    let (_dir, manager) = manager("binary/2");
    let engine = engine(&manager);
    let elapsed = sleep_concurrently(&engine, 6, "0.2");
    assert!(
        elapsed >= Duration::from_millis(600),
        "6 calls over 2 slots took {elapsed:?}"
    );
    assert_eq!(text(&engine, "peak"), "2");
}

#[test]
fn should_serialize_calls_when_the_sidecar_declares_no_limit() {
    let (_dir, manager) = manager("binary");
    let engine = engine(&manager);
    assert_eq!(text(&engine, "mode"), "binary");
    sleep_concurrently(&engine, 3, "0.1");
    assert_eq!(text(&engine, "peak"), "1");
}

#[test]
fn should_keep_serving_others_when_one_request_times_out() {
    let (_dir, manager) = manager("binary/8");
    let engine = engine(&manager);
    let pid = text(&engine, "pid");
    let start = Instant::now();
    let slow = {
        let engine = Arc::clone(&engine);
        std::thread::spawn(move || engine.invoke_bytes("sleep", b"3").unwrap_err())
    };
    while !slow.is_finished() {
        assert_eq!(engine.invoke_bytes("echo", b"meanwhile").unwrap(), b"meanwhile");
        std::thread::sleep(Duration::from_millis(100));
    }
    let error = slow.join().unwrap();
    assert_eq!(error.code(), "sidecar_timeout", "{error}");
    assert_eq!(
        text(&engine, "pid"),
        pid,
        "a timeout while the sidecar answers others keeps it"
    );
    // Let the abandoned request's response arrive: it is dropped, not handed to a later call.
    std::thread::sleep(Duration::from_secs(3).saturating_sub(start.elapsed()) + Duration::from_millis(500));
    for _ in 0..3 {
        assert_eq!(engine.invoke_bytes("echo", b"after").unwrap(), b"after");
    }
    assert_eq!(text(&engine, "pid"), pid);
    assert_eq!(manager.sidecar_restart_count("demo"), Some(0));
}

#[test]
fn should_restart_a_sidecar_that_answers_nothing_for_a_whole_deadline() {
    let (_dir, manager) = manager("binary/8");
    let engine = engine(&manager);
    let first = text(&engine, "pid");
    let error = engine.invoke_bytes("hang", b"").unwrap_err();
    assert_eq!(error.code(), "sidecar_timeout", "{error}");
    assert!(!is_alive(&first), "the stalled child must be killed and reaped");
    assert_ne!(text(&engine, "pid"), first, "the next call is served by a fresh child");
}

#[test]
fn should_fail_every_in_flight_call_when_the_sidecar_dies() {
    let (_dir, manager) = manager("binary/8");
    let engine = engine(&manager);
    let first = text(&engine, "pid");
    let waiting: Vec<_> = (0..4)
        .map(|_| {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || engine.invoke_bytes("sleep", b"10").unwrap_err())
        })
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    let died = Instant::now();
    assert!(engine.invoke_bytes("exit", b"").is_err());
    for thread in waiting {
        let error = thread.join().unwrap();
        assert_eq!(error.code(), "sidecar_protocol", "{error}");
    }
    assert!(
        died.elapsed() < Duration::from_secs(2),
        "in-flight calls fail at once, not at their deadline"
    );
    assert_ne!(text(&engine, "pid"), first);
    assert_eq!(manager.sidecar_restart_count("demo"), Some(1));
}

#[test]
fn should_restart_after_a_protocol_violation() {
    for method in ["bad_id", "garbage"] {
        let (_dir, manager) = manager("binary/8");
        let engine = engine(&manager);
        let first = text(&engine, "pid");
        let error = engine.invoke_bytes(method, b"").unwrap_err();
        assert_eq!(error.code(), "sidecar_protocol", "{method}: {error}");
        assert_ne!(text(&engine, "pid"), first, "{method}");
    }
}

#[test]
fn should_replay_the_session_setup_over_binary_frames() {
    let (_dir, manager) = manager("binary/4");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let setup: SidecarSessionSetup = {
        let calls = Arc::clone(&calls);
        Arc::new(move |session: &SidecarSession| {
            let mode = session.invoke("engine", "mode", b"")?;
            calls.lock().unwrap().push(mode);
            Ok(())
        })
    };
    let manager = manager.with_sidecar_session(setup);
    let engine = engine(&manager);
    assert!(engine.invoke_bytes("exit", b"").is_err());
    assert_eq!(engine.invoke_bytes("echo", b"back").unwrap(), b"back");
    assert_eq!(*calls.lock().unwrap(), vec![b"binary".to_vec(), b"binary".to_vec()]);
}
