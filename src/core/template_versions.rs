//! Centralized third-party dependency version strings for code generation.
//!
//! This module consolidates all hardcoded version strings used in scaffold and e2e
//! code generation. Each const that should be auto-bumped by Renovate carries a
//! marker comment on the line directly above it:
//! `// renovate: datasource=... depName=...`
//!
//! The regex customManager in `renovate.json` matches those markers against the
//! adjacent `pub const NAME: &str = "..."` and keeps the version fresh so
//! regenerated bindings ship current dependencies.
//!
//! When adding a new version: choose the appropriate submodule based on package ecosystem,
//! add the renovate marker (if the value is a single auto-bumpable version/range for a real
//! registry package), and use the const in templates. Consts without a marker (toolchain
//! targets, language/engine constraints, compound strings, `>= x and < y` ranges, and
//! artifacts hosted on non-default registries) are tracked manually.

pub mod npm {
    pub const NODE_ENGINE: &str = ">= 22";

    // renovate: datasource=npm depName=@napi-rs/cli
    pub const NAPI_RS_CLI_DEVDEPS: &str = "^3.0.0";

    // renovate: datasource=npm depName=@napi-rs/cli
    pub const NAPI_RS_CLI_CRATE: &str = "^3.7.3";

    // renovate: datasource=npm depName=typescript
    pub const TYPESCRIPT: &str = "^7.0.0";

    // renovate: datasource=npm depName=vitest
    pub const VITEST: &str = "^4.1.10";

    // renovate: datasource=npm depName=@types/node
    pub const TYPES_NODE: &str = "^26.0.0";

    /// Filename `napi build` writes its own auto-derived type declarations to.
    ///
    /// Must never be `index.d.ts`: alef's node backend hand-derives its own
    /// `index.d.ts` (unions, doc comments, the `alef:hash:` provenance line) into the
    /// crate directory, and `napi build`'s `--dts` output defaults to the crate's
    /// `package.json` `"types"` field, which is also `index.d.ts`. Left unpinned, every
    /// `napi build` invocation — the default node build step, `alef publish`, and the
    /// scaffolded `npm run build` — silently overwrites alef's canonical type
    /// declarations with napi-rs's own, discarding the provenance header alef relies on
    /// to detect staleness.
    pub const NAPI_AUTO_DTS_FILENAME: &str = "index.native.d.ts";

    // renovate: datasource=npm depName=rollup
    pub const ROLLUP: &str = "^4.53.3";

    // renovate: datasource=npm depName=vite-plugin-top-level-await
    pub const VITE_PLUGIN_TOP_LEVEL_AWAIT: &str = "^1.4.0";

    // renovate: datasource=npm depName=vite-plugin-wasm
    pub const VITE_PLUGIN_WASM: &str = "^3.4.0";
}

pub mod cargo {
    // renovate: datasource=crate depName=tracing
    pub const TRACING: &str = "0.1";

    // renovate: datasource=crate depName=napi
    pub const NAPI: &str = "3";

    // renovate: datasource=crate depName=napi-derive
    pub const NAPI_DERIVE: &str = "3";

    // renovate: datasource=crate depName=napi-build
    pub const NAPI_BUILD: &str = "2";

    // renovate: datasource=crate depName=pyo3
    pub const PYO3: &str = "0.29";

    // renovate: datasource=crate depName=pyo3-async-runtimes
    pub const PYO3_ASYNC_RUNTIMES: &str = "0.29";

    // renovate: datasource=crate depName=magnus
    pub const MAGNUS: &str = "0.8";

    // renovate: datasource=crate depName=ext-php-rs
    pub const EXT_PHP_RS: &str = "0.15.15";

    // renovate: datasource=crate depName=js-sys
    pub const JS_SYS: &str = "0.3";

    // renovate: datasource=crate depName=wasm-bindgen
    pub const WASM_BINDGEN: &str = "0.2";

