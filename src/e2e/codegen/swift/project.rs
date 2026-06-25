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

// The server-pattern harness (`Sources/Harness/main.swift`) is now emitted by a
// consumer extension via `Extension::emit_e2e`; alef no longer emits it. Retained
// for the project tests pending the dead-code sweep.
#[allow(dead_code)]
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

/// Extract the SwiftPM package name from a git URL. E.g., `https://github.com/foo/bar.git` → `bar`.
/// Falls back to `default_name` if extraction fails.
fn extract_package_name(url: &str, default_name: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .unwrap_or(default_name)
        .to_string()
}

/// Splice `dependencies` and `dev_dependencies` from a [`ManifestExtras`] into the
/// Package.swift's `dependencies` array and the test target's `dependencies` array.
///
/// Supports both Simple form (bare version string, URL comes from the key) and Detailed
/// form (url + version in the table). For each extra dependency, appends a `.package(...)`
/// entry to the Package's dependencies array and a `.product(name:package:)` entry to the
/// test target's dependencies list.
///
/// Idempotent: re-running with the same extras yields the same output (sorted by package name).
fn inject_package_swift_extras(
    dependencies_block: &mut String,
    test_target_dep: &mut String,
    extras: &crate::core::config::manifest_extras::ManifestExtras,
) {
    use crate::core::config::manifest_extras::ExtraDepSpec;

    // Collect all extra dependencies (runtime + dev) into a sorted map keyed by the SwiftPM
    // package identity (the URL's last path component). The value carries the url, version, and
    // the PRODUCT name to reference in the test target. The product often differs from the
    // package identity (e.g. repo `swift-tree-sitter` exposes product `SwiftTreeSitter`), so the
    // Detailed form accepts an explicit `product` key; otherwise it defaults to the identity.
    let mut all_extras: std::collections::BTreeMap<String, (String, String, String)> =
        std::collections::BTreeMap::new();

    let mut collect = |key: &str, spec: &ExtraDepSpec| match spec {
        ExtraDepSpec::Simple(version) => {
            // Simple form: key is the URL; package identity and product both derive from it.
            let pkg_id = extract_package_name(key, key);
            all_extras.insert(pkg_id.clone(), (key.to_string(), version.clone(), pkg_id));
        }
        ExtraDepSpec::Detailed(table) => {
            // Detailed form: requires "url" + "version"; optional "product" overrides the
            // product name referenced via `.product(name:package:)`.
            if let (Some(url), Some(version)) = (
                table.get("url").and_then(|u| u.as_str()),
                table.get("version").and_then(|v| v.as_str()),
            ) {
                let pkg_id = extract_package_name(url, key);
                let product = table
                    .get("product")
                    .and_then(|p| p.as_str())
                    .map_or_else(|| pkg_id.clone(), str::to_string);
                all_extras.insert(pkg_id, (url.to_string(), version.to_string(), product));
            }
        }
    };

    for (key, spec) in &extras.dependencies {
        collect(key, spec);
    }
    for (key, spec) in &extras.dev_dependencies {
        collect(key, spec);
    }

    if all_extras.is_empty() {
        return;
    }

    // Inject `.package(...)` entries into the dependencies array.
    // Insert before the final `    ],\n` line.
    let extras_packages: String = all_extras
        .values()
        .map(|(url, version, _product)| format!("        .package(url: \"{url}\", from: \"{version}\"),\n"))
        .collect();

    // Find the position to insert extras (before the closing bracket).
    if let Some(pos) = dependencies_block.rfind("    ],") {
        dependencies_block.insert_str(pos, &extras_packages);
    }

    // Append `.product(name:package:)` entries to the test target's dependencies. `package` is the
    // SwiftPM package identity (map key); `name` is the product (overridable, defaults to identity).
    let extras_products: String = all_extras
        .iter()
        .map(|(pkg_id, (_url, _version, product))| format!(", .product(name: \"{product}\", package: \"{pkg_id}\")"))
        .collect();

    test_target_dep.push_str(&extras_products);
}

