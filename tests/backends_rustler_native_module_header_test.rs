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
