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
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    // Suppress unused_local_variable: `final result = await api.method(...)` is
    // emitted for every test case; tests that only check for absence of errors
    // do not consume `result`, triggering this dart-analyze warning.
    out.push_str("// ignore_for_file: unused_local_variable\n\n");

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

    // Detect whether any fixture uses file_path or bytes args — if so, setUpAll must chdir
    // to the test_documents directory so that relative paths like "docx/fake.docx" resolve.
    // Mirrors the Ruby/Python conftest and Swift setUp patterns.
    let needs_chdir = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        f.resolved_args(call_config)
            .iter()
            .any(|a| a.arg_type == "file_path" || a.arg_type == "bytes")
    });

    // Detect whether any non-HTTP fixture uses a json_object arg that resolves to a JSON array —
    // those are materialized via `jsonDecode` at test-run time and cast to `List<String>`.
    // Handle args themselves no longer require `jsonDecode` since they construct the config via
    // the FRB-generated `createCrawlConfigFromJson(json:)` helper which accepts the JSON string
    // directly. The variable name is kept as `has_handle_args` for config stability.
    let has_handle_args = fixtures.iter().any(|f| {
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

    // Detect whether any fixture uses a PageAction array argument (for interact calls).
    // PageAction and ScrollDirection types must be emitted in the test helper code only if used.
    let has_page_action = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return false;
        }
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        f.resolved_args(call_config).iter().any(|a| {
            a.element_type.as_deref() == Some("PageAction") && resolve_field(&f.input, &a.field).is_array()
        })
    });

    // Collect plugin trait types used in test_backend arguments. These types must be imported
    // from the main package so test stubs can extend them.
    let used_trait_types: std::collections::HashSet<String> = fixtures
        .iter()
        .flat_map(|f| {
            if f.is_http_test() {
                return vec![];
            }
            let call_config = e2e_config.resolve_call_for_fixture(
                f.call.as_deref(),
                &f.id,
                &f.resolved_category(),
                &f.tags,
                &f.input,
            );
            f.resolved_args(call_config)
                .iter()
                .filter_map(|a| {
                    if a.arg_type == "test_backend" {
                        a.trait_name.clone()
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

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
    // Import plugin trait types used in test_backend arguments so stubs can extend them.
    for trait_type in &used_trait_types {
        let _ = writeln!(out, "import 'package:{pkg_name}/{pkg_name}.dart' show {trait_type};");
    }
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
    // dart:convert provides jsonDecode for handle-arg engine construction, HTTP response parsing,
    // and PageAction array deserialization, plus utf8/LineSplitter for decoding the mock-server's
    // startup stdout (MOCK_SERVER_URL= / MOCK_SERVERS=) in the spawn harness.
    if has_http_fixtures || has_handle_args || has_page_action || has_mock_url_refs {
        let _ = writeln!(out, "import 'dart:convert';");
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

    // Emit a helper function to normalize enum values to their serde wire format.
    // Dart enums' .toString() returns "EnumName.variant" but fixtures use serde wire format
    // (e.g. "stop" for FinishReason.stop, "tool_calls" for FinishReason.toolCalls).
    // This helper handles enum-to-wire conversion by calling .name (which gives the Dart
    // variant name like "toolCalls") and converting back to snake_case for multi-word variants.
    let _ = writeln!(out, "String _alefE2eText(Object? value) {{");
    let _ = writeln!(out, "  if (value == null) return '';");
    let _ = writeln!(
        out,
        "  // Check if it's an enum by examining its toString representation."
    );
    let _ = writeln!(out, "  final str = value.toString();");
    let _ = writeln!(out, "  if (str.contains('.')) {{");
    let _ = writeln!(
        out,
        "    // Enum.toString() returns 'EnumName.variantName'. Extract the variant name."
    );
    let _ = writeln!(out, "    final parts = str.split('.');");
    let _ = writeln!(out, "    if (parts.length == 2) {{");
    let _ = writeln!(out, "      final variantName = parts[1];");
    let _ = writeln!(
        out,
        "      // Convert camelCase variant names to snake_case for serde compatibility."
    );
    let _ = writeln!(out, "      // E.g. 'toolCalls' -> 'tool_calls', 'stop' -> 'stop'.");
    let _ = writeln!(out, "      return _camelToSnake(variantName);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "  return str;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Helper to convert camelCase to snake_case.
    let _ = writeln!(out, "String _camelToSnake(String camel) {{");
    let _ = writeln!(out, "  final buffer = StringBuffer();");
    let _ = writeln!(out, "  for (int i = 0; i < camel.length; i++) {{");
    let _ = writeln!(out, "    final char = camel[i];");
    let _ = writeln!(out, "    if (char.contains(RegExp(r'[A-Z]'))) {{");
    let _ = writeln!(out, "      if (i > 0) buffer.write('_');");
    let _ = writeln!(out, "      buffer.write(char.toLowerCase());");
    let _ = writeln!(out, "    }} else {{");
    let _ = writeln!(out, "      buffer.write(char);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "  return buffer.toString();");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Only emit _parsePageAction if any fixture uses PageAction arrays.
    if has_page_action {
        let _ = writeln!(out, "PageAction _parsePageAction(Map<String, dynamic> json) {{");
        let _ = writeln!(out, "  final actionType = json['type'] as String?;");
        let _ = writeln!(out, "  switch (actionType) {{");
        let _ = writeln!(out, "    case 'click':");
        let _ = writeln!(
            out,
            "      return PageAction.click(selector: json['selector'] as String);"
        );
        let _ = writeln!(out, "    case 'type':");
        let _ = writeln!(out, "      return PageAction.typeText(");
        let _ = writeln!(out, "        selector: json['selector'] as String,");
        let _ = writeln!(out, "        text: json['text'] as String,");
        let _ = writeln!(out, "      );");
        let _ = writeln!(out, "    case 'press':");
        let _ = writeln!(out, "      return PageAction.press(");
        let _ = writeln!(out, "        key: json['key'] as String,");
        let _ = writeln!(out, "      );");
        let _ = writeln!(out, "    case 'scroll':");
        let _ = writeln!(out, "      return PageAction.scroll(");
        let _ = writeln!(out, "        direction: ScrollDirection.down,");
        let _ = writeln!(out, "        selector: json['selector'] as String? ?? '',");
        let _ = writeln!(out, "        amount: json['amount'] as int? ?? 0,");
        let _ = writeln!(out, "      );");
        let _ = writeln!(out, "    case 'wait':");
        let _ = writeln!(out, "      return PageAction.wait(");
        let _ = writeln!(out, "        milliseconds: json['timeout_ms'] as int? ?? 0,");
        let _ = writeln!(out, "        selector: json['selector'] as String,");
        let _ = writeln!(out, "      );");
        let _ = writeln!(out, "    case 'screenshot':");
        let _ = writeln!(
            out,
            "      return PageAction.screenshot(fullPage: json['full_page'] as bool? ?? false);"
        );
        let _ = writeln!(out, "    case 'executeJs':");
        let _ = writeln!(
            out,
            "      return PageAction.executeJs(script: json['script'] as String);"
        );
        let _ = writeln!(out, "    case 'scrape':");
        let _ = writeln!(out, "      return const PageAction.scrape();");
        let _ = writeln!(out, "    default:");
        let _ = writeln!(
            out,
            "      throw UnsupportedError('Unknown PageAction type: $actionType');"
        );
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

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

    // First pass: collect module-level test stub class definitions BEFORE void main().
    // Dart does not allow class definitions inside functions, so we must emit them
    // at the module level before void main().
    let mut test_stub_classes = String::new();
    for fixture in fixtures {
        collect_dart_test_stub_classes(&mut test_stub_classes, fixture, e2e_config, config, type_defs);
    }
    if !test_stub_classes.is_empty() {
        out.push_str(&test_stub_classes);
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "void main() {{");

    // Emit setUpAll to initialize the flutter_rust_bridge before any test runs and,
    // when fixtures load files by path, chdir to test_documents so that relative
    // paths like "docx/fake.docx" resolve correctly.
    //
    // The test_documents directory lives two levels above e2e/dart/ (at the repo root).
    // The FIXTURES_DIR environment variable can override this for CI environments.
    let _ = writeln!(out, "  setUpAll(() async {{");
    let _ = writeln!(out, "    await RustLib.init();");
    if needs_chdir {
        let test_docs_path = e2e_config.test_documents_relative_from(0);
        let _ = writeln!(
            out,
            "    final _testDocs = Platform.environment['FIXTURES_DIR'] ?? '{test_docs_path}';"
        );
        let _ = writeln!(out, "    final _dir = Directory(_testDocs);");
        let _ = writeln!(out, "    if (_dir.existsSync()) Directory.current = _dir;");
    }
    if needs_sut_spawn {
        render_dart_sut_spawn(&mut out);
    }
    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);

    // Always emit tearDownAll to dispose of RustLib singleton and close resources.
    // RustLib is initialized in setUpAll and must be cleaned up after all tests.
    // RustLib.dispose() is always called to ensure proper cleanup (required for non-empty body).
    let _ = writeln!(out, "  tearDownAll(() async {{");
    let _ = writeln!(out, "    RustLib.dispose();");
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
            },
        );
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

/// Collect module-level test stub class definitions for Dart.
/// Dart does not allow class definitions inside functions, so we must emit them
/// at the module level before void main(). This function checks if the fixture
/// uses a test_backend argument and if so, emits the class definition.
fn collect_dart_test_stub_classes(
    out: &mut String,
    fixture: &Fixture,
    _e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // HTTP fixtures do not use test_backend.
    if fixture.is_http_test() {
        return;
    }

    // Check fixture.args directly (not call_config.args, which is empty for trait-bridge calls).
    // The fixture JSON defines the actual arguments including test_backend definitions.
    for arg_def in &fixture.args {
        if arg_def.arg_type != "test_backend" {
            continue;
        }
        if let Some(trait_name) = &arg_def.trait_name {
            if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                    .iter()
                    .find(|t| t.name == *trait_name)
                    .map(|t| t.methods.iter().collect())
                    .unwrap_or_default();
                let emission = super::stubs::emit_test_backend(trait_bridge, &methods, fixture);
                // Emit only the class definition at module-level.
                let _ = writeln!(out, "{}", emission.setup_block);
                let _ = writeln!(out);
            }
        }
    }
}
