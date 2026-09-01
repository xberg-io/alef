use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    lang: &str,
    pkg_name: &str,
    frb_module_name: &str,
    bridge_class: &str,
    dart_first_class_map: &crate::e2e::field_access::DartFirstClassMap,
    adapters: &[crate::core::config::extras::AdapterConfig],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    // Suppress unused_local_variable: `final result = await api.method(...)` is
    // emitted for every test case; tests that only check for absence of errors
    // do not consume `result`, triggering this dart-analyze warning.
    //
    // Suppress unused_element: helpers like `_fixtureUrl`/`_setEnv`/`_withRetry` are
    // emitted whenever their gating condition (e.g. `has_http_fixtures`) is met for the
    // category, but not every fixture that triggers the gate ends up calling every helper
    // it unlocks, leaving one technically unused in that file.
    out.push_str("// ignore_for_file: unused_local_variable, unused_element\n\n");

    // Check if any fixture needs the http package (HTTP server tests).
    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());

    // Check if any fixture needs Uint8List (trait_bridge byte args/returns).
    let has_batch_byte_items = fixtures.iter().any(|f| {
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        f.resolved_args(call_config).iter().any(|a| {
            a.arg_type == "test_backend" // trait_bridge stubs may use Uint8List in method params
        })
    });

    let file_input_scan = super::super::file_inputs::FileInputScan::new(type_defs, enums);
    // Detect file-backed values, including byte fields nested in typed request objects, so
    // setUpAll can resolve their relative paths from the test-documents directory.
    let needs_chdir = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        file_input_scan.fixture_uses_test_documents(f, call_config)
    });

    // Detect whether any non-HTTP fixture uses a json_object arg that resolves to a JSON array —
    // historically these were materialized via `jsonDecode` at test-run time and cast to
    // `List<String>`. The current emitter routes both handle args and array json_objects
    // through `create<Config>FromJson(json:)` or direct Dart list literals, so `dart:convert`
    // is no longer required for this path. The detection is retained for forward compatibility
    // and to keep the analysis structure stable; the variable is intentionally not consumed.
    let _has_handle_args = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        call_config
            .args
            .iter()
            .any(|a| a.arg_type == "json_object" && resolve_field(&f.input, &a.field).is_array())
    });

    // Non-HTTP fixtures that build a mock-server URL still reference `Platform.environment`
    // (from `dart:io`). This applies to `mock_url` and `mock_url_list` args and to fixtures
    // routed through a `client_factory` (per-call override or per-language override) that
    // derives `_mockUrl` inline. Without this, the generated tests fail to compile with
    // `Error: Undefined name 'Platform'`.
    let lang_client_factory = e2e_config
        .call
        .overrides
        .get(lang)
        .and_then(|o| o.client_factory.as_deref())
        .is_some();
    let has_mock_url_refs = lang_client_factory
        || fixtures.iter().any(|f| {
            // A `$mock_url` placeholder anywhere in the fixture input is rewritten to a
            // `_fixtureUrl(...)` call in the generated test body, so the helper (and the
            // `_fixtureUrls` map) must be emitted even for non-HTTP contract fixtures.
            if serde_json::to_string(&f.input)
                .map(|s| s.contains("$mock_url"))
                .unwrap_or(false)
            {
                return true;
            }
            if f.is_http_test() {
                return false;
            }
            let call_config = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            if call_config
                .args
                .iter()
                .any(|a| a.arg_type == "mock_url" || a.arg_type == "mock_url_list")
            {
                return true;
            }
            call_config
                .overrides
                .get(lang)
                .and_then(|o| o.client_factory.as_deref())
                .is_some()
        });

    let _ = writeln!(out, "import 'package:test/test.dart';");
    // `dart:io` provides HttpClient/SocketException (HTTP fixtures), Platform/Directory
    // (file-path/bytes fixtures requiring chdir), and Platform.environment (mock-url
    // fixtures). Skip the import when none of these are in play — unconditional emission
    // triggers `unused_import` warnings.
    if has_http_fixtures || needs_chdir || has_mock_url_refs {
        let _ = writeln!(out, "import 'dart:io';");
    }
    if has_batch_byte_items {
        let _ = writeln!(out, "import 'dart:typed_data';");
    }
    let _ = writeln!(out, "import 'package:{pkg_name}/{pkg_name}.dart';");
    // RustLib is the flutter_rust_bridge entrypoint; must be initialized before any FRB call.
    // FRB places its generated dart sources under `lib/src/{module_name}_bridge_generated/`,
    // where `module_name` is the snake_cased crate name (independent of the pubspec `name`,
    // which may be a short alias). `RustLib` lives in `frb_generated.dart` and
    // is not re-exported by the FRB barrel `lib.dart`, so we import it directly.
    let _ = writeln!(
        out,
        "import 'package:{pkg_name}/src/{frb_module_name}_bridge_generated/frb_generated.dart' show RustLib;"
    );
    // dart:async provides Completer (HTTP response handling + the mock-server
    // spawn harness, which awaits a Completer for the startup URL line).
    if has_http_fixtures || has_mock_url_refs {
        let _ = writeln!(out, "import 'dart:async';");
    }
    // dart:convert provides jsonDecode for HTTP response parsing and typed JSON arrays
    // deserialization, plus utf8/LineSplitter for decoding the mock-server's startup stdout
    // (MOCK_SERVER_URL= / MOCK_SERVERS=) in the spawn harness. Handle-arg engine construction
    // no longer needs jsonDecode — it routes through `create<Config>FromJson(json:)` which
    // accepts the JSON string directly, so `has_handle_args` is intentionally excluded here
    // to avoid an unused `dart:convert` import.
    // Generic typed json_object arrays (e.g. batch items) materialize via
    // `jsonDecode(r'…')` in the test body, so the import is required whenever
    // any fixture passes a json_object array argument with no element_type
    // element decoding.
    let has_json_array_args = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        f.resolved_args(call_config).iter().any(|a| {
            a.arg_type == "json_object"
                && a.element_type.is_some()
                && a.element_type.as_deref() != Some("String")
                && resolve_field(&f.input, &a.field).is_array()
        })
    });
    if has_http_fixtures || has_mock_url_refs || has_json_array_args {
        let _ = writeln!(out, "import 'dart:convert';");
    }
    // Require dart:ffi for setenv if e2e config has env vars to inject
    if !e2e_config.env.is_empty() {
        let _ = writeln!(out, "import 'dart:ffi';");
        let _ = writeln!(out, "import 'package:ffi/ffi.dart';");
    }
    let _ = writeln!(out);

    // Emit file-level HTTP client and serialization mutex.
    //
    // The shared HttpClient reuses keep-alive connections to minimize TCP overhead.
    // The mutex (_lock) ensures requests are serialized within the file so the
    // connection pool is not exercised concurrently by dart:test's async runner.
    //
    // _withRetry wraps the entire request closure with one automatic retry on
    // transient connection errors (keep-alive connections can be silently closed
    // by the server just as the client tries to reuse them).
    if has_http_fixtures {
        let _ = writeln!(out, "HttpClient _httpClient = HttpClient()..maxConnectionsPerHost = 1;");
        let _ = writeln!(out);
        let _ = writeln!(out, "var _lock = Future<void>.value();");
        let _ = writeln!(out);
        let _ = writeln!(out, "Future<T> _serialized<T>(Future<T> Function() fn) async {{");
        let _ = writeln!(out, "  final current = _lock;");
        let _ = writeln!(out, "  final next = Completer<void>();");
        let _ = writeln!(out, "  _lock = next.future;");
        let _ = writeln!(out, "  try {{");
        let _ = writeln!(out, "    await current;");
        let _ = writeln!(out, "    return await fn();");
        let _ = writeln!(out, "  }} finally {{");
        let _ = writeln!(out, "    next.complete();");
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        // The `fn` here is the full request closure. Transient connection errors
        // (`SocketException` / `HttpException: Connection reset by peer`) happen rarely
        // but non-deterministically when the local mock server drops a connection mid-flight;
        // a single retry is not always enough, so retry several times with a short backoff,
        // recreating the HttpClient each time to drop any poisoned pooled connection. The
        // final attempt is outside the catch so a genuine, persistent failure still surfaces.
        let _ = writeln!(out, "Future<T> _withRetry<T>(Future<T> Function() fn) async {{");
        let _ = writeln!(out, "  for (var attempt = 0; attempt < 5; attempt++) {{");
        let _ = writeln!(out, "    try {{");
        let _ = writeln!(out, "      return await fn();");
        let _ = writeln!(out, "    }} on SocketException {{");
        let _ = writeln!(out, "      _httpClient.close(force: true);");
        let _ = writeln!(out, "      _httpClient = HttpClient()..maxConnectionsPerHost = 1;");
        let _ = writeln!(out, "    }} on HttpException {{");
        let _ = writeln!(out, "      _httpClient.close(force: true);");
        let _ = writeln!(out, "      _httpClient = HttpClient()..maxConnectionsPerHost = 1;");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(
            out,
            "    await Future<void>.delayed(Duration(milliseconds: 25 * (attempt + 1)));"
        );
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "  return await fn();");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out);

    // Whether this test file must spawn the SUT app harness. True for direct HTTP
    // fixtures and for any fixture that derives a URL from `SUT_URL`
    // (mock_url args / client_factory). `package:test` has no cross-file global
    // setup, so each file spawns its own server in `setUpAll` and tears it down
    // in `tearDownAll`; `dart_test.yaml` pins `concurrency: 1` so at most one
    // server runs at a time. A pre-set `SUT_URL` environment variable (external CI
    // orchestration) short-circuits the spawn. Mirrors the Python conftest /
    // Ruby spec_helper / Java MockServerListener pattern.
    let needs_sut_spawn = has_http_fixtures || has_mock_url_refs;

    // Top-level SUT app harness state. `Platform.environment` is read-only in Dart,
    // so the spawned server's URL is held in mutable globals and read through
    // helper functions (rather than re-reading the environment) by the test
    // bodies below.
    if needs_sut_spawn {
        let _ = writeln!(out, "Process? _sutProcess;");
        let _ = writeln!(out, "String? _spawnedSutUrl;");
        // Per-fixture origin-root URLs captured from the `MOCK_SERVERS=` sentinel
        // line. Populated by the spawn-and-listen setUpAll body below or seeded
        // from `MOCK_SERVERS` env when a parent process already started the server.
        let _ = writeln!(out, "final Map<String, String> _fixtureUrls = <String, String>{{}};");
        let _ = writeln!(out);
        // Prefer `MOCK_SERVER_URL` (exported by `scripts/e2e/run-with-mock-server.sh`
        // and by `alef test --e2e` mock-server bootstrap) so the tests hit the
        // ephemeral port the alef-spawned mock-server picked; fall back to a
        // pre-set `SUT_URL` (external CI orchestration) or the legacy `localhost:8008`
        // only if neither env var is set.
        let _ = writeln!(
            out,
            "String _sutUrl() => _spawnedSutUrl ?? Platform.environment['MOCK_SERVER_URL'] ?? Platform.environment['SUT_URL'] ?? 'http://localhost:8008';"
        );
        let _ = writeln!(out);
        // Resolve a fixture URL. Fixtures with origin-root routes (e.g. inline
        // host-absolute anchors `<a href=\"/page1\">`, `/robots*`, `/sitemap*`)
        // get a dedicated per-fixture listener so that root-relative links the
        // SUT follows are served by the same fixture. When `MOCK_SERVERS` has
        // an entry for the fixture, prefer the per-fixture URL; otherwise fall
        // back to the shared listener under `/fixtures/<id>`.
        let _ = writeln!(out, "String _fixtureUrl(String fixtureId) {{");
        let _ = writeln!(out, "  final perFixture = _fixtureUrls[fixtureId];");
        let _ = writeln!(out, "  if (perFixture != null) return perFixture;");
        let _ = writeln!(out, "  final env = Platform.environment['MOCK_SERVERS'];");
        let _ = writeln!(out, "  if (env != null && env.isNotEmpty) {{");
        let _ = writeln!(out, "    try {{");
        let _ = writeln!(out, "      final decoded = jsonDecode(env);");
        let _ = writeln!(out, "      if (decoded is Map && decoded[fixtureId] is String) {{");
        let _ = writeln!(out, "        return decoded[fixtureId] as String;");
        let _ = writeln!(out, "      }}");
        let _ = writeln!(out, "    }} catch (_) {{}}");
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "  return '${{_sutUrl()}}/fixtures/$fixtureId';");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    // Emit _setEnv helper if e2e config has env vars to inject.
    if !e2e_config.env.is_empty() {
        let _ = writeln!(out, "void _setEnv(String key, String value) {{");
        let _ = writeln!(out, "  final libc = DynamicLibrary.process();");
        let _ = writeln!(out, "  final setenv = libc.lookupFunction<");
        let _ = writeln!(out, "      Int32 Function(Pointer<Utf8>, Pointer<Utf8>, Int32),");
        let _ = writeln!(out, "      int Function(Pointer<Utf8>, Pointer<Utf8>, int)>('setenv');");
        let _ = writeln!(out, "  final keyPtr = key.toNativeUtf8();");
        let _ = writeln!(out, "  final valuePtr = value.toNativeUtf8();");
        let _ = writeln!(out, "  try {{");
        let _ = writeln!(out, "    final result = setenv(keyPtr, valuePtr, 1);");
        let _ = writeln!(out, "    if (result != 0) {{");
        let _ = writeln!(
            out,
            "      throw StateError('setenv failed for ${{key}}=${{value}} with return code $result');"
        );
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "  }} finally {{");
        let _ = writeln!(out, "    calloc.free(keyPtr);");
        let _ = writeln!(out, "    calloc.free(valuePtr);");
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    // First pass: collect module-level test stub class definitions BEFORE void main().
    // Dart does not allow class definitions inside functions, so we must emit them
    // at the module level before void main().
    let mut test_stub_classes = String::new();
    for fixture in fixtures {
        super::stubs::collect_test_stub_classes(&mut test_stub_classes, fixture, e2e_config, config, type_defs, enums);
    }
    if !test_stub_classes.is_empty() {
        out.push_str(&test_stub_classes);
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "void main() {{");

    // Track whether RustLib.init() in `setUpAll` succeeded. When it fails (e.g. the
    // prebuilt native library is missing on this host), `tearDownAll` must NOT call
    // `RustLib.dispose()` — flutter_rust_bridge's `disposeImpl` runs a non-null
    // assertion on the lazily-initialised `api` field and fails with
    // "Null check operator used on a null value", masking the original load error.
    let _ = writeln!(out, "  var _rustLibInitialized = false;");
    let _ = writeln!(out);

    // Emit setUpAll to initialize the flutter_rust_bridge before any test runs and,
    // when fixtures load files by path, chdir to test_documents so that relative
    // paths like "docx/fake.docx" resolve correctly.
    //
    // The test_documents directory lives two levels above e2e/dart/ (at the repo root).
    // The FIXTURES_DIR environment variable can override this for CI environments.
    let _ = writeln!(out, "  setUpAll(() async {{");
    // Inject e2e env vars before initializing the binding engine.
    if !e2e_config.env.is_empty() {
        let mut keys: Vec<_> = e2e_config.env.keys().collect();
        keys.sort();
        for key in keys {
            let value = &e2e_config.env[key];
            // Escape backslashes and quotes in value for Dart string literal.
            let escaped_value = value.replace('\\', "\\\\").replace('"', "\\\"");
            let _ = writeln!(out, "    _setEnv('{key}', '{escaped_value}');");
        }
    }
    let _ = writeln!(out, "    await RustLib.init();");
    let _ = writeln!(out, "    _rustLibInitialized = true;");
    // ~keep The SUT/mock-server spawn MUST render before the test-documents chdir below: both
    // `render_dart_sut_spawn`'s `app_harness.dart` probe and its standalone mock-server binary
    // paths (`../rust/target/release/mock-server`, `../rust/Cargo.toml`) resolve relative to
    // `Directory.current` at the point they run. Emitting the chdir first left `Directory.current`
    // pointed at `test_documents/` (or `FIXTURES_DIR`) by the time the spawn code ran, so
    // `'../rust/Cargo.toml'` resolved from `.../e2e/`, not `.../e2e/dart/` -- `Bad state:
    // mock-server build failed: error: manifest path ... does not exist`, six suites failing in
    // `setUpAll` with zero fixture assertions run. The two concerns are otherwise independent
    // (the spawn doesn't need to run from `test_documents/`), so reordering is side-effect-free.
    if needs_sut_spawn {
        render_dart_sut_spawn(&mut out);
    }
    if needs_chdir {
        let test_docs_path = e2e_config.test_documents_relative_from(0);
        let _ = writeln!(
            out,
            "    final _testDocs = Platform.environment['FIXTURES_DIR'] ?? '{test_docs_path}';"
        );
        let _ = writeln!(out, "    final _dir = Directory(_testDocs);");
        let _ = writeln!(out, "    if (_dir.existsSync()) Directory.current = _dir;");
    }
    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);

    // A test that registers a Dart-backed plugin leaves the callback in the *process-global*
    // Rust plugin registry. Because each `dart test` file runs in its own isolate, a callback
    // left registered here is later invoked by another file's (now-dead) isolate and deadlocks
    // on the block_on that drives the DartFnFuture. Clear every registry this file populated in
    // `tearDownAll`. A `register_*` fixture existing guarantees the paired `clear*` fn exists.
    let clear_pairs: &[(&str, &str)] = &[
        ("register_embedding_backend", "clearEmbeddingBackends"),
        ("register_ocr_backend", "clearOcrBackends"),
        ("register_post_processor", "clearPostProcessors"),
        ("register_reranker_backend", "clearRerankerBackends"),
        ("register_renderer", "clearRenderers"),
        ("register_validator", "clearValidators"),
    ];
    let mut needed_clears: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for f in fixtures {
        for (reg, clear) in clear_pairs {
            if f.id.contains(reg) {
                needed_clears.insert(*clear);
            }
        }
    }

    // Always emit tearDownAll to dispose of RustLib singleton and close resources.
    // RustLib is initialized in setUpAll and must be cleaned up after all tests, but
    // only dispatch `dispose()` when init succeeded — see `_rustLibInitialized` above.
    let _ = writeln!(out, "  tearDownAll(() async {{");
    let _ = writeln!(out, "    if (_rustLibInitialized) {{");
    for clear in &needed_clears {
        let _ = writeln!(out, "      await {bridge_class}.{clear}();");
    }
    let _ = writeln!(out, "      RustLib.dispose();");
    let _ = writeln!(out, "    }}");
    if has_http_fixtures {
        let _ = writeln!(out, "    _httpClient.close(force: true);");
    }
    if needs_sut_spawn {
        let _ = writeln!(out, "    final proc = _sutProcess;");
        let _ = writeln!(out, "    if (proc != null) {{");
        let _ = writeln!(out, "      proc.kill();");
        let _ = writeln!(out, "      await proc.exitCode;");
        let _ = writeln!(out, "    }}");
    }
    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);

    for fixture in fixtures {
        super::test_case::render_test_case(
            &mut out,
            fixture,
            super::test_case::DartTestCaseContext {
                e2e_config,
                lang,
                bridge_class,
                dart_first_class_map,
                adapters,
                config,
                type_defs,
                enums,
                functions,
                errors,
                native_typed_dtos: false,
                is_snippet: false,
            },
        );
        // ~keep Dart's error path (`render_test_case`'s `expects_error` branches) emits
        // `expectLater(..., throwsA(..))` and nothing else, so every other assertion on an
        // error fixture — an `equals` against `error.status_code`, most often — leaves no trace
        // in the generated test at all. The marker lands here, immediately after the emitted
        // `test(...)` call inside `main()`, rather than inside the test body: the body is built
        // in `test_case.rs`, which another change owns.
        crate::e2e::codegen::error_path_assertions::emit(&mut out, fixture, "  // ", "dart");
    }

    let _ = writeln!(out, "}}");
    out
}

