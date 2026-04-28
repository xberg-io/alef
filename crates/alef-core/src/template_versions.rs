//! Centralized third-party dependency version strings for code generation.
//!
//! This module consolidates all hardcoded version strings used in scaffold and e2e
//! code generation. Each const that should be auto-bumped by Renovate includes a
//! marker comment: `// renovate: datasource=... depName=...`
//!
//! When adding a new version: choose the appropriate submodule based on package ecosystem,
//! add the renovate marker (if applicable), and use the const in templates.

pub mod npm {
    // renovate: datasource=npm depName=@napi-rs/cli
    pub const NAPI_RS_CLI_DEVDEPS: &str = "^3.0.0";

    // renovate: datasource=npm depName=@napi-rs/cli
    pub const NAPI_RS_CLI_CRATE: &str = "^3.6.2";

    // renovate: datasource=npm depName=typescript
    pub const TYPESCRIPT: &str = "^6.0.3";

    // renovate: datasource=npm depName=vitest
    pub const VITEST: &str = "^4.1.5";

    // renovate: datasource=npm depName=vite-plugin-top-level-await
    pub const VITE_PLUGIN_TOP_LEVEL_AWAIT: &str = "^1.4.0";

    // renovate: datasource=npm depName=vite-plugin-wasm
    pub const VITE_PLUGIN_WASM: &str = "^3.4.0";
}

pub mod cargo {
    // napi major-only; manual bump required
    pub const NAPI: &str = "3";

    // napi-derive major-only; manual bump required
    pub const NAPI_DERIVE: &str = "3";

    // napi-build major-only; manual bump required
    pub const NAPI_BUILD: &str = "2";

    // renovate: datasource=crate depName=pyo3
    pub const PYO3: &str = "0.28";

    // renovate: datasource=crate depName=pyo3-async-runtimes
    pub const PYO3_ASYNC_RUNTIMES: &str = "0.28";

    // renovate: datasource=crate depName=magnus
    pub const MAGNUS: &str = "0.8";

    // renovate: datasource=crate depName=ext-php-rs
    pub const EXT_PHP_RS: &str = "0.15";

    // renovate: datasource=crate depName=js-sys
    pub const JS_SYS: &str = "0.3";

    // renovate: datasource=crate depName=wasm-bindgen
    pub const WASM_BINDGEN: &str = "0.2";

    // renovate: datasource=crate depName=wasm-bindgen-futures
    pub const WASM_BINDGEN_FUTURES: &str = "0.4";

    // renovate: datasource=crate depName=futures-util
    pub const FUTURES_UTIL: &str = "0.3";

    // renovate: datasource=crate depName=serde-wasm-bindgen
    pub const SERDE_WASM_BINDGEN: &str = "0.6";

    // renovate: datasource=crate depName=cbindgen
    pub const CBINDGEN: &str = "0.29";

    // tempfile major-only; manual bump required
    pub const TEMPFILE: &str = "3";

    // renovate: datasource=crate depName=rustler
    pub const RUSTLER: &str = "0.37";

    // renovate: datasource=crate depName=async-trait
    pub const ASYNC_TRAIT: &str = "0.1";

    // renovate: datasource=crate depName=extendr-api
    pub const EXTENDR_API: &str = "0.7";

    // renovate: datasource=crate depName=axum
    pub const AXUM: &str = "0.8";

    // renovate: datasource=crate depName=tokio-stream
    pub const TOKIO_STREAM: &str = "0.1";

    // walkdir major-only; manual bump required
    pub const WALKDIR: &str = "2";

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
    // renovate: datasource=pypi depName=maturin
    // Note: floor+ceil constraint; managed as single string, no Renovate auto-bump
    pub const MATURIN_BUILD_REQUIRES: &str = "maturin>=1.0,<2.0";

    // renovate: datasource=pypi depName=ruff
    pub const RUFF: &str = ">=0.14.8";

    // renovate: datasource=pypi depName=mypy
    pub const MYPY: &str = ">=1.19.0";
}

pub mod gem {
    // renovate: datasource=rubygems depName=rb_sys
    pub const RB_SYS: &str = "~> 0.9";

    // renovate: datasource=rubygems depName=rake-compiler
    pub const RAKE_COMPILER: &str = "~> 1.2";

    // renovate: datasource=rubygems depName=rspec
    pub const RSPEC_SCAFFOLD: &str = "~> 3.0";

    // renovate: datasource=rubygems depName=rspec
    pub const RSPEC_E2E: &str = "~> 3.13";

    // renovate: datasource=rubygems depName=rubocop
    pub const RUBOCOP_SCAFFOLD: &str = "~> 1.0";

    // renovate: datasource=rubygems depName=rubocop
    pub const RUBOCOP_E2E: &str = "~> 1.86";