    // renovate: datasource=crate depName=wasm-bindgen-futures
    pub const WASM_BINDGEN_FUTURES: &str = "0.4";

    // renovate: datasource=crate depName=futures
    pub const FUTURES: &str = "0.3";

    // renovate: datasource=crate depName=futures-util
    pub const FUTURES_UTIL: &str = "0.3";

    // renovate: datasource=crate depName=serde-wasm-bindgen
    pub const SERDE_WASM_BINDGEN: &str = "0.6";

    // renovate: datasource=crate depName=cbindgen
    pub const CBINDGEN: &str = "0.29";

    // renovate: datasource=crate depName=tempfile
    pub const TEMPFILE: &str = "3";

    // renovate: datasource=crate depName=rustler
    pub const RUSTLER: &str = "0.38";

    // renovate: datasource=crate depName=async-trait
    pub const ASYNC_TRAIT: &str = "0.1";

    // Hoisted out of `scaffold::languages::jni`, where it sat as an inline `"0.22"`. Only
    // this file is in `renovate.json`'s `managerFilePatterns`, so a version literal written
    // anywhere else is bumped by nothing and drifts behind the ecosystem for as long as it
    // stays there -- which is how consumers on 0.23 came to be handed 0.22 on every regen.
    // Every other Cargo literal still inlined in a scaffolder has the same defect latent in
    // it; the floor in `scaffold::version_floor` is what keeps a stale one from being
    // destructive, not this hoist. The prose has to sit *above* the marker: the
    // `customManager` regex requires the marker line to be followed directly by the
    // `pub const`, so a comment wedged between the two silently un-tracks the entry. ~keep
    // renovate: datasource=crate depName=base64
    pub const BASE64: &str = "0.23";

    // renovate: datasource=crate depName=jni
    pub const JNI: &str = "0.22";

    // renovate: datasource=crate depName=extendr-api
    pub const EXTENDR_API: &str = "0.9";

    // renovate: datasource=crate depName=axum
    pub const AXUM: &str = "0.8";

    // renovate: datasource=crate depName=tokio-stream
    pub const TOKIO_STREAM: &str = "0.1";

    // renovate: datasource=crate depName=walkdir
    pub const WALKDIR: &str = "2";

    // renovate: datasource=crate depName=tower-http
    pub const TOWER_HTTP: &str = "0.7";

    // renovate: datasource=crate depName=serde
    pub const SERDE: &str = "1";

    // renovate: datasource=crate depName=serde_json
    pub const SERDE_JSON: &str = "1";

    // renovate: datasource=crate depName=tokio
    pub const TOKIO: &str = "1";

    // renovate: datasource=crate depName=flutter_rust_bridge
    pub const FLUTTER_RUST_BRIDGE: &str = "2.12.0";

    // renovate: datasource=crate depName=flutter_rust_bridge_codegen
    pub const FLUTTER_RUST_BRIDGE_CODEGEN: &str = "2.12.0";

    // renovate: datasource=crate depName=swift-bridge
    pub const SWIFT_BRIDGE: &str = "0.1.59";

    // renovate: datasource=crate depName=swift-bridge-build
    pub const SWIFT_BRIDGE_BUILD: &str = "0.1.59";
}

pub mod pypi {
    pub const MATURIN_BUILD_REQUIRES: &str = "maturin>=1.0,<2.0";

    // Replaces mypy: pyrefly is a fast single-binary Rust type-checker, run as a
    // renovate: datasource=pypi depName=pyrefly
    pub const PYREFLY: &str = ">=1.1.1";

    // renovate: datasource=pypi depName=pytest
    pub const PYTEST: &str = ">=7.4";

    // renovate: datasource=pypi depName=pytest-asyncio
    pub const PYTEST_ASYNCIO: &str = ">=0.23";

    // renovate: datasource=pypi depName=pytest-timeout
    pub const PYTEST_TIMEOUT: &str = ">=2.1";

    // renovate: datasource=pypi depName=setuptools
    pub const SETUPTOOLS: &str = ">=68";
}

