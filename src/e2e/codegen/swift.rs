//! Swift e2e test generator using XCTest.
//!
//! Generates a standalone Swift package at `e2e/swift_e2e/` that depends on the
//! binding at `packages/swift/` via `.package(path:)`.
//!
//! IMPORTANT: SwiftPM 6.0 derives the identity of path-based dependencies from
//! the path's *basename* and ignores any explicit `name:` override. If the
//! consumer (`e2e/swift/`) and the dep (`packages/swift/`) share the same path
//! basename `swift`, SwiftPM treats them as the same package and fails
//! resolution with: `product '<X>' required by package 'swift' target '...' not
//! found in package 'swift'`. The e2e package is therefore emitted under
//! `swift_e2e/` to guarantee a distinct identity from any sibling
//! `packages/swift/` dep.

use crate::codegen::keywords::swift_ident;
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::template_versions::toolchain;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{
    escape_java as escape_swift_str, expand_fixture_templates, sanitize_filename, sanitize_ident,
};
use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup, ValidationErrorExpectation};
use anyhow::Result;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use regex::Regex;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

// Empty `result_field_accessor` map shared across calls that don't configure
// one. Using a `OnceLock` lets `render_test_method` hand out a stable
// reference without rebuilding the empty `HashMap` for every fixture.
static EMPTY_FIELD_ACCESSOR_MAP: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();

fn empty_field_accessor_map() -> &'static HashMap<String, String> {
    EMPTY_FIELD_ACCESSOR_MAP.get_or_init(HashMap::new)
}
use super::client;

/// Swift e2e code generator.
pub struct SwiftE2eCodegen;

