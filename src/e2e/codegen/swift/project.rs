use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions::toolchain;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

/// Directive telling Apple's `swift-format` to skip the file entirely.
///
/// The e2e generator emits Swift source with 4-space indentation, fixed import
/// order (`XCTest, Foundation, <Module>`) and unwrapped long lines
/// — all of which violate `swift-format`'s defaults (2-space indent, sorted
/// imports, 100-char line width). Reformatting after every regen would force
/// every consumer repo to either bake `swift-format` into their pre-commit set
/// or eat noisy diffs. Marking the files as ignored is the same workaround the
/// Swift binding backend uses for `DemoMarkup.swift` (see
/// `alef-backend-swift/src/gen_bindings.rs`) and keeps the file
/// byte-identical between `alef generate` runs and `swift-format` hooks.
pub(super) const SWIFT_FORMAT_IGNORE_DIRECTIVE: &str = "// swift-format-ignore-file\n\n";

/// Render the shared `TestHelpers.swift` file emitted into each Swift e2e
/// test target. Adds a `CustomStringConvertible` conformance to swift-bridge's
/// `RustString` so error messages from bridge throws print their actual Rust
/// content instead of the bare class name.
pub(super) fn render_test_helpers_swift() -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let ignore = SWIFT_FORMAT_IGNORE_DIRECTIVE;
    format!(
        r#"{header}{ignore}import Foundation
#if canImport(FoundationNetworking)
// URLSession, URLRequest, HTTPURLResponse, and URLSessionTaskDelegate live in
// the FoundationNetworking submodule on swift-corelibs-foundation (Linux). On
// Apple platforms these types remain in plain Foundation and this submodule
// does not exist; the canImport guard skips the import there.
import FoundationNetworking
#endif
import RustBridge

// Make `RustString` print its content in XCTest failure output. Without this,
// every error thrown from the swift-bridge layer surfaces as
// `caught error: "RustBridge.RustString"` with the actual message hidden
// inside the opaque class instance. The `@retroactive` keyword acknowledges
// that the conformed-to protocol (`CustomStringConvertible`) and the
// conforming type (`RustString`) both live outside this module — required by
// Swift 6 to silence the retroactive-conformance warning. swift-bridge does
// not give `RustString` a `description` of its own, so there is no conflict.
extension RustString: @retroactive CustomStringConvertible {{
    public var description: String {{ self.toString() }}
}}

// URLSession delegate that does not follow redirects, so tests can assert on 3xx status codes
// and Location headers instead of transparently chasing them to the final response.
final class AlefE2ENoRedirectDelegate: NSObject, URLSessionTaskDelegate {{
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {{
        completionHandler(nil)
    }}
}}

// Mock server base URL accessor used by the generated test bodies.
// The `MOCK_SERVER_URL` env var is exported by `scripts/e2e/run-with-mock-server.sh`
// (which spawns the `mock-server` binary built from `e2e/rust`) before invoking
// `swift test`. We fall back to a `localhost` URL that will fail-fast at request
// time so misconfigured runs surface a clear error instead of silently hitting
// production endpoints.
enum AlefE2EMockServer {{
    static var baseURL: String {{
        ProcessInfo.processInfo.environment["MOCK_SERVER_URL"]
            ?? "http://127.0.0.1:0"
    }}
}}
"#
    )
}