pub mod gem {
    // Ruby gems are pinned as pessimistic (`~>`) constraints. Renovate's regex
    // custom manager cannot bump these: it derives no range strategy for a
    // custom-manager range, and the `ruby` versioning then emits an
    // "Unsupported range strategy" warning and computes no update. They are
    // tracked manually here — the `~>` floors already admit newer releases at
    // `bundle install`. (Other ecosystems whose versioning tolerates a missing
    // strategy, e.g. hex, keep their markers.)
    pub const RB_SYS: &str = "\">= 0.9\", \"< 0.9.128\"";

    pub const SORBET_RUNTIME: &str = "~> 0.5";

    pub const RAKE_COMPILER: &str = "~> 1.2";

    pub const RSPEC_SCAFFOLD: &str = "~> 3.0";

    pub const RSPEC_E2E: &str = "~> 3.13";

    pub const RUBOCOP_SCAFFOLD: &str = "~> 1.0";

    pub const RUBOCOP_E2E: &str = "~> 1.86";

    pub const RUBOCOP_PERFORMANCE: &str = "~> 1.0";

    pub const RUBOCOP_RSPEC_SCAFFOLD: &str = "~> 3.0";

    pub const RUBOCOP_RSPEC_E2E: &str = "~> 3.9";

    pub const STEEP: &str = "~> 1.0";

    pub const FARADAY: &str = "~> 2.0";
}

pub mod packagist {

    // renovate: datasource=packagist depName=phpunit/phpunit
    // ~keep Span every PHPUnit major that supports the generated `"php": ">=8.2"` floor: 11.x needs
    // >=8.2, 12.x needs >=8.3, 13.x needs >=8.4.1. A 13-only constraint is unsatisfiable on 8.2/8.3,
    // so `composer install` hard-fails there, and Dependabot — which resolves Composer against the
    // declared platform floor rather than the runtime PHP — reported "requirements could not be
    // resolved" on every run. Composer picks the highest major the actual PHP supports.
    pub const PHPUNIT: &str = "^11.5 || ^12.0 || ^13.1";

    // renovate: datasource=packagist depName=guzzlehttp/guzzle
    // Accept both ^7 and ^8: a Composer resolver picking guzzle 8.x (e.g. because
    // a committed composer.lock already resolved to 8.x) must not be rejected by a
    // stale ^7-only constraint, which makes `composer install` hard-fail on a
    // require/lock mismatch. If a generated e2e test depends on guzzle 7's
    // client-side Content-Length validation behavior, gate or rework that test
    // rather than narrowing this constraint back to ^7 only.
    pub const GUZZLE: &str = "^7.0 || ^8.0";
}

pub mod maven {
    // renovate: datasource=maven depName=org.junit:junit-bom
    pub const JUNIT: &str = "6.1.3";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-compiler-plugin
    pub const MAVEN_COMPILER_PLUGIN: &str = "3.15.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-surefire-plugin
    pub const MAVEN_SUREFIRE_PLUGIN: &str = "3.5.6";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-surefire-plugin
    pub const MAVEN_SUREFIRE_PLUGIN_E2E: &str = "3.5.6";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-checkstyle-plugin
    pub const MAVEN_CHECKSTYLE_PLUGIN: &str = "3.6.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-source-plugin
    pub const MAVEN_SOURCE_PLUGIN: &str = "3.4.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-javadoc-plugin
    pub const MAVEN_JAVADOC_PLUGIN: &str = "3.12.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-gpg-plugin
    pub const MAVEN_GPG_PLUGIN: &str = "3.2.8";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-clean-plugin
    pub const MAVEN_CLEAN_PLUGIN: &str = "3.5.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-resources-plugin
    pub const MAVEN_RESOURCES_PLUGIN: &str = "3.5.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-jar-plugin
    pub const MAVEN_JAR_PLUGIN: &str = "3.5.1";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-install-plugin
    pub const MAVEN_INSTALL_PLUGIN: &str = "3.1.4";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-deploy-plugin
    pub const MAVEN_DEPLOY_PLUGIN: &str = "3.1.4";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-site-plugin
    pub const MAVEN_SITE_PLUGIN: &str = "4.0.0-M16";

