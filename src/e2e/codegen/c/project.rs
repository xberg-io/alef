//! C e2e project bootstrap file rendering.

use crate::core::hash::{self, CommentStyle};
use std::fmt::Write as FmtWrite;

pub(super) fn render_makefile(
    categories: &[String],
    header_name: &str,
    ffi_crate_path: &str,
    lib_name: &str,
    needs_mock_server: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "CC = gcc");
    let _ = writeln!(out, "FFI_DIR = ffi");
    let _ = writeln!(out);
    let _ = writeln!(out, ".DEFAULT_GOAL := all");
    let _ = writeln!(out);

    // Rust's cdylib output normalizes hyphens to underscores in the filename
    // (e.g. crate "example-ffi" -> "libexample_ffi.dylib").
    // The -l linker flag must therefore use the underscore form, while the
    // pkg-config package name retains the original form (as declared in the .pc file).
    let link_lib_name = lib_name.replace('-', "_");

    // Always invoke download_ffi.sh as a prerequisite of every build. The
    // script is idempotent: it checks a per-version marker file
    // (ffi/.alef-ffi-version) and skips the network round-trip when the
    // on-disk artifacts already match the pinned VERSION. This avoids the
    // stale-header trap where a prior rc left ffi/include/<header>.h on disk
    // and the Makefile previously short-circuited the download, causing the
    // test_app to compile against headers missing the new rc's symbols.
    let _ = writeln!(out, ".PHONY: download_ffi");
    let _ = writeln!(out, "download_ffi:");
    let _ = writeln!(out, "\tbash download_ffi.sh");
    let _ = writeln!(out);

    // Dynamically select FFI library location using shell tests (evaluated at compilation time for each command)
    // Priority: downloaded ffi/ > in-tree > pkg-config
    let _ = writeln!(
        out,
        "FFI_INCLUDE = $(if $(wildcard $(FFI_DIR)/include/{header_name}),$(FFI_DIR)/include,$(if $(wildcard {ffi_crate_path}/include/{header_name}),{ffi_crate_path}/include))"
    );
    let _ = writeln!(
        out,
        "FFI_LIB_DIR = $(if $(wildcard $(FFI_DIR)/lib),$(FFI_DIR)/lib,$(if $(wildcard ../../target/release),../../target/release))"
    );
    let _ = writeln!(out);
    // Detect if we're linking dynamic (.so/.dylib)
    let _ = writeln!(
        out,
        "IS_DYNAMIC = $(if $(or $(wildcard $(FFI_LIB_DIR)/lib{link_lib_name}.dylib),$(wildcard $(FFI_LIB_DIR)/lib{link_lib_name}.so)),1,)"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "ifneq ($(FFI_INCLUDE),)");
    let _ = writeln!(out, "    CFLAGS = -Wall -Wextra -I. -I$(FFI_INCLUDE)");
    // Use ifeq for cleaner linker flag logic (avoids comma confusion in nested $(if ...))
    let _ = writeln!(out, "    ifneq ($(IS_DYNAMIC),)");
    let _ = writeln!(out, "        # Link dynamically with rpath");
    let _ = writeln!(
        out,
        "        LDFLAGS = -L$(FFI_LIB_DIR) -Wl,-rpath,$(FFI_LIB_DIR) -l{link_lib_name}"
    );
    let _ = writeln!(out, "    else");
    let _ = writeln!(out, "        # Link statically (fallback)");
    let _ = writeln!(
        out,
        "        LDFLAGS = -L$(FFI_LIB_DIR) $(FFI_LIB_DIR)/lib{link_lib_name}.a"
    );
    let _ = writeln!(out, "    endif");
    let _ = writeln!(out, "else");
    let _ = writeln!(
        out,
        "    CFLAGS = -Wall -Wextra -I. $(shell pkg-config --cflags {lib_name} 2>/dev/null)"
    );
    let _ = writeln!(out, "    LDFLAGS = $(shell pkg-config --libs {lib_name} 2>/dev/null)");
    let _ = writeln!(out, "endif");
    let _ = writeln!(out);

    let src_files: Vec<String> = categories.iter().map(|c| format!("test_{c}.c")).collect();
    let srcs = src_files.join(" ");

    let _ = writeln!(out, "SRCS = main.c {srcs}");
    let _ = writeln!(out, "TARGET = run_tests");
    let _ = writeln!(out);
    let _ = writeln!(out, ".PHONY: all build clean test");
    let _ = writeln!(out);
    // Two-phase build via recursive make: phase 1 (`all`) ensures the FFI
    // tarball matching the pinned VERSION is on disk, phase 2 (`build`)
    // re-enters make so `$(wildcard ...)` re-evaluates FFI_INCLUDE/FFI_LIB_DIR
    // against the freshly downloaded files. Without recursion, wildcard would
    // bind at parse time to whatever stale state existed before the download.
    let _ = writeln!(out, "all: download_ffi");
    let _ = writeln!(out, "\t$(MAKE) build");
    let _ = writeln!(out);
    let _ = writeln!(out, "build: $(TARGET)");
    let _ = writeln!(out);
    let _ = writeln!(out, "$(TARGET): $(SRCS)");
    let _ = writeln!(out, "\t$(CC) $(CFLAGS) -o $@ $(SRCS) $(LDFLAGS)");
    let _ = writeln!(out);

    if !needs_mock_server {
        // No fixtures require an HTTP mock backend; run the test binary directly.
        let _ = writeln!(out, ".PHONY: all build clean test smoke download_ffi");
        let _ = writeln!(out);
        // test/smoke depend on `all` (not $(TARGET)) so the unconditional
        // download_ffi prerequisite runs before the compile step.
        let _ = writeln!(out, "test: all");
        let _ = writeln!(out, "\t./$(TARGET)");
        let _ = writeln!(out);
        let _ = writeln!(out, "smoke: all");
        let _ = writeln!(out, "\t./$(TARGET) --smoke");
        let _ = writeln!(out);
        let _ = writeln!(out, "clean:");
        let _ = writeln!(out, "\trm -f $(TARGET)");
        return out;
    }

    // The mock-server orchestration is parameterized via a `define`/`endef` macro
    // that encapsulates build, spawn, env setup, and cleanup logic. Both `smoke`
    // and `test` targets invoke this macro with different TEST_CMD variables.
    //
    // The mock-server emits MOCK_SERVERS={...json...} mapping fixture IDs to
    // their per-fixture listener URLs. We parse this with python3 and export
    // MOCK_SERVER_<UPPER_ID> env vars so the test binary can look them up.
    let _ = writeln!(out, "MOCK_SERVER_BIN ?= ../rust/target/release/mock-server");
    let _ = writeln!(out, "MOCK_SERVER_MANIFEST ?= ../rust/Cargo.toml");
    let _ = writeln!(out, "FIXTURES_DIR ?= ../../fixtures");
    let _ = writeln!(out);
    let _ = writeln!(out, ".PHONY: all build clean test smoke download_ffi");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "# Mock-server orchestration macro: build, spawn, setup env, run $(TEST_CMD), cleanup"
    );
    let _ = writeln!(out, "define run_with_mock_server");
    let _ = writeln!(out, "\t@if [ -n \"$$MOCK_SERVER_URL\" ]; then \\");
    let _ = writeln!(out, "\t\tif [ -n \"$$MOCK_SERVERS\" ]; then \\");
    let _ = writeln!(
        out,
        "\t\t\teval $$(python3 -c \"import json,os; d=json.loads(os.environ.get('MOCK_SERVERS','{{}}')); print(' '.join('export MOCK_SERVER_'+k.upper()+'='+v for k,v in d.items()))\"); \\"
    );
    let _ = writeln!(out, "\t\tfi; \\");
    let _ = writeln!(out, "\t\t$(TEST_CMD); \\");
    let _ = writeln!(out, "\telse \\");
    let _ = writeln!(out, "\t\tif [ ! -x \"$(MOCK_SERVER_BIN)\" ]; then \\");
    let _ = writeln!(
        out,
        "\t\t\techo \"Building mock-server from $(MOCK_SERVER_MANIFEST)...\"; \\"
    );
    let _ = writeln!(
        out,
        "\t\t\tcargo build --release --manifest-path \"$(MOCK_SERVER_MANIFEST)\" --bin mock-server || exit 1; \\"
    );
    let _ = writeln!(out, "\t\tfi; \\");
    let _ = writeln!(out, "\t\trm -f mock_server.stdout mock_server.stdin; \\");
    let _ = writeln!(out, "\t\tmkfifo mock_server.stdin; \\");
    let _ = writeln!(
        out,
        "\t\t\"$(MOCK_SERVER_BIN)\" \"$(FIXTURES_DIR)\" <mock_server.stdin >mock_server.stdout 2>&1 & \\"
    );
    let _ = writeln!(out, "\t\tMOCK_PID=$$!; \\");
    let _ = writeln!(out, "\t\texec 9>mock_server.stdin; \\");
    let _ = writeln!(out, "\t\tMOCK_URL=\"\"; MOCK_SERVERS_JSON=\"\"; \\");
    let _ = writeln!(out, "\t\tfor _ in $$(seq 1 100); do \\");
    let _ = writeln!(out, "\t\t\tif [ -s mock_server.stdout ]; then \\");
    let _ = writeln!(
        out,
        "\t\t\t\tMOCK_URL=$$(grep -o 'MOCK_SERVER_URL=[^ ]*' mock_server.stdout | head -1 | cut -d= -f2); \\"
    );
    let _ = writeln!(out, "\t\t\t\tif [ -n \"$$MOCK_URL\" ]; then break; fi; \\");
    let _ = writeln!(out, "\t\t\tfi; \\");
    let _ = writeln!(out, "\t\t\tsleep 0.05; \\");
    let _ = writeln!(out, "\t\tdone; \\");
    let _ = writeln!(
        out,
        "\t\tMOCK_SERVERS_JSON=$$(grep -o 'MOCK_SERVERS={{.*}}' mock_server.stdout | head -1 | cut -d= -f2-); \\"
    );
    let _ = writeln!(
        out,
        "\t\tif [ -z \"$$MOCK_URL\" ]; then echo 'failed to start mock-server' >&2; cat mock_server.stdout >&2; kill $$MOCK_PID 2>/dev/null || true; exit 1; fi; \\"
    );
    let _ = writeln!(
        out,
        "\t\tif [ -n \"$$MOCK_SERVERS_JSON\" ] && command -v python3 >/dev/null 2>&1; then \\"
    );
    let _ = writeln!(
        out,
        "\t\t\teval $$(python3 -c \"import json,sys; d=json.loads(sys.argv[1]); print(' '.join('export MOCK_SERVER_{{}}={{}}'.format(k.upper(),v) for k,v in d.items()))\" \"$$MOCK_SERVERS_JSON\"); \\"
    );
    let _ = writeln!(out, "\t\tfi; \\");
    let _ = writeln!(out, "\t\tMOCK_SERVER_URL=\"$$MOCK_URL\" $(TEST_CMD); STATUS=$$?; \\");
    let _ = writeln!(out, "\t\texec 9>&-; \\");
    let _ = writeln!(out, "\t\tkill $$MOCK_PID 2>/dev/null || true; \\");
    let _ = writeln!(out, "\t\trm -f mock_server.stdout mock_server.stdin; \\");
    let _ = writeln!(out, "\t\texit $$STATUS; \\");
    let _ = writeln!(out, "\tfi");
    let _ = writeln!(out, "endef");
    let _ = writeln!(out);
    // test/smoke depend on `all` (not $(TARGET)) so the unconditional
    // download_ffi prerequisite runs before the compile step.
    let _ = writeln!(out, "test: all");
    let _ = writeln!(out, "\t@TEST_CMD='./$(TARGET)' $(MAKE) -s run_with_mock_server");
    let _ = writeln!(out);
    let _ = writeln!(out, "smoke: all");
    let _ = writeln!(out, "\t@TEST_CMD='./$(TARGET) --smoke' $(MAKE) -s run_with_mock_server");
    let _ = writeln!(out);
    let _ = writeln!(out, "run_with_mock_server:");
    let _ = writeln!(out, "\t$(run_with_mock_server)");
    let _ = writeln!(out);
    let _ = writeln!(out, "clean:");
    let _ = writeln!(out, "\trm -f $(TARGET) mock_server.stdout mock_server.stdin");
    out
}

