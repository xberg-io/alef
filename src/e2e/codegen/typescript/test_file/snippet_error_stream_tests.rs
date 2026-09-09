use super::snippet::{SnippetContext, render_snippet_body};
use crate::core::ir::TypeDef;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn render(language: &str, streaming: bool, typed: bool) -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "deferred_error", "description": "Report stream errors", "input": {},
        "docs": {"topic": "streaming"}, "assertions": [{"type": "error"}]
    }))
    .expect("error fixture");
    let streaming = if streaming {
        serde_json::json!({"enabled": true, "item_type": "Chunk"})
    } else {
        serde_json::json!(false)
    };
    let config: E2eConfig = serde_json::from_value(serde_json::json!({
        "call": {"function": "stream", "module": "./fixture", "result_var": "response",
            "async": true, "streaming": streaming}
    }))
    .expect("stream configuration");
    let types = if typed {
        vec![TypeDef {
            name: "Chunk".into(),
            ..TypeDef::default()
        }]
    } else {
        vec![]
    };
    render_snippet_body(SnippetContext {
        lang: language,
        fixture: &fixture,
        module: "./fixture",
        client_factory: Some("createClient"),
        e2e_config: &config,
        type_defs: &types,
        enums: &[],
        functions: &[],
        wasm_type_prefix: "Wasm",
        config: &Default::default(),
    })
}

#[test]
fn node_error_stream_is_consumed_inside_the_error_handler() {
    let body = render("node", true, true);
    let start = body.find("const response = await client.stream(").expect(&body);
    let consume = body.find("for await (const responseChunk of response)").expect(&body);
    let catch = body.find("catch (error)").expect(&body);
    assert!(start < consume && consume < catch, "{body}");
    assert!(!body.contains(".free()"), "Node objects have no manual free: {body}");
}

#[test]
fn wasm_error_stream_drives_next_and_cleans_up_before_catching() {
    let body = render("wasm", true, true);
    let next = body
        .find("const responseChunk: WasmChunk | null = await response.next();")
        .expect(&body);
    let chunk_free = body.find("responseChunk.free();").expect(&body);
    let iterator_free = body.find("response.free();").expect(&body);
    let catch = body.find("catch (error)").expect(&body);
    let client_free = body.find("client.free();").expect(&body);
    assert!(body.contains("if (responseChunk === null) break;"), "{body}");
    assert!(
        next < chunk_free && chunk_free < iterator_free && iterator_free < catch && catch < client_free,
        "{body}"
    );
}

#[test]
fn untyped_wasm_error_stream_only_releases_the_iterator() {
    let body = render("wasm", true, false);
    assert!(body.contains("const responseChunk = await response.next();"), "{body}");
    assert!(body.contains("response.free();"), "{body}");
    assert!(!body.contains("responseChunk.free();"), "{body}");
}

#[test]
fn ordinary_error_calls_keep_their_existing_catch_and_client_cleanup() {
    for language in ["node", "wasm"] {
        let body = render(language, false, true);
        assert!(body.contains("await client.stream("), "{body}");
        assert!(body.contains("catch (error)"), "{body}");
        assert!(
            !body.contains("response.next()") && !body.contains("for await"),
            "{body}"
        );
        assert_eq!(body.contains("client.free();"), language == "wasm", "{body}");
    }
}

fn run_tool(mut command: std::process::Command) -> crate::snippets::validators::CapturedStreams {
    crate::snippets::validators::run_command_streams(&mut command, 30)
        .expect("explicit stream oracle requires installed tsc and Node within 30 seconds")
}

fn runtime_error_is_caught(language: &str) {
    let directory = tempfile::tempdir().expect("private external stream fixture");
    let method = if language == "node" { NODE_METHOD } else { WASM_METHOD };
    let fixture = format!("{FIXTURE_PREFIX}\n{method}\n}}; }}\n");
    std::fs::write(directory.path().join("fixture.ts"), fixture).expect("fixture");
    std::fs::write(directory.path().join("snippet.ts"), render(language, true, true)).expect("real rendered source");
    std::fs::write(directory.path().join("runner.cjs"), RUNNER).expect("runtime assertions");
    let mut command = crate::core::tool_command("tsc");
    command.current_dir(directory.path()).args([
        "--strict",
        "--target",
        "ES2022",
        "--module",
        "Node16",
        "--moduleResolution",
        "Node16",
        "--outDir",
        "out",
        "snippet.ts",
        "fixture.ts",
    ]);
    let output = run_tool(command);
    assert!(
        output.success,
        "TypeScript must compile before runtime proof: {}{}",
        output.stdout, output.stderr
    );
    let mut command = crate::core::tool_command("node");
    command.current_dir(directory.path()).arg("runner.cjs").arg(language);
    let output = run_tool(command);
    assert!(
        output.success,
        "consumption error must reach snippet catch: {}{}",
        output.stdout, output.stderr
    );
    assert_eq!(output.stdout, format!("PASS {language}\n"));
}

#[test]
#[ignore = "requires installed tsc and Node; run the error-stream runtime oracle explicitly"]
fn actual_node_error_on_consumption_reaches_catch() {
    runtime_error_is_caught("node");
}

#[test]
#[ignore = "requires installed tsc and Node; run the error-stream runtime oracle explicitly"]
fn actual_wasm_error_on_consumption_releases_resources_and_reaches_catch() {
    runtime_error_is_caught("wasm");
}

const FIXTURE_PREFIX: &str = r#"
export const events: string[] = [];
export class WasmChunk { free() { events.push('chunk:free'); } }
export function createClient(_key: string) {
    events.push('created');
    return {
        free() { events.push('client:free'); },
"#;

const NODE_METHOD: &str = r#"
    async *stream() {
        try {
            events.push('next0');
            yield new WasmChunk();
            events.push('next1');
            throw new Error('deferred failure');
        } finally { events.push('iterator:closed'); }
    }
"#;

const WASM_METHOD: &str = r#"
    stream() {
        let position = 0;
        return {
            async next(): Promise<WasmChunk | null> {
                events.push(`next${position}`);
                if (position++ === 0) return new WasmChunk();
                throw new Error('deferred failure');
            },
            free() { events.push('iterator:free'); }
        };
    }
"#;

const RUNNER: &str = r#"
const assert = require('node:assert/strict');
const fixture = require('./out/fixture.js');
const language = process.argv[2];
let unhandled;
process.on('unhandledRejection', error => { unhandled = error.message; });
console.error = value => { fixture.events.push(`caught:${value}`); };
process.once('beforeExit', () => {
    const expected = language === 'node'
        ? ['created', 'next0', 'next1', 'iterator:closed', 'caught:Error: deferred failure']
        : ['created', 'next0', 'chunk:free', 'next1', 'iterator:free', 'caught:Error: deferred failure', 'client:free'];
    assert.deepEqual(fixture.events, expected, 'deferred error consumption and cleanup sequence');
    assert.equal(unhandled, undefined, 'error escaped snippet catch');
    process.stdout.write(`PASS ${language}\n`);
});
require('./out/snippet.js');
"#;