impl E2eCodegen for SwiftE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        // Emit under `<output>/swift_e2e/` so the consumer's SwiftPM identity
        // (derived from path basename) does not collide with the dep at
        // `packages/swift/` (also basename `swift`). SwiftPM 6.0 deprecated the
        // `name:` parameter on `.package(path:)` and uses the path basename as
        // the package's identity unconditionally, so disambiguation must happen
        // at the filesystem level. Consumers of the alef-emitted e2e must
        // `cd e2e/swift_e2e/` to run `swift test`.
        let output_base = PathBuf::from(e2e_config.effective_output()).join("swift_e2e");

        let mut files = Vec::new();

        // Check if any fixture is an HTTP test (needs app harness and HTTP framework).
        let has_http_fixtures = groups.iter().any(|g| g.fixtures.iter().any(|f| f.is_http_test()));

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let swift_pkg = e2e_config.resolve_package("swift");
        let pkg_name = swift_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_upper_camel_case());
        let pkg_path = swift_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/swift".to_string());
        let pkg_version = swift_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // The Swift module name: UpperCamelCase of the package name.
        let module_name = pkg_name.as_str();

        // Resolve the registry URL: derive from the configured repository when
        // available (with a `.git` suffix per SwiftPM convention). Falls back
        // to a vendor-neutral placeholder when no repo is configured.
        let registry_url = config
            .try_github_repo()
            .map(|repo| {
                let base = repo.trim_end_matches('/').trim_end_matches(".git");
                format!("{base}.git")
            })
            .unwrap_or_else(|_| format!("https://example.invalid/{module_name}.git"));

        // Generate Package.swift for the standalone e2e consumer at
        // `<output>/swift_e2e/`. `swift test` is run from that directory.
        files.push(GeneratedFile {
            path: output_base.join("Package.swift"),
            content: render_package_swift(
                module_name,
                &registry_url,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
                has_http_fixtures,
            ),
            generated_header: false,
        });

        // For registry mode, emit a pre-test script that computes the artifact checksum.
        if matches!(e2e_config.dep_mode, crate::e2e::config::DependencyMode::Registry) {
            files.push(GeneratedFile {
                path: output_base.join("download_swift_artifact.sh"),
                content: render_download_swift_artifact_script(module_name, &registry_url, &pkg_version),
                generated_header: false,
            });
        }

        // Generate the app harness executable that runs the SUT server for tests.
        // Only emit when there are HTTP fixtures; consumers without HTTP tests
        // don't need the harness or its HTTP framework dependency.
        if has_http_fixtures {
            let app_harness_body = render_app_harness(e2e_config, groups, module_name);
            let app_harness_content = format!("{}{}", hash::header(CommentStyle::DoubleSlash), app_harness_body);
            files.push(GeneratedFile {
                path: output_base.join("Sources").join("Harness").join("main.swift"),
                content: app_harness_content,
                generated_header: false,
            });
        }

        // Tests are placed alongside Package.swift under `<output>/swift_e2e/Tests/...`.
        let tests_base = output_base.clone();

        // Build the Swift first-class/opaque classification map for per-segment
        // dispatch in `render_swift_with_first_class_map`. A TypeDef is treated
        // as first-class (Codable struct → property access) when it's not opaque,
        // has serde derives, and every binding field is primitive/optional. This
        // mirrors `can_emit_first_class_struct` in alef-backend-swift.
        let swift_first_class_map = build_swift_first_class_map(type_defs, enums, e2e_config);

        let swift_first_class_map_ref = swift_first_class_map;

        // Resolve client_factory override for swift (enables client-instance dispatch).
        let client_factory: Option<&str> = overrides.and_then(|o| o.client_factory.as_deref());

        // Emit a shared TestHelpers.swift that gives `RustString` a
        // `CustomStringConvertible` conformance. swift-bridge generates the
        // `RustString` opaque class but does NOT make it print readably — so
        // any error thrown from a bridge function (the `throw RustString(...)`
        // branches) surfaces in XCTest's failure output as the bare type name
        // `"RustBridge.RustString"`, with the actual Rust error message
        // hidden inside the unprinted instance. The retroactive extension
        // here pulls `.toString()` into `.description` so failures print
        // something diagnostic. Single file per test target; idempotent
        // across regens.
        files.push(GeneratedFile {
            path: tests_base
                .join("Tests")
                .join(format!("{module_name}E2ETests"))
                .join("TestHelpers.swift"),
            content: render_test_helpers_swift(),
            generated_header: true,
        });

        // One test file per fixture group.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_name = format!("{}Tests", sanitize_filename(&group.category).to_upper_camel_case());
            let filename = format!("{class_name}.swift");
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                module_name,
                &class_name,
                &function_name,
                result_var,
                &e2e_config.call.args,
                result_is_simple,
                client_factory,
                &swift_first_class_map_ref,
                config,
                type_defs,
                has_http_fixtures,
            );
            files.push(GeneratedFile {
                path: tests_base
                    .join("Tests")
                    .join(format!("{module_name}E2ETests"))
                    .join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "swift"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Directive telling Apple's `swift-format` to skip the file entirely.
///
/// The e2e generator emits Swift source with 4-space indentation, fixed import
/// order (`XCTest, Foundation, <Module>`) and unwrapped long lines
/// — all of which violate `swift-format`'s defaults (2-space indent, sorted
/// imports, 100-char line width). Reformatting after every regen would force
/// every consumer repo to either bake `swift-format` into their pre-commit set
/// or eat noisy diffs. Marking the files as ignored is the same workaround the
/// Swift binding backend uses for `SampleMarkdown.swift` (see
/// `alef-backend-swift/src/gen_bindings.rs`) and keeps the file
/// byte-identical between `alef generate` runs and `swift-format` hooks.
const SWIFT_FORMAT_IGNORE_DIRECTIVE: &str = "// swift-format-ignore-file\n\n";

/// Render the shared `TestHelpers.swift` file emitted into each Swift e2e
/// test target. Adds a `CustomStringConvertible` conformance to swift-bridge's
/// `RustString` so error messages from bridge throws print their actual Rust
/// content instead of the bare class name.
fn render_test_helpers_swift() -> String {
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

// ---------------------------------------------------------------------------
// App Harness Rendering
// ---------------------------------------------------------------------------

fn render_app_harness(e2e_config: &E2eConfig, groups: &[FixtureGroup], module_name: &str) -> String {
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
    // Prepend the binding module_name if not already present.
    if !imports.iter().any(|i| i == module_name) {
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
        fixtures_json => fixtures_json,
    };

    crate::e2e::template_env::render("swift/app_harness.swift.jinja", ctx)
}

fn render_package_swift(
    module_name: &str,
    registry_url: &str,
    pkg_path: &str,
    pkg_version: &str,
    dep_mode: crate::e2e::config::DependencyMode,
    include_harness_target: bool,
) -> String {
    let min_macos = toolchain::SWIFT_MIN_MACOS;

    // For local deps SwiftPM identity = last path component (e.g. "../../packages/swift" → "swift").
    // For registry deps we use .binaryTarget(url:, checksum:) to avoid SwiftPM tag-URL pinning
    // which fails when tags carry placeholder substitution. The pre-test script
    // `download_swift_artifact.sh` computes the actual checksum at test time.
    // Use explicit .product(name:package:) to avoid ambiguity under tools-version 6.0.
    let (binary_target_block, test_target_dep) = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // Binary target URL: https://github.com/<owner>/<repo>/releases/download/v<version>/<Module>-rs.artifactbundle.zip
            let github_repo_url = registry_url.trim_end_matches(".git");
            let artifact_url =
                format!("{github_repo_url}/releases/download/v{pkg_version}/{module_name}-rs.artifactbundle.zip");
            let target = format!(
                r#"        .binaryTarget(name: "{module_name}", url: "{artifact_url}", checksum: "__ALEF_SWIFT_CHECKSUM__")"#
            );
            let dep = format!(r#".target(name: "{module_name}")"#);
            (Some(target), dep)
        }
        crate::e2e::config::DependencyMode::Local => {
            // SwiftPM 6.0 deprecated the `name:` parameter on `.package(path:)`:
            // package identity is derived from the path's last component, ignoring
            // any explicit `name:`. For local mode, the dependency is still the external package,
            // but we reference it via .product(name:package:) in the test target dependencies.
            // We do NOT emit a binary target block for local deps.
            let pkg_id = pkg_path.trim_end_matches('/').rsplit('/').next().unwrap_or(module_name);
            let prod = format!(r#".product(name: "{module_name}", package: "{pkg_id}")"#);
            (None, prod)
        }
    };
    // Local deps must be declared as a top-level package dependency so the
    // `.product(package:)` reference in the test target resolves. Registry deps
    // are vendored via `.binaryTarget` (a target, not a package dependency), so
    // no top-level `dependencies:` array is emitted in that mode.
    let dependencies_block = match dep_mode {
        crate::e2e::config::DependencyMode::Local => {
            format!("    dependencies: [\n        .package(path: \"{pkg_path}\"),\n    ],\n")
        }
        crate::e2e::config::DependencyMode::Registry => String::new(),
    };
    // SwiftPM platform enums use the major version only (.v13, .v14, ...);
    // strip patch components to match the scaffold's `Package.swift`.
    let min_macos_major = min_macos.split('.').next().unwrap_or(min_macos);
    let min_ios = toolchain::SWIFT_MIN_IOS;
    let min_ios_major = min_ios.split('.').next().unwrap_or(min_ios);
    // The consumer's minimum iOS must be >= the dep's minimum iOS or SwiftPM hides
    // the product as platform-incompatible. Use the same constant the swift backend
    // emits into the dep's Package.swift.
    let targets_block = if let Some(binary_target) = binary_target_block {
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
        format!(
            r#"        {binary_target},
{harness_target}        .testTarget(
            name: "{module_name}E2ETests",
            dependencies: [{test_target_dep}]
        ),
"#
        )
    } else {
        // Local mode: no binary target, just the executable harness and test target.
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
        format!(
            r#"{harness_target}        .testTarget(
            name: "{module_name}E2ETests",
            dependencies: [{test_target_dep}]
        ),
"#
        )
    };
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

/// Render a pre-test shell script that computes the Swift artifact checksum at runtime.
/// Registry-mode e2e tests use .binaryTarget(url:, checksum:) with a placeholder
/// checksum (__ALEF_SWIFT_CHECKSUM__). This script downloads the artifact bundle,
/// computes its SHA256 checksum, and validates that it matches the expected checksum
/// in Package.swift before using it. If a cached artifact has a mismatched checksum,
/// the cache is invalidated and re-downloaded.
fn render_download_swift_artifact_script(module_name: &str, registry_url: &str, pkg_version: &str) -> String {
    let github_repo_url = registry_url.trim_end_matches(".git");
    let artifact_url =
        format!("{github_repo_url}/releases/download/v{pkg_version}/{module_name}-rs.artifactbundle.zip");
    format!(
        r#"#!/bin/bash
set -euo pipefail

# Download the Swift artifact bundle and compute its checksum.
# SwiftPM requires a stable SHA256 checksum for binary targets.
# Cache is validated against the expected checksum in Package.swift to detect
# version mismatches (e.g., when upgrading from rc.49 to rc.50, the filename
# stays the same but the URL changes and the cached zip becomes stale).

ARTIFACT_URL="{artifact_url}"
ARTIFACT_FILE="{module_name}-rs.artifactbundle.zip"
PACKAGE_SWIFT="Package.swift"

# Extract the expected checksum from Package.swift.
# Look for the pattern: checksum: "0123456789abcdef..."
EXPECTED_CHECKSUM=$(grep -oE 'checksum:\s+"[a-f0-9]{{64}}"' "$PACKAGE_SWIFT" | head -1 | grep -oE '[a-f0-9]{{64}}' || true)

# Determine whether to use or invalidate the cache.
SHOULD_DOWNLOAD=true
if [ -f "$ARTIFACT_FILE" ]; then
  if [ -n "$EXPECTED_CHECKSUM" ]; then
    # Cache exists and we know the expected checksum: validate before reusing.
    ACTUAL_CHECKSUM=$(swift package compute-checksum "$ARTIFACT_FILE")
    if [ "$EXPECTED_CHECKSUM" = "$ACTUAL_CHECKSUM" ]; then
      echo "Using cached artifact (checksum validated): $ARTIFACT_FILE"
      SHOULD_DOWNLOAD=false
    else
      echo "Cached artifact checksum mismatch (expected: $EXPECTED_CHECKSUM, got: $ACTUAL_CHECKSUM)"
      echo "Removing stale cache and re-downloading"
      rm -f "$ARTIFACT_FILE"
    fi
  else
    # Expected checksum not yet resolved (placeholder not substituted): assume cache is stale
    echo "Unable to extract expected checksum from $PACKAGE_SWIFT; invalidating cache"
    rm -f "$ARTIFACT_FILE"
  fi
fi

# Download if needed
if [ "$SHOULD_DOWNLOAD" = true ]; then
  echo "Downloading Swift artifact from $ARTIFACT_URL"
  curl -fsSL -o "$ARTIFACT_FILE" "$ARTIFACT_URL"
fi

# Compute SHA256 checksum
CHECKSUM=$(swift package compute-checksum "$ARTIFACT_FILE")
echo "Computed checksum: $CHECKSUM"

# Substitute the placeholder checksum in Package.swift
sed -i.bak "s/__ALEF_SWIFT_CHECKSUM__/$CHECKSUM/g" "$PACKAGE_SWIFT"
rm -f "${{PACKAGE_SWIFT}}.bak"

echo "Updated $PACKAGE_SWIFT with checksum"
"#
    )
}

#[allow(clippy::too_many_arguments)]
fn render_test_file(
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

    // Spawn the harness subprocess if SUT_URL is not already set.
    // Only emit when there are HTTP fixtures; consumers without HTTP tests
    // don't need the harness.
    let _ = writeln!(
        out,
        "        let _existing = ProcessInfo.processInfo.environment[\"SUT_URL\"]"
    );
    if has_http_fixtures {
        let _ = writeln!(out, "        if _existing == nil {{");
        let _ = writeln!(out, "            let _harness = URL(fileURLWithPath: #filePath)");
        let _ = writeln!(out, "                .deletingLastPathComponent() // <Module>Tests/");
        let _ = writeln!(out, "                .deletingLastPathComponent() // Tests/");
        let _ = writeln!(out, "                .deletingLastPathComponent() // swift_e2e/");
        let _ = writeln!(out, "                .deletingLastPathComponent() // e2e/");
        let _ = writeln!(out, "                .appendingPathComponent(\"swift_e2e\")");
        let _ = writeln!(
            out,
            "                .appendingPathComponent(\".build/release/Harness\")"
        );
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
        let _ = writeln!(
            out,
            "            let _probeURL = URL(string: \"http://127.0.0.1:8009/\")!"
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
        let _ = writeln!(
            out,
            "            ProcessInfo.processInfo.environment[\"SUT_URL\"] = \"http://127.0.0.1:8009\""
        );
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
            render_http_test_method(&mut out, fixture);
        } else {
            render_test_method(
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
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "}}");
    out
}

// ---------------------------------------------------------------------------
// HTTP test rendering — TestClientRenderer impl + thin driver wrapper
// ---------------------------------------------------------------------------

/// Renderer that emits XCTest `func test...() throws` methods using `URLSession`
/// against the mock server (`ProcessInfo.processInfo.environment["MOCK_SERVER_URL"]`).
struct SwiftTestClientRenderer;

impl client::TestClientRenderer for SwiftTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "swift"
    }

    fn sanitize_test_name(&self, id: &str) -> String {
        // Swift test methods are `func testFoo()` — upper-camel-case after "test".
        sanitize_ident(id).to_upper_camel_case()
    }

    /// Emit `func test{FnName}() throws {` (or a skip stub when the fixture is skipped).
    ///
    /// XCTest has no first-class skip annotation prior to Swift Testing (`@Test`).
    /// For skipped fixtures we emit `try XCTSkipIf(true, reason)` inside the
    /// function body so XCTest records them as skipped rather than omitting them.
    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        let _ = writeln!(out, "    /// {description}");
        let _ = writeln!(out, "    func test{fn_name}() throws {{");
        if let Some(reason) = skip_reason {
            let escaped = escape_swift(reason);
            let _ = writeln!(out, "        try XCTSkipIf(true, \"{escaped}\")");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "    }}");
    }

    /// Emit a synchronous `URLSession` round-trip to the SUT server.
    ///
    /// `ProcessInfo.processInfo.environment["SUT_URL"]!` provides the base
    /// URL; the fixture path is appended directly.  The call uses a semaphore so the
    /// generated test body stays synchronous (compatible with `throws` functions —
    /// no `async` XCTest support needed).
    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_path = escape_swift(ctx.path);

        let _ = writeln!(
            out,
            "        let _baseURL = ProcessInfo.processInfo.environment[\"SUT_URL\"] ?? \"http://127.0.0.1:8009\""
        );
        let _ = writeln!(
            out,
            "        var _req = URLRequest(url: URL(string: _baseURL + \"{fixture_path}\")!)"
        );
        let _ = writeln!(out, "        _req.httpMethod = \"{method}\"");

        // Headers
        let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
        header_pairs.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in &header_pairs {
            let expanded_v = expand_fixture_templates(v);
            let ek = escape_swift(k);
            let ev = escape_swift(&expanded_v);
            let _ = writeln!(out, "        _req.setValue(\"{ev}\", forHTTPHeaderField: \"{ek}\")");
        }

        // Body
        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped_body = escape_swift(&json_str);
            let _ = writeln!(out, "        _req.httpBody = \"{escaped_body}\".data(using: .utf8)");
            let _ = writeln!(
                out,
                "        _req.setValue(\"application/json\", forHTTPHeaderField: \"Content-Type\")"
            );
        }

        let _ = writeln!(out, "        var {}: HTTPURLResponse?", ctx.response_var);
        let _ = writeln!(out, "        var _responseData: Data?");
        let _ = writeln!(out, "        let _sema = DispatchSemaphore(value: 0)");
        let _ = writeln!(
            out,
            "        let _session = URLSession(configuration: .ephemeral, delegate: AlefE2ENoRedirectDelegate(), delegateQueue: nil)"
        );
        let _ = writeln!(out, "        _session.dataTask(with: _req) {{ data, resp, _ in");
        let _ = writeln!(out, "            {} = resp as? HTTPURLResponse", ctx.response_var);
        let _ = writeln!(out, "            _responseData = data");
        let _ = writeln!(out, "            _sema.signal()");
        let _ = writeln!(out, "        }}.resume()");
        let _ = writeln!(out, "        _sema.wait()");
        let _ = writeln!(out, "        let _resp = try XCTUnwrap({})", ctx.response_var);
    }

    fn render_assert_status(&self, out: &mut String, _response_var: &str, status: u16) {
        let _ = writeln!(out, "        XCTAssertEqual(_resp.statusCode, {status})");
    }

    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let lower_name = name.to_lowercase();
        let header_expr = format!("_resp.value(forHTTPHeaderField: \"{}\")", escape_swift(&lower_name));
        // Header names contain characters illegal in Swift identifiers (e.g. the `-` in
        // `x-request-id`), so derive a safe local-variable suffix for any binding we emit.
        let var_suffix: String = lower_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        match expected {
            "<<present>>" => {
                let _ = writeln!(out, "        XCTAssertNotNil({header_expr})");
            }
            "<<absent>>" => {
                let _ = writeln!(out, "        XCTAssertNil({header_expr})");
            }
            "<<uuid>>" => {
                let _ = writeln!(out, "        let _hdrVal_{var_suffix} = try XCTUnwrap({header_expr})");
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil(_hdrVal_{var_suffix}.range(of: #\"^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$\"#, options: .regularExpression))"
                );
            }
            exact => {
                let escaped = escape_swift(exact);
                let _ = writeln!(out, "        XCTAssertEqual({header_expr}, \"{escaped}\")");
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let serde_json::Value::String(s) = expected {
            let escaped = escape_swift(s);
            let _ = writeln!(
                out,
                "        let _bodyStr = String(data: try XCTUnwrap(_responseData), encoding: .utf8) ?? \"\""
            );
            let _ = writeln!(
                out,
                "        XCTAssertEqual(_bodyStr.trimmingCharacters(in: .whitespacesAndNewlines), \"{escaped}\")"
            );
        } else {
            let json_str = serde_json::to_string(expected).unwrap_or_default();
            let escaped = escape_swift(&json_str);
            let _ = writeln!(
                out,
                "        let _expected = try JSONSerialization.jsonObject(with: \"{escaped}\".data(using: .utf8)!)"
            );
            // Unwrap the response data inline rather than via a shared `_bodyData` local: a
            // fixture may trigger several body assertions in one test, and a repeated
            // `let _bodyData` would be an invalid redeclaration. The leading `try` covers the
            // nested XCTUnwrap call.
            let _ = writeln!(
                out,
                "        let _actual = try JSONSerialization.jsonObject(with: XCTUnwrap(_responseData))"
            );
            let _ = writeln!(
                out,
                "        XCTAssertEqual(NSDictionary(dictionary: _expected as? [String: AnyHashable] ?? [:]), NSDictionary(dictionary: _actual as? [String: AnyHashable] ?? [:]))"
            );
        }
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            let _ = writeln!(
                out,
                "        let _bodyObj = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(_responseData)) as? [String: Any])"
            );
            for (key, val) in obj {
                let escaped_key = escape_swift(key);
                let swift_val = json_to_swift(val);
                let _ = writeln!(
                    out,
                    "        XCTAssertEqual(_bodyObj[\"{escaped_key}\"] as? AnyHashable, ({swift_val}) as AnyHashable)"
                );
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[ValidationErrorExpectation],
    ) {
        let _ = writeln!(
            out,
            "        let _errorsBodyObj = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(_responseData)) as? [String: Any])"
        );
        let _ = writeln!(
            out,
            "        let _errors = _errorsBodyObj[\"errors\"] as? [[String: Any]] ?? []"
        );
        for ve in errors {
            let escaped_msg = escape_swift(&ve.msg);
            let _ = writeln!(
                out,
                "        XCTAssertTrue(_errors.contains(where: {{ ($0[\"msg\"] as? String)?.contains(\"{escaped_msg}\") == true }}), \"expected validation error: {escaped_msg}\")"
            );
        }
    }
}

/// Render an XCTest method for an HTTP server fixture via the shared driver.
///
/// HTTP 101 (WebSocket upgrade) is emitted as a skip stub because `URLSession`
/// cannot handle Upgrade responses.
fn render_http_test_method(out: &mut String, fixture: &Fixture) {
    let Some(http) = &fixture.http else {
        return;
    };

    // HTTP 101 (WebSocket upgrade) — URLSession cannot handle upgrade responses.
    if http.expected_response.status_code == 101 {
        let method_name = sanitize_ident(&fixture.id).to_upper_camel_case();
        let description = fixture.description.replace('"', "\\\"");
        let _ = writeln!(out, "    /// {description}");
        let _ = writeln!(out, "    func test{method_name}() throws {{");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"HTTP 101 WebSocket upgrade cannot be tested via URLSession\")"
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    client::http_call::render_http_test(out, &SwiftTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_method(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::e2e::config::ArgMapping],
    result_is_simple: bool,
    global_client_factory: Option<&str>,
    swift_first_class_map: &SwiftFirstClassMap,
    module_name: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Build per-call field resolver using the effective field sets for this call.
    let call_field_resolver = FieldResolver::new_with_swift_first_class(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
        &HashMap::new(),
        swift_first_class_map.clone(),
    );
    let field_resolver = &call_field_resolver;
    let enum_fields = e2e_config.effective_fields_enum(call_config);
    let lang = "swift";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| swift_ident(&call_config.function.to_lower_camel_case()));
    // Per-call client_factory takes precedence over the global one.
    let client_factory: Option<&str> = call_overrides
        .and_then(|o| o.client_factory.as_deref())
        .or(global_client_factory);
    let result_var = &call_config.result_var;
    let args = fixture.resolved_args(call_config);
    // Per-call flags: base call flag OR per-language override OR global flag.
    // Also treat the call as simple when *any* language override marks it as bytes.
    // Calls like `speech()` have `result_is_bytes = true` on C/C#/Java overrides but
    // no explicit `result_is_simple` on the Swift override — yet the Swift binding
    // returns `Data` directly (not a struct), so assertions must use `result.isEmpty`
    // rather than `result.audio().toString().isEmpty`.
    let result_is_bytes_any_lang =
        call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    let result_is_simple = call_config.result_is_simple
        || call_overrides.is_some_and(|o| o.result_is_simple)
        || result_is_simple
        || result_is_bytes_any_lang;
    let result_is_array = call_config.result_is_array;
    // When the call returns `Option<T>` the Swift binding exposes the result as
    // `Optional<…>` (e.g. `getEmbeddingPreset(...) -> EmbeddingPreset?`). Bare-result
    // `is_empty`/`not_empty` assertions must use `XCTAssertNil` / `XCTAssertNotNil`
    // rather than `.toString().isEmpty`, which is undefined on opaque optionals.
    let result_is_option = call_config.result_is_option || call_overrides.is_some_and(|o| o.result_is_option);
    let result_element_is_string =
        call_config.result_element_is_string || call_overrides.is_some_and(|o| o.result_element_is_string);
    // Per-language map of array-result-field → element accessor method (e.g.
    // `structure → kind`). Empty map when no override is configured.
    let result_field_accessor: &HashMap<String, String> = call_overrides
        .map(|o| &o.result_field_accessor)
        .unwrap_or_else(|| empty_field_accessor_map());

    let method_name = fixture.id.to_upper_camel_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_async = call_overrides.and_then(|o| o.r#async).unwrap_or(call_config.r#async);

    // Streaming detection (call-level `streaming` opt-out is honored).
    let is_streaming = crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming);

    // Infer the stream chunk item type from the function name or default to ChatCompletionChunk.
    // Streaming adapters define item_type (e.g., "CrawlEvent" in sample-crawler).
    // When the function name contains "stream", infer the concrete item type based on context.
    let chunk_item_type = if is_streaming && !expects_error {
        // For sample-crawler crawl_stream and batch_crawl_stream, the chunk type is CrawlEvent
        if call_config.function.contains("stream") {
            match call_config.function.to_lowercase().as_str() {
                s if s.contains("crawl_stream") || s.contains("crawlstream") => Some("CrawlEvent"),
                // Default to ChatCompletionChunk for LLM-like streaming (sample-llm pattern)
                _ => Some("ChatCompletionChunk"),
            }
        } else {
            Some("ChatCompletionChunk")
        }
    } else {
        None
    };

    let collect_snippet_opt = if is_streaming && !expects_error {
        crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet_typed(
            lang,
            result_var,
            "chunks",
            chunk_item_type,
        )
    } else {
        None
    };
    // When swift has streaming-virtual-field assertions but no collect snippet
    // is available (the swift-bridge surface does not yet expose a typed
    // `chatStream` async sequence we can drain into a typed
    // `[ChatCompletionChunk]`), emit a skip stub rather than reference an
    // undefined `chunks` local in the assertion expressions. This keeps the
    // swift test target compiling while the binding catches up.
    if is_streaming && !expects_error && collect_snippet_opt.is_none() {
        if is_async {
            let _ = writeln!(out, "    func test{method_name}() async throws {{");
        } else {
            let _ = writeln!(out, "    func test{method_name}() throws {{");
        }
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"swift: streaming chunk collection is not yet supported via the swift-bridge surface (fixture: {})\")",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }
    let collect_snippet = collect_snippet_opt.unwrap_or_default();
    // The shared streaming snippet may reference unqualified types like `ChatCompletionChunk`
    // or `CrawlEvent`. Swift consumers import both `<Module>` (the alef-emitted first-class
    // types) AND `RustBridge` (swift-bridge generated types). Without module qualification
    // for ambiguous types, Swift fails with "'Type' is ambiguous for type lookup".
    // Qualify all bracketed type names to the first-class module type.
    let collect_snippet = if collect_snippet.is_empty() {
        collect_snippet
    } else {
        // Replace `[<ItemType>]` with module-qualified `[<Module>.<ItemType>]`
        // This handles both ChatCompletionChunk (sample-llm) and CrawlEvent (sample-crawler).
        let re = Regex::new(r"\[([A-Za-z][A-Za-z0-9]*)\]").expect("valid regex");
        let module_qualifier = module_name;
        re.replace_all(&collect_snippet, |caps: &regex::Captures| {
            format!("[{}.{}]", module_qualifier, &caps[1])
        })
        .to_string()
    };

    // Detect whether this call has any json_object args that cannot be constructed
    // in Swift. Most json_object args are now handled:
    // - Scalar element types (Vec<String>, Vec<i32>, etc.) map to Swift arrays directly
    // - Array element types (Vec<DataEnum>, Vec<Struct>, etc.) are serialized to JSON strings
    // - config args are handled via options_via or default helpers
    // The only unresolvable case is a json_object arg with NO array (not a Vec) and no
    // options_via configured, which should not occur in practice. We skip in only that case.
    let has_unresolvable_json_object_arg = {
        let options_via = call_overrides.and_then(|o| o.options_via.as_deref());
        options_via.is_none()
            && args.iter().any(|a| {
                // json_object args with an element_type (Vec<T>) are always resolvable.
                // Skip only non-array json_object args without options_via.
                a.arg_type == "json_object" && a.name != "config" && a.element_type.is_none() && options_via.is_none()
            })
    };

    if has_unresolvable_json_object_arg {
        if is_async {
            let _ = writeln!(out, "    func test{method_name}() async throws {{");
        } else {
            let _ = writeln!(out, "    func test{method_name}() throws {{");
        }
        let _ = writeln!(out, "        // {description}");
        let _ = writeln!(
            out,
            "        try XCTSkipIf(true, \"swift: json_object requires options_via configuration (fixture: {})\");",
            fixture.id
        );
        let _ = writeln!(out, "    }}");
        return;
    }

    // Visitor-driven fixtures: emit a class that conforms to `HtmlVisitorProtocol`
    // and wrap it via `makeHtmlVisitorHandle(...)`. The handle is then threaded
    // into the options via `conversionOptionsFromJsonWithVisitor(json, handle)`.
    let mut visitor_setup_lines: Vec<String> = Vec::new();
    let visitor_handle_expr: Option<String> = fixture
        .visitor
        .as_ref()
        .map(|spec| super::swift_visitors::build_swift_visitor(&mut visitor_setup_lines, spec, &fixture.id));

    // Resolve extra_args from per-call swift overrides (e.g. `nil` for optional
    // query-param arguments on list_files/list_batches that have no fixture-level
    // input field).
    let extra_args: Vec<String> = call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();

    // Merge per-call enum_fields keys into the effective enum set so that
    // fields like "status" (BatchStatus, BatchObject) are treated as enum-typed
    // even when they are not globally listed in fields_enum (they are context-
    // dependent — BatchStatus on BatchObject but plain String on ResponseObject).
    let effective_enum_fields: std::borrow::Cow<HashSet<String>> = {
        let per_call = call_overrides.map(|o| &o.enum_fields);
        if let Some(pc) = per_call {
            if !pc.is_empty() {
                let mut merged = enum_fields.clone();
                merged.extend(pc.keys().cloned());
                std::borrow::Cow::Owned(merged)
            } else {
                std::borrow::Cow::Borrowed(enum_fields)
            }
        } else {
            std::borrow::Cow::Borrowed(enum_fields)
        }
    };

    let options_via_str: Option<&str> = call_overrides.and_then(|o| o.options_via.as_deref());
    let options_type_str: Option<&str> = call_overrides
        .and_then(|o| o.options_type.as_deref())
        .or(call_config.options_type.as_deref());
    // Derive the Swift handle-config parsing function from the C override's
    // `c_engine_factory` field. E.g. `"CrawlConfig"` → snake → `"crawl_config_from_json"`
    // → camelCase → `"crawlConfigFromJson"`.
    let handle_config_fn_owned: Option<String> = call_config
        .overrides
        .get("c")
        .and_then(|c| c.c_engine_factory.as_deref())
        .map(|ty| format!("{}_from_json", ty.to_snake_case()).to_lower_camel_case());
    let unnamed_arg_indices: &[usize] = call_overrides.map(|o| &o.unnamed_arg_indices[..]).unwrap_or(&[]);
    let arg_name_map = call_overrides.map(|o| &o.arg_name_map);
    let (mut setup_lines, args_str) = build_args_and_setup(
        &fixture.input,
        args,
        &fixture.id,
        fixture.has_host_root_route(),
        &function_name,
        options_via_str,
        options_type_str,
        handle_config_fn_owned.as_deref(),
        visitor_handle_expr.as_deref(),
        client_factory.is_some(),
        module_name,
        unnamed_arg_indices,
        config,
        type_defs,
        fixture,
        arg_name_map,
    );
    // Prepend visitor class declarations (before any setup lines that reference the handle).
    if !visitor_setup_lines.is_empty() {
        visitor_setup_lines.extend(setup_lines);
        setup_lines = visitor_setup_lines;
    }

    // Append extra_args to the argument list.
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // When a client_factory is set, dispatch via a client instance:
    //   let client = try <FactoryType>(apiKey: "test-key", baseUrl: <mock_url>)
    //   try await client.<method>(args)
    // Otherwise fall back to free-function call (SampleCrate / non-client-factory libraries).
    let has_mock = fixture.mock_response.is_some();
    let (call_setup, call_expr) = if let Some(_factory) = client_factory {
        let env_key = format!("MOCK_SERVER_{}", fixture.id.to_ascii_uppercase().replace('-', "_"));
        let mock_url = if fixture.has_host_root_route() {
            format!(
                "ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{}\")",
                fixture.id
            )
        } else {
            format!("AlefE2EMockServer.baseURL + \"/fixtures/{}\"", fixture.id)
        };
        let client_constructor = if has_mock {
            format!("let _client = try DefaultClient(apiKey: \"test-key\", baseUrl: {mock_url})")
        } else {
            // Live API: check for api_key_var; if not present use mock URL anyway.
            if let Some(env_var) = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref()) {
                format!(
                    "let _apiKey = ProcessInfo.processInfo.environment[\"{env_var}\"]\n        \
                     let _baseUrl: String? = _apiKey != nil ? nil : {mock_url}\n        \
                     let _client = try DefaultClient(apiKey: _apiKey ?? \"test-key\", baseUrl: _baseUrl)"
                )
            } else {
                format!("let _client = try DefaultClient(apiKey: \"test-key\", baseUrl: {mock_url})")
            }
        };
        let expr = if is_async {
            format!("try await _client.{function_name}({args_str})")
        } else {
            format!("try _client.{function_name}({args_str})")
        };
        (Some(client_constructor), expr)
    } else {
        // Free-function call (no client_factory).
        // Qualify with module name to disambiguate between high-level and swift-bridge symbols.
        let expr = if is_async {
            format!("try await {module_name}.{function_name}({args_str})")
        } else {
            format!("try {module_name}.{function_name}({args_str})")
        };
        (None, expr)
    };
    // For backwards compatibility: qualified_function_name unused when client_factory is set.
    let _ = function_name;

    if is_async {
        let _ = writeln!(out, "    func test{method_name}() async throws {{");
    } else {
        let _ = writeln!(out, "    func test{method_name}() throws {{");
    }
    let _ = writeln!(out, "        // {description}");

    if expects_error {
        // For error fixtures, setup may itself throw (e.g. config validation
        // happens at engine construction). Wrap the whole pipeline — setup
        // and the call — in a single do/catch so any throw counts as success.
        if is_async {
            // XCTAssertThrowsError is a synchronous macro; for async-throwing
            // functions use a do/catch with explicit XCTFail to enforce that
            // the throw actually happens. `await XCTAssertThrowsError(...)` is
            // not valid Swift — it evaluates `await` against a non-async expr.
            let _ = writeln!(out, "        do {{");
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            if let Some(setup) = &call_setup {
                let _ = writeln!(out, "            {setup}");
            }
            let _ = writeln!(out, "            _ = {call_expr}");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        } else {
            // Synchronous: emit setup outside (it's expected to succeed) and
            // wrap only the throwing call in XCTAssertThrowsError. If setup
            // itself throws, that propagates as the test's own failure — but
            // sync tests use `throws` so the test method itself rethrows,
            // which XCTest still records as caught. Keep this simple: use a
            // do/catch so setup-time throws also count as expected failures.
            let _ = writeln!(out, "        do {{");
            for line in &setup_lines {
                let _ = writeln!(out, "            {line}");
            }
            if let Some(setup) = &call_setup {
                let _ = writeln!(out, "            {setup}");
            }
            let _ = writeln!(out, "            _ = {call_expr}");
            let _ = writeln!(out, "            XCTFail(\"expected to throw\")");
            let _ = writeln!(out, "        }} catch {{");
            let _ = writeln!(out, "            // success");
            let _ = writeln!(out, "        }}");
        }
        let _ = writeln!(out, "    }}");
        return;
    }

    for line in &setup_lines {
        let _ = writeln!(out, "        {line}");
    }

    // Emit client construction if a client_factory is configured.
    if let Some(setup) = &call_setup {
        let _ = writeln!(out, "        {setup}");
    }

    let _ = writeln!(out, "        let {result_var} = {call_expr}");

    // Emit the collect snippet for streaming fixtures (drains the async sequence into
    // a local `chunks: [ChatCompletionChunk]` array used by streaming-virtual assertions).
    if !collect_snippet.is_empty() {
        for line in collect_snippet.lines() {
            let _ = writeln!(out, "        {line}");
        }
    }

    // Each fixture's call returns a different IR type. Override the resolver's
    // Swift first-class-map `root_type` with the call's `result_type` (looked up
    // across c/csharp/java/kotlin/go/php overrides — these are language-agnostic
    // IR type names that any backend can use to anchor field-access dispatch).
    let fixture_root_type: Option<String> = swift_call_result_type(call_config);
    let fixture_resolver = field_resolver.with_swift_root_type(fixture_root_type);

    for assertion in &fixture.assertions {
        let mut assertion_out = String::new();
        render_assertion(
            &mut assertion_out,
            assertion,
            result_var,
            &fixture_resolver,
            result_is_simple,
            result_is_array,
            result_is_option,
            result_element_is_string,
            result_field_accessor,
            &effective_enum_fields,
            is_streaming,
        );
        // Module-qualify swift-bridge-ambiguous DTO type names that appear in
        // streaming-virtual assertion expressions (e.g. `[StreamToolCall]`,
        // `[ToolCall]`). Both `<Module>` (first-class Codable struct) and
        // `RustBridge` (swift-bridge opaque class) export the same identifier,
        // so unqualified usage fails Swift compilation with "X is ambiguous for
        // type lookup". Mirrors the `[ChatCompletionChunk]` replacement in
        // `render_test_method`.
        for unqualified in ["StreamToolCall", "ToolCall"] {
            assertion_out =
                assertion_out.replace(&format!("[{unqualified}]"), &format!("[{module_name}.{unqualified}]"));
        }
        out.push_str(&assertion_out);
    }

    // Emit teardown for test backends: unregister to prevent leaking into subsequent tests.
    for arg in args {
        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let unregister_fn = format!("unregister{}", trait_bridge.trait_name.to_upper_camel_case());
                    let adapter_name = format!("swift-bridge-{}", trait_bridge.trait_name.to_snake_case());
                    let _ = writeln!(out, "        try? {module_name}.{unregister_fn}(\"{adapter_name}\")");
                }
            }
        }
    }

    let _ = writeln!(out, "    }}");
}

