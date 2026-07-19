//! Centralized third-party dependency version strings for code generation.
//!
//! This module consolidates all hardcoded version strings used in scaffold and e2e
//! code generation. Each const that should be auto-bumped by Renovate includes a
//! marker comment: `// renovate: datasource=... depName=...`
//!
//! When adding a new version: choose the appropriate submodule based on package ecosystem,
//! add the renovate marker (if applicable), and use the const in templates.

pub mod npm {
    pub const NODE_ENGINE: &str = ">= 22";

    pub const NAPI_RS_CLI_DEVDEPS: &str = "^3.0.0";

    pub const NAPI_RS_CLI_CRATE: &str = "^3.7.3";

    pub const TYPESCRIPT: &str = "^6.0.3";

    pub const VITEST: &str = "^4.1.10";

    pub const TYPES_NODE: &str = "^26.0.0";

    pub const ROLLUP: &str = "^4.53.3";

    pub const VITE_PLUGIN_TOP_LEVEL_AWAIT: &str = "^1.4.0";

    pub const VITE_PLUGIN_WASM: &str = "^3.4.0";
}

pub mod cargo {
    pub const NAPI: &str = "3";

    pub const NAPI_DERIVE: &str = "3";

    pub const NAPI_BUILD: &str = "2";

    pub const PYO3: &str = "0.29";

    pub const PYO3_ASYNC_RUNTIMES: &str = "0.29";

    pub const MAGNUS: &str = "0.8";

    pub const EXT_PHP_RS: &str = "0.15.15";

    pub const JS_SYS: &str = "0.3";

    pub const WASM_BINDGEN: &str = "0.2";

    pub const WASM_BINDGEN_FUTURES: &str = "0.4";

    pub const FUTURES: &str = "0.3";

    pub const FUTURES_UTIL: &str = "0.3";

    pub const SERDE_WASM_BINDGEN: &str = "0.6";

    pub const CBINDGEN: &str = "0.29";

    pub const TEMPFILE: &str = "3";

    pub const RUSTLER: &str = "0.38";

    pub const ASYNC_TRAIT: &str = "0.1";

    pub const EXTENDR_API: &str = "0.9";

    pub const AXUM: &str = "0.8";

    pub const TOKIO_STREAM: &str = "0.1";

    pub const WALKDIR: &str = "2";

    pub const TOWER_HTTP: &str = "0.6";

    pub const FLUTTER_RUST_BRIDGE: &str = "2.12.0";

    pub const FLUTTER_RUST_BRIDGE_CODEGEN: &str = "2.12.0";

    pub const SWIFT_BRIDGE: &str = "0.1.59";

    pub const SWIFT_BRIDGE_BUILD: &str = "0.1.59";
}

pub mod pypi {
    pub const MATURIN_BUILD_REQUIRES: &str = "maturin>=1.0,<2.0";

    pub const RUFF: &str = ">=0.14.8";

    // Replaces mypy: pyrefly is a fast single-binary Rust type-checker, run as a
    pub const PYREFLY: &str = ">=1.1.1";
}

pub mod gem {
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

    pub const PHPUNIT: &str = "^13.1";

    pub const GUZZLE: &str = "^7.0";
}

pub mod maven {
    pub const JUNIT: &str = "6.1.0";

    pub const MAVEN_COMPILER_PLUGIN: &str = "3.15.0";

    pub const MAVEN_SUREFIRE_PLUGIN: &str = "3.5.5";

    pub const MAVEN_SUREFIRE_PLUGIN_E2E: &str = "3.5.2";

    pub const MAVEN_CHECKSTYLE_PLUGIN: &str = "3.6.0";

    pub const MAVEN_PMD_PLUGIN: &str = "3.28.0";

    pub const MAVEN_SOURCE_PLUGIN: &str = "3.4.0";

    pub const MAVEN_JAVADOC_PLUGIN: &str = "3.12.0";

    pub const MAVEN_GPG_PLUGIN: &str = "3.2.8";

    pub const MAVEN_CLEAN_PLUGIN: &str = "3.5.0";

    pub const MAVEN_RESOURCES_PLUGIN: &str = "3.5.0";

