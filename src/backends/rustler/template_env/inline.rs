pub(super) static TEMPLATES: &[(&str, &str)] = &[
    (
        "nif_stub_single_line.jinja",
        "  def {{ sig }}, do: :erlang.nif_error(:nif_not_loaded)\n",
    ),
    (
        "nif_stub_multi_line.jinja",
        r#"{% if not prev_was_multiline %}
{% endif %}  def {{ sig }},
    do: :erlang.nif_error(:nif_not_loaded)

"#,
    ),
    (
        "native_module_header.jinja",
        r#"defmodule {{ app_module }}.Native do
  @moduledoc false

  use RustlerPrecompiled,
    otp_app: :{{ app_name }},
    crate: "{{ app_name }}_nif",
    base_url: "{{ repo_url }}/releases/download/v#{{ '{' }}Mix.Project.config()[:version]{{ '}' }}",
    version: Mix.Project.config()[:version],
    targets: [
{{ nif_targets_block }}
    ],
    nif_versions: ["2.16", "2.17"],
    force_build: System.get_env("{{ build_env_var }}") in ["1", "true"] or Mix.env() in [:dev]

"#,
    ),
    ("native_module_footer.jinja", "end\n"),
    (
        "struct_module_header.jinja",
        r#"defmodule {{ app_module }}.{{ type_name }} do
"#,
    ),
    ("struct_module_footer.jinja", "end\n"),
    ("struct_empty.jinja", "  defstruct []\n"),
    (
        "enum_module_header.jinja",
        r#"defmodule {{ app_module }}.{{ enum_name }} do
"#,
    ),
    ("enum_module_footer.jinja", "end\n"),
    (
        "sync_method_body.rs.jinja",
        include_str!("../templates/sync_method_body.rs.jinja"),
    ),
    (
        "trait_sync_method_body.rs.jinja",
        include_str!("../templates/trait_sync_method_body.rs.jinja"),
    ),
    (
        "trait_async_method_body.rs.jinja",
        include_str!("../templates/trait_async_method_body.rs.jinja"),
    ),
    (
        "trait_constructor.rs.jinja",
        include_str!("../templates/trait_constructor.rs.jinja"),
    ),
    (
        "trait_unregistration_fn.rs.jinja",
        include_str!("../templates/trait_unregistration_fn.rs.jinja"),
    ),
    (
        "trait_clear_fn.rs.jinja",
        include_str!("../templates/trait_clear_fn.rs.jinja"),
    ),
    (
        "trait_registration_fn.rs.jinja",
        include_str!("../templates/trait_registration_fn.rs.jinja"),
    ),
    (
        "trait_support_nifs.rs.jinja",
        include_str!("../templates/trait_support_nifs.rs.jinja"),
    ),
    (
        "nif_with_visitor_async_body.rs.jinja",
        include_str!("../templates/nif_with_visitor_async_body.rs.jinja"),
    ),
    (
        "nif_with_visitor_field_async_body.rs.jinja",
        include_str!("../templates/nif_with_visitor_field_async_body.rs.jinja"),
    ),
    (
        "visitor_bridge_helper.rs.jinja",
        include_str!("../templates/visitor_bridge_helper.rs.jinja"),
    ),
    (
        "visitor_bridge_globals.rs.jinja",
        include_str!("../templates/visitor_bridge_globals.rs.jinja"),
    ),
    (
        "visitor_bridge_struct.rs.jinja",
        include_str!("../templates/visitor_bridge_struct.rs.jinja"),
    ),
    (
        "visitor_bridge_debug.rs.jinja",
        include_str!("../templates/visitor_bridge_debug.rs.jinja"),
    ),
    (
        "visitor_bridge_constructors.rs.jinja",
        include_str!("../templates/visitor_bridge_constructors.rs.jinja"),
    ),
    (
        "visitor_send_and_wait.rs.jinja",
        include_str!("../templates/visitor_send_and_wait.rs.jinja"),
    ),
    (
        "visitor_reply_nif.rs.jinja",
        include_str!("../templates/visitor_reply_nif.rs.jinja"),
    ),
    (
        "nif_function.rs.jinja",
        "#[rustler::nif]\npub fn {{ func_name }}({{ params_str }}) -> {{ ret }} {\n    {{ body }}\n}\n",
    ),
    (
        "dirty_cpu_nif_function.rs.jinja",
        "#[rustler::nif(schedule = \"DirtyCpu\")]\npub fn {{ func_name }}({{ params_str }}) -> {{ ret }} {\n    {{ body }}\n}\n",
    ),
    (
        "visitor_method.rs.jinja",
        include_str!("../templates/visitor_method.rs.jinja"),
    ),
    (
        "elixir_module_header.jinja",
        "defmodule {{ app_module }} do\n  @moduledoc \"{{ moduledoc }}\"\n\n",
    ),
    ("elixir_doc_line.jinja", "  @doc \"{{ doc_line }}\"\n"),
    ("elixir_enum_type_single_line.jinja", "  @type t :: {{ arms }}\n"),
    (
        "elixir_enum_type_multi_line.jinja",
        "  @type t ::\n{%- for arm in arms %}\n    {%- if loop.first %}\n    {{ arm }}\n    {%- else %}\n    | {{ arm }}\n    {%- endif %}\n{%- endfor %}\n",
    ),
    ("elixir_enum_attr.jinja", "  @{{ attr_name }} :{{ atom_name }}\n"),
    (
        "elixir_enum_accessor.jinja",
        "  @spec {{ atom_name }}() :: t()\n  def {{ atom_name }}, do: @{{ attr_name }}\n",
    ),
    (
        "elixir_enum_variant_constructor.jinja",
        "  def {{ fn_name }}({{ params }}), do: {:{{ atom }}, %{{ '{' }}{{ map_entries }}{{ '}' }}}\n",
    ),
    (
        "elixir_data_enum_type.jinja",
        "  @type {{ type_name }} :: {%- if is_unit %} {{ variant_atom }}\n{%- else %} %{type: {{ variant_atom }}, {{ field_types | join(\", \") }}}\n{%- endif %}",
    ),
    (
        "elixir_opaque_struct.jinja",
        "pub struct {{ struct_name }} {\n    inner: Arc<std::sync::RwLock<{{ core_path }}>>,\n}\n\n// SAFETY: See gen_opaque_resource in alef-backend-rustler for rationale.\n\nimpl std::panic::RefUnwindSafe for {{ struct_name }} {}\n\nimpl rustler::Resource for {{ struct_name }} {}\n",
    ),
    (
        "elixir_binding_struct.jinja",
        "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifStruct)]\n#[module = \"{{ module_prefix }}.{{ struct_name }}\"]\npub struct {{ struct_name }} {\n{%- for field in fields %}\n    pub {{ field.name }}: {{ field.type }},\n{%- endfor %}\n}\n",
    ),
    (
        "elixir_config_struct.jinja",
        "#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, rustler::NifMap)]\npub struct {{ struct_name }} {\n{%- for field in fields %}\n    pub {{ field.name }}: {{ field.type }},\n{%- endfor %}\n}\n",
    ),
    (
        "elixir_impl_header.jinja",
        r#"impl {{ struct_name }} {
"#,
    ),
    (
        "rust_let_binding.jinja",
        "let {{ var_name }}: {{ var_type }} = {{ expr }};\n    ",
    ),
    ("elixir_enum_field_first.jinja", "{{ name }}: {{ default }}"),
    (
        "elixir_enum_field_rest.jinja",
        ",\n            {{ name }}: {{ default }}",
    ),
    // mix-format aligns the first arm with the `|`-prefixed continuation arms at column 10
    // (i.e. 10-space indent for the bare arm so its `:atom` lines up with each `| :atom`).
    // Emitting the first arm at column 0 breaks `mix format --check-formatted` and forces
    // the consumer's pre-commit `mix-format` hook to rewrite the file, invalidating alef's
    // embedded `alef:hash:` line.
    ("elixir_enum_type_arm_first.jinja", "          {{ arm }}\n"),
    ("elixir_enum_type_arm_rest.jinja", "          | {{ arm }}\n"),
    (
        "elixir_data_enum_unit_type.jinja",
        "  @type {{ type_name }} :: {{ variant_atom }}\n",
    ),
    (
        "elixir_data_enum_struct_type.jinja",
        "  @type {{ type_name }} :: %{type: {{ variant_atom }}, {{ field_types }}}\n",
    ),
    (
        "rust_opaque_struct.jinja",
        "pub struct {{ struct_name }} {\n    inner: Arc<std::sync::RwLock<{{ core_path }}>>,\n}\n\n// SAFETY: See gen_opaque_resource in alef-backend-rustler for rationale.\n\nimpl std::panic::RefUnwindSafe for {{ struct_name }} {}\n\nimpl rustler::Resource for {{ struct_name }} {}\n",
    ),
    (
        "rust_struct_derive.jinja",
        "pub struct {{ struct_name }} {\n{%- for field in fields %}\n    pub {{ field.name }}: {{ field.type }},\n{%- endfor %}\n}\n",
    ),
    ("rust_struct_header.jinja", "pub struct {{ struct_name }} {\n"),
    ("rust_struct_field.jinja", "    pub {{ name }}: {{ type }},\n"),
    (
        "rust_impl_header.jinja",
        r#"impl {{ struct_name }} {
"#,
    ),
    (
        "rust_module_attr.jinja",
        "#[module = \"{{ module_prefix }}.{{ struct_name }}\"]\n",
    ),
    (
        "elixir_spec_multiline.jinja",
        // alef's minijinja env has trim_blocks=true (first newline after every
        // {% ... %} tag is stripped). Use an explicit {{ "\n" }} expression at
        // the start of each loop iteration so the per-arg newline survives.
        "  @spec {{ func_name }}({% for type_str in param_types %}{{ \"\\n          \" }}{{ type_str }}{% if not loop.last %},{% endif %}{% endfor %}{{ \"\\n        \" }}) :: {{ return_spec }}\n",
    ),
    (
        "elixir_def_with_guard.jinja",
        "  def {{ func_name }}({{ params }}) when is_map({{ guard_param }}) do\n",
    ),
    (
        "elixir_map_pop_unpack.jinja",
        "    {visitor, clean_opts} = Map.pop({{ opts_param }}, :{{ field_name }})\n",
    ),
    (
        "elixir_visitor_call.jinja",
        "      {:ok, _} = {{ native_mod }}.{{ func_name }}_with_visitor({{ args }})\n",
    ),
    (
        "elixir_visitor_receive.jinja",
        "      do_visitor_receive_loop({{ visitor_param }})\n",
    ),
    ("elixir_def_simple.jinja", "  def {{ func_name }}({{ params }}) do\n"),
    ("elixir_def_zero_arity.jinja", "  def {{ func_name }} do\n"),
    (
        "elixir_def_nif_call.jinja",
        "    {{ native_mod }}.{{ func_name }}({{ args }})\n",
    ),
    (
        "elixir_if_else_visitor.jinja",
        r#"    if is_map({{ visitor_param }}) do
{{ with_visitor_block }}    else
{{ no_visitor_block }}    end
"#,
    ),
    (
        "elixir_visitor_helper_functions.jinja",
        include_str!("../templates/elixir_visitor_helper_functions.jinja"),
    ),
];