    // renovate: datasource=rubygems depName=rubocop-performance
    pub const RUBOCOP_PERFORMANCE: &str = "~> 1.0";

    // renovate: datasource=rubygems depName=rubocop-rspec
    pub const RUBOCOP_RSPEC_SCAFFOLD: &str = "~> 3.0";

    // renovate: datasource=rubygems depName=rubocop-rspec
    pub const RUBOCOP_RSPEC_E2E: &str = "~> 3.9";

    // renovate: datasource=rubygems depName=steep
    pub const STEEP: &str = "~> 1.0";

    // renovate: datasource=rubygems depName=faraday
    pub const FARADAY: &str = "~> 2.0";
}

pub mod packagist {
    // renovate: datasource=packagist depName=phpstan/phpstan
    pub const PHPSTAN: &str = "^2.1";

    // renovate: datasource=packagist depName=friendsofphp/php-cs-fixer
    pub const PHP_CS_FIXER: &str = "^3.95";

    // renovate: datasource=packagist depName=phpunit/phpunit
    pub const PHPUNIT: &str = "^13.1";

    // renovate: datasource=packagist depName=guzzlehttp/guzzle
    pub const GUZZLE: &str = "^7.0";
}

pub mod maven {
    // renovate: datasource=maven depName=org.junit.jupiter:junit-jupiter
    pub const JUNIT: &str = "5.11.4";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-compiler-plugin
    pub const MAVEN_COMPILER_PLUGIN: &str = "3.15.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-surefire-plugin
    pub const MAVEN_SUREFIRE_PLUGIN: &str = "3.5.5";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-surefire-plugin
    pub const MAVEN_SUREFIRE_PLUGIN_E2E: &str = "3.5.2";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-checkstyle-plugin
    pub const MAVEN_CHECKSTYLE_PLUGIN: &str = "3.6.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-pmd-plugin
    pub const MAVEN_PMD_PLUGIN: &str = "3.28.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-source-plugin
    pub const MAVEN_SOURCE_PLUGIN: &str = "3.4.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-javadoc-plugin
    pub const MAVEN_JAVADOC_PLUGIN: &str = "3.12.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-gpg-plugin
    pub const MAVEN_GPG_PLUGIN: &str = "3.2.8";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-clean-plugin
    pub const MAVEN_CLEAN_PLUGIN: &str = "3.4.1";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-resources-plugin
    pub const MAVEN_RESOURCES_PLUGIN: &str = "3.3.1";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-jar-plugin
    pub const MAVEN_JAR_PLUGIN: &str = "3.4.2";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-install-plugin
    pub const MAVEN_INSTALL_PLUGIN: &str = "3.1.3";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-deploy-plugin
    pub const MAVEN_DEPLOY_PLUGIN: &str = "3.1.3";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-site-plugin
    pub const MAVEN_SITE_PLUGIN: &str = "4.0.0-M16";

    // renovate: datasource=maven depName=org.sonatype.central:central-publishing-maven-plugin
    pub const CENTRAL_PUBLISHING_PLUGIN: &str = "0.10.0";

    // renovate: datasource=maven depName=com.diffplug.spotless:spotless-maven-plugin
    pub const SPOTLESS_MAVEN_PLUGIN: &str = "3.4.0";

    // renovate: datasource=maven depName=org.codehaus.mojo:versions-maven-plugin
    pub const VERSIONS_MAVEN_PLUGIN: &str = "2.21.0";

    // renovate: datasource=maven depName=org.apache.maven.plugins:maven-enforcer-plugin
    pub const MAVEN_ENFORCER_PLUGIN: &str = "3.6.2";

    // renovate: datasource=maven depName=org.jacoco:jacoco-maven-plugin
    pub const JACOCO_MAVEN_PLUGIN: &str = "0.8.14";

    // renovate: datasource=maven depName=com.puppycrawl.tools:checkstyle
    pub const CHECKSTYLE: &str = "13.4.0";

    // renovate: datasource=maven depName=net.sourceforge.pmd:pmd-java
    pub const PMD: &str = "7.19.0";

    // renovate: datasource=maven depName=org.jspecify:jspecify
    pub const JSPECIFY: &str = "1.0.0";

    // renovate: datasource=maven depName=com.fasterxml.jackson.core:jackson-databind
    pub const JACKSON: &str = "2.21.2";

    // renovate: datasource=maven depName=com.fasterxml.jackson.core:jackson-databind
    pub const JACKSON_E2E: &str = "2.18.2";

    // renovate: datasource=maven depName=org.assertj:assertj-core
    pub const ASSERTJ: &str = "4.0.0-M1";

    // renovate: datasource=maven depName=org.codehaus.mojo:build-helper-maven-plugin
    pub const BUILD_HELPER_MAVEN_PLUGIN: &str = "3.6.0";

