use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// ~keep The third-party stream API is represented by a synchronous iterator facade. Actual
// generated code runs unchanged; owned stream state verifies drainage independently of bindings.
const RUNTIME: &str = r#"
mod tokio_stream {
    pub trait StreamExt: Iterator + Sized {
        fn collect<T: FromIterator<Self::Item>>(self) -> std::future::Ready<T> {
            std::future::ready(Iterator::collect(self))
        }
    }
    impl<I: Iterator> StreamExt for I {}
}
mod common {
    pub struct Runtime;
    pub fn runtime() -> Runtime { Runtime }
    impl Runtime {
        pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
            let mut future = std::pin::pin!(future);
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            match std::future::Future::poll(future.as_mut(), &mut context) {
                std::task::Poll::Ready(value) => value,
                std::task::Poll::Pending => panic!("fixture future unexpectedly pending"),
            }
        }
    }
}
pub struct TrackedStream { index: usize }
impl Iterator for TrackedStream {
    type Item = Result<String, &'static str>;
    fn next(&mut self) -> Option<Self::Item> {
        self.index += 1;
        match self.index {
            1 => Some(Ok("first".to_string())),
            2 if __FAIL_ITEM__ => Some(Err("fixture item error")),
            2 => Some(Ok("second".to_string())),
            _ => None,
        }
    }
}
impl Drop for TrackedStream {
    fn drop(&mut self) {
        assert_eq!(self.index, 3, "stream was not completely drained");
    }
}
mod sample_core {
    pub async fn read_events() -> Result<super::TrackedStream, &'static str> {
        if __FAIL_CALL__ { Err("fixture call error") } else { Ok(super::TrackedStream { index: 0 }) }
    }
}
"#;

fn save(name: &str, suffix: &str, content: &[u8]) {
    let Some(root) = std::env::var_os("ALEF_TEST_STREAM_ARTIFACTS") else {
        return;
    };
    let directory = PathBuf::from(root);
    assert!(directory.is_absolute(), "test artifact directory must be absolute");
    std::fs::create_dir_all(&directory).expect("create test artifact directory");
    std::fs::write(directory.join(format!("{name}.{suffix}")), content).expect("write test artifact");
}

fn compile(name: &str, source: &str, directory: &Path, runtime_macro: Option<&Path>) -> PathBuf {
    let path = directory.join("generated.rs");
    let executable = directory.join(format!("generated{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&path, source).expect("write emitted Rust");
    let mut command = Command::new("rustc");
    if let Some(path) = runtime_macro {
        command.arg("--extern").arg(format!("tokio={}", path.display()));
    }
    let compiler = command
        .args(["--edition=2024", "--test", "--deny=warnings"])
        .arg(&path)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("invoke committed Rust compiler");
    save(name, "rs", source.as_bytes());
    save(name, "compiler.stderr", &compiler.stderr);
    assert!(
        compiler.status.success(),
        "actual generated Rust must compile under denied warnings:\n{}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    executable
}

pub(super) enum FailurePoint {
    None,
    Call,
    Item,
}

pub(super) fn compile_and_run(name: &str, emitted: &str, failure: FailurePoint, expected_error: Option<&str>) {
    let directory = tempfile::tempdir().expect("compiler fixture directory");
    let runtime = RUNTIME
        .replace(
            "__FAIL_ITEM__",
            if matches!(failure, FailurePoint::Item) {
                "true"
            } else {
                "false"
            },
        )
        .replace(
            "__FAIL_CALL__",
            if matches!(failure, FailurePoint::Call) {
                "true"
            } else {
                "false"
            },
        );
    let source = format!("{runtime}\nuse sample_core::read_events;\n{emitted}");
    let executable = compile(name, &source, directory.path(), None);
    let execution = Command::new(executable)
        .args(["--exact", "test_stream_multiple_choices", "--nocapture"])
        .output()
        .expect("run actual compiled fixture");
    save_execution(name, &execution);
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&execution.stdout),
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(message.contains("running 1 test"), "{message}");
    if let Some(expected) = expected_error {
        assert!(!execution.status.success(), "expected stream failure");
        assert!(message.contains(expected), "{message}");
    } else {
        assert!(execution.status.success(), "{message}");
        assert!(message.contains("1 passed; 0 failed"), "{message}");
    }
}

fn save_execution(name: &str, output: &Output) {
    save(name, "runtime.stdout", &output.stdout);
    save(name, "runtime.stderr", &output.stderr);
}

// ~keep The identity attribute leaves the generated async main intact for our explicit test runner.
fn runtime_macro(directory: &Path) -> PathBuf {
    let path = directory.join("tokio.rs");
    std::fs::write(
        &path,
        r#"
extern crate proc_macro;
#[proc_macro_attribute]
pub fn main(_arguments: proc_macro::TokenStream, input: proc_macro::TokenStream) -> proc_macro::TokenStream { input }
"#,
    )
    .expect("write runtime attribute facade");
    let library = directory.join(format!(
        "{}tokio{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let compiler = Command::new("rustc")
        .args(["--edition=2024", "--crate-type=proc-macro", "--deny=warnings"])
        .arg(&path)
        .arg("-o")
        .arg(&library)
        .output()
        .expect("compile runtime attribute facade");
    assert!(
        compiler.status.success(),
        "{}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    library
}

pub(super) fn compile_snippet_and_run(emitted: &str) {
    compile_snippet("docs_snippet", emitted, RUNTIME);
}

pub(super) fn compile_snippet_with_input(emitted: &str, expected: &str) {
    let runtime = RUNTIME.replace("read_events()", "read_events(input: &str)").replace(
        "        if __FAIL_CALL__",
        &format!("        assert_eq!(input, {expected:?}, \"generated docs changed literal input\");\n        if __FAIL_CALL__"),
    );
    compile_snippet("docs_multiline_input", emitted, &runtime);
}

fn compile_snippet(name: &str, emitted: &str, runtime: &str) {
    let directory = tempfile::tempdir().expect("snippet compiler fixture directory");
    let attribute = runtime_macro(directory.path());
    let runtime = runtime
        .replace("__FAIL_ITEM__", "false")
        .replace("__FAIL_CALL__", "false");
    let source = format!("{runtime}\n{emitted}\n#[test] fn snippet_runs() {{ common::runtime().block_on(main()); }}");
    let executable = compile(name, &source, directory.path(), Some(&attribute));
    let execution = Command::new(executable)
        .args(["--exact", "snippet_runs", "--nocapture"])
        .output()
        .expect("run actual docs snippet");
    save_execution(name, &execution);
    let stdout = String::from_utf8_lossy(&execution.stdout);
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(
        stdout.contains("running 1 test") && stdout.contains("1 passed; 0 failed"),
        "{stdout}"
    );
    assert!(stdout.contains(r#"["first", "second"]"#), "{stdout}");
}
