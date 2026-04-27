use crate::{
    capitalize_first, cargo_package_header, core_dep_features, detect_workspace_inheritance, render_extra_deps,
    scaffold_meta,
};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

pub(crate) fn scaffold_elixir_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let nif_name = format!("{app_name}_nif");
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let pkg_dir = config.package_dir(Language::Elixir);
    let ws = detect_workspace_inheritance(config.crate_config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(
        &nif_name,
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let extra_deps = render_extra_deps(config, Language::Elixir);
    let extra_deps_section = if extra_deps.is_empty() {
        String::new()
    } else {
        format!("\n{extra_deps}")
    };
    let has_async =
        api.functions.iter().any(|f| f.is_async) || api.types.iter().any(|t| t.methods.iter().any(|m| m.is_async));
    // Trait bridges generate a tokio::sync::oneshot-based reply channel, so tokio is required
    // whenever there are active Elixir trait bridges even if no API functions are async.
    let has_trait_bridges = config
        .trait_bridges
        .iter()
        .any(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"));
    let tokio_dep = if has_async || has_trait_bridges {
        "\ntokio = { version = \"1\", features = [\"rt-multi-thread\", \"sync\"] }"
    } else {
        ""
    };
    // Async/streaming NIF code uses futures_util::StreamExt to consume Stream returns.
    let futures_util_dep = if has_async {
        format!("\nfutures-util = \"{}\"", tv::cargo::FUTURES_UTIL)
    } else {
        String::new()
    };
    let content = format!(
        r#"{pkg_header}

[lib]
name = "{nif_name}"
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../../../../crates/{core_crate_dir}"{features} }}
rustler = "{rustler}"
async-trait = "{async_trait}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"{tokio_dep}{futures_util_dep}{extra_deps_section}

[workspace]
"#,
        pkg_header = pkg_header,
        nif_name = nif_name,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Elixir),
        rustler = tv::cargo::RUSTLER,
        async_trait = tv::cargo::ASYNC_TRAIT,
        tokio_dep = tokio_dep,
        futures_util_dep = futures_util_dep,
        extra_deps_section = extra_deps_section,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("{pkg_dir}/native/{nif_name}/Cargo.toml")),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_elixir(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let version = &api.version;
    let pkg_dir = config.package_dir(Language::Elixir);

    let content = format!(
        r#"defmodule {module}.MixProject do
  use Mix.Project

  def project do
    [
      app: :{app_name},
      version: "{version}",
      elixir: "~> 1.14",
      rustler_crates: [{nif_atom}: [mode: :release]],
      description: "{description}",
      package: package(),
      deps: deps()
    ]
  end

  defp package do
    [
      licenses: ["{license}"],
      links: %{{"GitHub" => "{repository}"}},
      files: ~w(lib native .formatter.exs mix.exs README* checksum-*.exs)
    ]
  end

  defp deps do
    [
      {{:rustler, "{rustler_hex}", optional: true, runtime: false}},
      {{:rustler_precompiled, "{rustler_precompiled}"}},
      {{:credo, "{credo}", only: [:dev, :test], runtime: false}},
      {{:ex_doc, "{ex_doc}", only: :dev, runtime: false}}
    ]
  end
end
"#,
        module = app_name.to_pascal_case(),
        app_name = app_name,
        nif_atom = format_args!("{app_name}_nif"),
        version = version,
        description = meta.description,
        license = meta.license,
        repository = meta.repository,
        rustler_hex = tv::hex::RUSTLER,
        rustler_precompiled = tv::hex::RUSTLER_PRECOMPILED,
        credo = tv::hex::CREDO,
        ex_doc = tv::hex::EX_DOC,
    );

    let formatter_content = r#"[
  import_deps: [:rustler],
  inputs: ["{mix,.formatter}.exs", "{config,lib,test}/**/*.{ex,exs}"],
  line_length: 120
]
"#;

    let mut files = vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/mix.exs")),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.formatter.exs")),
            content: formatter_content.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.credo.exs")),
            content: r#"%{
  configs: [
    %{
      name: "default",
      strict: true,
      parse_timeout: 5000,
      files: %{
        included: [
          "lib/",
          "src/",
          "test/",
          "web/",
          "apps/*/lib/",
          "apps/*/src/",
          "apps/*/test/",
          "apps/*/web/"
        ],
        excluded: [
          ~r"/_build/",
          ~r"/deps/",
          ~r"/node_modules/"
        ]
      },
      checks: %{
        enabled: [
          {Credo.Check.Refactor.CyclomaticComplexity, max_complexity: 16}
        ]
      }
    }
  ]
}
"#
            .to_string(),
            generated_header: false,
        },
    ];

    // Generate trait bridge GenServer modules
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .iter()
            .any(|l| l == "elixir" || l == "rustler")
        {
            continue;
        }
        let trait_name_snake = bridge_cfg.trait_name.to_snake_case();
        let trait_name_camel = capitalize_first(&bridge_cfg.trait_name);
        let module_name = format!("{}{}Bridge", app_name.to_pascal_case(), trait_name_camel);
        let native_mod = format!("{}.Native", app_name.to_pascal_case());

        let bridge_content = format!(
            r#"defmodule {module_name} do
  @moduledoc """
  GenServer bridge for {trait_name} implementation in {app_name}.

  Handles incoming trait method calls from Rust and dispatches them to an implementation module.
  """

  use GenServer

  require Logger

  @doc """
  Start a GenServer linked to the current process.

  impl_module should be a module that implements the {trait_name} trait methods.
  """
  def start_link(impl_module) do
    GenServer.start_link(__MODULE__, impl_module, name: __MODULE__)
  end

  @impl GenServer
  def init(impl_module) do
    {{:ok, impl_module}}
  end

  @doc """
  Handle an incoming trait call message.

  Message format: {{:trait_call, method_atom, args_json, reply_id}}
  """
  @impl GenServer
  def handle_info({{:trait_call, method, args_json, reply_id}}, impl_module) do
    try do
      args = Jason.decode!(args_json)

      # Dispatch to the implementation module
      result = apply(impl_module, String.to_atom(method), args)

      # Send result back to Rust
      {native_mod}.complete_trait_call(reply_id, Jason.encode!(result))
    rescue
      e ->
        Logger.error("Error calling {{impl_module}}.{{method}}: {{Exception.message(e)}}")
        {native_mod}.fail_trait_call(reply_id, Exception.message(e))
    end

    {{:noreply, impl_module}}
  end

  @doc """
  Register an implementation module, starting a GenServer to handle trait calls.
  """
  def register(impl_module) do
    {{:ok, _pid}} = start_link(impl_module)
    {native_mod}.register_{trait_name_snake}(self(), Atom.to_string(impl_module))
  end
end
"#,
            module_name = module_name,
            trait_name = bridge_cfg.trait_name,
            app_name = app_name,
            trait_name_snake = trait_name_snake,
            native_mod = native_mod,
        );

        let bridge_path = PathBuf::from(format!("{pkg_dir}/lib/{app_name}/{trait_name_snake}_bridge.ex"));
        files.push(GeneratedFile {
            path: bridge_path,
            content: bridge_content,
            generated_header: true,
        });
    }

    Ok(files)
}
