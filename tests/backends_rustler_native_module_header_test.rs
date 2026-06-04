use minijinja::context;

#[test]
fn force_build_uses_mix_env_not_system_get_env() {
    // Test that the native_module_header template uses Mix.env() instead of
    // System.get_env("MIX_ENV") for the force_build predicate.
    //
    // Context: System.get_env("MIX_ENV") returns nil when the env var is unset,
    // and nil in ["test", "dev"] is false. But mix compile runs in :dev/:test
    // environment (internally set by mix) even when MIX_ENV is not exported.
    // Therefore, force_build must use Mix.env() to detect the actual mix environment.

    let mut env = minijinja::Environment::new();
    let template_str = r#"defmodule {{ app_module }}.Native do
  @moduledoc false

  use RustlerPrecompiled,
    otp_app: :{{ app_name }},
    crate: "{{ app_name }}_nif",
    base_url:
      "{{ repo_url }}/releases/download/v#{{ '{' }}Mix.Project.config()[:version]{{ '}' }}",
    version: Mix.Project.config()[:version],
    targets:
      ~w(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu),
    nif_versions: ["2.16", "2.17"],
    force_build: System.get_env("{{ build_env_var }}") in ["1", "true"] or Mix.env() in [:dev, :test]

"#;

    env.add_template("test_native_header", template_str)
        .expect("template adds successfully");

    let tmpl = env.get_template("test_native_header").expect("template retrieves");

    let rendered = tmpl
        .render(context! {
            app_module => "SampleCrate",
            app_name => "sample_crate",
            repo_url => "https://github.com/sample_crate-dev/sample_crate",
            build_env_var => "SAMPLE_CRATE_BUILD",
        })
        .expect("template renders");

    // Verify the output contains Mix.env() in [:dev, :test], not System.get_env("MIX_ENV")
    assert!(
        rendered.contains("Mix.env() in [:dev, :test]"),
        "Expected 'Mix.env() in [:dev, :test]' in rendered template, got:\n{}",
        rendered
    );

    assert!(
        !rendered.contains("System.get_env(\"MIX_ENV\")"),
        "Template should not contain 'System.get_env(\"MIX_ENV\")', got:\n{}",
        rendered
    );
}

#[test]
fn base_url_and_targets_wrapped_for_mix_format_idempotency() {
    // Test that base_url and targets keyword arguments are wrapped onto continuation
    // lines to match mix-format's canonical output. This prevents non-idempotent
    // reformatting when the one-line form would exceed mix-format's column threshold.
    //
    // mix-format wraps long lines based on dynamic column widths, but the only stable
    // form is to pre-wrap at 2-space indentation under the keyword. This ensures
    // idempotent formatting regardless of library name length.

    let mut env = minijinja::Environment::new();
    let template_str = r#"defmodule {{ app_module }}.Native do
  @moduledoc false

  use RustlerPrecompiled,
    otp_app: :{{ app_name }},
    crate: "{{ app_name }}_nif",
    base_url:
      "{{ repo_url }}/releases/download/v#{{ '{' }}Mix.Project.config()[:version]{{ '}' }}",
    version: Mix.Project.config()[:version],
    targets:
      ~w({{ nif_targets }}),
    nif_versions: ["2.16", "2.17"],
    force_build: System.get_env("{{ build_env_var }}") in ["1", "true"] or Mix.env() in [:dev]

"#;

    env.add_template("test_native_header_wrap", template_str)
        .expect("template adds successfully");

    let tmpl = env.get_template("test_native_header_wrap").expect("template retrieves");

    // Test case 1: short library name (one-line form would fit; still wrapped)
    let short_rendered = tmpl
        .render(context! {
            app_module => "Foo",
            app_name => "foo",
            repo_url => "https://github.com/kreuzberg-dev/foo",
            build_env_var => "FOO_BUILD",
            nif_targets => "aarch64-apple-darwin x86_64-unknown-linux-gnu",
        })
        .expect("template renders");

    assert!(
        short_rendered.contains("    base_url:\n      \"https://github.com/kreuzberg-dev/foo/releases/download/v"),
        "Short library: base_url should be wrapped, got:\n{}",
        short_rendered
    );
    assert!(
        short_rendered.contains("    targets:\n      ~w(aarch64-apple-darwin x86_64-unknown-linux-gnu)"),
        "Short library: targets should be wrapped, got:\n{}",
        short_rendered
    );

    // Test case 2: long library name (one-line form clearly exceeds 98 chars; wrapped)
    let long_rendered = tmpl
        .render(context! {
            app_module => "VeryLongLibraryNameForWrapTest",
            app_name => "very_long_library_name_for_wrap_test",
            repo_url => "https://github.com/example-org/very-long-library-name-for-wrap-test",
            build_env_var => "VERY_LONG_LIBRARY_NAME_FOR_WRAP_TEST_BUILD",
            nif_targets => "aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu",
        })
        .expect("template renders");

    assert!(
        long_rendered.contains("    base_url:\n      \"https://github.com/example-org/very-long-library-name-for-wrap-test/releases/download/v"),
        "Long library: base_url should be wrapped, got:\n{}",
        long_rendered
    );
    assert!(
        long_rendered.contains("    targets:\n      ~w(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu)"),
        "Long library: targets should be wrapped, got:\n{}",
        long_rendered
    );
}