/// Split a string into UTF-8-safe chunks of max ~30000 bytes.
/// Each chunk is wrapped as a Swift raw string literal with a safe delimiter level.
fn chunk_fixtures_for_swift(json: &str) -> Vec<String> {
    const CHUNK_SIZE: usize = 30000;

    // Find the longest consecutive run of `#` characters in the JSON.
    // Swift raw string delimiters use `#"..."#`, `##"..."##`, etc., where
    // the number of `#` on both sides must match and exceed any consecutive `#`
    // run inside the string content.
    let mut max_hash_run = 0;
    let mut current_run = 0;
    for c in json.chars() {
        if c == '#' {
            current_run += 1;
            max_hash_run = max_hash_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    // Use one more `#` than the longest run to guarantee no collision.
    let delimiter_level = max_hash_run + 1;
    let delimiter = "#".repeat(delimiter_level);

    let mut chunks = Vec::new();

    // Split at UTF-8 char boundaries (not byte boundaries) to avoid breaking
    // multi-byte UTF-8 sequences.
    let mut current_chunk = String::new();
    for c in json.chars() {
        // If adding this char would exceed CHUNK_SIZE, save current chunk and start new one.
        if !current_chunk.is_empty() && current_chunk.len() + c.len_utf8() > CHUNK_SIZE {
            // Wrap the chunk in a raw string literal.
            chunks.push(format!("{0}\"{1}\"{0}", delimiter, current_chunk));
            current_chunk.clear();
        }
        current_chunk.push(c);
    }

    // Don't forget the last chunk.
    if !current_chunk.is_empty() {
        chunks.push(format!("{0}\"{1}\"{0}", delimiter, current_chunk));
    }

    chunks
}

pub(super) fn render_app_harness(e2e_config: &E2eConfig, groups: &[FixtureGroup], module_name: &str) -> String {
    // Collect all HTTP fixtures from all groups.
    let mut fixtures_map = serde_json::Map::new();

    for group in groups {
        for fixture in &group.fixtures {
            if fixture.http.is_none() {
                continue;
            }
            let http_data = &fixture.http.as_ref().unwrap();
            let fixture_json = serde_json::json!({
                "http": {
                    "handler": {
                        "route": &http_data.handler.route,
                        "method": &http_data.handler.method,
                        "body_schema": http_data.handler.body_schema.clone(),
                    },
                    "request": {
                        "path": &http_data.request.path,
                    },
                    "expected_response": {
                        "status_code": http_data.expected_response.status_code,
                        "body": &http_data.expected_response.body,
                        "headers": &http_data.expected_response.headers,
                    }
                }
            });
            fixtures_map.insert(fixture.id.clone(), fixture_json);
        }
    }

    let fixtures_json = serde_json::to_string(&fixtures_map).unwrap_or_default();
    let fixtures_json_chunks = chunk_fixtures_for_swift(&fixtures_json);

    let host = &e2e_config.harness.host;
    let port = e2e_config.harness.port;
    let app_class = e2e_config.harness.app_class_for_lang("swift");
    // Swift methods are camelCase per Swift API design guidelines.
    let register_route_method = e2e_config
        .harness
        .register_method_idiomatic("swift")
        .unwrap_or_else(|| "registerRoute".to_string());
    let body_schema_setter = &e2e_config.harness.body_schema_setter;
    let method_enum = &e2e_config.harness.method_enum;
    let run_method = e2e_config.harness.run_method_for_lang("swift");

    let header = hash::header(CommentStyle::DoubleSlash);

    // Build imports: include harness.imports config plus the binding module_name.
    // Get language-specific imports for swift with fallback to global imports.
    let mut imports = e2e_config.harness.imports_for_lang("swift");
    // Prepend the binding module_name if not already present (case-insensitive check).
    if !imports.iter().any(|i| i.to_lowercase() == module_name.to_lowercase()) {
        imports.insert(0, module_name.to_string());
    }
    let imports_str = imports
        .iter()
        .map(|m| format!("import {}", m))
        .collect::<Vec<_>>()
        .join("\n");

    let ctx = minijinja::context! {
        header => header,
        imports => imports_str,
        app_class => app_class.as_deref().unwrap_or("App"),
        route_builder_constructor => "RouteBuilder",
        route_builder_schema_setter => body_schema_setter.as_deref().unwrap_or("requestSchemaJson"),
        method_enum_class => method_enum.as_deref().unwrap_or("Method"),
        register_route_method => register_route_method.as_str(),
        run_method => run_method.as_deref().unwrap_or("run"),
        response_body_field => e2e_config.harness.response_body_field.as_str(),
        host => host,
        port => port,
        fixtures_json_chunks => fixtures_json_chunks,
    };

    crate::e2e::template_env::render("swift/app_harness.swift.jinja", ctx)
}

pub(super) fn render_package_swift(
    module_name: &str,
    registry_url: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    include_harness_target: bool,
) -> String {
    let min_macos = toolchain::SWIFT_MIN_MACOS;

    // For local deps SwiftPM identity = last path component (e.g. "../../packages/swift" → "swift").
    // For registry deps we use .package(url:, from:) to pull the Swift package from GitHub.
    // SwiftPM will resolve the tag v<version> to the package, making the Swift module available.
    // Use explicit .product(name:package:) to avoid ambiguity under tools-version 6.0.
    let (dependencies_block, test_target_dep) = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // Registry mode: fetch the full Swift package from GitHub at the release tag.
            let github_repo_url = registry_url.trim_end_matches(".git");
            let package_dep = format!(
                r#"        .package(url: "{github_repo_url}", from: "{pkg_version}"),
"#
            );
            let deps_block = format!("    dependencies: [\n{package_dep}    ],\n");
            // SPM identity for url-based deps is the URL basename (last path
            // component, sans `.git`). For `github.com/<org>/<repo>` that's
            // `<repo>`. We strip `.git` again defensively even though
            // `github_repo_url` already has it trimmed.
            let pkg_id = github_repo_url
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(module_name)
                .trim_end_matches(".git");
            let prod = format!(r#".product(name: "{module_name}", package: "{pkg_id}")"#);
            (deps_block, prod)
        }
        crate::e2e::config::DependencyMode::Local => {
            // SwiftPM 6.0 deprecated the `name:` parameter on `.package(path:)`:
            // package identity is derived from the path's last component, ignoring
            // any explicit `name:`. For local mode, the dependency is still the external package,
            // but we reference it via .product(name:package:) in the test target dependencies.
            let pkg_id = pkg_path.trim_end_matches('/').rsplit('/').next().unwrap_or(module_name);
            let deps_block = format!("    dependencies: [\n        .package(path: \"{pkg_path}\"),\n    ],\n");
            let prod = format!(r#".product(name: "{module_name}", package: "{pkg_id}")"#);
            (deps_block, prod)
        }
    };
    // SwiftPM platform enums use the major version only (.v13, .v14, ...);
    // strip patch components to match the scaffold's `Package.swift`.
    let min_macos_major = min_macos.split('.').next().unwrap_or(min_macos);
    let min_ios = toolchain::SWIFT_MIN_IOS;
    let min_ios_major = min_ios.split('.').next().unwrap_or(min_ios);
    // The consumer's minimum iOS must be >= the dep's minimum iOS or SwiftPM hides
    // the product as platform-incompatible. Use the same constant the swift backend
    // emits into the dep's Package.swift.
    let harness_target = if include_harness_target {
        format!(
            r#"        .executableTarget(
            name: "Harness",
            dependencies: [{test_target_dep}],
            path: "Sources/Harness"
        ),
"#
        )
    } else {
        String::new()
    };
    let targets_block = format!(
        r#"{harness_target}        .testTarget(
            name: "{module_name}E2ETests",
            dependencies: [{test_target_dep}]
        ),
"#
    );
    format!(
        r#"// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "E2eSwift",
    platforms: [
        .macOS(.v{min_macos_major}),
        .iOS(.v{min_ios_major}),
    ],
{dependencies_block}    targets: [
{targets_block}    ]
)
"#
    )
}