    // renovate: datasource=maven depName=org.sonatype.central:central-publishing-maven-plugin
    pub const CENTRAL_PUBLISHING_PLUGIN: &str = "0.11.0";

    // renovate: datasource=maven depName=org.codehaus.mojo:versions-maven-plugin
    pub const VERSIONS_MAVEN_PLUGIN: &str = "2.21.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-enforcer-plugin
    pub const MAVEN_ENFORCER_PLUGIN: &str = "3.6.3";

    // renovate: datasource=maven depName=org.jacoco:jacoco-maven-plugin
    pub const JACOCO_MAVEN_PLUGIN: &str = "0.8.15";

    // renovate: datasource=maven depName=com.puppycrawl.tools:checkstyle
    pub const CHECKSTYLE: &str = "13.11.0";

    // renovate: datasource=maven depName=org.jspecify:jspecify
    pub const JSPECIFY: &str = "1.0.1";

    // renovate: datasource=maven depName=com.fasterxml.jackson.core:jackson-databind
    pub const JACKSON: &str = "2.22.2";

    // renovate: datasource=maven depName=com.fasterxml.jackson.core:jackson-annotations
    pub const JACKSON_ANNOTATIONS: &str = "2.22";

    // renovate: datasource=maven depName=com.fasterxml.jackson.core:jackson-databind
    pub const JACKSON_E2E: &str = "2.22.2";

    // renovate: datasource=maven depName=org.assertj:assertj-core
    pub const ASSERTJ: &str = "4.0.0-M1";

    // renovate: datasource=maven depName=org.codehaus.mojo:build-helper-maven-plugin
    pub const BUILD_HELPER_MAVEN_PLUGIN: &str = "3.6.1";

    // renovate: datasource=maven depName=org.jetbrains.kotlin:kotlin-gradle-plugin
    pub const KOTLIN_JVM_PLUGIN: &str = "2.4.10";

    // Android Gradle plugin — hosted on Google's Maven repo, not Maven Central; tracked manually.
    pub const ANDROID_GRADLE_PLUGIN: &str = "9.2.1";

    // renovate: datasource=maven depName=org.jlleitschuh.gradle:ktlint-gradle
    pub const KTLINT_GRADLE_PLUGIN: &str = "14.2.0";

    // renovate: datasource=maven depName=com.github.ben-manes:gradle-versions-plugin
    pub const GRADLE_VERSIONS_PLUGIN: &str = "0.54.0";

    // renovate: datasource=maven depName=com.pinterest.ktlint:ktlint-cli
    pub const KTLINT: &str = "1.8.0";

    // renovate: datasource=maven depName=org.jetbrains.kotlinx:kotlinx-coroutines-core
    pub const KOTLINX_COROUTINES_CORE: &str = "1.11.0";

    // renovate: datasource=maven depName=net.java.dev.jna:jna
    pub const JNA: &str = "5.19.1";

    // renovate: datasource=maven depName=junit:junit
    pub const JUNIT_LEGACY: &str = "4.13.2";

    // androidx — hosted on Google's Maven repo, not Maven Central; tracked manually.
    pub const ANDROIDX_TEST_EXT_JUNIT: &str = "1.3.0";

    // androidx — hosted on Google's Maven repo, not Maven Central; tracked manually.
    pub const ANDROIDX_TEST_ESPRESSO_CORE: &str = "3.7.0";

    // renovate: datasource=maven depName=com.vanniktech:gradle-maven-publish-plugin
    pub const VANNIKTECH_MAVEN_PUBLISH: &str = "0.37.0";