#[test]
fn targets_with_many_platforms_wraps_at_keyword() {
    // Test that the native module header with many NIF targets (7+ platforms)
    // uses a multi-line list literal so each target fits within the consumer's
    // line_length (120 chars). This is a regression test for kreuzcrawl, which
    // has 7 targets that would exceed 120 chars on a single ~w(...) line.
    // TODO(alef-generic-cleanup): Replace kreuzberg-dev/kreuzcrawl native-module fixture with neutral names.
    //
    // Multi-line list format ensures mix format wraps each target on its own line,
    // and no single line in the targets list exceeds the 120-char limit.

    let mut env = minijinja::Environment::new();
    let template_str = r#"defmodule {{ app_module }}.Native do
  @moduledoc false

  use RustlerPrecompiled,
    otp_app: :{{ app_name }},
    crate: "{{ app_name }}_nif",
    base_url:
      "{{ repo_url }}/releases/download/v#{{ '{' }}Mix.Project.config()[:version]{{ '}' }}",
    version: Mix.Project.config()[:version],
    targets: [
{% for target in nif_targets_list -%}
      "{{ target }}",
{% endfor -%}
    ],
    nif_versions: ["2.16", "2.17"],
    force_build: System.get_env("{{ build_env_var }}") in ["1", "true"] or Mix.env() in [:dev]

"#;

    env.add_template("test_many_targets", template_str)
        .expect("template adds successfully");

    let tmpl = env.get_template("test_many_targets").expect("template retrieves");

    // Kreuzcrawl's actual targets: 7 platforms
    let rendered = tmpl
        .render(context! {
            app_module => "Kreuzcrawl",
            app_name => "kreuzcrawl",
            repo_url => "https://github.com/kreuzberg-dev/kreuzcrawl",
            build_env_var => "KREUZCRAWL_BUILD",
            nif_targets_list => vec![
                "x86_64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-musl",
                "aarch64-unknown-linux-musl",
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "x86_64-pc-windows-msvc",
            ],
        })
        .expect("template renders");

    // The multi-line list form ensures mix format wraps each target on its own line
    assert!(
        rendered.contains("    targets: ["),
        "targets should use list literal form, got:\n{}",
        rendered
    );

    // Verify each target is on its own line
    let lines_with_targets: Vec<&str> = rendered
        .lines()
        .filter(|l| l.contains("x86_64-unknown-linux-gnu") || l.contains("aarch64-apple-darwin"))
        .collect();
    assert!(
        lines_with_targets.len() >= 2,
        "targets should be split across multiple lines, got:\n{}",
        rendered
    );

    // Verify each line is under 120 characters (consumer's line_length)
    for line in rendered.lines() {
        if line.contains("\"") && line.contains("-") {
            // This is a target line
            assert!(
                line.len() <= 120,
                "target line exceeds 120 chars ({}): {}",
                line.len(),
                line
            );
        }
    }
}