/// Emit the `setUpAll` body that spawns the app_harness.dart subprocess and
/// captures its URL into the top-level `_spawnedSutUrl` global.
///
/// The app_harness binds an ephemeral `127.0.0.1:8008` and prints
/// `SUT_URL=http://127.0.0.1:8008` on stdout once it is listening.
/// A pre-set `SUT_URL` environment variable (external CI orchestration)
/// short-circuits the spawn. Mirrors the Python conftest /
/// Ruby spec_helper / Java MockServerListener spawn pattern.
///
/// Emitted inside an `async` `setUpAll`; the harness lives at
/// `app_harness.dart` relative to `Directory.current`, which points to the test_app /
/// e2e suite root because the Taskfile / harness invokes `dart test` from there.
/// `Platform.script` is unusable here because `dart test` stages test files to a tmpdir
/// (`/var/folders/.../T/dart_test.kernel.<random>/test.dart_<n>.dill`); relative
/// resolves against that URI escape the source tree entirely.
fn render_dart_sut_spawn(out: &mut String) {
    // Skip spawning any server when either `MOCK_SERVER_URL` (alef e2e
    // wrapper / `scripts/e2e/run-with-mock-server.sh`) or `SUT_URL` (external
    // CI orchestration) is already set — the parent process has already
    // arranged the HTTP target the tests should hit.
    let _ = writeln!(
        out,
        "    if (Platform.environment['MOCK_SERVER_URL'] == null && Platform.environment['SUT_URL'] == null) {{"
    );
    let _ = writeln!(
        out,
        "      final _harness = Directory.current.uri.resolve('app_harness.dart').toFilePath();"
    );
    let _ = writeln!(out, "      if (File(_harness).existsSync()) {{");
    let _ = writeln!(
        out,
        "        _sutProcess = await Process.start('dart', ['run', _harness], mode: ProcessStartMode.normal);"
    );
    // A single `listen` keeps draining stdout after the startup line is seen
    // (so a full pipe never blocks the child); the Completer resolves once the
    // URL has been captured. `Process.stdout` is a single-subscription stream,
    // so it must be consumed exactly once — re-reading `.stdout` would throw.
    let _ = writeln!(out, "        final _ready = Completer<void>();");
    let _ = writeln!(out, "        _sutProcess!.stdout");
    let _ = writeln!(out, "            .transform(utf8.decoder)");
    let _ = writeln!(out, "            .transform(const LineSplitter())");
    let _ = writeln!(out, "            .listen((_line) {{");
    let _ = writeln!(out, "          final _trimmed = _line.trim();");
    let _ = writeln!(out, "          if (_trimmed.startsWith('SUT_URL=')) {{");
    let _ = writeln!(
        out,
        "            _spawnedSutUrl = _trimmed.substring('SUT_URL='.length);"
    );
    let _ = writeln!(out, "            if (!_ready.isCompleted) _ready.complete();");
    let _ = writeln!(out, "          }}");
    let _ = writeln!(out, "        }}, onDone: () {{");
    let _ = writeln!(out, "          if (!_ready.isCompleted) _ready.complete();");
    let _ = writeln!(out, "        }});");
    let _ = writeln!(
        out,
        "        await _ready.future.timeout(const Duration(seconds: 15), onTimeout: () {{}});"
    );
    // When app_harness.dart is absent this is a mock-server test (not a server-pattern
    // test). Build the alef-generated mock-server binary if it is missing, then spawn
    // it and capture `MOCK_SERVER_URL=` from its stdout — the same sentinel line that
    // Ruby spec_helper and the `alef test-apps run` orchestrator read.
    // Resolve paths relative to the test file to locate the mock-server project.
    let _ = writeln!(out, "      }} else {{");
    let _ = writeln!(
        out,
        "        // Standalone mock-server mode: build if missing, then spawn."
    );
    let _ = writeln!(
        out,
        "        final _mockBin = Directory.current.uri.resolve('../rust/target/release/mock-server').toFilePath();"
    );
    let _ = writeln!(
        out,
        "        final _mockManifest = Directory.current.uri.resolve('../rust/Cargo.toml').toFilePath();"
    );
    let _ = writeln!(out, "        if (!File(_mockBin).existsSync()) {{");
    let _ = writeln!(
        out,
        "          final _build = await Process.run('cargo', ['build', '--release', '--manifest-path', _mockManifest, '--bin', 'mock-server']);"
    );
    let _ = writeln!(
        out,
        "          if (_build.exitCode != 0) throw StateError('mock-server build failed: ${{_build.stderr}}');"
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(
        out,
        "        final _fixturesDir = Directory.current.uri.resolve('../../fixtures').toFilePath();"
    );
    let _ = writeln!(
        out,
        "        _sutProcess = await Process.start(_mockBin, [_fixturesDir], mode: ProcessStartMode.normal);"
    );
    let _ = writeln!(out, "        final _ready2 = Completer<void>();");
    let _ = writeln!(out, "        _sutProcess!.stdout");
    let _ = writeln!(out, "            .transform(utf8.decoder)");
    let _ = writeln!(out, "            .transform(const LineSplitter())");
    let _ = writeln!(out, "            .listen((_line) {{");
    let _ = writeln!(out, "          final _trimmed = _line.trim();");
    let _ = writeln!(out, "          if (_trimmed.startsWith('MOCK_SERVER_URL=')) {{");
    let _ = writeln!(
        out,
        "            _spawnedSutUrl = _trimmed.substring('MOCK_SERVER_URL='.length);"
    );
    let _ = writeln!(out, "          }}");
    let _ = writeln!(out, "          if (_trimmed.startsWith('MOCK_SERVERS=')) {{");
    let _ = writeln!(
        out,
        "            final _payload = _trimmed.substring('MOCK_SERVERS='.length);"
    );
    let _ = writeln!(out, "            try {{");
    let _ = writeln!(out, "              final _decoded = jsonDecode(_payload);");
    let _ = writeln!(out, "              if (_decoded is Map) {{");
    let _ = writeln!(out, "                _decoded.forEach((k, v) {{");
    let _ = writeln!(out, "                  if (k is String && v is String) {{");
    let _ = writeln!(out, "                    _fixtureUrls[k] = v;");
    let _ = writeln!(out, "                  }}");
    let _ = writeln!(out, "                }});");
    let _ = writeln!(out, "              }}");
    let _ = writeln!(out, "            }} catch (_) {{}}");
    let _ = writeln!(out, "            if (!_ready2.isCompleted) _ready2.complete();");
    let _ = writeln!(out, "          }} else if (_spawnedSutUrl != null) {{");
    let _ = writeln!(out, "            if (!_ready2.isCompleted) _ready2.complete();");
    let _ = writeln!(out, "          }}");
    let _ = writeln!(out, "        }}, onDone: () {{");
    let _ = writeln!(out, "          if (!_ready2.isCompleted) _ready2.complete();");
    let _ = writeln!(out, "        }});");
    let _ = writeln!(
        out,
        "        await _ready2.future.timeout(const Duration(seconds: 60), onTimeout: () {{}});"
    );
    let _ = writeln!(out, "      }}");
    let _ = writeln!(out, "    }}");
}