    // Minimum Maven runtime enforced by the generated pom's requireMavenVersion rule.
    // This is a compatibility FLOOR, not a tracked dependency: keep it at or below the
    // Maven version GitHub-hosted runners ship so `enforce-maven` never fails in CI.
    // Do NOT add a `renovate:` annotation here — auto-bumping the floor to the newest
    // maven-core release is what broke publishing in 0.48.2 (runners had 3.9.11 while
    // the floor had been bumped to 3.9.16).
    pub const MAVEN_CORE: &str = "3.6.3";

    // renovate: datasource=maven depName=org.jetbrains:annotations
    pub const JETBRAINS_ANNOTATIONS: &str = "26.1.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-antrun-plugin
    pub const MAVEN_ANTRUN_PLUGIN: &str = "3.2.0";
}

pub mod nuget {
    // renovate: datasource=nuget depName=Microsoft.NET.Test.Sdk
    pub const MICROSOFT_NET_TEST_SDK: &str = "18.5.1";

    // renovate: datasource=nuget depName=xunit
    pub const XUNIT: &str = "2.9.3";

    // renovate: datasource=nuget depName=xunit.runner.visualstudio
    pub const XUNIT_RUNNER_VISUALSTUDIO: &str = "3.1.5";
}

pub mod hex {
    // renovate: datasource=hex depName=rustler
    pub const RUSTLER: &str = "~> 0.38";

    // renovate: datasource=hex depName=rustler_precompiled
    pub const RUSTLER_PRECOMPILED: &str = "~> 0.9";

    // renovate: datasource=hex depName=credo
    pub const CREDO: &str = "~> 1.7";

    // renovate: datasource=hex depName=ex_doc
    pub const EX_DOC: &str = "~> 0.40";

    // renovate: datasource=hex depName=finch
    pub const FINCH: &str = "~> 0.23";

    // renovate: datasource=hex depName=req
    pub const REQ: &str = "~> 0.7";

    // renovate: datasource=hex depName=jason
    pub const JASON: &str = "~> 1.4";

    // Gleam range constraints (`>= x and < y`) are not single auto-bumpable versions.
    pub const GLEAM_STDLIB_VERSION_RANGE: &str = ">= 0.34.0 and < 2.0.0";

    pub const GLEEUNIT_VERSION_RANGE: &str = ">= 1.0.0 and < 2.0.0";

    pub const GLEAM_HTTPC_VERSION_RANGE: &str = ">= 4.0.0 and < 6.0.0";

    pub const ENVOY_VERSION_RANGE: &str = ">= 1.0.0 and < 2.0.0";
}

/// pub.dev (Dart) ecosystem.
pub mod pub_dev {
    // renovate: datasource=dart depName=test
    pub const TEST_PACKAGE: &str = "^1.25.0";

    // renovate: datasource=dart depName=lints
    pub const LINTS: &str = "^6.1.0";

    // renovate: datasource=dart depName=ffi
    pub const FFI_PACKAGE: &str = "^2.2.0";

    // renovate: datasource=dart depName=http
    pub const HTTP_PACKAGE: &str = "^1.2.0";

    // renovate: datasource=dart depName=crypto
    pub const CRYPTO: &str = "^3.0.0";

    // renovate: datasource=dart depName=freezed_annotation
    pub const FREEZED_ANNOTATION: &str = "^3.1.0";

    // renovate: datasource=dart depName=json_annotation
    pub const JSON_ANNOTATION: &str = "^4.11.0";

    // renovate: datasource=dart depName=freezed
    pub const FREEZED: &str = "^4.0.0";

    // renovate: datasource=dart depName=build_runner
    pub const BUILD_RUNNER: &str = "^2.15.0";

    // renovate: datasource=dart depName=json_serializable
    pub const JSON_SERIALIZABLE: &str = "^6.13.2";

    // renovate: datasource=dart depName=native_assets_cli
    pub const NATIVE_ASSETS_CLI: &str = "^0.18.0";
}