    pub const MAVEN_JAR_PLUGIN: &str = "3.5.0";

    pub const MAVEN_INSTALL_PLUGIN: &str = "3.1.4";

    pub const MAVEN_DEPLOY_PLUGIN: &str = "3.1.4";

    pub const MAVEN_SITE_PLUGIN: &str = "4.0.0-M16";

    pub const CENTRAL_PUBLISHING_PLUGIN: &str = "0.10.0";

    pub const VERSIONS_MAVEN_PLUGIN: &str = "2.21.0";

    pub const MAVEN_ENFORCER_PLUGIN: &str = "3.6.3";

    pub const JACOCO_MAVEN_PLUGIN: &str = "0.8.14";

    pub const CHECKSTYLE: &str = "13.4.2";

    pub const PMD: &str = "7.17.0";

    pub const JSPECIFY: &str = "1.0.0";

    pub const JACKSON: &str = "2.19.0";

    pub const JACKSON_ANNOTATIONS: &str = "2.19.0";

    pub const JACKSON_E2E: &str = "2.19.0";

    pub const ASSERTJ: &str = "4.0.0-M1";

    pub const BUILD_HELPER_MAVEN_PLUGIN: &str = "3.6.1";

    pub const KOTLIN_JVM_PLUGIN: &str = "2.4.0";

    pub const ANDROID_GRADLE_PLUGIN: &str = "9.2.1";

    pub const KTLINT_GRADLE_PLUGIN: &str = "14.2.0";

    pub const GRADLE_VERSIONS_PLUGIN: &str = "0.54.0";

    pub const KTLINT: &str = "1.8.0";

    pub const KOTLINX_COROUTINES_CORE: &str = "1.11.0";

    pub const JNA: &str = "5.18.1";

    pub const JUNIT_LEGACY: &str = "4.13.2";

    pub const ANDROIDX_TEST_EXT_JUNIT: &str = "1.3.0";

    pub const ANDROIDX_TEST_ESPRESSO_CORE: &str = "3.7.0";

    pub const VANNIKTECH_MAVEN_PUBLISH: &str = "0.37.0";
}

pub mod nuget {
    pub const MICROSOFT_NET_TEST_SDK: &str = "18.5.1";

    pub const XUNIT: &str = "2.9.3";

    pub const XUNIT_RUNNER_VISUALSTUDIO: &str = "3.1.5";
}

pub mod hex {
    pub const RUSTLER: &str = "~> 0.37";

    pub const RUSTLER_PRECOMPILED: &str = "~> 0.9";

    pub const CREDO: &str = "~> 1.7";

    pub const EX_DOC: &str = "~> 0.40";

    pub const FINCH: &str = "~> 0.18";

    pub const REQ: &str = "~> 0.5";

    pub const JASON: &str = "~> 1.4";

    pub const GLEAM_STDLIB_VERSION_RANGE: &str = ">= 0.34.0 and < 2.0.0";

    pub const GLEEUNIT_VERSION_RANGE: &str = ">= 1.0.0 and < 2.0.0";

    pub const GLEAM_HTTPC_VERSION_RANGE: &str = ">= 4.0.0 and < 6.0.0";

    pub const ENVOY_VERSION_RANGE: &str = ">= 1.0.0 and < 2.0.0";
}

/// pub.dev (Dart) ecosystem.
pub mod pub_dev {
    pub const TEST_PACKAGE: &str = "^1.25.0";

    pub const LINTS: &str = "^6.1.0";

    pub const FFI_PACKAGE: &str = "^2.2.0";

    pub const HTTP_PACKAGE: &str = "^1.2.0";

    pub const FREEZED_ANNOTATION: &str = "^3.1.0";

    pub const JSON_ANNOTATION: &str = "^4.11.0";

    pub const FREEZED: &str = "^3.2.5";

    pub const BUILD_RUNNER: &str = "^2.15.0";

    pub const JSON_SERIALIZABLE: &str = "^6.13.2";

    pub const NATIVE_ASSETS_CLI: &str = "^0.13.0";
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
    pub const REXTENDR: &str = "0.4.2";
}

pub mod precommit {
    pub const ALEF_REV: &str = "v0.38.1";

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
