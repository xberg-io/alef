use std::ffi::OsStr;
use std::time::Duration;

#[test]
fn stable_path_probe_returns_none_for_a_missing_tool() {
    assert!(
        super::tool_available_with_stable_path("alef-missing-poly-negative-control-487488").is_none(),
        "a genuinely absent tool must keep the guarded availability check's skip path reachable"
    );
}

#[test]
fn stable_path_resolution_returns_none_for_a_missing_tool() {
    assert_eq!(
        super::tool_path_with_stable_path("alef-missing-poly-negative-control-487488"),
        None,
        "a genuinely absent tool must keep the absolute-path resolver's skip path reachable"
    );
}

#[test]
fn extensionless_tool_name_matches_a_pathext_executable() {
    let directory = tempfile::tempdir().expect("tool directory");
    std::fs::write(directory.path().join("poly.EXE"), b"").expect("write representative Windows executable");

    assert!(
        !super::directory_contains_tool(directory.path(), "poly", None),
        "an extensionless lookup must not invent executable suffixes without PATHEXT"
    );
    assert!(
        super::directory_contains_tool(directory.path(), "poly", Some(OsStr::new(".COM;.EXE;.BAT;.CMD"))),
        "an extensionless lookup must match the concrete executable named by PATHEXT"
    );
}

#[test]
fn stable_path_resolution_waits_for_mutation_guard_before_resolving_tool() {
    let tool_name = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    let expected = super::tool_path_with_stable_path(tool_name).expect("cargo tests require rustc on PATH");
    let without_tool = super::PathWithoutToolGuard::exclude(tool_name);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (resolved_tx, resolved_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).expect("signal resolution start");
        resolved_tx
            .send(super::tool_path_with_stable_path(tool_name))
            .expect("send resolved tool path");
    });
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("resolution worker started");
    assert_eq!(
        resolved_rx.recv_timeout(Duration::from_millis(250)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout),
        "absolute-path resolution must wait while another test has removed the tool from PATH"
    );

    drop(without_tool);
    assert_eq!(
        resolved_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("resolution completed after PATH was restored"),
        Some(expected),
        "resolution after the mutation guard drops must return the original absolute path"
    );
    worker.join().expect("resolution worker completed");
}
