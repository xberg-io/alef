//! Node error shape and ESM named exports.
//!
//! The first group checks the generated Rust and `.d.ts` text. The second group builds a real
//! package directory from the generated files and runs it in Node: an ESM named import must
//! resolve, and an error thrown by a call must carry its kind, HTTP status and retry delay.

use alef::backends::napi::NapiBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;
use std::path::Path;
use std::process::Command;

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "sample-pkg"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn function(name: &str, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        return_type: TypeRef::String,
        is_async,
        error_type: Some("ProcessError".to_string()),
        ..Default::default()
    }
}

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "test_lib::Client".to_string(),
            is_opaque: true,
            is_clone: true,
            methods: vec![method("get_value", TypeRef::String)],
            ..Default::default()
        }],
        functions: vec![
            function("failing", true),
            function("failing_sync", false),
            function("plain", true),
            function("open_stream", true),
        ],
        errors: vec![ErrorDef {
            exception_base: None,
            name: "ProcessError".to_string(),
            rust_path: "test_lib::ProcessError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "RateLimited".to_string(),
                    error_code: None,
                    fields: vec![],
                    doc: String::new(),
                    message_template: Some("rate limited".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "Timeout".to_string(),
                    error_code: None,
                    fields: vec![],
                    doc: String::new(),
                    message_template: Some("timeout".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
            ],
            doc: String::new(),
            methods: vec![
                method("status_code", TypeRef::Primitive(PrimitiveType::U16)),
                method("is_transient", TypeRef::Primitive(PrimitiveType::Bool)),
                method("error_type", TypeRef::String),
                method("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..Default::default()
    }
}

fn lib_rs() -> String {
    let files = NapiBackend.generate_bindings(&api(), &config()).unwrap();
    files
        .into_iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs")
        .content
}

fn stub_files() -> Vec<GeneratedFile> {
    NapiBackend.generate_type_stubs(&api(), &config()).unwrap()
}

fn stub(name: &str) -> String {
    stub_files()
        .into_iter()
        .find(|f| f.path.ends_with(name))
        .unwrap_or_else(|| panic!("{name} must be generated"))
        .content
}

#[test]
fn every_call_site_converts_errors_through_the_typed_converter() {
    let lib = lib_rs();
    assert!(
        lib.contains("fn alef_napi_err<E: std::fmt::Display + 'static>(e: E) -> napi::Error"),
        "the shared conversion must exist:\n{lib}"
    );
    assert!(
        lib.contains("boxed.downcast::<test_lib::ProcessError>()")
            && lib.contains("return process_error_to_napi_err(*typed)"),
        "the shared conversion must route ProcessError to its typed converter:\n{lib}"
    );
    assert!(
        lib.contains(".map_err(alef_napi_err)"),
        "call sites must use the shared conversion:\n{lib}"
    );
    assert!(
        !lib.contains(".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"),
        "no call site may build a bare GenericFailure from a core error:\n{lib}"
    );
}

#[test]
fn the_converter_serialises_the_properties_the_d_ts_declares() {
    let lib = lib_rs();
    let dts = stub("index.d.ts");
    assert!(
        dts.contains("export interface ThrownProcessError extends Error {"),
        "the thrown error shape must be declared:\n{dts}"
    );
    for (name, ts_type) in [
        ("kind", "\"RateLimited\" | \"Timeout\" | \"Unknown\""),
        ("errorCode", "number"),
        ("statusCode", "number"),
        ("isTransient", "boolean"),
        ("errorType", "string"),
        ("retryAfter", "number | null"),
    ] {
        assert!(
            dts.contains(&format!("  {name}: {ts_type}")),
            "d.ts must declare {name}: {ts_type}:\n{dts}"
        );
        assert!(
            lib.contains(&format!("\\\"{name}\\\":{{}}")),
            "the converter must serialise {name} under the same name:\n{lib}"
        );
    }
    assert!(
        lib.contains("e.retry_after().map(|d| d.as_millis() as i64)"),
        "retryAfter must be whole milliseconds:\n{lib}"
    );
    assert!(
        lib.contains("alef:error-info:"),
        "the payload must carry the marker binding.cjs strips:\n{lib}"
    );
}

#[test]
fn the_esm_wrapper_names_every_runtime_export() {
    let mjs = stub("index.mjs");
    for name in ["Client", "failing", "failingSync", "openStream"] {
        assert!(
            mjs.contains(&format!("export const {name} = binding.{name};")),
            "index.mjs must export {name}:\n{mjs}"
        );
    }
    assert!(mjs.contains("import binding from \"./index.js\";"), "{mjs}");
    assert!(mjs.contains("export default binding;"), "{mjs}");
    assert!(
        !mjs.contains("ThrownProcessError"),
        "type-only declarations have no runtime value:\n{mjs}"
    );
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

const FAKE_NATIVE: &str = r#"
const info = { kind: "RateLimited", errorCode: 101, statusCode: 429, retryAfter: 7000, isTransient: true, errorType: "rate_limited" };
// Same shape the napi runtime produces: a plain Error whose `cause` carries the marked payload.
function thrown(message) {
  const error = new Error(message);
  error.cause = new Error("alef:error-info:" + JSON.stringify(info));
  return error;
}
class Client {
  getValue() { throw thrown("method failed"); }
  async chat() { throw thrown("async method failed"); }
}
async function failing() { throw thrown("slow down"); }
function failingSync() { throw thrown("sync failure"); }
async function plain() { throw new Error("no payload"); }
async function openStream(mode) {
  if (mode === "open-error") throw thrown("open failed");
  const iterator = {};
  iterator[Symbol.asyncIterator] = function () {
    let count = 0;
    return {
      next() {
        count += 1;
        return count === 1 ? Promise.resolve({ value: "first", done: false }) : Promise.reject(thrown("late failure"));
      },
    };
  };
  return iterator;
}
module.exports = { Client, failing, failingSync, plain, openStream };
"#;

const PRELOAD: &str = r#"
// Stands in for dlopen: every `.node` file the generated loader tries resolves to the fake.
require.extensions[".node"] = (module) => { module.exports = require("./fake-native.cjs"); };
"#;

const SCRIPT: &str = r#"
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { Client, failing, failingSync, plain, openStream } from "sample-pkg";

function check(error) {
  assert.equal(error.kind, "RateLimited");
  assert.equal(error.statusCode, 429);
  assert.equal(error.retryAfter, 7000);
  assert.equal(error.isTransient, true);
  assert.equal(error.errorType, "rate_limited");
  assert.equal(error.errorCode, 101);
  assert.equal(error.cause, undefined);
  return true;
}

await assert.rejects(failing(), (e) => check(e) && e.message === "slow down");
assert.throws(() => failingSync(), check);
await assert.rejects(new Client().chat(), check);
assert.throws(() => new Client().getValue(), check);

// A stream that fails to open rejects the call. A stream that fails later fails on read.
await assert.rejects(openStream("open-error"), check);
const stream = await openStream("ok");
const seen = [];
await assert.rejects(
  (async () => { for await (const item of stream) seen.push(item); })(),
  check,
);
assert.deepEqual(seen, ["first"]);

// Positive twin: an error without a payload passes through untouched.
await assert.rejects(plain(), (e) => e.message === "no payload" && e.kind === undefined);

// The CommonJS entry decorates the same way.
const cjs = createRequire(import.meta.url)("sample-pkg");
await assert.rejects(cjs.failing(), check);
assert.equal(typeof cjs.Client, "function");
console.log("ok");
"#;

fn run_node(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("node")
        .current_dir(root)
        .args(args)
        .output()
        .expect("node runs")
}

#[test]
fn a_node_package_built_from_the_generated_files_resolves_named_imports_and_decorates_errors() {
    assert!(node_available(), "node must be on PATH for the Node runtime test");

    let config = config();
    let scaffold = alef::scaffold::scaffold(&api(), &config, &[Language::Node]).unwrap();
    let scaffolded = |name: &str| -> String {
        scaffold
            .iter()
            .find(|f| f.path == Path::new("crates/test-lib-node").join(name))
            .unwrap_or_else(|| panic!("scaffold must emit {name}"))
            .content
            .clone()
    };
    let index_js = scaffolded("index.js");
    let package_json = scaffolded("package.json");

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let package = "node_modules/sample-pkg";
    write(root, &format!("{package}/package.json"), &package_json);
    write(root, &format!("{package}/index.js"), &index_js);
    for file in stub_files() {
        let name = file.path.file_name().unwrap().to_string_lossy().to_string();
        write(root, &format!("{package}/{name}"), &file.content);
    }
    write(root, "fake-native.cjs", FAKE_NATIVE);
    write(root, "preload.cjs", PRELOAD);
    write(root, "check.mjs", SCRIPT);
    // The generated loader looks for one `.node` file per platform; an empty file for each makes
    // the first candidate resolve, and the preload hook above answers the load.
    for token in index_js.split('"') {
        if let Some(name) = token.strip_prefix("./").filter(|n| n.ends_with(".node")) {
            write(root, &format!("{package}/{name}"), "");
        }
    }

    let output = run_node(root, &["--require", "./preload.cjs", "check.mjs"]);
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("ok"),
        "node run failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
