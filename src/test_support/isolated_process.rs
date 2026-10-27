use std::io::Write as _;

const CHILD_TEST: &str = "ALEF_ISOLATED_TEST";
const PARENT_PID: &str = "ALEF_ISOLATED_PARENT_PID";

// ~keep PATH mutation must happen in a child: a mutex protects only callers that acquire it.
pub(crate) fn run_in_isolated_process(test_name: &str) -> bool {
    let receipt = format!("alef isolated child: {test_name}\n");
    if std::env::var(CHILD_TEST).as_deref() == Ok(test_name) {
        std::io::stdout()
            .write_all(receipt.as_bytes())
            .expect("write child receipt");
        return false;
    }
    let original_path = std::env::var_os("PATH");
    let output = super::spawn_from_stable_dir(std::env::current_exe().expect("current test executable"))
        .args([test_name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_TEST, test_name)
        .env(PARENT_PID, std::process::id().to_string())
        .output()
        .expect("run isolated test child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "isolated test failed:\n{stdout}\n{stderr}");
    assert_child_report(&stdout, &receipt);
    assert_eq!(
        std::env::var_os("PATH"),
        original_path,
        "child PATH must not reach the parent"
    );
    true
}

fn assert_child_report(stdout: &str, receipt: &str) {
    assert_eq!(
        stdout.matches(receipt).count(),
        1,
        "expected one child receipt:\n{stdout}"
    );
    let summaries: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("test result: "))
        .collect();
    assert_eq!(summaries.len(), 1, "expected exactly one child summary:\n{stdout}");
    assert!(
        summaries[0].starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;"),
        "expected one passing child test:\n{stdout}"
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    const TEST_NAME: &str = "test_support::isolated_process::tests::should_isolate_path_mutation_from_parent";

    #[test]
    fn should_isolate_path_mutation_from_parent() {
        let is_child = std::env::var(super::CHILD_TEST).as_deref() == Ok(TEST_NAME);
        let parent = if is_child {
            std::env::var(super::PARENT_PID).expect("isolated parent pid")
        } else {
            std::process::id().to_string()
        };
        let root = std::env::temp_dir().join(format!("alef-path-isolation-{parent}"));
        let observer = if is_child {
            None
        } else {
            std::fs::create_dir(&root).expect("unique isolation fixture directory");
            let root = root.clone();
            let original = std::env::var_os("PATH");
            Some(std::thread::spawn(move || {
                wait_for(&root.join("mutated"));
                let actual = std::env::var_os("PATH");
                std::fs::write(root.join("observed"), b"one parent PATH observation").unwrap();
                assert_eq!(
                    actual, original,
                    "parent PATH must remain unchanged during child mutation"
                );
            }))
        };
        if super::run_in_isolated_process(TEST_NAME) {
            observer.unwrap().join().expect("one parent PATH observation completed");
            std::fs::remove_dir_all(root).expect("remove isolation fixture");
            return;
        }
        assert!(is_child, "PATH mutation must execute inside the selected child process");
        // SAFETY: the exact child runs one test and exits without sharing this PATH with its parent. ~keep
        unsafe { std::env::set_var("PATH", "alef-isolated-path-negative-control") };
        std::fs::write(root.join("mutated"), b"child PATH changed").unwrap();
        wait_for(&root.join("observed"));
        assert_eq!(std::env::var("PATH").unwrap(), "alef-isolated-path-negative-control");
    }

    #[test]
    fn should_reject_eleven_passing_child_tests_with_one_receipt() {
        let receipt = "alef isolated child: selected-test\n";
        let stdout = format!(
            "{receipt}test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n"
        );
        let error = std::panic::catch_unwind(|| super::assert_child_report(&stdout, receipt))
            .expect_err("eleven passing tests must not satisfy the one-test child contract");
        let message = error.downcast_ref::<String>().expect("assertion panic string");
        assert!(message.contains("expected one passing child test"), "{message}");
    }

    fn wait_for(path: &Path) {
        let started = Instant::now();
        while !path.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
