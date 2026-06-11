use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

pub(crate) fn scaffold_go(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let go_module = config.go_module();
    let version = &api.version;
    let _ = version; // go.mod doesn't embed the package version
    let package_dir = config.package_dir(Language::Go);

    let content = format!("module {module}\n\ngo 1.26\n", module = go_module,);

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from(format!("{package_dir}/go.mod")),
            content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{package_dir}/.golangci.yml")),
            content: r#"version: "2"

run:
  timeout: 5m
  issues-exit-code: 1
  tests: true
  concurrency: 4
  modules-download-mode: readonly
  allow-serial-runners: false
  allow-parallel-runners: true

linters:
  default: none
  enable:
    - errcheck
    - govet
    - ineffassign
    - staticcheck
    - unused
    - revive
    - gocyclo
    - goconst
    - gocritic
    - gosec
    - misspell
    - nakedret
  settings:
    errcheck:
      check-type-assertions: true
      check-blank: true
      exclude-functions:
        - (net/http.ResponseWriter).Write
        - (io.Closer).Close
        - fmt.Fprintf
        - fmt.Printf
        - fmt.Println
        - os.Setenv
        - os.Unsetenv
    goconst:
      min-len: 3
      min-occurrences: 3
    gocyclo:
      min-complexity: 50
    govet:
      enable-all: true
      disable:
        - shadow
    gocritic:
      disabled-checks:
        - dupSubExpr
    misspell:
      locale: US
    nakedret:
      max-func-lines: 30
    revive:
      confidence: 0.8
      severity: warning
      enable-all-rules: false
      rules:
        - name: blank-imports
        - name: context-keys-type
        - name: time-naming
        - name: var-declaration
        - name: unexported-return
        - name: errorf
        - name: context-as-argument
        - name: dot-imports
        - name: error-return
        - name: error-strings
        - name: error-naming
        - name: if-return
        - name: increment-decrement
        - name: var-naming
        - name: range
        - name: receiver-naming
        - name: indent-error-flow
        - name: exported
          disabled: true
        - name: package-comments
          disabled: true
  exclusions:
    generated: lax
    rules:
      - linters:
          - goconst
        path: _test\.go
      - linters:
          - gocyclo
        path: _test\.go
      - linters:
          - gosec
        path: _test\.go
      - linters:
          - revive
        path: _test\.go
        text: "context-as-argument"
      - linters:
          - govet
        text: "fieldalignment:"
      - linters:
          - govet
        text: "unsafeptr:"
    paths:
      - vendor
      - build
      - third_party$

issues:
  max-issues-per-linter: 0
  max-same-issues: 0
  uniq-by-line: true
  new: false

formatters:
  exclusions:
    generated: lax
    paths:
      - third_party$
"#
            .to_string(),
            generated_header: false,
        },
    ];

    // Create .lib/.gitkeep to ensure the .lib directory exists in the module.
    // This directory will be referenced by go:embed directives in embed_ffi.go.
    // Pre-built FFI libraries for different platforms should be placed here.
    files.push(GeneratedFile {
        path: PathBuf::from(format!("{package_dir}/.lib/.gitkeep")),
        content: String::new(),
        generated_header: false,
    });

    Ok(files)
}
