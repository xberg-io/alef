use minijinja::context;

#[test]
fn force_build_uses_mix_env_not_system_get_env() {
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

    let short_rendered = tmpl
        .render(context! {
            app_module => "Foo",
            app_name => "foo",
            repo_url => "https://github.com/xberg-io/foo",
            build_env_var => "FOO_BUILD",
            nif_targets => "aarch64-apple-darwin x86_64-unknown-linux-gnu",
        })
        .expect("template renders");

    assert!(
        short_rendered.contains("    base_url:\n      \"https://github.com/xberg-io/foo/releases/download/v"),
        "Short library: base_url should be wrapped, got:\n{}",
        short_rendered
    );
    assert!(
        short_rendered.contains("    targets:\n      ~w(aarch64-apple-darwin x86_64-unknown-linux-gnu)"),
        "Short library: targets should be wrapped, got:\n{}",
        short_rendered
    );

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

    let rendered = tmpl
        .render(context! {
            app_module => "Kreuzcrawl",
            app_name => "kreuzcrawl",
            repo_url => "https://github.com/xberg-io/kreuzcrawl",
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

    assert!(
        rendered.contains("    targets: ["),
        "targets should use list literal form, got:\n{}",
        rendered
    );

    let lines_with_targets: Vec<&str> = rendered
        .lines()
        .filter(|l| l.contains("x86_64-unknown-linux-gnu") || l.contains("aarch64-apple-darwin"))
        .collect();
    assert!(
        lines_with_targets.len() >= 2,
        "targets should be split across multiple lines, got:\n{}",
        rendered
    );

    for line in rendered.lines() {
        if line.contains("\"") && line.contains("-") {
            assert!(
                line.len() <= 120,
                "target line exceeds 120 chars ({}): {}",
                line.len(),
                line
            );
        }
    }
}