/// Platform / toolchain pins. None of these auto-bump; track manually.
pub mod toolchain {
    pub const MIN_ZIG_VERSION: &str = "0.16.0";

    pub const DART_SDK_CONSTRAINT: &str = ">=3.11.0 <4.0.0";

    /// JVM bytecode target for the Java backend (Panama FFM, JDK 22+ required).
    pub const JAVA_JVM_TARGET: &str = "25";

    /// JVM bytecode target for the Kotlin/JVM backend.
    pub const KOTLIN_JVM_TARGET: &str = "21";

    #[deprecated(since = "0.16.4", note = "use JAVA_JVM_TARGET or KOTLIN_JVM_TARGET")]
    pub const JVM_TARGET: &str = "25";

    pub const SWIFT_MIN_MACOS: &str = "13.0";

    pub const SWIFT_MIN_IOS: &str = "16.0";

    pub const GRADLE_VERSION: &str = "9.6.0";

    pub const ANDROID_COMPILE_SDK: &str = "36";
    pub const ANDROID_MIN_SDK: &str = "24";
    pub const ANDROID_JVM_TARGET: &str = "17";
}

pub mod cran {
    // Manually tracked: Renovate has no CRAN datasource.
    pub const REXTENDR: &str = "0.4.2";
}

pub mod precommit {
    pub const ALEF_REV: &str = "v0.65.0";

    /// Codegen format version — bumped only when output-affecting codegen
    /// changes require all generated files to be re-stamped. Unlike
    /// `ALEF_REV`, this is NOT incremented on every crate release, so
    /// routine alef upgrades do not mass-invalidate client bindings.
    ///
    /// Increment when:
    /// - the domain separator in `compute_inputs_hash` changes
    /// - a structural codegen change means existing generated files are
    ///   incompatible with the new generator
    ///
    /// Do NOT increment for dependency version bumps, style fixes, or any
    /// change that does not affect `compute_inputs_hash` output.
    pub const CODEGEN_FORMAT_VERSION: &str = "2";
}

/// Wire-representation generations for the FFI ABI surfaces that binding
/// backends hand-declare rather than derive from the cbindgen header.
pub mod abi {
    /// Generation of the opaque-handle wire representation that the FFI glue
    /// and every hand-declaring binding backend currently agree on.
    ///
    /// Stamped into generated files under
    /// [`crate::core::hash::HANDLE_ABI_STAMP_KEY`] so
    /// `crate::bin_cli::helpers::find_stamp_disagreement` can fail `alef
    /// verify` when one tree contains artifacts from two different handle
    /// generations. The value itself carries no meaning beyond "artifacts
    /// bearing the same value speak the same handle ABI".
    ///
    /// # Bump policy
    ///
    /// Increment **only** when the handle's wire representation changes —
    /// its C-level kind (opaque `struct X *` vs. scalar `uint64_t`), its
    /// width, or its encoding (e.g. adding a generation counter to the
    /// packed bits). Nothing else bumps it: not codegen refactors, not
    /// template edits, not new handle-taking functions, not alef releases.
    ///
    /// # Why not `CODEGEN_FORMAT_VERSION`
    ///
    /// That constant is deliberately NOT reused here, and is verifiably
    /// unusable for this job: it is `"2"` and only two commits in alef's
    /// entire history touch it (its introduction in 0.34.0 and a lint sweep
    /// in 0.34.7). Artifacts generated on *both* sides of the pointer→`u64`
    /// handle migration therefore carry the identical value, so comparing it
    /// across a straddled tree can never detect the straddle. It also has the
    /// opposite blast radius: it feeds `compute_inputs_hash`, so bumping it
    /// re-stamps every generated file in every consumer repo — a cost that
    /// must not be coupled to an ABI change that affects only the handle-
    /// carrying subset, and a mass-invalidation that must not be triggered by
    /// one. The two versions answer different questions and move on different
    /// schedules; keep them independent. ~keep
    pub const HANDLE_ABI_VERSION: &str = "1";
}