pub(super) fn render_package_swift(
    module_name: &str,
    registry_url: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    include_harness_target: bool,
    extras: Option<&crate::core::config::manifest_extras::ManifestExtras>,
) -> String {
    let min_macos = toolchain::SWIFT_MIN_MACOS;

    // For local deps SwiftPM identity = last path component (e.g. "../../packages/swift" → "swift").
    // For registry deps we use .package(url:, branch:) to pull the Swift package from GitHub.
    // SwiftPM will resolve the release/swift/<version> branch which contains the correct
    // XCFramework checksum. SemVer-based resolution (from:) would fail because the actual
    // package artifact lives on the release/swift/ branch, not a SemVer tag.
    // Use explicit .product(name:package:) to avoid ambiguity under tools-version 6.0.
    let (mut dependencies_block, mut test_target_dep) = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // Registry mode: fetch the full Swift package from GitHub at the release/swift/<version> branch.
            let github_repo_url = registry_url.trim_end_matches(".git");
            let package_dep = format!(
                r#"        .package(url: "{github_repo_url}", branch: "release/swift/{pkg_version}"),
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

    // Inject harness_extras (both runtime and dev dependencies) into the Package.swift.
    // Both buckets are injected into the same `dependencies:` array with their corresponding
    // product dependencies wired into the test target. The expected alef.toml shape:
    //
    //   [crates.e2e.harness_extras.swift]
    //   dependencies = { "SwiftTreeSitter" = { url = "https://github.com/.../tree-sitter-swift.git", version = "0.25.0" } }
    //   dev_dependencies = { ... }
    //
    // Both the Simple form (bare version string, URL comes from the key) and Detailed form
    // (url + version in the table) are supported.
    if let Some(extra) = extras {
        if !extra.is_empty() {
            inject_package_swift_extras(&mut dependencies_block, &mut test_target_dep, extra);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::manifest_extras::{ExtraDepSpec, ManifestExtras};

    #[test]
    fn render_package_swift_local_mode_baseline() {
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            None,
        );
        assert!(
            out.contains(".package(path:"),
            "baseline Package.swift should have .package(path:)"
        );
        assert!(out.contains("\"../../packages/swift\""), "should contain local path");
        assert!(
            out.contains(".product(name: \"TreeSitter\", package: \"swift\")"),
            "should have product dep"
        );
    }

    #[test]
    fn render_package_swift_registry_mode_baseline() {
        let out = render_package_swift(
            "TreeSitter",
            "https://github.com/tree-sitter/tree-sitter-swift.git",
            "",
            "0.25.0",
            crate::e2e::config::DependencyMode::Registry,
            false,
            None,
        );
        assert!(out.contains(".package(url:"), "registry mode should use .package(url:)");
        assert!(
            out.contains("https://github.com/tree-sitter/tree-sitter-swift"),
            "should contain GitHub URL"
        );
        assert!(
            out.contains("branch: \"release/swift/0.25.0\""),
            "should pin release/swift branch to resolve correct XCFramework checksum"
        );
        assert!(
            out.contains(".product(name: \"TreeSitter\", package: \"tree-sitter-swift\")"),
            "should have product dep"
        );
    }

    #[test]
    fn render_package_swift_with_extras_detailed_form() {
        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "upstream-key".to_string(),
            ExtraDepSpec::Detailed({
                let mut t = toml::Table::new();
                t.insert("url".to_string(), "https://github.com/foo/SwiftTreeSitter.git".into());
                t.insert("version".to_string(), "0.25.0".into());
                t
            }),
        );
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        assert!(
            out.contains(".package(url: \"https://github.com/foo/SwiftTreeSitter.git\", from: \"0.25.0\")"),
            "should inject extras .package with url and version from Detailed form. Got:\n{out}"
        );
        assert!(
            out.contains(".product(name: \"SwiftTreeSitter\", package: \"SwiftTreeSitter\")"),
            "should inject product dep from extracted package name. Got:\n{out}"
        );
    }

    #[test]
    fn render_package_swift_extras_detailed_product_override() {
        // Repo identity (swift-tree-sitter) differs from the SwiftPM product (SwiftTreeSitter):
        // the explicit `product` key must drive `.product(name:)` while `package:` stays the identity.
        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "swift-tree-sitter".to_string(),
            ExtraDepSpec::Detailed({
                let mut t = toml::Table::new();
                t.insert(
                    "url".to_string(),
                    "https://github.com/tree-sitter/swift-tree-sitter".into(),
                );
                t.insert("version".to_string(), "0.25.0".into());
                t.insert("product".to_string(), "SwiftTreeSitter".into());
                t
            }),
        );
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        assert!(
            out.contains(".product(name: \"SwiftTreeSitter\", package: \"swift-tree-sitter\")"),
            "product override should set name=SwiftTreeSitter, package=swift-tree-sitter. Got:\n{out}"
        );
    }

    #[test]
    fn render_package_swift_with_extras_simple_form() {
        let mut extras = ManifestExtras::default();
        extras.dev_dependencies.insert(
            "https://github.com/bar/MyLib.git".to_string(),
            ExtraDepSpec::Simple("1.0.0".to_string()),
        );
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        assert!(
            out.contains(".package(url: \"https://github.com/bar/MyLib.git\", from: \"1.0.0\")"),
            "should inject extras .package with URL as key and version as value. Got:\n{out}"
        );
        assert!(
            out.contains(".product(name: \"MyLib\", package: \"MyLib\")"),
            "should extract package name from URL. Got:\n{out}"
        );
    }

    #[test]
    fn render_package_swift_extras_both_buckets() {
        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "runtime-pkg".to_string(),
            ExtraDepSpec::Detailed({
                let mut t = toml::Table::new();
                t.insert("url".to_string(), "https://github.com/x/RuntimePkg.git".into());
                t.insert("version".to_string(), "1.0.0".into());
                t
            }),
        );
        extras.dev_dependencies.insert(
            "https://github.com/y/DevPkg.git".to_string(),
            ExtraDepSpec::Simple("2.0.0".to_string()),
        );
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        // Should inject both runtime and dev deps in sorted order (DevPkg, RuntimePkg).
        assert!(
            out.contains(".package(url: \"https://github.com/x/RuntimePkg.git\", from: \"1.0.0\")"),
            "should inject runtime dep. Got:\n{out}"
        );
        assert!(
            out.contains(".package(url: \"https://github.com/y/DevPkg.git\", from: \"2.0.0\")"),
            "should inject dev dep. Got:\n{out}"
        );
        assert!(
            out.contains(".product(name: \"RuntimePkg\", package: \"RuntimePkg\")"),
            "should include runtime product. Got:\n{out}"
        );
        assert!(
            out.contains(".product(name: \"DevPkg\", package: \"DevPkg\")"),
            "should include dev product. Got:\n{out}"
        );
    }

    #[test]
    fn render_package_swift_empty_extras_matches_none() {
        let extras = ManifestExtras::default();
        let with_empty = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        let without = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            None,
        );
        assert_eq!(
            with_empty, without,
            "empty extras should produce identical output to None"
        );
    }

    #[test]
    fn render_package_swift_extras_idempotent() {
        let mut extras = ManifestExtras::default();
        extras.dependencies.insert(
            "https://github.com/a/PkgA.git".to_string(),
            ExtraDepSpec::Simple("1.0.0".to_string()),
        );
        let first = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        let second = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            Some(&extras),
        );
        assert_eq!(first, second, "re-rendering with same extras should be byte-stable");
    }

    #[test]
    fn render_package_swift_includes_harness_target_when_needed() {
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            true,
            None,
        );
        assert!(
            out.contains(".executableTarget("),
            "should include harness executable target"
        );
        assert!(out.contains("\"Harness\""), "harness target name should be present");
    }

    #[test]
    fn render_package_swift_omits_harness_target_when_not_needed() {
        let out = render_package_swift(
            "TreeSitter",
            "https://example.com/tree-sitter.git",
            "../../packages/swift",
            "0.25.0",
            crate::e2e::config::DependencyMode::Local,
            false,
            None,
        );
        assert!(
            !out.contains(".executableTarget("),
            "should omit harness executable target"
        );
    }

    #[test]
    fn render_package_swift_registry_mode_never_uses_from() {
        // Regression: .from() resolves SemVer tags (v1.2.3) which contain __ALEF_SWIFT_CHECKSUM__
        // placeholder. The real checksum lives on the release/swift/<version> branch created by
        // the publish workflow. This test ensures we always use .branch() for registry mode.
        let out = render_package_swift(
            "MyLib",
            "https://example.com/my-lib.git",
            "",
            "1.2.3",
            crate::e2e::config::DependencyMode::Registry,
            false,
            None,
        );
        assert!(
            !out.contains("from: \""),
            "registry mode should never use .package(url:, from:); use .branch() instead. Got:\n{out}"
        );
        assert!(
            out.contains("branch: \"release/swift/1.2.3\""),
            "should pin release/swift/<version> branch to resolve correct XCFramework checksum. Got:\n{out}"
        );
    }
}
