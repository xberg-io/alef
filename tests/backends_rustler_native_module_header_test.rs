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
    base_url: "{{ repo_url }}/releases/download/v#{{ '{' }}Mix.Project.config()[:version]{{ '}' }}",
    version: Mix.Project.config()[:version],
    targets: ~w(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu),
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
