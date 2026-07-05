use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::SwiftFirstClassMap;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::project::SWIFT_FORMAT_IGNORE_DIRECTIVE;
use super::{http, test_method};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module_name: &str,
    class_name: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    result_is_simple: bool,
    client_factory: Option<&str>,
    swift_first_class_map: &SwiftFirstClassMap,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    has_http_fixtures: bool,
    enums: &[crate::core::ir::EnumDef],
) -> String {
    // Detect whether any fixture in this group uses a file_path or bytes arg — if so
    // the test class chdir's to <repo>/test_documents at setUp time so the
    // fixture-relative paths in test bodies (e.g. "docx/fake.docx") resolve correctly.
    // The Swift binding's `extractBytes`/`extractFile` e2e wrappers consult
    // `FIXTURES_DIR` first, otherwise resolve against the current directory.
    // Mirrors the Ruby/Python conftest pattern that chdirs to test_documents.
    let needs_chdir = fixtures.iter().any(|f| {
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        call_config
            .args
            .iter()
            .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
    });

    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push_str(SWIFT_FORMAT_IGNORE_DIRECTIVE);
    let _ = writeln!(out, "import XCTest");
    let _ = writeln!(out, "import Foundation");
    // URLSession et al. are in FoundationNetworking on Linux (swift-corelibs-foundation)
    // but in plain Foundation on Apple platforms. The canImport guard makes the import
    // a no-op where the submodule is absent.
    let _ = writeln!(out, "#if canImport(FoundationNetworking)");
    let _ = writeln!(out, "import FoundationNetworking");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out, "import {module_name}");
    // RustBridge is needed for low-level types (RustVec<UInt8>, RustString) constructed
    // in bytes/string argument setup. It is exposed as a product by the swift package
    // for e2e test use.
    let _ = writeln!(out, "import RustBridge");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// E2e tests for category: {category}.");
    let _ = writeln!(out, "final class {class_name}: XCTestCase {{");

    // Always emit a setUp that spawns the harness and optionally chdirs.
    let _ = writeln!(out, "    override class func setUp() {{");
    let _ = writeln!(out, "        super.setUp()");

    // Inject environment variables from e2e.env, sorted alphabetically.
    if !e2e_config.env.is_empty() {
        let mut keys: Vec<_> = e2e_config.env.keys().collect();
        keys.sort();
        for key in keys {
            let value = &e2e_config.env[key];
            let _ = writeln!(
                out,
                "        _ = \"{}\".withCString {{ val in",
                value.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let _ = writeln!(out, "            \"{}\".withCString {{ key in", key);
            let _ = writeln!(out, "                setenv(key, val, 0)");
            let _ = writeln!(out, "            }}");
            let _ = writeln!(out, "        }}");
        }
    }

    // Spawn the harness subprocess if SUT_URL is not already set.
    // Only emit when there are HTTP fixtures; consumers without HTTP tests
    // don't need the harness.
    if has_http_fixtures {
        let _ = writeln!(
            out,
            "        let _existing = ProcessInfo.processInfo.environment[\"SUT_URL\"]"
        );
        let _ = writeln!(out, "        if _existing == nil {{");
        let _ = writeln!(out, "            let _harness = URL(fileURLWithPath: #filePath)");
        let _ = writeln!(out, "                .deletingLastPathComponent() // <Module>Tests/");
        let _ = writeln!(out, "                .deletingLastPathComponent() // Tests/");
        let _ = writeln!(out, "                .deletingLastPathComponent() // swift_e2e/");
        let _ = writeln!(out, "                .deletingLastPathComponent() // e2e/");
        let _ = writeln!(out, "                .appendingPathComponent(\"swift_e2e\")");
        let _ = writeln!(out, "                .appendingPathComponent(\".build/debug/Harness\")");
        let _ = writeln!(out, "            let proc = Process()");
        let _ = writeln!(out, "            proc.executableURL = _harness");
        let _ = writeln!(out, "            let stdoutPipe = Pipe()");
        let _ = writeln!(out, "            proc.standardOutput = stdoutPipe");
        let _ = writeln!(out, "            proc.standardInput = Pipe()");
        let _ = writeln!(out, "            do {{");
        let _ = writeln!(out, "                try proc.run()");
        let _ = writeln!(out, "            }} catch {{");
        let _ = writeln!(
            out,
            "                fatalError(\"Failed to start harness: \\(error)\")"
        );
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "            let deadline = Date(timeIntervalSinceNow: 15.0)");
        let _ = writeln!(out, "            var ready = false");
        let host = &e2e_config.harness.host;
        let port = e2e_config.harness.port;
        let _ = writeln!(
            out,
            "            let _probeURL = URL(string: \"http://{}:{}/\")!",
            host, port
        );
        let _ = writeln!(out, "            while Date.now < deadline {{");
        let _ = writeln!(out, "                if proc.isRunning == false {{ break }}");
        let _ = writeln!(out, "                var _probeReq = URLRequest(url: _probeURL)");
        let _ = writeln!(out, "                _probeReq.timeoutInterval = 0.5");
        let _ = writeln!(out, "                let _probeSema = DispatchSemaphore(value: 0)");
        let _ = writeln!(
            out,
            "                let _probeSession = URLSession(configuration: .ephemeral)"
        );
        let _ = writeln!(
            out,
            "                _probeSession.dataTask(with: _probeReq) {{ _, _, _ in _probeSema.signal() }}.resume()"
        );
        let _ = writeln!(
            out,
            "                if _probeSema.wait(timeout: .now() + 0.6) == .timedOut {{"
        );
        let _ = writeln!(out, "                    usleep(100000)");
        let _ = writeln!(out, "                    continue");
        let _ = writeln!(out, "                }}");
        let _ = writeln!(out, "                ready = true");
        let _ = writeln!(out, "                break");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "            if !ready {{");
        let _ = writeln!(out, "                proc.terminate()");
        let _ = writeln!(
            out,
            "                fatalError(\"Harness did not become ready within 15s\")"
        );
        let _ = writeln!(out, "            }}");
        // `ProcessInfo.processInfo.environment` is read-only; use the C `setenv`
        // function to mutate the actual process environment so subsequent
        // `getenv("SUT_URL")` lookups (and Swift's `ProcessInfo` snapshot) see it.
        let _ = writeln!(
            out,
            "            _ = \"http://{}:{}\".withCString {{ url in",
            host, port
        );
        let _ = writeln!(out, "                \"SUT_URL\".withCString {{ key in");
        let _ = writeln!(out, "                    setenv(key, url, 1)");
        let _ = writeln!(out, "                }}");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "        }}");
    }

    if needs_chdir {
        // Chdir once at class setUp so all fixture file_path arguments resolve relative
        // to the repository's test_documents directory.
        //
        // #filePath = <repo>/e2e/swift_e2e/Tests/<Module>E2ETests/<Class>.swift
        // 5 deletingLastPathComponent() calls climb to the repo root before appending
        // "test_documents". Mirrors the Ruby/Python conftest pattern that chdirs to
        // test_documents.
        let _ = writeln!(out, "        let _testDocs = URL(fileURLWithPath: #filePath)");
        let _ = writeln!(out, "            .deletingLastPathComponent() // <Module>Tests/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // Tests/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // swift_e2e/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // e2e/");
        let _ = writeln!(out, "            .deletingLastPathComponent() // <repo root>");
        let _ = writeln!(
            out,
            "            .appendingPathComponent(\"{}\")",
            e2e_config.test_documents_dir
        );
        let _ = writeln!(
            out,
            "        if FileManager.default.fileExists(atPath: _testDocs.path) {{"
        );
        let _ = writeln!(
            out,
            "            FileManager.default.changeCurrentDirectoryPath(_testDocs.path)"
        );
        let _ = writeln!(out, "        }}");
    }

    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);

    for fixture in fixtures {
        if fixture.is_http_test() {
            http::render_http_test_method(&mut out, fixture);
        } else {
            test_method::render_test_method(
                &mut out,
                fixture,
                e2e_config,
                function_name,
                result_var,
                args,
                result_is_simple,
                client_factory,
                swift_first_class_map,
                module_name,
                config,
                type_defs,
                enums,
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
}
