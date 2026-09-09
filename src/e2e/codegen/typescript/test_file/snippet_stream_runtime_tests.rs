use super::snippet::{SnippetContext, render_snippet_body};
use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::path::Path;

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty,
        ..FieldDef::default()
    }
}

fn render() -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "ownership", "description": "Show usage details", "input": {},
        "docs": {"topic": "streaming"},
        "assertions": [{"type": "not_empty", "field": "usage.details.total_tokens"}]
    }))
    .expect("fixture");
    let config: E2eConfig = serde_json::from_value(serde_json::json!({
        "call": {"function": "stream", "module": "./fixture", "result_var": "response",
            "async": true, "streaming": {"enabled": true, "item_type": "Chunk"}},
        "fields_optional": ["usage"]
    }))
    .expect("call configuration");
    let types = vec![
        TypeDef {
            name: "Chunk".into(),
            fields: vec![field(
                "usage",
                TypeRef::Optional(Box::new(TypeRef::Named("Usage".into()))),
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Usage".into(),
            fields: vec![field("details", TypeRef::Named("Details".into()))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Details".into(),
            fields: vec![field("total_tokens", TypeRef::Primitive(PrimitiveType::U64))],
            ..TypeDef::default()
        },
    ];
    render_snippet_body(SnippetContext {
        lang: "wasm",
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

fn run(mut command: std::process::Command) -> crate::snippets::validators::CapturedStreams {
    crate::snippets::validators::run_command_streams(&mut command, 30)
        .expect("this explicit oracle requires installed TypeScript and Node within a 30-second deadline")
}

fn compile(directory: &Path, source: &str) {
    std::fs::write(directory.join("snippet.ts"), source).expect("snippet source");
    std::fs::write(directory.join("fixture.ts"), FIXTURE).expect("external fixture source");
    std::fs::write(directory.join("runner.cjs"), RUNNER).expect("runtime assertions");
    let mut command = crate::core::tool_command("tsc");
    command.current_dir(directory).args([
        "--strict",
        "--noUncheckedIndexedAccess",
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
    let output = run(command);
    assert!(
        output.success,
        "generated TypeScript failed: {}{}",
        output.stdout, output.stderr
    );
}

fn execute(directory: &Path, scenario: &str) -> crate::snippets::validators::CapturedStreams {
    let mut command = crate::core::tool_command("node");
    command.current_dir(directory).arg("runner.cjs").arg(scenario);
    run(command)
}

#[test]
#[ignore = "requires installed tsc and Node; run this runtime oracle explicitly"]
fn generated_wasm_stream_releases_owned_objects_on_all_exit_paths() {
    let directory = tempfile::tempdir().expect("private fixture");
    compile(directory.path(), &render());
    let scenarios = ["success", "next", "presentation", "getter", "absent"];
    assert_eq!(scenarios.len(), 5);
    for scenario in scenarios {
        let output = execute(directory.path(), scenario);
        assert!(output.success, "{scenario}: {}{}", output.stdout, output.stderr);
        assert_eq!(output.stdout, format!("PASS {scenario}\n"));
    }
}

#[test]
#[ignore = "requires installed tsc and Node; run this runtime oracle explicitly"]
fn runtime_oracle_rejects_private_missing_cleanup_mutation() {
    let directory = tempfile::tempdir().expect("private negative-control fixture");
    let source = render();
    let cleanup = "responseChunkShow0Resource0?.free();";
    assert_eq!(
        source.matches(cleanup).count(),
        1,
        "mutation must target exactly one real cleanup"
    );
    compile(directory.path(), &source.replace(cleanup, ""));
    let output = execute(directory.path(), "success");
    assert!(!output.success, "missing cleanup falsely passed");
    assert!(
        output.stderr.contains("cleanup event sequence"),
        "negative control failed for the wrong reason: {}",
        output.stderr
    );
}

const FIXTURE: &str = r#"
export const events: string[] = [];
let scenario = "success";
export function configure(value: string) { scenario = value; }
class WasmDetails {
    get totalTokens() { return 37; }
    free() { events.push("details:free"); }
}
class WasmUsage {
    get details() {
        events.push("details:get");
        if (scenario === "getter") throw new Error("getter failed");
        return new WasmDetails();
    }
    free() { events.push("usage:free"); }
}
export class WasmChunk {
    get usage(): WasmUsage | undefined {
        events.push("usage:get");
        return scenario === "absent" ? undefined : new WasmUsage();
    }
    free() { events.push("chunk:free"); }
}
export function createClient(_key: string) {
    events.push("client:new");
    return {
        stream() {
            let position = 0;
            return {
                async next(): Promise<WasmChunk | null> {
                    events.push(`next:${position}`);
                    if (position++ === 0) return new WasmChunk();
                    if (scenario === "next") throw new Error("next failed");
                    return null;
                },
                free() { events.push("iterator:free"); }
            };
        },
        free() { events.push("client:free"); }
    };
}
"#;

const RUNNER: &str = r#"
const assert = require('node:assert/strict');
const fixture = require('./out/fixture.js');
const scenario = process.argv[2];
fixture.configure(scenario);
let failure;
process.on('unhandledRejection', error => { failure = error.message; });
console.log = value => {
    fixture.events.push(`print:${value}`);
    if (scenario === 'presentation') throw new Error('presentation failed');
};
process.once('beforeExit', () => {
    const prefix = ['client:new', 'next:0', 'usage:get'];
    const suffix = ['iterator:free', 'client:free'];
    const expected = scenario === 'getter'
        ? [...prefix, 'details:get', 'usage:free', 'chunk:free', ...suffix]
        : scenario === 'absent'
        ? [...prefix, 'print:undefined', 'chunk:free', 'next:1', ...suffix]
        : [...prefix, 'details:get', 'print:37', 'details:free', 'usage:free', 'chunk:free',
            ...(scenario === 'presentation' ? [] : ['next:1']), ...suffix];
    assert.deepEqual(fixture.events, expected, 'cleanup event sequence');
    assert.equal(failure, ['getter', 'next', 'presentation'].includes(scenario) ? `${scenario} failed` : undefined,
        'original operation error must survive cleanup');
    process.stdout.write(`PASS ${scenario}\n`);
});
require('./out/snippet.js');
"#;