#[allow(clippy::too_many_arguments)]
/// Build setup lines and the argument list for the function call.
///
/// Swift-bridge wrappers require strongly-typed values that don't have implicit
/// Swift literal conversions:
///
/// - `bytes` args become `RustVec<UInt8>` — fixture supplies a relative file path
///   string which is read at test time and pushed into a `RustVec<UInt8>` setup
///   variable. A literal byte array is base64-decoded or UTF-8 encoded inline.
/// - `json_object` args become opaque config/request instances — a JSON string is
///   decoded via the matching `{Type}FromJson(...)` helper in a setup line.
/// - Optional args missing from the fixture must still appear at the call site
///   as `nil` whenever a later positional arg is present, otherwise Swift slots
///   subsequent values into the wrong parameter.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture_id: &str,
    has_host_root_route: bool,
    function_name: &str,
    options_via: Option<&str>,
    options_type: Option<&str>,
    handle_config_fn: Option<&str>,
    visitor_handle_expr: Option<&str>,
    is_method_call: bool,
    module_name: &str,
    unnamed_arg_indices: &[usize],
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
    arg_name_map: Option<&std::collections::HashMap<String, String>>,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<(usize, String)> = Vec::new();

    // Pre-compute, for each arg index, whether any later arg has a fixture-provided
    // value (or is required and will emit a default). When an optional arg is empty
    // but a later arg WILL emit, we must keep the slot with `nil` so positional
    // alignment is preserved.
    let later_emits: Vec<bool> = (0..args.len())
        .map(|i| {
            args.iter().skip(i + 1).any(|a| {
                let f = a.field.strip_prefix("input.").unwrap_or(&a.field);
                let v = input.get(f);
                let has_value = matches!(v, Some(x) if !x.is_null());
                has_value || !a.optional || (a.arg_type == "json_object" && a.name == "config")
            })
        })
        .collect();

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_ascii_uppercase().replace('-', "_"));
            let url_expr = if has_host_root_route {
                format!(
                    "ProcessInfo.processInfo.environment[\"{env_key}\"] ?? (AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\")"
                )
            } else {
                format!("AlefE2EMockServer.baseURL + \"/fixtures/{fixture_id}\"")
            };
            setup_lines.push(format!("let {} = {url_expr}", arg.name));

            // For Swift streaming functions (crawlStream, batchCrawlStream), wrap the URL
            // in a CrawlStreamRequest or BatchCrawlStreamRequest object instead of passing
            // it directly. These functions take a *Request type as the second parameter.
            let is_streaming_fn = function_name.contains("crawlStream") || function_name.contains("CrawlStream");
            if is_streaming_fn && idx > 0 {
                // Determine the request type name from the function name.
                let request_type = if function_name.contains("batch") || function_name.contains("Batch") {
                    "BatchCrawlStreamRequest"
                } else {
                    "CrawlStreamRequest"
                };
                let request_var = format!("{}Request", arg.name.to_lower_camel_case());
                setup_lines.push(format!("let {request_var} = {request_type}(url: {})", arg.name));
                parts.push((idx, request_var));
            } else {
                parts.push((idx, arg.name.clone()));
            }
            continue;
        }

        if arg.arg_type == "handle" {
            let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_val = input.get(field);
            let has_config = config_val
                .is_some_and(|v| !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty())));
            // Swift binding's engine factory declares `createEngine(config: ConfigType?)`,
            // so calls require the `config:` argument label even when passing `nil`.
            if has_config {
                if let Some(from_json_fn) = handle_config_fn {
                    let json_str = serde_json::to_string(config_val.unwrap()).unwrap_or_default();
                    let escaped = escape_swift_str(&json_str);
                    let config_var = format!("{}Config", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {config_var} = try {from_json_fn}(\"{escaped}\")"));
                    setup_lines.push(format!("let {var_name} = try createEngine(config: {config_var})"));
                } else {
                    setup_lines.push(format!("let {var_name} = try createEngine(config: nil)"));
                }
            } else {
                setup_lines.push(format!("let {var_name} = try createEngine(config: nil)"));
            }
            parts.push((idx, var_name));
            continue;
        }

        // bytes args: behavior depends on whether this is an e2e async wrapper (e.g. extractBytes
        // with unnamed_arg_indices) or a regular binding function. Swift's extractBytes/extractBytesSync
        // e2e wrappers take [UInt8] bytes (not path strings). When the fixture provides a path string,
        // read the file to bytes. Regular bindings also emit [UInt8] arrays from path strings.
        if arg.arg_type == "bytes" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);

            match val {
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    // Empty byte array
                    parts.push((idx, "[UInt8]()".to_string()));
                }
                Some(serde_json::Value::String(s)) => {
                    let escaped = escape_swift(s);
                    // Both unnamed and named bytes args: read file to bytes
                    let var_name = format!("{}Bytes", arg.name.to_lower_camel_case());
                    let data_var = format!("{}Data", arg.name.to_lower_camel_case());
                    setup_lines.push(format!(
                        "let {data_var} = try Data(contentsOf: URL(fileURLWithPath: \"{escaped}\"))"
                    ));
                    setup_lines.push(format!("let {var_name} = Array({data_var})"));
                    parts.push((idx, var_name));
                }
                Some(serde_json::Value::Array(arr)) => {
                    // Inline byte array literal
                    let bytes: Vec<String> = arr.iter().filter_map(|v| v.as_u64().map(|n| n.to_string())).collect();
                    parts.push((idx, format!("[UInt8]({})", bytes.join(", "))));
                }
                Some(other) => {
                    // Fallback: encode the JSON serialisation as UTF-8 bytes.
                    let json_str = serde_json::to_string(other).unwrap_or_default();
                    let escaped = escape_swift(&json_str);
                    let var_name = format!("{}Bytes", arg.name.to_lower_camel_case());
                    setup_lines.push(format!("let {var_name} = Array(\"{escaped}\".utf8)"));
                    parts.push((idx, var_name));
                }
            }
            continue;
        }

        // file_path args: pass path strings directly (for extract_file, extract_file_sync, etc.)
        if arg.arg_type == "file_path" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);

            match val {
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    parts.push((idx, "\"\"".to_string()));
                }
                Some(serde_json::Value::String(s)) => {
                    let escaped = escape_swift(s);
                    parts.push((idx, format!("\"{}\"", escaped)));
                }
                Some(other) => {
                    // Fallback: convert to JSON string
                    let json_str = serde_json::to_string(other).unwrap_or_default();
                    let escaped = escape_swift(&json_str);
                    parts.push((idx, format!("\"{}\"", escaped)));
                }
            }
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let emission = crate::e2e::codegen::emit_test_backend("swift", trait_bridge, &methods, fixture);
                    setup_lines.push(emission.setup_block);
                    parts.push((idx, emission.arg_expr));
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("swift");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push((idx, "nil".to_string()));
            continue;
        }

        // json_object "config" args: behavior depends on whether this is an e2e wrapper or regular binding.
        // E2e wrappers (all args in unnamed_arg_indices) take JSON strings and deserialize internally.
        // Regular bindings (config arg not unnamed) expect deserialized objects (via options_via or default helper).
        let is_config_arg = arg.name == "config" && arg.arg_type == "json_object";
        if is_config_arg {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            let json_str = match val {
                None | Some(serde_json::Value::Null) => "{}".to_string(),
                Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            let escaped = escape_swift(&json_str);

            // Detect if config arg is unnamed (index `idx` in unnamed_arg_indices).
            // E2e wrappers keep config unnamed and receive JSON strings.
            let config_is_unnamed = unnamed_arg_indices.contains(&idx);

            if config_is_unnamed {
                // E2e wrapper: pass JSON string directly (positional, no label).
                parts.push((idx, format!("\"{}\"", escaped)));
            } else {
                // Regular binding: deserialize to an opaque object.
                let var_name = format!("{}Obj", arg.name.to_lower_camel_case());
                let from_json_fn = from_json_helper_for_arg(arg, options_type);
                // Qualify with module name to avoid ambiguity when both SampleCrate and RustBridge are imported.
                setup_lines.push(format!(
                    "let {var_name} = try {module_name}.{from_json_fn}(\"{escaped}\")"
                ));
                parts.push((idx, var_name));
            }
            continue;
        }

        // json_object non-config args with array values: construct Swift data-enum objects
        // from the JSON array using the {TypeName}FromJson helper. This handles cases like
        // interact(actions: [PageAction]) where we deserialize JSON into enum instances.
        if arg.arg_type == "json_object"
            && arg.element_type.is_some()
            && !is_scalar_element_type(arg.element_type.as_deref())
        {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            let elem_type = arg.element_type.as_deref().unwrap_or("Unknown");
            // Convert element type to camelCase for the from-json helper name
            let from_json_fn = format!("{}FromJson", elem_type.to_lower_camel_case());

            match val {
                Some(serde_json::Value::Array(arr)) => {
                    let var_name = format!("{}Array", arg.name.to_lower_camel_case());

                    if arr.is_empty() {
                        // Empty array literal
                        parts.push((idx, "[]".to_string()));
                    } else {
                        // For each JSON item in the array, call the helper to deserialize it
                        let json_strs: Vec<String> =
                            arr.iter().filter_map(|item| serde_json::to_string(item).ok()).collect();

                        let mut item_vars = Vec::new();
                        for (i, json_str) in json_strs.iter().enumerate() {
                            let escaped = escape_swift(json_str);
                            let item_var = format!("_item_{var_name}_{i}");
                            // Call the wrapper-module's `{type}FromJson` helper rather than the
                            // raw `RustBridge` one so the resulting element is the
                            // wrapper-module's `PageAction` (etc.), matching the type the
                            // function signature expects. The wrapper internally delegates to
                            // `RustBridge.{type}FromJson` which understands the
                            // serde(tag = "type") format.
                            setup_lines.push(format!(
                                "let {item_var} = try {module_name}.{from_json_fn}(\"{escaped}\")"
                            ));
                            item_vars.push(item_var);
                        }

                        // Construct the final array from all item variables
                        setup_lines.push(format!("let {var_name} = [{}]", item_vars.join(", ")));
                        parts.push((idx, var_name));
                    }
                }
                None | Some(serde_json::Value::Null) if arg.optional => {
                    if later_emits[idx] {
                        parts.push((idx, "nil".to_string()));
                    }
                }
                None | Some(serde_json::Value::Null) => {
                    // Required but missing — emit empty array
                    parts.push((idx, "[]".to_string()));
                }
                Some(_other) => {
                    // Non-array value — emit empty array (shouldn't happen)
                    parts.push((idx, "[]".to_string()));
                }
            }
            continue;
        }

        // json_object non-config args with options_via = "from_json":
        // Use the generated `{typeCamelCase}FromJson(_:)` helper so the fixture JSON is
        // deserialised into the opaque swift-bridge type rather than passed as a raw string.
        // When arg.field == "input", the entire fixture input IS the request object.
        // When a visitor handle is present, use `{typeCamelCase}FromJsonWithVisitor(json, handle)`
        // instead to attach the visitor to the options in one step.
        if arg.arg_type == "json_object" && options_via == Some("from_json") {
            if let Some(type_name) = options_type {
                let resolved_val = super::resolve_field(input, &arg.field);
                let json_str = match resolved_val {
                    serde_json::Value::Null => "{}".to_string(),
                    v => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
                };
                let escaped = escape_swift(&json_str);
                let var_name = format!("_{}", arg.name.to_lower_camel_case());
                if let Some(handle_expr) = visitor_handle_expr {
                    // Use the visitor-aware helper: `{typeCamelCase}FromJsonWithVisitor(json, handle)`.
                    // The handle expression builds a VisitorHandle from the local class instance.
                    // The function name mirrors emit_options_field_options_helper: camelCase of
                    // `{options_snake}_from_json_with_visitor`.
                    let with_visitor_fn = format!("{}FromJsonWithVisitor", type_name.to_lower_camel_case());
                    let handle_var = format!("_visitorHandle_{}", var_name.trim_start_matches('_'));
                    setup_lines.push(format!("let {handle_var} = {handle_expr}"));
                    setup_lines.push(format!(
                        "let {var_name} = try {module_name}.{with_visitor_fn}(\"{escaped}\", {handle_var})"
                    ));
                } else {
                    let from_json_fn = format!("{}FromJson", type_name.to_lower_camel_case());
                    setup_lines.push(format!(
                        "let {var_name} = try {module_name}.{from_json_fn}(\"{escaped}\")"
                    ));
                }
                parts.push((idx, var_name));
                continue;
            }
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: keep the slot with `nil`
                // when a later arg will emit, so positional alignment matches
                // the swift-bridge wrapper signature.
                if later_emits[idx] {
                    parts.push((idx, "nil".to_string()));
                }
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "nil".to_string(),
                };
                parts.push((idx, default_val));
            }
            Some(v) => {
                parts.push((idx, json_to_swift(v)));
            }
        }
    }

    // Method calls on the DefaultClient handle (e.g. `_client.chat(req)`) use
    // anonymous Swift argument labels (`func chat(_ req:)`), so omit `name:` prefixes.
    // Free-function calls (e.g. `process(source:, config:)`) keep labelled args.
    // Registration functions (e.g. `registerOcrBackend(_:)`) also use positional args.
    // Swift argument labels must be camelCase, so convert from snake_case.
    // Some APIs like detectMimeTypeFromBytes take unnamed first parameters —
    // omit labels for indices listed in unnamed_arg_indices.
    let is_register_call = function_name.starts_with("register") || function_name.starts_with("Register");
    let args_str = parts
        .into_iter()
        .map(|(idx, val)| {
            if is_method_call || is_register_call || unnamed_arg_indices.contains(&idx) {
                val
            } else {
                // Apply per-language argument renames before emitting the call.
                let arg_name: &str = arg_name_map
                    .and_then(|m| m.get(&args[idx].name).map(String::as_str))
                    .unwrap_or(&args[idx].name);
                let label = arg_name.to_lower_camel_case();
                format!("{label}: {val}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    (setup_lines, args_str)
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_array: bool,
    result_is_option: bool,
    result_element_is_string: bool,
    result_field_accessor: &HashMap<String, String>,
    enum_fields: &HashSet<String>,
    is_streaming: bool,
) {
    // When the bare result is `Optional<T>` (no field path) the opaque class
    // exposed by swift-bridge has no `.toString()` method, so the usual
    // `.toString().isEmpty` pattern produces compile errors. Detect the
    // "bare result" case and prefer `XCTAssertNil` / `XCTAssertNotNil`.
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    // Streaming virtual fields resolve against the `chunks` collected-array variable.
    // Intercept before is_valid_for_result so they are never skipped.
    // Also intercept `usage.*` deep-paths in streaming tests: `AsyncThrowingStream` does
    // not have a `usage()` method, so we must route them through the chunks accessor.
    if let Some(f) = &assertion.field {
        let is_streaming_usage_path =
            is_streaming && (f == "usage" || (f.starts_with("usage.") || f.starts_with("usage[")));
        // Only route through the streaming-virtual `chunks` accessor when this is
        // actually a streaming fixture. Non-streaming fixtures (e.g. `process()`
        // with `chunkMaxSize`) expose `chunks` as a real `ProcessResult` field, so
        // emit `result.chunks()` via the regular field-accessor path below.
        if is_streaming
            && !f.is_empty()
            && (crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) || is_streaming_usage_path)
        {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "swift", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThanOrEqual(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertEqual(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!("        XCTAssertEqual({expr}, \"{escaped}\")\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        XCTAssertEqual({expr}, {b})\n")
                        } else {
                            String::new()
                        }
                    }
                    "not_empty" => {
                        format!("        XCTAssertFalse({expr}.isEmpty, \"expected non-empty\")\n")
                    }
                    "is_empty" => {
                        format!("        XCTAssertTrue({expr}.isEmpty, \"expected empty\")\n")
                    }
                    "is_true" => {
                        format!("        XCTAssertTrue({expr})\n")
                    }
                    "is_false" => {
                        format!("        XCTAssertFalse({expr})\n")
                    }
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThan(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!(
                                "        XCTAssertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\")\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    _ => format!(
                        "        // streaming field '{f}': assertion type '{}' not rendered\n",
                        assertion.assertion_type
                    ),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
            return;
        }
    }

    // Skip assertions that traverse a tagged-union variant boundary.
    // In Swift, FormatMetadata and similar enum-backed opaque types are exposed as
    // plain classes by swift-bridge — variant accessor methods (e.g., `.excel()`)
    // are not generated, so such assertions cannot be expressed.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && field_resolver.tagged_union_split(f).is_some() {
            let _ = writeln!(
                out,
                "        // skipped: field '{f}' crosses a tagged-union variant boundary (not expressible in Swift)"
            );
            return;
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_is_optional = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty() && (field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f)))
    });
    let field_is_array = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty()
            && (field_resolver.is_array(f)
                || field_resolver.is_array(field_resolver.resolve(f))
                || field_resolver.is_collection_root(f)
                || field_resolver.is_collection_root(field_resolver.resolve(f)))
    });

    let field_expr_raw = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "swift", result_var),
            _ => result_var.to_string(),
        }
    };

    // swift-bridge `RustVec<T>` exposes its elements as `T.SelfRef`, which holds
    // a raw pointer into the parent Vec's storage. When the Vec is a temporary
    // (e.g. `result.json_ld()` called inline), Swift ARC may release it before
    // the ref is used, leaving the ref's pointer dangling. Materialise the
    // temporary into a local so it survives the full expression chain.
    //
    // The local name is suffixed with the assertion type plus a hash of the
    // assertion's discriminating fields so multiple assertions on the same
    // collection don't redeclare the same name.
    let local_suffix = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        assertion.field.hash(&mut hasher);
        assertion
            .value
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut hasher);
        format!(
            "{}_{:x}",
            assertion.assertion_type.replace(['-', '.'], "_"),
            hasher.finish() & 0xffff_ffff,
        )
    };
    let (vec_setup, field_expr, is_map_subscript) = materialise_vec_temporaries(&field_expr_raw, &local_suffix);
    // The `contains` / `not_contains` traversal branch builds its own
    // accessor from `field_resolver.accessor(array_part, ...)`, ignoring
    // `field_expr`. Emitting the vec_setup there would produce dead
    // `let _vec_… = …` lines, so skip it for those traversal cases.
    let field_uses_traversal = assertion.field.as_deref().is_some_and(|f| f.contains("[]."));
    let traversal_skips_field_expr = field_uses_traversal
        && matches!(
            assertion.assertion_type.as_str(),
            "contains" | "not_contains" | "not_empty" | "is_empty"
        );
    if !traversal_skips_field_expr {
        for line in &vec_setup {
            let _ = writeln!(out, "        {line}");
        }
    }

    // In Swift, optional chaining with `?.` makes the result optional even if the
    // called method's return type isn't marked optional. For example:
    // `result.markdown()?.content()` returns `Optional<RustString>` because
    // `markdown()` is optional and the `?.` operator wraps the result.
    // Detect this by checking if the accessor contains `?.`.
    let accessor_is_optional = field_expr.contains("?.");
    // First-class Codable Swift struct property access leaves no trailing `()`
    // on the leaf segment — e.g. `result.text` (Swift `String`) vs
    // `result.text()` (RustBridge.RustString). When the leaf is property
    // access, we already have a Swift `String` (or `String?`) and must NOT
    // re-wrap with `.toString()`. Detect this by looking at the final segment
    // after the last `.` — property access ends in a bare identifier (no
    // trailing `()` or `()?`).
    let leaf_is_property_access = {
        let trimmed = field_expr.trim_end_matches('?');
        // Skip subscripts: `name?[0]` should still see `name` as the field.
        let last_segment = trimmed.rsplit_once('.').map(|(_, s)| s).unwrap_or(trimmed);
        let last_segment = last_segment.split('[').next().unwrap_or(last_segment);
        !last_segment.ends_with(')') && !last_segment.is_empty()
    };

    // Bare-result Option<T> case: the call returns `Optional<String>` (or
    // similar) so the field_expr is `result` typed as `String?`. String
    // assertions like `XCTAssertEqual(result.trimmingCharacters(...), …)` will
    // not compile against an optional — coalesce to `""` so the macro sees a
    // concrete Swift `String`.
    let bare_result_is_simple_option =
        result_is_simple && result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();

    // For enum fields, need to handle the string representation differently in Swift.
    // Swift enums don't have `.rawValue` unless they're explicitly RawRepresentable.
    // Check if this is an enum type and handle accordingly.
    // For optional fields (Optional<RustString>), use optional chaining before toString().
    // For other fields: swift-bridge returns all Rust `String` fields as `RustString`.
    // We add .toString() here so string assertions (contains, hasPrefix, etc.) work.
    // Non-string opaque fields (DocumentStructure, etc.) should not appear in string
    // assertions — the fixture schema controls which assertions apply to which fields.
    let string_expr = if is_map_subscript {
        // The field_expr already evaluates to `String?` (from a JSON-decoded
        // `[String: String]` subscript). No `.toString()` chain needed —
        // coalesce the optional to "" and use the Swift String directly.
        format!("({field_expr} ?? \"\")")
    } else if leaf_is_property_access {
        // First-class Codable struct field access: leaf is already a Swift
        // `String` (or `String?`/enum type) — never a `RustString` requiring
        // `.toString()`. For optional leaves, coalesce to "" so XCTAssert
        // receives a non-optional Swift `String`.
        if field_is_enum && (field_is_optional || accessor_is_optional) {
            // Optional first-class Codable enum (e.g. `FinishReason?` where
            // `FinishReason: String, Codable`). `.rawValue` gives the serde
            // wire value (e.g. "tool_calls") so assertions match fixture JSON.
            format!("(({field_expr})?.rawValue ?? \"\")")
        } else if field_is_enum {
            format!("{field_expr}.rawValue")
        } else if field_is_optional || accessor_is_optional || bare_result_is_simple_option {
            format!("({field_expr} ?? \"\")")
        } else {
            field_expr.to_string()
        }
    } else if field_is_enum && (field_is_optional || accessor_is_optional) {
        // Enum-typed fields that are also optional (e.g. `finish_reason() -> Optional<RustString>`)
        // must use optional chaining: `?.toString() ?? ""` to unwrap before converting to Swift String.
        format!("({field_expr}?.toString() ?? \"\")")
    } else if field_is_enum {
        // Enum-typed fields are now bridged as `String` (RustString in Swift) rather than
        // as opaque enum handles. The getter on the Rust side calls `to_string()` internally
        // and returns a `String` across the FFI. In Swift this arrives as `RustString`, so
        // `.toString()` converts it to a Swift `String` — one call, not two.
        format!("{field_expr}.toString()")
    } else if field_is_optional {
        // Leaf field itself is Optional<RustString> — need ?.toString() to unwrap.
        format!("({field_expr}?.toString() ?? \"\")")
    } else if accessor_is_optional {
        // Ancestor optional chain propagates; leaf is non-optional RustString within chain.
        // Use .toString() directly — the whole expr is Optional<String> due to propagation.
        format!("({field_expr}.toString() ?? \"\")")
    } else {
        format!("{field_expr}.toString()")
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                if expected.is_string() {
                    if field_is_enum {
                        // Enum fields: `to_string()` (snake_case) returns RustString;
                        // `.toString()` converts it to a Swift String.
                        // `string_expr` already incorporates this call chain.
                        let trim_expr =
                            format!("{string_expr}.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)");
                        let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                    } else {
                        // For optional strings (String?), use ?? to coalesce before trimming.
                        // `.toString()` converts RustString → Swift String before calling
                        // `.trimmingCharacters`, which requires a concrete String type.
                        // string_expr already incorporates field_is_optional via ?.toString() ?? "".
                        let trim_expr =
                            format!("{string_expr}.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)");
                        let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                    }
                } else {
                    // For numeric fields, cast the expected value to match the field's type (e.g., UInt).
                    let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                    let _ = writeln!(out, "        XCTAssertEqual({field_expr}, {cast_swift_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                // When the root result IS the array (result_is_simple + result_is_array) and
                // there is no field path, check array membership via map+contains.
                let no_field = assertion.field.as_deref().is_none_or(|f| f.is_empty());
                if result_is_simple && result_is_array && no_field {
                    if result_element_is_string {
                        // The Swift binding exposes the result as a native
                        // `[String]` (e.g. `manifestLanguages() -> [String]`),
                        // not the opaque `RustVec<RustString>`. Iterating
                        // elements yields plain Swift `String`, which has no
                        // `asStr()` — emit a direct `.contains(...)` instead.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({result_var}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    } else {
                        // RustVec<RustString> iteration yields RustStringRef (no `toString()`);
                        // use `.asStr().toString()` to convert each element to a Swift String.
                        // swift-bridge renames `as_str` → `asStr` automatically.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({result_var}.map {{ $0.asStr().toString() }}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                } else {
                    // []. traversal: field like "links[].url" → contains(where:) closure.
                    let traversal_handled = if let Some(f) = assertion.field.as_deref() {
                        if let Some(dot) = f.find("[].") {
                            let array_part = &f[..dot];
                            let elem_part = &f[dot + 3..];
                            let line = swift_traversal_contains_assert(
                                array_part,
                                elem_part,
                                f,
                                &swift_val,
                                result_var,
                                false,
                                &format!("expected to contain: \\({swift_val})"),
                                enum_fields,
                                field_resolver,
                            );
                            let _ = writeln!(out, "{line}");
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !traversal_handled {
                        // For array fields (RustVec<RustString>), check membership via map+contains.
                        let field_is_array = assertion
                            .field
                            .as_deref()
                            .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));
                        if field_is_array {
                            // First try the "stringy aggregator" path: when the array element
                            // is an opaque DTO with several text-bearing accessors (e.g.
                            // ImportInfo with source/items/alias, or StructureItem with
                            // kind/name/signature/...), emit a `contains(where: { ... })`
                            // closure that walks every accessor and does substring matching,
                            // mirroring python's `_alef_e2e_item_texts`. This avoids the
                            // brittle "primary accessor" guess (e.g. ImportInfo → source
                            // misses imports whose name lives in `items`).
                            let aggregator = swift_stringy_aggregator_contains_assert(
                                assertion.field.as_deref(),
                                result_var,
                                field_resolver,
                                &swift_val,
                            );
                            if let Some(line) = aggregator {
                                let _ = writeln!(out, "{line}");
                            } else {
                                let (contains_expr, is_optional) = swift_array_contains_expr(
                                    assertion.field.as_deref(),
                                    result_var,
                                    field_resolver,
                                    result_field_accessor,
                                );
                                let wrapped = if is_optional {
                                    format!("({contains_expr} ?? [])")
                                } else {
                                    contains_expr
                                };
                                let _ = writeln!(
                                    out,
                                    "        XCTAssertTrue({wrapped}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                                );
                            }
                        } else if field_is_enum {
                            // Enum fields: use `toString().toString()` (via string_expr) to get the
                            // serde variant name as a Swift String, then check substring containment.
                            // Swift's `String.contains("")` returns false; guard with `.isEmpty` so
                            // fixtures that assert containment of an empty string still pass.
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({swift_val}.isEmpty || {string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        } else {
                            // Same `isEmpty` guard as the enum branch — every string trivially
                            // "contains" the empty string, but Swift's `String.contains` does not.
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({swift_val}.isEmpty || {string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    }
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                // []. traversal: field like "links[].link_type" → contains(where:) per value.
                if let Some(f) = assertion.field.as_deref() {
                    if let Some(dot) = f.find("[].") {
                        let array_part = &f[..dot];
                        let elem_part = &f[dot + 3..];
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let line = swift_traversal_contains_assert(
                                array_part,
                                elem_part,
                                f,
                                &swift_val,
                                result_var,
                                false,
                                &format!("expected to contain: \\({swift_val})"),
                                enum_fields,
                                field_resolver,
                            );
                            let _ = writeln!(out, "{line}");
                        }
                        // handled — skip remaining branches
                    } else {
                        // For array fields (RustVec<RustString>), check membership via map+contains.
                        let field_is_array = field_resolver.is_array(field_resolver.resolve(f));
                        if field_is_array {
                            let (contains_expr, is_optional) = swift_array_contains_expr(
                                assertion.field.as_deref(),
                                result_var,
                                field_resolver,
                                result_field_accessor,
                            );
                            let wrapped = if is_optional {
                                format!("({contains_expr} ?? [])")
                            } else {
                                contains_expr
                            };
                            for val in values {
                                let swift_val = json_to_swift(val);
                                let _ = writeln!(
                                    out,
                                    "        XCTAssertTrue({wrapped}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                                );
                            }
                        } else if field_is_enum {
                            // Enum fields: use `toString().toString()` (via string_expr) to get the
                            // serde variant name as a Swift String, then check substring containment.
                            for val in values {
                                let swift_val = json_to_swift(val);
                                let _ = writeln!(
                                    out,
                                    "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                                );
                            }
                        } else {
                            for val in values {
                                let swift_val = json_to_swift(val);
                                let _ = writeln!(
                                    out,
                                    "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                                );
                            }
                        }
                    }
                } else {
                    // No field — fall back to existing string_expr path.
                    for val in values {
                        let swift_val = json_to_swift(val);
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                // []. traversal: "links[].url" → XCTAssertFalse(array.contains(where:))
                let traversal_handled = if let Some(f) = assertion.field.as_deref() {
                    if let Some(dot) = f.find("[].") {
                        let array_part = &f[..dot];
                        let elem_part = &f[dot + 3..];
                        let line = swift_traversal_contains_assert(
                            array_part,
                            elem_part,
                            f,
                            &swift_val,
                            result_var,
                            true,
                            &format!("expected NOT to contain: \\({swift_val})"),
                            enum_fields,
                            field_resolver,
                        );
                        let _ = writeln!(out, "{line}");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !traversal_handled {
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({string_expr}.contains({swift_val}), \"expected NOT to contain: \\({swift_val})\")"
                    );
                }
            }
        }
        "not_empty" => {
            // For optional fields (Optional<T>), check that the value is non-nil.
            // For array fields (RustVec<T>), check .isEmpty on the vec directly.
            // For result_is_simple (e.g. Data, String), use .isEmpty directly on
            // the result — avoids calling .toString() on non-RustString types.
            // For string fields, convert to Swift String and check .isEmpty.
            // []. traversal: "links[].url" → contains(where: { !elem.isEmpty })
            let traversal_not_empty_handled = if let Some(f) = assertion.field.as_deref() {
                if let Some(dot) = f.find("[].") {
                    let array_part = &f[..dot];
                    let elem_part = &f[dot + 3..];
                    let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
                    let resolved_full = field_resolver.resolve(f);
                    let resolved_elem_part = resolved_full
                        .find("[].")
                        .map(|d| &resolved_full[d + 3..])
                        .unwrap_or(elem_part);
                    let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
                    let elem_is_enum = enum_fields.contains(f) || enum_fields.contains(resolved_full);
                    let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
                        || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
                    let elem_str = if elem_is_enum {
                        format!("{elem_accessor}.to_string().toString()")
                    } else if elem_is_optional {
                        format!("({elem_accessor}?.toString() ?? \"\")")
                    } else {
                        format!("{elem_accessor}.toString()")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertTrue({array_accessor}.contains(where: {{ !{elem_str}.isEmpty }}), \"expected non-empty value\")"
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !traversal_not_empty_handled {
                if bare_result_is_option {
                    let _ = writeln!(out, "        XCTAssertNotNil({result_var}, \"expected non-nil value\")");
                } else if field_is_optional {
                    let _ = writeln!(out, "        XCTAssertNotNil({field_expr}, \"expected non-nil value\")");
                } else if field_is_array {
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({field_expr}.isEmpty, \"expected non-empty value\")"
                    );
                } else if result_is_simple {
                    // result_is_simple: result is a primitive (Data, String, etc.) — use .isEmpty directly.
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({result_var}.isEmpty, \"expected non-empty value\")"
                    );
                } else {
                    // First-class Swift struct fields are properties typed as native Swift
                    // `String` / `[T]` / `Data` etc — all of which expose `.count` (and
                    // `String`/`Array` also expose `.isEmpty`). Use `.count > 0` so the same
                    // path works whether the field is a String or an Array.
                    //
                    // When the accessor contains a `?.` optional chain, `.count` returns an
                    // Optional which Swift cannot compare directly to `0`; coalesce via `?? 0`
                    // so the assertion typechecks.
                    //
                    // For opaque method-call accessors (`result.id()`), the returned type is
                    // `RustString`, which lacks `.count`. Convert to Swift `String` first via
                    // `.toString()`. Array fields short-circuit above via `field_is_array`, so
                    // method-call accessors landing here are guaranteed to be the scalar /
                    // string flavour; vec accessors return `RustVec` (whose `.count` is fine).
                    let count_target = swift_count_target(&field_expr, field_resolver, assertion.field.as_deref());
                    let len_expr = if accessor_is_optional {
                        format!("({count_target}.count ?? 0)")
                    } else {
                        format!("{count_target}.count")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertGreaterThan({len_expr}, 0, \"expected non-empty value\")"
                    );
                }
            }
        }
        "is_empty" => {
            if bare_result_is_option {
                let _ = writeln!(out, "        XCTAssertNil({result_var}, \"expected nil value\")");
            } else if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNil({field_expr}, \"expected nil value\")");
            } else if field_is_array {
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({field_expr}.isEmpty, \"expected empty value\")"
                );
            } else {
                // Symmetric with not_empty: use .count == 0 on first-class Swift types.
                // Wrap opaque method-call accessors (`result.id()`) with `.toString()` so
                // `.count` lands on Swift `String`, not `RustString` (which lacks `.count`).
                let count_target = swift_count_target(&field_expr, field_resolver, assertion.field.as_deref());
                let len_expr = if accessor_is_optional {
                    format!("({count_target}.count ?? 0)")
                } else {
                    format!("{count_target}.count")
                };
                let _ = writeln!(out, "        XCTAssertEqual({len_expr}, 0, \"expected empty value\")");
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let swift_val = json_to_swift(v);
                        format!("{string_expr}.contains({swift_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({joined}, \"expected to contain at least one of the specified values\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(out, "        XCTAssertGreaterThan({compare_expr}, {cast_swift_val})");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(out, "        XCTAssertLessThan({compare_expr}, {cast_swift_val})");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(
                    out,
                    "        XCTAssertGreaterThanOrEqual({compare_expr}, {cast_swift_val})"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(
                    out,
                    "        XCTAssertLessThanOrEqual({compare_expr}, {cast_swift_val})"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasPrefix({swift_val}), \"expected to start with: \\({swift_val})\")"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasSuffix({swift_val}), \"expected to end with: \\({swift_val})\")"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // Use string_expr.count: for RustString fields string_expr already has
                    // .toString() appended, giving a Swift String whose .count is character count.
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({string_expr}.count, {n})");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertLessThanOrEqual({string_expr}.count, {n})");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // For fields nested inside an optional parent (e.g. document.nodes where
                    // document is Optional), the accessor generates `result.document().nodes()`
                    // which doesn't compile in Swift without optional chaining.
                    let count_expr = swift_array_count_expr(assertion.field.as_deref(), result_var, field_resolver);
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({count_expr}, {n})");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let count_expr = swift_array_count_expr(assertion.field.as_deref(), result_var, field_resolver);
                    let _ = writeln!(out, "        XCTAssertEqual({count_expr}, {n})");
                }
            }
        }
        "is_true" => {
            let assert_expr = if accessor_is_optional {
                format!("({field_expr} ?? false)")
            } else {
                field_expr.clone()
            };
            let _ = writeln!(out, "        XCTAssertTrue({assert_expr})");
        }
        "is_false" => {
            let assert_expr = if accessor_is_optional {
                format!("({field_expr} ?? true)")
            } else {
                field_expr.clone()
            };
            let _ = writeln!(out, "        XCTAssertFalse({assert_expr})");
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil({string_expr}.range(of: {swift_val}, options: .regularExpression), \"expected value to match regex: \\({swift_val})\")"
                );
            }
        }
        "not_error" => {
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        "method_result" => {
            let _ = writeln!(out, "        // method_result assertions not yet implemented for Swift");
        }
        other => {
            panic!("Swift e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build a Swift accessor path for the given fixture field, inserting `()` on
/// every segment and `?` after every optional non-leaf segment.
///
/// This is the core helper for count/contains helpers that need to reconstruct
/// the path with correct optional chaining from the raw fixture field name.
///
/// Rewrite a Swift accessor expression to capture any `RustVec` temporaries
/// in a local before subscripting them. Returns `(setup_lines, rewritten_expr)`.
///
/// swift-bridge's `Vec_<T>$get` returns a raw pointer into the Vec's storage
/// wrapped in a `T.SelfRef`. If the Vec was a temporary, ARC may release it
/// before the ref is dereferenced, leaving the pointer dangling and reads
/// returning empty/garbage. Hoisting the Vec into a `let` binding ties the
/// Vec's lifetime to the enclosing function scope, so the ref stays valid.
///
/// Only the first `()[...]` occurrence per expression is materialised — that
/// covers all current fixture access patterns (single-level subscripts on a
/// result field). Nested subscripts are rare and would need a more elaborate
/// pass; if they appear, this returns conservative output (just the first
/// hoist) which is still correct.
/// Returns `(setup_lines, rewritten_expr, is_map_subscript)`. `is_map_subscript` is
/// true when the subscript key was a string literal, indicating the parent
/// accessor returns a JSON-encoded Map (RustString) and the rewritten expression
/// already evaluates to `String?` so callers should NOT append `.toString()`.
fn materialise_vec_temporaries(expr: &str, name_suffix: &str) -> (Vec<String>, String, bool) {
    let Some(idx) = expr.find("()[") else {
        return (Vec::new(), expr.to_string(), false);
    };
    let after_open = idx + 3; // position after `()[`
    let Some(close_rel) = expr[after_open..].find(']') else {
        return (Vec::new(), expr.to_string(), false);
    };
    let subscript_end = after_open + close_rel; // index of `]`
    let prefix = &expr[..idx + 2]; // includes `()`
    let subscript = &expr[idx + 2..=subscript_end]; // `[N]`
    let tail = &expr[subscript_end + 1..]; // everything after `]`
    let method_dot = expr[..idx].rfind('.').unwrap_or(0);
    let method = &expr[method_dot + 1..idx];
    let local = format!("_vec_{}_{}", method, name_suffix);

    // String-key subscript (e.g. `["title"]`) signals a Map-like access. swift-bridge
    // serialises non-leaf Maps (e.g. `HashMap<String, String>`) as JSON-encoded
    // RustString rather than exposing a Swift dictionary. Decode the RustString to
    // `[String: String]` before subscripting so `_vec_X["title"]` works.
    let inner = subscript.trim_start_matches('[').trim_end_matches(']');
    let is_string_key = inner.starts_with('"') && inner.ends_with('"');
    let setup = if is_string_key {
        format!(
            "let {local} = (try? JSONSerialization.jsonObject(with: ({prefix}.toString() ?? \"{{}}\").data(using: .utf8)!) as? [String: String]) ?? [:]"
        )
    } else {
        format!("let {local} = {prefix}")
    };

    let rewritten = format!("{local}{subscript}{tail}");
    (vec![setup], rewritten, is_string_key)
}

/// Returns `(accessor_expr, has_optional)` where `has_optional` is true when
/// at least one `?.` was inserted.
fn swift_build_accessor(field: &str, result_var: &str, field_resolver: &FieldResolver) -> (String, bool) {
    let resolved = field_resolver.resolve(field);
    let parts: Vec<&str> = resolved.split('.').collect();

    // Track the current IR type as we walk segments so each segment can be
    // emitted with property syntax (first-class Codable struct) or method-call
    // syntax (typealias-to-`RustBridge.X`). Mirrors the per-segment dispatch in
    // `render_swift_with_first_class_map`.
    let mut current_type: Option<String> = field_resolver.swift_root_type().cloned();
    // Once a chain crosses a `[N]` subscript, we are operating on a RustVec
    // element, which is always the OPAQUE `RustBridge.T` (swift-bridge does not
    // convert RustVec elements into the first-class Codable struct). Pin
    // opaque method-call syntax after the first index step.
    let mut via_rust_vec = false;
    // Once a chain crosses an opaque (typealias-to-`RustBridge.X`) segment, every
    // subsequent accessor must also be opaque (method-call syntax). Calling a
    // method on `RustBridge.X` returns the OPAQUE wrapper of the next type, even
    // when that next type is independently eligible for first-class emission.
    // See `field_access::render_swift_with_first_class_map` for the matching
    // invariant. Without this, `metrics.total_lines` on an opaque parent emits
    // `.metrics().totalLines` instead of `.metrics().totalLines()`.
    let mut via_opaque = false;

    let mut out = result_var.to_string();
    let mut has_optional = false;
    let mut path_so_far = String::new();
    let total = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let is_leaf = i == total - 1;
        // Handle array index subscripts within a segment, e.g. `data[0]`.
        // `data[0]` must become `.data()[0]` (opaque) or `.data[0]` (first-class).
        // Split at the first `[` if present.
        let (field_name, subscript): (&str, Option<&str>) = if let Some(bracket_pos) = part.find('[') {
            (&part[..bracket_pos], Some(&part[bracket_pos..]))
        } else {
            (part, None)
        };

        if !path_so_far.is_empty() {
            path_so_far.push('.');
        }
        // Build the base path (without subscript) for the optional check. When the
        // segment is e.g. `tool_calls[0]`, we want to check `is_optional` against
        // "choices[0].message.tool_calls" not "choices[0].message.tool_calls[0]".
        let base_path = {
            let mut p = path_so_far.clone();
            p.push_str(field_name);
            p
        };
        // Now push the full part (with subscript if any) so path_so_far is correct
        // for subsequent segment checks.
        path_so_far.push_str(part);

        // First-class struct fields → property access (no `()`); typealias-to-
        // opaque fields → method-call access (`()`). Once we've indexed through
        // a RustVec, every subsequent segment is on an opaque element.
        // When current_type is None (opaque parent that doesn't appear in field_types),
        // treat it as opaque and use method-call syntax.
        let is_first_class = current_type
            .as_ref()
            .is_some_and(|t| field_resolver.swift_is_first_class(Some(t)));
        let property_syntax = !via_rust_vec && !via_opaque && is_first_class;
        if !property_syntax {
            via_opaque = true;
        }
        out.push('.');
        // Swift bindings (both first-class `public let` props and swift-bridge
        // method names) always use lowerCamelCase — never raw snake_case from IR.
        out.push_str(&field_name.to_lower_camel_case());
        if let Some(sub) = subscript {
            // When the getter for this subscripted field is itself optional
            // (e.g. tool_calls returns Optional<RustVec<T>>), insert `?` before
            // the subscript so Swift unwraps the Optional before indexing.
            let field_is_optional = field_resolver.is_optional(&base_path);
            let access = if property_syntax { "" } else { "()" };
            if field_is_optional {
                out.push_str(&format!("{access}?"));
                has_optional = true;
            } else {
                out.push_str(access);
            }
            out.push_str(sub);
            // Do NOT append a trailing `?` after the subscript index: in Swift,
            // `optionalVec?[N]` via `Collection.subscript` returns the element
            // type `T` directly. The parent `has_optional` flag is still set
            // when `field_is_optional` is true, which causes the enclosing
            // expression to be wrapped in `(... ?? fallback)` correctly.
            // Indexing into a Vec<Named> yields a Named element. Only pin opaque
            // syntax when the array itself was opaque (method-call); when the
            // owner is first-class, the array is a Swift `[T]` whose elements
            // are first-class T (property access).
            current_type = field_resolver.swift_advance(current_type.as_deref(), field_name);
            if !property_syntax {
                via_rust_vec = true;
            }
        } else {
            if !property_syntax {
                out.push_str("()");
            }
            // Insert `?` after the accessor for non-leaf optional fields so the
            // next member access becomes `?.`.
            if !is_leaf && field_resolver.is_optional(&base_path) {
                out.push('?');
                has_optional = true;
            }
            current_type = field_resolver.swift_advance(current_type.as_deref(), field_name);
        }
    }
    (out, has_optional)
}

/// Generate a `[String]` (or `[String]?`) expression for a `RustVec<RustString>`
/// field so that `contains` membership checks work against plain Swift Strings.
///
/// We use `.map { $0.asStr().toString() }` because:
/// 1. Iterating a `RustVec<RustString>` yields `RustStringRef` (not `RustString`), which
///    only has `asStr()` but not `toString()` directly. swift-bridge auto-renames the
///    Rust `as_str` method to lowerCamelCase `asStr` on the Swift side.
/// 2. The accessor may end with an `Optional<RustVec<RustString>>` (e.g. `sheet_names()` is
///    `Option<Vec<String>>` in Rust, which becomes `Optional<RustVec<RustString>>` in Swift).
/// 3. Optional chaining from parent `?.` already produces `Optional<RustVec<T>>`.
///
/// The returned tuple's bool indicates whether the result is `Optional<[String]>`
/// (callers coalesce with `?? []`) or already a concrete `[String]`. Emitting
/// `?? []` against a non-optional value compiles with a Swift warning but is
/// surfaced as an error in strict CI configurations, so we only emit `?.map`
/// + `?? []` when the accessor is genuinely optional.
///
/// Generate a `XCTAssert{True|False}(array.contains(where: { elem_str.contains(val) }), msg)` line
/// for field paths that traverse a collection with `[].` notation (e.g. `links[].url`).
///
/// `array_part` — left side of `[].` (e.g. `"links"`)
/// `element_part` — right side (e.g. `"url"` or `"link_type"`)
/// `full_field` — original assertion.field (used for enum lookup against the full path)
#[allow(clippy::too_many_arguments)]
fn swift_traversal_contains_assert(
    array_part: &str,
    element_part: &str,
    full_field: &str,
    val_expr: &str,
    result_var: &str,
    negate: bool,
    msg: &str,
    enum_fields: &std::collections::HashSet<String>,
    field_resolver: &FieldResolver,
) -> String {
    let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
    let resolved_full = field_resolver.resolve(full_field);
    let resolved_elem_part = resolved_full
        .find("[].")
        .map(|d| &resolved_full[d + 3..])
        .unwrap_or(element_part);
    let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
    let elem_is_enum = enum_fields.contains(full_field) || enum_fields.contains(resolved_full);
    let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
        || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
    let elem_str = if elem_is_enum {
        // Enum-typed fields are bridged as `String` (RustString in Swift).
        // A single `.toString()` converts RustString → Swift String.
        format!("{elem_accessor}.toString()")
    } else if elem_is_optional {
        format!("({elem_accessor}?.toString() ?? \"\")")
    } else {
        format!("{elem_accessor}.toString()")
    };
    let assert_fn = if negate { "XCTAssertFalse" } else { "XCTAssertTrue" };
    format!("        {assert_fn}({array_accessor}.contains(where: {{ {elem_str}.contains({val_expr}) }}), \"{msg}\")")
}

/// Returns `(map_expr, is_optional)` where `map_expr` is the `.map { … }` chain
/// that converts each element to a Swift `String`, and `is_optional` reports
/// whether the resulting expression is `Optional<[String]>` (callers should
/// coalesce with `?? []`) or already a concrete `[String]`.
fn swift_array_contains_expr(
    field: Option<&str>,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_field_accessor: &HashMap<String, String>,
) -> (String, bool) {
    // swift-bridge auto-renames Rust snake_case methods to lowerCamelCase on the
    // Swift side. `RustStringRef::as_str()` is exposed as `asStr()` — emitting
    // `as_str()` produces "value of type 'XRef' has no member 'as_str'" at
    // compile time.
    let Some(f) = field else {
        return (format!("{result_var}.map {{ $0.asStr().toString() }}"), false);
    };
    // Allow per-call overrides to name a different element accessor — used when
    // the array element is an opaque struct whose "name string" accessor is
    // not `as_str` (e.g. `StructureItem` exposes `kind() -> String`). The map
    // is keyed on the fixture field name (and resolved alias as a fallback).
    let resolved_field = field_resolver.resolve(f);
    let elem_accessor_name = result_field_accessor
        .get(f)
        .or_else(|| result_field_accessor.get(resolved_field))
        .cloned()
        .unwrap_or_else(|| "as_str".to_string());
    let elem_call = swift_ident(&elem_accessor_name.to_lower_camel_case());
    let (accessor, has_optional) = swift_build_accessor(f, result_var, field_resolver);
    // Only chain `?.map` when the accessor is actually optional. The previous
    // unconditional `?.map` produced "cannot use optional chaining on
    // non-optional value of type 'RustVec<…>'" for plain `Vec<T>` fields.
    let field_is_optional =
        has_optional || field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f));
    if field_is_optional {
        (format!("{accessor}?.map {{ $0.{elem_call}().toString() }}"), true)
    } else {
        (format!("{accessor}.map {{ $0.{elem_call}().toString() }}"), false)
    }
}

/// Emit a `XCTAssertTrue(array.contains(where: { ... }), msg)` line that
/// aggregates every text-bearing accessor on the element type of a `Vec<T>`
/// field, mirroring python's `_alef_e2e_item_texts` helper.
///
/// Returns `None` when:
///   - `field` is missing
///   - The field's root or leaf type cannot be resolved
///   - The element type has fewer than 2 stringy fields (the existing
///     single-accessor path is good enough and emits simpler code)
///
/// When matched, emits a closure that gathers `source().toString()`,
/// `items().map { $0.asStr().toString() }`, `alias()?.toString()`, etc. into
/// a flat `[String]` and substring-matches the expected value against every
/// entry. The matcher is lenient so that fixtures asserting `"os"` against
/// the `imports` field — where `ImportInfo.source` may be the bare module
/// name (`"os"`), the entire import statement (`"import os"`), or the
/// imported items (`from os import path` → items=["path"]) — succeed
/// regardless of how the language extractor surfaces the value.
fn swift_stringy_aggregator_contains_assert(
    field: Option<&str>,
    result_var: &str,
    field_resolver: &FieldResolver,
    swift_val: &str,
) -> Option<String> {
    use crate::e2e::field_access::StringyFieldKind;
    let field = field?;
    let resolved = field_resolver.resolve(field);
    // Only handle simple top-level array fields (no nested chains) for now.
    // Field path containing `.` or `[` is left to the existing traversal/array
    // paths.
    if resolved.contains('.') || resolved.contains('[') {
        return None;
    }
    let root_type = field_resolver.swift_root_type()?.clone();
    let elem_type = field_resolver.swift_advance(Some(&root_type), resolved)?;
    let stringy = field_resolver.swift_stringy_fields(&elem_type)?;
    if stringy.len() < 2 {
        return None;
    }
    let array_accessor = field_resolver.accessor(field, "swift", result_var);
    let mut texts_lines: Vec<String> = Vec::new();
    for sf in stringy {
        let call = swift_ident(&sf.name.to_lower_camel_case());
        match sf.kind {
            StringyFieldKind::Plain => {
                texts_lines.push(format!("                texts.append(item.{call}().toString())"));
            }
            StringyFieldKind::Optional => {
                texts_lines.push(format!(
                    "                if let v = item.{call}() {{ texts.append(v.toString()) }}"
                ));
            }
            StringyFieldKind::Vec => {
                // `item.field()` returns `RustVec<RustString>`. Mapping its
                // elements yields `RustStringRef` — a swift-bridge wrapper
                // around the borrowed RustString — which has `as_str()`
                // (snake_case, defined in `SwiftBridgeCore.swift`), NOT
                // `toString()` (only `RustString` has the latter via the
                // extension that calls `self.as_str().toString()`).
                texts_lines.push(format!(
                    "                texts.append(contentsOf: item.{call}().map {{ $0.as_str().toString() }})"
                ));
            }
        }
    }
    let texts_block = texts_lines.join("\n");
    Some(format!(
        "        XCTAssertTrue({array_accessor}.contains(where: {{ item in\n            var texts = [String]()\n{texts_block}\n            return texts.contains(where: {{ $0.contains({swift_val}) }})\n        }}), \"expected to contain: \\({swift_val})\")"
    ))
}

/// Generate a `.count` expression for an array field that may be nested inside optional parents.
///
/// Swift-bridge exposes all Rust fields as methods with `()`. When ancestor segments are
/// optional, we use `?.` chaining. The final count is coalesced with `?? 0` when there
/// are optional ancestors so the XCTAssert macro receives a non-optional `Int`.
///
/// Also check if the field itself (the leaf) is optional, which happens when the field
/// returns Optional<RustVec<T>> (e.g., `links()` may return Optional).
fn swift_array_count_expr(field: Option<&str>, result_var: &str, field_resolver: &FieldResolver) -> String {
    let Some(f) = field else {
        return format!("{result_var}.count");
    };
    let (accessor, mut has_optional) = swift_build_accessor(f, result_var, field_resolver);
    // Also check if the leaf field itself is optional.
    if field_resolver.is_optional(f) {
        has_optional = true;
    }
    if has_optional {
        // In Swift, accessing .count on an optional with ?. returns Optional<Int>,
        // so we coalesce with ?? 0 to get a concrete Int for XCTAssert.
        if accessor.contains("?.") {
            format!("{accessor}.count ?? 0")
        } else {
            // If no ?. but field is optional, the field_expr itself is Optional<RustVec<T>>
            // so we need ?. to call count.
            format!("({accessor}?.count ?? 0)")
        }
    } else {
        format!("{accessor}.count")
    }
}

/// Convert a `serde_json::Value` to a Swift literal string.
/// Returns true when `element_type` names a scalar Rust/Swift element type.
///
/// Scalar element types describe `Vec<T>` Rust parameters that the swift-bridge
/// surface exposes as native Swift `[T]` arrays — these can be constructed from
/// a Swift array literal without any opaque-type intermediate. Object element
/// types (everything else) require an `options_via` configuration to construct.
fn is_scalar_element_type(element_type: Option<&str>) -> bool {
    matches!(
        element_type.map(str::trim),
        Some(
            "String"
                | "str"
                | "bool"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "usize"
                | "f32"
                | "f64",
        )
    )
}

fn from_json_helper_for_arg(arg: &crate::e2e::config::ArgMapping, options_type: Option<&str>) -> String {
    let type_name = options_type.unwrap_or(arg.name.as_str());
    format!("{}FromJson", type_name.to_lower_camel_case())
}

fn json_to_swift(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_swift(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_swift).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_swift(&json_str))
        }
    }
}

/// When comparing numeric values in Swift, integer and floating-point literals
/// should not be wrapped in type constructors. Swift's type inference will infer
/// the correct type based on the field expression's return type.
///
/// Booleans ("true"/"false") are never wrapped — they are Swift `Bool` literals
/// and should never be cast to numeric types.
///
/// Floating-point literals should never be wrapped, as they may compare against
/// fields that return `Double` or other floating-point types.
fn swift_numeric_literal_cast(_field_expr: &str, numeric_literal: &str) -> String {
    // Never wrap booleans.
    if numeric_literal == "true" || numeric_literal == "false" {
        return numeric_literal.to_string();
    }

    // Don't wrap any numeric literals — Swift's type inference will handle it.
    // This avoids type mismatches when fields return specific types like UInt16,
    // UInt32, Int, etc. The comparison operator and field type will guide inference.
    numeric_literal.to_string()
}

/// Escape a string for embedding in a Swift double-quoted string literal.
fn escape_swift(s: &str) -> String {
    escape_swift_str(s)
}

/// Return the count-able target expression for `field_expr`.
///
/// For opaque method-call accessors (ending in `()` or `()?`), the returned
/// value depends on the field's IR kind:
///
/// - `Vec<T>` ⇒ `RustVec<T>`, which exposes `.count` directly. No wrap.
/// - `String` ⇒ `RustString`, which does NOT expose `.count`. Wrap with
///   `.toString()` so `.count` lands on Swift `String`.
///
/// First-class property accessors (no trailing parens) return Swift values
/// that already support `.count` directly.
///
/// The discriminator is the field's resolved leaf type, looked up against the
/// `SwiftFirstClassMap`'s vec field set when available. If the field is
/// unknown (None), fall back to the conservative wrap — RustString is the
/// dominant scalar-leaf case for top-level assertions.
fn swift_count_target(field_expr: &str, field_resolver: &FieldResolver, field: Option<&str>) -> String {
    let is_method_call = field_expr.trim_end().ends_with(')');
    if !is_method_call {
        return field_expr.to_string();
    }
    if let Some(f) = field
        && field_resolver.leaf_is_vec_via_swift_map(field_resolver.resolve(f))
    {
        return field_expr.to_string();
    }
    format!("{field_expr}.toString()")
}

/// Resolve the IR type name backing this call's result.
///
/// Lookup order mirrors PHP's `derive_root_type` for `[crates.e2e.calls.*]`
/// configs: any of `c, csharp, java, kotlin, go, php` overrides may carry a
/// `result_type = "ChatCompletionResponse"` field. The first non-empty value
/// wins. These overrides are language-agnostic IR type names — they were
/// originally added for the C/C# backends and other backends piggy-back on them
/// because the IR names are shared across every binding.
///
/// Returns `None` when no override sets `result_type`; the renderer then falls
/// back to the workspace-default heuristic in `SwiftFirstClassMap` (which
/// defaults to property access — the right call for first-class result types
/// like `FileObject` but wrong for opaque types like `ChatCompletionResponse`).
fn swift_call_result_type(call_config: &crate::core::config::e2e::CallConfig) -> Option<String> {
    const LOOKUP_LANGS: &[&str] = &["c", "csharp", "java", "kotlin", "go", "php"];
    for lang in LOOKUP_LANGS {
        if let Some(o) = call_config.overrides.get(*lang)
            && let Some(rt) = o.result_type.as_deref()
            && !rt.is_empty()
        {
            return Some(rt.to_string());
        }
    }
    None
}

/// Returns true when the field type would be emitted as a Swift primitive value
/// or a known first-class Codable struct/unit-enum, so it can appear on a
/// first-class Codable Swift struct without forcing the host type into a
/// typealias. Mirrors `first_class_field_supported` in alef-backend-swift.
///
/// Accepts:
/// - `Primitive` and `String`
/// - `Named(S)` when `S` is in `known_dto_names` (seeded with unit-serde enums and
///   grown via fixed-point iteration over candidate struct DTOs)
/// - `Vec<T>` and `Optional<T>` recursively
///
/// Rejects `Map`, `Path`, `Bytes`, `Duration`, `Char`, `Json`, and unknown
/// `Named(_)` references (the backend treats those as typealias-to-opaque).
fn swift_first_class_field_supported(ty: &crate::core::ir::TypeRef, known_dto_names: &HashSet<String>) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(_) | TypeRef::String => true,
        TypeRef::Named(name) => known_dto_names.contains(name),
        TypeRef::Vec(inner) | TypeRef::Optional(inner) => swift_first_class_field_supported(inner, known_dto_names),
        _ => false,
    }
}

/// Build the per-type Swift first-class/opaque classification map used by
/// `render_swift_with_first_class_map`.
///
/// A TypeDef is treated as first-class (Codable Swift struct → property access)
/// when it is not opaque, has serde derives, has at least one field, and every
/// binding field is supported by `swift_first_class_field_supported` against the
/// current first-class set. All other public types end up as typealiases to
/// opaque `RustBridge.X` classes whose fields are swift-bridge methods
/// (`.id()`, `.status()`).
///
/// Mirrors the fixed-point iteration in `alef-backend-swift::gen_bindings.rs`
/// (lines 100-130). Without the fixed point, a type like `TranscriptionResponse`
/// that holds `Option<Vec<TranscriptionSegment>>` would be wrongly classified
/// opaque, causing the renderer to emit `.text()` against a first-class struct
/// whose `text` is a `public let` property.
///
/// `field_types` records the next-type that each Named field traverses into,
/// so the renderer can advance its current-type cursor through nested
/// `data[0].id` style paths.
fn build_swift_first_class_map(
    type_defs: &[crate::core::ir::TypeDef],
    enum_defs: &[crate::core::ir::EnumDef],
    e2e_config: &crate::e2e::config::E2eConfig,
) -> SwiftFirstClassMap {
    use crate::core::ir::TypeRef;
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut vec_field_names: HashSet<String> = HashSet::new();
    fn inner_named(ty: &TypeRef) -> Option<String> {
        match ty {
            TypeRef::Named(n) => Some(n.clone()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    fn is_vec_ty(ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Vec(_) => true,
            TypeRef::Optional(inner) => is_vec_ty(inner),
            _ => false,
        }
    }
    // Seed with unit serde enum names — Codable on the Swift side and can appear
    // as leaf fields on struct DTOs (matches gen_bindings.rs unit_serde_enum_names).
    let mut known_dto_names: HashSet<String> = enum_defs
        .iter()
        .filter(|e| e.has_serde && e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.clone())
        .collect();

    // Candidate struct DTOs: non-opaque, has_serde, non-empty fields.
    // Trait types and binding-excluded types are skipped (matches backend semantics
    // — note backend further filters via `exclude_types`, which we don't have here,
    // but accepting a superset is safe: types not actually emitted simply never
    // appear in path-access chains).
    let candidates: Vec<&crate::core::ir::TypeDef> = type_defs
        .iter()
        .filter(|td| !td.is_trait && !td.is_opaque && td.has_serde && !td.fields.is_empty())
        .collect();

    loop {
        let prev = known_dto_names.len();
        for td in &candidates {
            if known_dto_names.contains(&td.name) {
                continue;
            }
            let all_supported = td
                .fields
                .iter()
                .filter(|f| !f.binding_excluded)
                .all(|f| swift_first_class_field_supported(&f.ty, &known_dto_names));
            if all_supported {
                known_dto_names.insert(td.name.clone());
            }
        }
        if known_dto_names.len() == prev {
            break;
        }
    }

    // The first-class set on SwiftFirstClassMap conceptually represents structs
    // accessed via property syntax. Unit enums never appear as the *owner* of a
    // chain segment (they are leaves), but including them is harmless since
    // `advance()` never returns them as a current_type for further traversal.
    let first_class_types: HashSet<String> = candidates
        .iter()
        .filter(|td| known_dto_names.contains(&td.name))
        .map(|td| td.name.clone())
        .collect();

    use crate::e2e::field_access::{StringyField, StringyFieldKind};
    // Enums are bridged as `String` on the swift-bridge surface (the binding
    // emits `fn kind(&self) -> String` for `kind: SomeEnum`), so they must
    // also count as text-bearing accessors when aggregating contains-matchers.
    let enum_names: HashSet<&str> = enum_defs.iter().map(|e| e.name.as_str()).collect();
    let classify_stringy = |ty: &TypeRef, field_optional: bool| -> Option<StringyFieldKind> {
        match ty {
            TypeRef::String => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(if field_optional {
                StringyFieldKind::Optional
            } else {
                StringyFieldKind::Plain
            }),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Optional),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Optional),
                _ => None,
            },
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::String => Some(StringyFieldKind::Vec),
                TypeRef::Named(name) if enum_names.contains(name.as_str()) => Some(StringyFieldKind::Vec),
                _ => None,
            },
            _ => None,
        }
    };
    let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
    for td in type_defs {
        let mut td_field_types: HashMap<String, String> = HashMap::new();
        let mut td_stringy: Vec<StringyField> = Vec::new();
        for f in &td.fields {
            if let Some(named) = inner_named(&f.ty) {
                td_field_types.insert(f.name.clone(), named);
            }
            if is_vec_ty(&f.ty) {
                vec_field_names.insert(f.name.clone());
            }
            if f.binding_excluded {
                continue;
            }
            if let Some(kind) = classify_stringy(&f.ty, f.optional) {
                td_stringy.push(StringyField {
                    name: f.name.clone(),
                    kind,
                });
            }
        }
        if !td_field_types.is_empty() {
            field_types.insert(td.name.clone(), td_field_types);
        }
        if !td_stringy.is_empty() {
            stringy_fields_by_type.insert(td.name.clone(), td_stringy);
        }
    }
    // Best-effort root-type detection: pick a unique TypeDef that contains all
    // `result_fields`. Falls back to `None` (renderer defaults to first-class
    // property syntax for unknown roots).
    let root_type = if e2e_config.result_fields.is_empty() {
        None
    } else {
        let matches: Vec<&crate::core::ir::TypeDef> = type_defs
            .iter()
            .filter(|td| {
                let names: HashSet<&str> = td.fields.iter().map(|f| f.name.as_str()).collect();
                e2e_config.result_fields.iter().all(|rf| names.contains(rf.as_str()))
            })
            .collect();
        if matches.len() == 1 {
            Some(matches[0].name.clone())
        } else {
            None
        }
    };
    SwiftFirstClassMap {
        first_class_types,
        field_types,
        vec_field_names,
        root_type,
        stringy_fields_by_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::field_access::FieldResolver;
    use std::collections::{HashMap, HashSet};

    fn make_resolver_tool_calls() -> FieldResolver {
        // Resolver for `choices[0].message.tool_calls[0].function.name`:
        //   - `choices` is a registered array field
        //   - `choices.message.tool_calls` is optional (Optional<RustVec<ToolCall>>)
        let mut optional = HashSet::new();
        optional.insert("choices.message.tool_calls".to_string());
        let mut arrays = HashSet::new();
        arrays.insert("choices".to_string());
        FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new())
    }

    /// Regression: after the optional `[0]` subscript, the codegen must NOT
    /// append a trailing `?`. The Swift compiler sees `?[0]` as consuming the
    /// optional chain, yielding the non-optional element type, so a subsequent
    /// `?.member` would trigger "cannot use optional chaining on non-optional
    /// value".
    ///
    /// With no `SwiftFirstClassMap` configured (default in this test), every
    /// accessor is emitted as a swift-bridge method call — so accessors are
    /// `result.choices()[0].message().toolCalls()?[0].function().name()`.
    #[test]
    fn optional_vec_subscript_does_not_emit_trailing_question_mark_before_next_segment() {
        let resolver = make_resolver_tool_calls();
        let (accessor, has_optional) =
            swift_build_accessor("choices[0].message.tool_calls[0].function.name", "result", &resolver);
        // `?` before `[0]` is correct (tool_calls is optional). Method-call
        // syntax (with `()`) is the default when no SwiftFirstClassMap is
        // supplied.
        assert!(
            accessor.contains("toolCalls()?[0]"),
            "expected `toolCalls()?[0]` for optional tool_calls, got: {accessor}"
        );
        // There must NOT be `?[0]?` (trailing `?` after the index).
        assert!(
            !accessor.contains("?[0]?"),
            "must not emit trailing `?` after subscript index: {accessor}"
        );
        // The expression IS optional overall (tool_calls may be nil).
        assert!(has_optional, "expected has_optional=true for optional field chain");
        // Subsequent member access uses `.` (non-optional chain) not `?.`.
        assert!(
            accessor.contains("[0].function"),
            "expected `.function` (non-optional) after subscript: {accessor}"
        );
    }

    /// `contains` against an array of opaque DTOs must aggregate every
    /// text-bearing accessor of the element type and substring-match the
    /// expected value, mirroring python's `_alef_e2e_item_texts`. This
    /// avoids the brittle "primary accessor" guess (e.g. ImportInfo →
    /// source) that misses values surfaced through sibling fields like
    /// `items` or `alias`.
    #[test]
    fn contains_against_vec_dto_aggregates_stringy_accessors() {
        use crate::e2e::field_access::{StringyField, StringyFieldKind, SwiftFirstClassMap};
        // Simulate the ImportInfo element type with its three text-bearing
        // accessors: source (plain), items (vec), alias (optional).
        let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
        stringy_fields_by_type.insert(
            "ImportInfo".to_string(),
            vec![
                StringyField {
                    name: "source".to_string(),
                    kind: StringyFieldKind::Plain,
                },
                StringyField {
                    name: "items".to_string(),
                    kind: StringyFieldKind::Vec,
                },
                StringyField {
                    name: "alias".to_string(),
                    kind: StringyFieldKind::Optional,
                },
            ],
        );
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut process_fields = HashMap::new();
        process_fields.insert("imports".to_string(), "ImportInfo".to_string());
        field_types.insert("ProcessResult".to_string(), process_fields);

        let mut arrays = HashSet::new();
        arrays.insert("imports".to_string());

        let map = SwiftFirstClassMap {
            first_class_types: HashSet::new(),
            field_types,
            vec_field_names: HashSet::new(),
            root_type: None,
            stringy_fields_by_type,
        };
        let resolver = FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &arrays,
            &HashSet::new(),
            &HashMap::new(),
            map,
        )
        .with_swift_root_type(Some("ProcessResult".to_string()));

        let line = swift_stringy_aggregator_contains_assert(Some("imports"), "result", &resolver, "\"os\"")
            .expect("aggregator should fire for Vec<ImportInfo> contains");
        assert!(
            line.contains("result.imports().contains(where: { item in"),
            "expected contains(where:) over result.imports(): {line}"
        );
        assert!(
            line.contains("texts.append(item.source().toString())"),
            "expected plain source() accessor: {line}"
        );
        assert!(
            line.contains("texts.append(contentsOf: item.items().map { $0.as_str().toString() })"),
            "expected vec items() flattened via .map as_str(): {line}"
        );
        assert!(
            line.contains("if let v = item.alias()"),
            "expected optional alias() unwrap: {line}"
        );
        // Substring match — NOT exact equality.
        assert!(
            line.contains("$0.contains(\"os\")"),
            "expected substring contains over expected value: {line}"
        );
        assert!(!line.contains("$0 == \"os\""), "must not use exact equality: {line}");
    }

    /// When the element type has fewer than 2 stringy accessors, the
    /// aggregator should bow out and let the simpler single-accessor path
    /// emit code — keeping diff churn minimal on fixtures that already pass.
    #[test]
    fn contains_aggregator_skips_when_only_one_stringy_field() {
        use crate::e2e::field_access::{StringyField, StringyFieldKind, SwiftFirstClassMap};
        let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
        stringy_fields_by_type.insert(
            "TagInfo".to_string(),
            vec![StringyField {
                name: "name".to_string(),
                kind: StringyFieldKind::Plain,
            }],
        );
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut root_fields = HashMap::new();
        root_fields.insert("tags".to_string(), "TagInfo".to_string());
        field_types.insert("Root".to_string(), root_fields);
        let mut arrays = HashSet::new();
        arrays.insert("tags".to_string());
        let map = SwiftFirstClassMap {
            first_class_types: HashSet::new(),
            field_types,
            vec_field_names: HashSet::new(),
            root_type: None,
            stringy_fields_by_type,
        };
        let resolver = FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &arrays,
            &HashSet::new(),
            &HashMap::new(),
            map,
        )
        .with_swift_root_type(Some("Root".to_string()));
        assert!(
            swift_stringy_aggregator_contains_assert(Some("tags"), "result", &resolver, "\"x\"").is_none(),
            "single-stringy-field types must not trigger the aggregator"
        );
    }

    /// Regression: registry-mode download_swift_artifact.sh must validate the
    /// cached zip's checksum against Package.swift's expected checksum before
    /// reusing it. Without this, a version bump (e.g. rc.49 → rc.50) leaves a
    /// stale cached zip in place — same filename, different URL contents — and
    /// SwiftPM rejects with "checksum of downloaded artifact does not match
    /// checksum specified by the manifest".
    #[test]
    fn download_swift_artifact_script_validates_cache_checksum() {
        let script =
            render_download_swift_artifact_script("DemoKit", "https://example.invalid/acme/demo-kit", "1.4.0-rc.50");
        assert!(
            script.contains("EXPECTED_CHECKSUM=") && script.contains("Package.swift"),
            "script must extract expected checksum from Package.swift"
        );
        assert!(
            script.contains("ACTUAL_CHECKSUM=$(swift package compute-checksum"),
            "script must compute checksum of cached artifact"
        );
        assert!(
            script.contains("rm -f \"$ARTIFACT_FILE\""),
            "script must invalidate cache on checksum mismatch"
        );
        assert!(
            script.contains("Cached artifact checksum mismatch"),
            "script must log mismatch with the canonical message"
        );
    }
}

/// Emit a Swift test backend stub class for a trait bridge.
///
/// Generates a class conforming to `Swift{TraitName}Bridge`. Required methods
/// are overridden with Swift-idiomatic defaults. Async methods use `async throws`
/// and return the default value directly. The `name` computed property is emitted
/// when a Plugin super-trait is configured.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    use crate::backends::swift::type_map::SwiftMapper;
    use crate::codegen::defaults::language_defaults;
    use crate::codegen::type_mapper::TypeMapper as _;
    use heck::{ToLowerCamelCase, ToUpperCamelCase};
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    // Use the canonical naming helper so this stays in sync with the production
    // codegen in `src/backends/swift/gen_bindings/trait_bridge.rs`.
    let protocol_name = crate::backends::swift::naming::bridge_protocol_name(&trait_bridge.trait_name);

    // Prefer the fixture's input "name" field (e.g. "test-extractor") over the
    // fixture id, which is an internal snake_case identifier, not a backend name.
    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let defaults = language_defaults("swift");
    let mapper = SwiftMapper;

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name}: {protocol_name} {{");

    // Plugin super-trait conformance: emit all SwiftPluginBridge required methods
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(setup, "    var name: String {{ \"{plugin_name}\" }}");
        let _ = writeln!(setup, "    func version() -> String {{ \"1.0.0\" }}");
        let _ = writeln!(setup, "    func initialize() throws {{}}");
        let _ = writeln!(setup, "    func shutdown() throws {{}}");
    }

    // Required methods — trait bridge protocols marshal excluded types as JSON strings.
    // Use concrete Swift types, converting Named types to String (JSON marshalling).
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        let method_name = method.name.to_lower_camel_case();

        // Build parameter list. Named types (excluded/internal) are marshalled as String in trait bridges.
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let param_type = match &p.ty {
                    crate::core::ir::TypeRef::Named(_) => "String".to_string(),
                    _ => mapper.map_type(&p.ty).to_string(),
                };
                format!("{}: {}", p.name.to_lower_camel_case(), param_type)
            })
            .collect();
        let params_str = params.join(", ");

        // Return type: Named types are marshalled as String (JSON).
        let return_type = match &method.return_type {
            crate::core::ir::TypeRef::Named(_) => "String".to_string(),
            _ => mapper.map_type(&method.return_type).to_string(),
        };

        // Default value: use String for marshalled types, otherwise use defaults.emit_default.
        let default_val = match &method.return_type {
            crate::core::ir::TypeRef::Named(_) => "\"\"".to_string(),
            _ => defaults.emit_default(&method.return_type),
        };

        // NOTE: Swift trait bridge methods are always sync (no async), even if the Rust trait
        // declares async. The adapter/bridge layer handles async-to-sync conversion.
        if method.error_type.is_some() {
            let _ = writeln!(
                setup,
                "    func {method_name}({params_str}) throws -> {return_type} {{ {default_val} }}"
            );
        } else {
            let _ = writeln!(
                setup,
                "    func {method_name}({params_str}) -> {return_type} {{ {default_val} }}"
            );
        }
    }

    let _ = writeln!(setup, "}}");

    // Emit teardown: unregister call to prevent test backends from leaking into subsequent tests.
    // The adapter class emitted by alef-backend-swift uses a fixed name derived from the trait.
    // Pattern: `try? <Module>.unregister<Trait>("<adapter-name>")`
    let unregister_fn = format!("unregister{}", trait_bridge.trait_name.to_upper_camel_case());
    let adapter_name = format!("swift-bridge-{}", trait_bridge.trait_name.to_snake_case());
    // Emit without module qualification: caller will add it when needed.
    let teardown = format!("try? {unregister_fn}(\"{adapter_name}\")");

    super::TestBackendEmission {
        setup_block: setup,
        arg_expr: format!("{class_name}()"),
        type_imports: Vec::new(),
        teardown_block: teardown,
    }
}

#[cfg(test)]
mod test_backend_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
        }
    }

    /// Verify that no sample_core-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn swift_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("do_work", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains("SampleCrate"),
            "must not contain literal 'SampleCrate', got:\n{output}"
        );
        assert!(
            !output.contains("sample_crate::"),
            "must not contain 'sample_crate::', got:\n{output}"
        );
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "class name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("SwiftTestTraitBridge"),
            "class must conform to the Swift protocol derived from trait name, got:\n{output}"
        );
        assert!(
            output.contains("doWork"),
            "required method must be emitted in camelCase, got:\n{output}"
        );
    }

    fn make_param(name: &str, ty: TypeRef) -> crate::core::ir::ParamDef {
        crate::core::ir::ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
        }
    }

    fn make_method_with_params(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![
                make_param("image_bytes", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            return_type: TypeRef::Named("ExtractionResult".to_string()),
            is_async: true,
            is_static: false,
            error_type: Some("anyhow::Error".to_string()),
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    /// Verify params use concrete Swift types (not `Any`) and named return types marshal as JSON strings.
    #[test]
    fn swift_stub_uses_typed_params_and_marshaled_named_return() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method_with_params("processImage", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains(": Any"),
            "param type must not be `Any`, got:\n{output}"
        );
        assert!(
            output.contains("imageBytes: Data"),
            "bytes param must map to Data, got:\n{output}"
        );
        assert!(
            output.contains("mimeType: String"),
            "string param must map to String, got:\n{output}"
        );
        assert!(
            output.contains("-> String"),
            "named return type must marshal as String, got:\n{output}"
        );
    }

    /// Verify that `fixture.input["name"]` is used as the plugin name when present.
    #[test]
    fn swift_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("do_work", true);
        let methods = [&required_method];
        let mut fixture = make_fixture("my_fixture_id");
        fixture.input = serde_json::json!({ "name": "my-backend-name" });

        let emission = emit_test_backend(&bridge, &methods, &fixture);
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("\"my-backend-name\""),
            "plugin name must come from fixture.input.name, got:\n{output}"
        );
    }
}