/// Render `.gitignore` for the `e2e/c/` directory.
///
/// `run_tests` is the linked test binary produced by `make`. When a
/// developer runs the suite locally on macOS, the resulting Mach-O binary
/// must not be committed — Linux CI will reject it with `Exec format error`.
/// `*.o` covers compiled object files. `mock_server.stdout`/`.stdin` are the
/// named-pipe artifacts created by fixtures that mock HTTP traffic.
pub(super) fn render_gitignore() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "run_tests");
    let _ = writeln!(out, "*.o");
    let _ = writeln!(out, "mock_server.stdout");
    let _ = writeln!(out, "mock_server.stdin");
    out
}

pub(super) fn render_download_script(github_repo: &str, version: &str, ffi_pkg_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
    let _ = writeln!(out, "REPO_URL=\"{github_repo}\"");
    let _ = writeln!(out, "VERSION=\"{version}\"");
    let _ = writeln!(out, "FFI_PKG_NAME=\"{ffi_pkg_name}\"");
    let _ = writeln!(out, "FFI_DIR=\"ffi\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Detect OS and architecture.");
    let _ = writeln!(out, "OS=\"$(uname -s | tr '[:upper:]' '[:lower:]')\"");
    let _ = writeln!(out, "ARCH=\"$(uname -m)\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "case \"$ARCH\" in");
    let _ = writeln!(out, "x86_64 | amd64) ARCH=\"x86_64\" ;;");
    let _ = writeln!(out, "arm64 | aarch64) ARCH=\"arm64\" ;;");
    let _ = writeln!(out, "*)");
    let _ = writeln!(out, "  echo \"Unsupported architecture: $ARCH\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    // alef publish package emits tarballs named ${FFI_PKG_NAME}-v${VERSION}-${TRIPLE}.tar.gz
    // (rust target triple, not the short platform label). Match that naming so the asset URL
    // resolves; otherwise the GitHub release returns 404 and the test_app falls back to a
    // stale cached header missing any newly-added FFI exports.
    let _ = writeln!(out, "case \"$OS\" in");
    let _ = writeln!(out, "linux)");
    let _ = writeln!(out, "  case \"$ARCH\" in");
    let _ = writeln!(out, "  x86_64) TRIPLE=\"x86_64-unknown-linux-gnu\" ;;");
    let _ = writeln!(out, "  arm64) TRIPLE=\"aarch64-unknown-linux-gnu\" ;;");
    let _ = writeln!(out, "  esac");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "darwin)");
    let _ = writeln!(out, "  case \"$ARCH\" in");
    let _ = writeln!(out, "  x86_64) TRIPLE=\"x86_64-apple-darwin\" ;;");
    let _ = writeln!(out, "  arm64) TRIPLE=\"aarch64-apple-darwin\" ;;");
    let _ = writeln!(out, "  esac");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "mingw* | msys* | cygwin* | windows)");
    let _ = writeln!(out, "  case \"$ARCH\" in");
    let _ = writeln!(out, "  x86_64) TRIPLE=\"x86_64-pc-windows-msvc\" ;;");
    let _ = writeln!(out, "  arm64) TRIPLE=\"aarch64-pc-windows-msvc\" ;;");
    let _ = writeln!(out, "  esac");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "*)");
    let _ = writeln!(out, "  echo \"Unsupported OS: $OS\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "if [ -z \"${{TRIPLE:-}}\" ]; then");
    let _ = writeln!(out, "  echo \"Unsupported platform: $OS/$ARCH\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    let _ = writeln!(out, "ASSET_STEM=\"${{FFI_PKG_NAME}}-v${{VERSION}}-${{TRIPLE}}\"");
    let _ = writeln!(out, "ARCHIVE=\"${{ASSET_STEM}}.tar.gz\"");
    let _ = writeln!(
        out,
        "URL=\"${{REPO_URL}}/releases/download/v${{VERSION}}/${{ARCHIVE}}\""
    );
    let _ = writeln!(out);
    // Idempotency marker: the Makefile invokes this script on every build to
    // guarantee freshness against the pinned VERSION. The marker file lets us
    // skip the network round-trip when the on-disk artifacts already match.
    // Without the marker, a prior rc's header/lib would linger across rc
    // bumps and the test_app would compile against stale symbols.
    let _ = writeln!(out, "MARKER=\"$FFI_DIR/.alef-ffi-version\"");
    let _ = writeln!(out, "EXPECTED=\"${{ASSET_STEM}}\"");
    let _ = writeln!(out);
    // Override: if TSLP_FFI_LOCAL_DIR is set and contains the FFI structure,
    // use it instead of downloading from GitHub. This allows CI to reuse
    // locally-built FFI artifacts (e.g. from a prior build-ffi job) and
    // avoids the 404 race when the GitHub release hasn't been created yet.
    let _ = writeln!(out, "if [ -n \"${{TSLP_FFI_LOCAL_DIR:-}}\" ] && [ -d \"${{TSLP_FFI_LOCAL_DIR}}/include\" ] && [ -d \"${{TSLP_FFI_LOCAL_DIR}}/lib\" ]; then");
    let _ = writeln!(out, "  echo \"Using FFI from TSLP_FFI_LOCAL_DIR=${{TSLP_FFI_LOCAL_DIR}}\"");
    let _ = writeln!(out, "  rm -rf \"${{FFI_DIR:?}}\"/include \"${{FFI_DIR:?}}\"/lib");
    let _ = writeln!(out, "  mkdir -p \"$FFI_DIR\"");
    let _ = writeln!(out, "  cp -R \"${{TSLP_FFI_LOCAL_DIR}}/include\" \"$FFI_DIR/include\"");
    let _ = writeln!(out, "  cp -R \"${{TSLP_FFI_LOCAL_DIR}}/lib\" \"$FFI_DIR/lib\"");
    let _ = writeln!(out, "  echo \"$EXPECTED\" > \"$MARKER\"");
    let _ = writeln!(out, "  echo \"FFI library staged into $FFI_DIR/ from local override.\"");
    let _ = writeln!(out, "  exit 0");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "if [ -f \"$MARKER\" ] && [ \"$(cat \"$MARKER\")\" = \"$EXPECTED\" ]; then"
    );
    let _ = writeln!(
        out,
        "  echo \"FFI v${{VERSION}} (${{TRIPLE}}) already present; skipping download.\""
    );
    let _ = writeln!(out, "  exit 0");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out);
    let _ = writeln!(out, "echo \"Downloading ${{ARCHIVE}} from v${{VERSION}}...\"");
    let _ = writeln!(out, "rm -rf \"${{FFI_DIR:?}}\"/include \"${{FFI_DIR:?}}\"/lib");
    let _ = writeln!(out, "mkdir -p \"$FFI_DIR\"");
    let _ = writeln!(out, "curl -fSL \"$URL\" | tar xz -C \"$FFI_DIR\"");
    let _ = writeln!(out, "# Flatten the platform subdirectory into the ffi/ root");
    let _ = writeln!(out, "EXTRACTED_DIR=\"$FFI_DIR/$ASSET_STEM\"");
    let _ = writeln!(out, "if [ -d \"$EXTRACTED_DIR\" ]; then");
    let _ = writeln!(out, "  rm -rf \"${{FFI_DIR:?}}\"/include \"${{FFI_DIR:?}}\"/lib");
    let _ = writeln!(
        out,
        "  mv \"$EXTRACTED_DIR\"/include \"$EXTRACTED_DIR\"/lib \"$FFI_DIR\"/"
    );
    let _ = writeln!(out, "  rm -rf \"${{FFI_DIR:?}}\"/${{FFI_PKG_NAME}}-*");
    let _ = writeln!(out, "fi");
    // Record the version stamp so subsequent invocations of this script can
    // short-circuit. The Makefile calls this script unconditionally on every
    // build; the marker is what makes the call cheap.
    let _ = writeln!(out, "echo \"$EXPECTED\" > \"$MARKER\"");
    let _ = writeln!(out, "echo \"FFI library extracted to $FFI_DIR/\"");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_makefile(needs_mock_server: bool) -> String {
        render_makefile(
            &["smoke".to_string()],
            "example_pack.h",
            "../../crates/example-pack-core-ffi",
            "example-pack-core-ffi",
            needs_mock_server,
        )
    }

    /// Regression: tslp v1.9.0-rc.48 test_apps run failed because the old
    /// Makefile shape gated download_ffi.sh on a wildcard check that resolved
    /// to a stale on-disk header from a prior rc. The unconditional
    /// `download_ffi` prerequisite on `all` is what guarantees freshness.
    #[test]
    fn makefile_always_invokes_download_ffi() {
        let makefile = sample_makefile(false);
        assert!(
            makefile.contains(".PHONY: download_ffi"),
            "download_ffi must be declared phony; got: {makefile}"
        );
        assert!(
            makefile.contains("download_ffi:\n\tbash download_ffi.sh"),
            "phony download_ffi target must invoke the script; got: {makefile}"
        );
        assert!(
            makefile.contains("all: download_ffi"),
            "`all` must depend on the unconditional download_ffi prerequisite; got: {makefile}"
        );
    }

    /// Regression: tslp v1.9.0-rc.48 test_apps run failed because the
    /// `HEADER_PATH`/`LIB_PATH` wildcard short-circuit caused make to skip the
    /// `$(FFI_DIR)/include/<header>` build dependency when a stale header
    /// from a prior rc was on disk. The Makefile must no longer contain that
    /// conditional-skip pattern, and the file-target rule on the header is
    /// dropped in favor of the phony `download_ffi` target.
    #[test]
    fn makefile_does_not_short_circuit_download_on_stale_header() {
        let makefile = sample_makefile(false);
        assert!(
            !makefile.contains("HEADER_PATH := $(if $(wildcard"),
            "stale-header wildcard short-circuit (HEADER_PATH := ...) must be removed; got: {makefile}"
        );
        assert!(
            !makefile.contains("LIB_PATH := $(or $(wildcard"),
            "stale-header wildcard short-circuit (LIB_PATH := ...) must be removed; got: {makefile}"
        );
        assert!(
            !makefile.contains("$(if $(and $(HEADER_PATH),$(LIB_PATH)),,"),
            "conditional skip of header dependency must be removed; got: {makefile}"
        );
        // The old file-target rule
        // `$(FFI_DIR)/include/example_pack.h: download_ffi.sh` was a no-op when the
        // file already existed. Confirm it is no longer emitted; the phony
        // target takes its place.
        assert!(
            !makefile.contains("$(FFI_DIR)/include/example_pack.h: download_ffi.sh"),
            "file-target rule on header must be replaced by phony download_ffi; got: {makefile}"
        );
    }

    /// `test` and `smoke` invoke the compiled binary; both must route through
    /// `all` (not `$(TARGET)`) so the unconditional `download_ffi` prerequisite
    /// is honored before linking.
    #[test]
    fn makefile_test_and_smoke_route_through_all() {
        for needs_mock_server in [false, true] {
            let makefile = sample_makefile(needs_mock_server);
            assert!(
                makefile.contains("test: all"),
                "test target must depend on `all` (mock={needs_mock_server}); got: {makefile}"
            );
            assert!(
                makefile.contains("smoke: all"),
                "smoke target must depend on `all` (mock={needs_mock_server}); got: {makefile}"
            );
        }
    }

    /// `download_ffi.sh` is invoked on every build; it must short-circuit when
    /// the on-disk artifacts already match the pinned VERSION, otherwise every
    /// `make` invocation would re-download. The marker file is what gates the
    /// short-circuit.
    #[test]
    fn download_script_is_idempotent_via_version_marker() {
        let script = render_download_script(
            "https://github.com/fixture-dev/example-language-pack",
            "1.9.0-rc.48",
            "example-pack-core-ffi",
        );
        assert!(
            script.contains("MARKER=\"$FFI_DIR/.alef-ffi-version\""),
            "script must declare a version marker; got: {script}"
        );
        assert!(
            script.contains("EXPECTED=\"${ASSET_STEM}\""),
            "marker must encode the asset stem (version + triple); got: {script}"
        );
        assert!(
            script.contains("[ \"$(cat \"$MARKER\")\" = \"$EXPECTED\" ]"),
            "script must compare marker contents to the expected stem; got: {script}"
        );
        assert!(
            script.contains("echo \"$EXPECTED\" > \"$MARKER\""),
            "script must write the marker after a successful download; got: {script}"
        );
    }
}