    // renovate: datasource=maven depName=org.jetbrains.kotlin:kotlin-gradle-plugin
    pub const KOTLIN_JVM_PLUGIN: &str = "2.1.10";

    // renovate: datasource=maven depName=org.jetbrains.kotlinx:kotlinx-coroutines-core
    pub const KOTLINX_COROUTINES_CORE: &str = "1.9.0";

    // renovate: datasource=maven depName=net.java.dev.jna:jna
    pub const JNA: &str = "5.14.0";

    // renovate: datasource=maven depName=junit:junit
    pub const JUNIT_LEGACY: &str = "4.13.2";
}

pub mod nuget {
    // renovate: datasource=nuget depName=Microsoft.NET.Test.Sdk
    pub const MICROSOFT_NET_TEST_SDK: &str = "17.12.0";

    // renovate: datasource=nuget depName=xunit
    pub const XUNIT: &str = "2.9.3";

    // renovate: datasource=nuget depName=xunit.runner.visualstudio
    pub const XUNIT_RUNNER_VISUALSTUDIO: &str = "2.8.2";
}

pub mod hex {
    // renovate: datasource=hex depName=rustler
    pub const RUSTLER: &str = "~> 0.37.0";

    // renovate: datasource=hex depName=rustler_precompiled
    pub const RUSTLER_PRECOMPILED: &str = "~> 0.9";

    // renovate: datasource=hex depName=credo
    pub const CREDO: &str = "~> 1.7";

    // renovate: datasource=hex depName=ex_doc
    pub const EX_DOC: &str = "~> 0.40";

    // renovate: datasource=hex depName=req
    pub const REQ: &str = "~> 0.5";

    // renovate: datasource=hex depName=jason
    pub const JASON: &str = "~> 1.4";

    // version range; manual bump required
    pub const GLEAM_STDLIB_VERSION_RANGE: &str = ">= 0.34.0 and < 2.0.0";

    // version range; manual bump required
    pub const GLEEUNIT_VERSION_RANGE: &str = ">= 1.0.0 and < 2.0.0";
}

/// pub.dev (Dart) ecosystem.
pub mod pub_dev {
    // renovate: datasource=pub depName=test
    pub const TEST_PACKAGE: &str = "^1.25.0";

    // renovate: datasource=pub depName=lints
    pub const LINTS: &str = "^5.0.0";

    // renovate: datasource=pub depName=ffi
    pub const FFI_PACKAGE: &str = "^2.1.0";
}

/// Platform / toolchain pins. None of these auto-bump; track manually.
pub mod toolchain {
    // minimum supported Zig; manual bump required
    pub const MIN_ZIG_VERSION: &str = "0.16.0";

    // version range; manual bump required
    pub const DART_SDK_CONSTRAINT: &str = ">=3.0.0 <4.0.0";

    // JVM bytecode target for Kotlin/Java scaffolds and e2e; manual bump required
    pub const JVM_TARGET: &str = "21";

    // minimum macOS deployment target for swift-bridge bindings; manual bump required
    pub const SWIFT_MIN_MACOS: &str = "13.0";

    // minimum iOS deployment target for swift-bridge bindings; manual bump required
    pub const SWIFT_MIN_IOS: &str = "16.0";
}

pub mod cran {
    // renovate: datasource=cran depName=rextendr
    pub const REXTENDR: &str = "0.4.2";
}

pub mod precommit {
    // renovate: datasource=github-tags packageName=Goldziher/gitfluff
    pub const GITFLUFF_REV: &str = "v0.7.10";

    // renovate: datasource=github-tags packageName=pre-commit/pre-commit-hooks
    pub const PRE_COMMIT_HOOKS_REV: &str = "v0.7.10";

    // renovate: datasource=github-tags packageName=tox-dev/pyproject-fmt
    pub const PYPROJECT_FMT_REV: &str = "v2.21.1";

    // renovate: datasource=github-tags packageName=DevinR528/cargo-sort
    pub const CARGO_SORT_REV: &str = "v2.1.3";

    // renovate: datasource=github-tags packageName=AndrejOrsula/pre-commit-cargo
    pub const PRE_COMMIT_CARGO_REV: &str = "0.5.0";

    // renovate: datasource=github-tags packageName=bnjbvr/cargo-machete
    pub const CARGO_MACHETE_REV: &str = "v0.7.10";

    // renovate: datasource=github-tags packageName=EmbarkStudios/cargo-deny
    pub const CARGO_DENY_REV: &str = "0.19.4";

    // renovate: datasource=github-tags packageName=rvben/rumdl-pre-commit
    pub const RUMDL_REV: &str = "v0.1.81";

    // renovate: datasource=github-tags packageName=crate-ci/typos
    pub const TYPOS_REV: &str = "v0.7.10";

    // alef rev: managed by sync-versions hook, no renovate marker
    pub const ALEF_REV: &str = "v0.11.1";
}
