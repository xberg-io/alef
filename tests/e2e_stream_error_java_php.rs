use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, java::JavaCodegen, php::PhpCodegen};
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn render<C: E2eCodegen>(codegen: &C, streaming: bool, declared: bool) -> String {
    let source = format!(
        r#"
[workspace]
languages = ["java", "php"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "stream_text"
module = "MyLib"
result_var = "events"
streaming = {streaming}
args = []
"#
    );
    let config: NewAlefConfig = toml::from_str(&source).expect("configuration parses");
    let e2e = config.crates[0].e2e.clone().expect("e2e configuration");
    let resolved = config.resolve().expect("configuration resolves").remove(0);
    let groups = vec![FixtureGroup {
        category: "streaming".into(),
        fixtures: vec![Fixture {
            id: "mid_stream_error".into(),
            category: Some("streaming".into()),
            assertions: vec![Assertion {
                assertion_type: "error".into(),
                value: declared.then(|| serde_json::json!("Rate limit reached")),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }],
    }];
    let files = codegen
        .generate(&groups, &e2e, &resolved, &[], &[], &[], &[])
        .expect("generation");
    assert!(!files.is_empty(), "generation must examine a real fixture");
    files
        .into_iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn java_stream_error_consumes_iterator_inside_the_error_assertion() {
    let output = render(&JavaCodegen, true, true);
    let assertion = output.find("assertThrows(Exception.class").expect("error assertion");
    let body = &output[assertion..];
    let end = body.find("        });").expect("lambda ends");
    let body = &body[..end];
    assert!(
        body.contains(".iterator()"),
        "stream must be opened inside assertion: {body}"
    );
    assert!(
        body.contains(".hasNext()") && body.contains(".next()"),
        "stream must be consumed: {body}"
    );
}

#[test]
fn java_non_stream_error_keeps_the_direct_call() {
    let output = render(&JavaCodegen, false, false);
    assert!(output.contains("assertThrows(Exception.class"), "{output}");
    assert!(!output.contains(".iterator()"), "{output}");
}

#[test]
fn php_stream_error_consumes_iterator_for_generic_and_declared_errors() {
    for declared in [false, true] {
        let output = render(&PhpCodegen, true, declared);
        assert!(
            output.contains("iterator_to_array($events)"),
            "stream must be consumed: {output}"
        );
        if declared {
            let consume = output.find("iterator_to_array($events)").expect("stream consumed");
            let catch = output.find("} catch (\\Exception $e)").expect("error observed");
            assert!(consume < catch, "lazy error must be caught: {output}");
        } else {
            assert!(
                output.contains("$this->expectException(\\Exception::class);"),
                "{output}"
            );
        }
    }
}

#[test]
fn php_non_stream_error_keeps_the_direct_call() {
    let output = render(&PhpCodegen, false, false);
    assert!(
        output.contains("$this->expectException(\\Exception::class);"),
        "{output}"
    );
    assert!(!output.contains("iterator_to_array($events)"), "{output}");
}

fn emitted_method(output: &str, signature: &str) -> String {
    let start = output.find(signature).expect("generated test method");
    let rest = &output[start..];
    let open = rest.find('{').expect("method opens");
    let mut depth = 0;
    for (offset, character) in rest[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[..open + offset + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("generated method must close: {rest}");
}

fn checked(command: &mut std::process::Command) -> std::process::Output {
    let output = command.output().expect("runtime tool starts");
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn emitted_java_assertion_observes_lazy_errors_and_rejects_clean_completion() {
    let directory = tempfile::tempdir().expect("runtime directory");
    for declared in [false, true] {
        let method = emitted_method(&render(&JavaCodegen, true, declared), "void testMidStreamError()");
        let source = format!(
            r#"
import java.util.*;
public class StreamProbe {{
    interface Throwing {{ void run() throws Exception; }}
    static Exception assertThrows(Class<?> expected, Throwing call) throws Exception {{
        try {{ call.run(); }} catch (Exception error) {{ return error; }}
        throw new AssertionError("stream completed without an exception");
    }}
    static void assertTrue(boolean value, Object... message) {{
        if (!value) throw new AssertionError("wrong error value");
    }}
    {method}
    public static void main(String[] args) throws Exception {{
        Mylib.mode = Integer.parseInt(args[0]);
        new StreamProbe().testMidStreamError();
        if (Mylib.mode == 1 && Mylib.items != 2) throw new AssertionError("did not read two chunks");
        System.out.println("observed=" + Mylib.items);
    }}
}}
class Mylib {{
    static int mode;
    static int items;
    static Iterable<Integer> streamText() {{
        if (mode == 0) throw new RuntimeException("Rate limit reached");
        return () -> new Iterator<Integer>() {{
            public boolean hasNext() {{
                if (items < 2) return true;
                if (mode == 3) return false;
                throw new RuntimeException(mode == 2 ? "unrelated error" : "Rate limit reached");
            }}
            public Integer next() {{ return ++items; }}
        }};
    }}
}}
"#
        );
        let path = directory.path().join("StreamProbe.java");
        std::fs::write(&path, source).expect("write emitted runtime probe");
        checked(std::process::Command::new("javac").arg(&path));
        for mode in 0..4 {
            let output = std::process::Command::new("java")
                .args([
                    "-cp",
                    directory.path().to_str().expect("runtime path"),
                    "StreamProbe",
                    &mode.to_string(),
                ])
                .output()
                .expect("Java runtime");
            assert_eq!(
                output.status.success(),
                mode < 2 || (mode == 2 && !declared),
                "declared={declared}, mode={mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
#[ignore = "requires the PHP executable"]
fn emitted_php_assertion_observes_lazy_errors_and_rejects_clean_completion() {
    let directory = tempfile::tempdir().expect("runtime directory");
    for declared in [false, true] {
        let method = emitted_method(
            &render(&PhpCodegen, true, declared),
            "public function test_mid_stream_error()",
        );
        let source = format!(
            r#"<?php
class Mylib {{
    public static int $mode;
    public static int $items = 0;
    public static function streamText(): Traversable {{
        if (self::$mode === 0) throw new Exception('Rate limit reached');
        return (function () {{
            yield ++self::$items;
            yield ++self::$items;
            if (self::$mode === 3) return;
            throw new Exception(self::$mode === 2 ? 'unrelated error' : 'Rate limit reached');
        }})();
    }}
}}
class StreamProbe {{
    public ?string $expected = null;
    public function expectException(string $type): void {{ $this->expected = $type; }}
    public function assertInstanceOf(string $type, mixed $value): void {{
        if (!($value instanceof $type)) throw new AssertionError('missing exception');
    }}
    public function assertTrue(bool $value, string $message): void {{
        if (!$value) throw new AssertionError($message);
    }}
    {method}
}}
Mylib::$mode = (int)$argv[1];
$probe = new StreamProbe();
try {{
    $probe->test_mid_stream_error();
    if ($probe->expected !== null) throw new AssertionError('stream completed without an exception');
}} catch (Exception $error) {{
    if ($probe->expected === null || !($error instanceof $probe->expected)) throw $error;
}}
if (Mylib::$mode === 1 && Mylib::$items !== 2) throw new AssertionError('did not read two chunks');
echo 'observed=' . Mylib::$items;
"#
        );
        let path = directory.path().join("stream_probe.php");
        std::fs::write(&path, source).expect("write emitted runtime probe");
        checked(std::process::Command::new("php").arg("-l").arg(&path));
        for mode in 0..4 {
            let output = std::process::Command::new("php")
                .arg(&path)
                .arg(mode.to_string())
                .output()
                .expect("PHP runtime");
            assert_eq!(
                output.status.success(),
                mode < 2 || (mode == 2 && !declared),
                "declared={declared}, mode={mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
