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

    // Ensure FFI artifacts are downloaded if not present locally
    let _ = writeln!(out, "$(FFI_DIR)/include/{header_name}: download_ffi.sh");
    let _ = writeln!(out, "\tbash download_ffi.sh");
    let _ = writeln!(out);

    // Resolve the header to whichever location holds it (downloaded ffi/ or
    // in-tree FFI crate). When set, the build target omits the
    // download-triggering prerequisite below — this avoids 404s on release
    // commits where download_ffi.sh's pinned VERSION points at assets not
    // yet published, but CI has already staged a locally-built header.
    let _ = writeln!(
        out,
        "HEADER_PATH := $(if $(wildcard $(FFI_DIR)/include/{header_name}),$(FFI_DIR)/include/{header_name},$(if $(wildcard {ffi_crate_path}/include/{header_name}),{ffi_crate_path}/include/{header_name}))"
    );
    // Resolve the shared lib to whichever location holds it (downloaded ffi/
    // or a local cargo build at ../../target/release). Symmetric to
    // HEADER_PATH: the build target only skips the download prerequisite when
    // BOTH header and lib are present locally. Without the lib check, a tree
    // that has the in-tree header but no local cargo build would skip the
    // download and then fail at link time with "library not found".
    // Prefer dynamic library (.dylib on macOS, .so on Linux) for system-dependent symbols.
    let _ = writeln!(
        out,
        "LIB_PATH := $(or $(wildcard $(FFI_DIR)/lib/lib{link_lib_name}.dylib),$(wildcard $(FFI_DIR)/lib/lib{link_lib_name}.so),$(wildcard $(FFI_DIR)/lib/lib{link_lib_name}.a),$(wildcard ../../target/release/lib{link_lib_name}.dylib),$(wildcard ../../target/release/lib{link_lib_name}.so),$(wildcard ../../target/release/lib{link_lib_name}.a))"
    );
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
    let _ = writeln!(out, ".PHONY: all clean test");
    let _ = writeln!(out);
    let _ = writeln!(out, "all: $(TARGET)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "$(TARGET): $(SRCS) $(if $(and $(HEADER_PATH),$(LIB_PATH)),,$(FFI_DIR)/include/{header_name})"
    );
    let _ = writeln!(out, "\t$(CC) $(CFLAGS) -o $@ $(SRCS) $(LDFLAGS)");
    let _ = writeln!(out);

    if !needs_mock_server {
        // No fixtures require an HTTP mock backend; run the test binary directly.
        let _ = writeln!(out, ".PHONY: all clean test smoke");
        let _ = writeln!(out);
        let _ = writeln!(out, "test: $(TARGET)");
        let _ = writeln!(out, "\t./$(TARGET)");
        let _ = writeln!(out);
        let _ = writeln!(out, "smoke: $(TARGET)");
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
    let _ = writeln!(out, ".PHONY: all clean test smoke");
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
    let _ = writeln!(out, "test: $(TARGET)");
    let _ = writeln!(out, "\t@TEST_CMD='./$(TARGET)' $(MAKE) -s run_with_mock_server");
    let _ = writeln!(out);
    let _ = writeln!(out, "smoke: $(TARGET)");
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
    let _ = writeln!(out, "arm64 | aarch64) ARCH=\"aarch64\" ;;");
    let _ = writeln!(out, "*)");
    let _ = writeln!(out, "  echo \"Unsupported architecture: $ARCH\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "case \"$OS\" in");
    let _ = writeln!(out, "linux) TRIPLE=\"${{ARCH}}-unknown-linux-gnu\" ;;");
    let _ = writeln!(out, "darwin) TRIPLE=\"${{ARCH}}-apple-darwin\" ;;");
    let _ = writeln!(out, "*)");
    let _ = writeln!(out, "  echo \"Unsupported OS: $OS\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "ARCHIVE=\"${{FFI_PKG_NAME}}-v${{VERSION}}-${{TRIPLE}}.tar.gz\"");
    let _ = writeln!(
        out,
        "URL=\"${{REPO_URL}}/releases/download/v${{VERSION}}/${{ARCHIVE}}\""
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "echo \"Downloading ${{ARCHIVE}} from v${{VERSION}}...\"");
    let _ = writeln!(out, "mkdir -p \"$FFI_DIR\"");
    let _ = writeln!(out, "curl -fSL \"$URL\" | tar xz -C \"$FFI_DIR\"");
    let _ = writeln!(out, "# Flatten the versioned subdirectory into the ffi/ root");
    let _ = writeln!(
        out,
        "EXTRACTED_DIR=\"$FFI_DIR\"/${{FFI_PKG_NAME}}-v${{VERSION}}-${{TRIPLE}}"
    );
    let _ = writeln!(out, "if [ -d \"$EXTRACTED_DIR\" ]; then");
    let _ = writeln!(out, "  rm -rf \"${{FFI_DIR:?}}\"/include \"${{FFI_DIR:?}}\"/lib");
    let _ = writeln!(
        out,
        "  mv \"$EXTRACTED_DIR\"/include \"$EXTRACTED_DIR\"/lib \"$FFI_DIR\"/"
    );
    let _ = writeln!(out, "  rm -rf \"${{FFI_DIR:?}}\"/${{FFI_PKG_NAME}}-*");
    let _ = writeln!(out, "fi");
    let _ = writeln!(out, "echo \"FFI library extracted to $FFI_DIR/\"");
    out
}
