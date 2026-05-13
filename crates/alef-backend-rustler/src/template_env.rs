use minijinja::Environment;

static TEMPLATES: &[(&str, &str)] = &[
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
    targets: ~w(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu x86_64-pc-windows-gnu),
    nif_versions: ["2.16", "2.17"],
    force_build: System.get_env("{{ build_env_var }}") in ["1", "true"] or Mix.env() in [:test, :dev]

"#,
    ),
    ("native_module_footer.jinja", "end\n"),
    (
        "struct_module_header.jinja",
        r#"defmodule {{ app_module }}.{{ type_name }} do
{% if has_doc %}
  @moduledoc "{{ doc }}"
{% else %}
  @moduledoc false
{% endif %}

"#,
    ),
    ("struct_module_footer.jinja", "end\n"),
    ("struct_empty.jinja", "  defstruct []\n"),
    (
        "enum_module_header.jinja",
        r#"defmodule {{ app_module }}.{{ enum_name }} do
{% if has_doc %}
  @moduledoc "{{ doc }}"
{% else %}
  @moduledoc false
{% endif %}

"#,
    ),
    ("enum_module_footer.jinja", "end\n"),
    (
        "sync_method_body.rs.jinja",
        r#"{%- for param in clone_params %}
let {{ param.name }} = {{ param.name }}.clone();
{%- endfor %}

let reply_id = TRAIT_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
TRAIT_REPLY_CHANNELS.lock().unwrap().insert(reply_id, tx);

let pid = self.inner;

let args_json = {
    let mut args = serde_json::Map::new();
{%- for param in params %}
    args.insert("{{ param.name }}".to_string(), {{ param.json_expr }});
{%- endfor %}
    serde_json::Value::Object(args).to_string()
};

let method = "{{ method_name }}";

tokio::task::spawn_blocking(move || {
    let mut env = rustler::OwnedEnv::new();
    let _ = env.send_and_clear(&pid, |env| {
        (rustler::types::atom::Atom::from_str(env, "trait_call").unwrap(),
         method, args_json.as_str(), reply_id).encode(env)
    });
});

match rx.blocking_recv() {
{%- if has_error %}
    Ok(Ok(json)) => serde_json::from_str(&json).map_err(|_e| {{ error_deser }}),
    Ok(Err(msg)) => Err({{ error_msg }}),
    Err(_) => Err({{ error_closed }}),
{%- else %}
    Ok(Ok(json)) => serde_json::from_str(&json).unwrap_or_default(),
    _ => Default::default()
{%- endif %}
}
"#,
    ),
    (
        "trait_sync_method_body.rs.jinja",
        r#"{%- for param_clone in param_clones %}
let {{ param_clone.name }} = {{ param_clone.name }}.clone();
{% endfor -%}

let reply_id = TRAIT_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
TRAIT_REPLY_CHANNELS.lock().unwrap().insert(reply_id, tx);

let pid = self.inner;

let args_json = {
    let mut args = serde_json::Map::new();
{%- for arg in args_json %}
    args.insert("{{ arg.name }}".to_string(), {{ arg.expr }});
{%- endfor %}
    serde_json::Value::Object(args).to_string()
};

let method = "{{ method_name }}";

tokio::task::spawn_blocking(move || {
    let mut env = rustler::OwnedEnv::new();
    let _ = env.send_and_clear(&pid, |env| {
        (rustler::types::atom::Atom::from_str(env, "trait_call").unwrap(), method, args_json.as_str(), reply_id).encode(env)
    });
});

match rx.blocking_recv() {
{%- if has_error %}
    Ok(Ok(json)) => serde_json::from_str(&json).map_err(|_e| {{ error_deser }}),
    Ok(Err(msg)) => Err({{ error_msg }}),
    Err(_) => Err({{ error_closed }})
{%- else %}
    Ok(Ok(json)) => serde_json::from_str(&json).unwrap_or_default(),
    _ => Default::default()
{%- endif %}
}
"#,
    ),
    (
        "trait_async_method_body.rs.jinja",
        r#"{%- for param_clone in param_clones %}
let {{ param_clone.name }} = {{ param_clone.name }}.clone();
{% endfor -%}

let reply_id = TRAIT_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
TRAIT_REPLY_CHANNELS.lock().unwrap().insert(reply_id, tx);

let pid = self.inner;

let args_json = {
    let mut args = serde_json::Map::new();
{%- for arg in args_json %}
    args.insert("{{ arg.name }}".to_string(), {{ arg.expr }});
{%- endfor %}
    serde_json::Value::Object(args).to_string()
};

let method = "{{ method_name }}";

tokio::task::spawn_blocking(move || {
    let mut env = rustler::OwnedEnv::new();
    let _ = env.send_and_clear(&pid, |env| {
        (rustler::types::atom::Atom::from_str(env, "trait_call").unwrap(), method, args_json.as_str(), reply_id).encode(env)
    });
}).await;

match rx.await {
{%- if has_error %}
    Ok(Ok(json)) => serde_json::from_str(&json).map_err(|_e| {{ error_deser }}),
    Ok(Err(msg)) => Err({{ error_msg }}),
    Err(_) => Err({{ error_closed }})
{%- else %}
    Ok(Ok(json)) => serde_json::from_str(&json).unwrap_or_default(),
    _ => Default::default()
{%- endif %}
}
"#,
    ),
    (
        "trait_constructor.rs.jinja",
        r#"impl {{ wrapper_name }} {
    /// Create a new bridge wrapping an Elixir GenServer PID.
    ///
    /// The PID is copied (LocalPid is Copy + Send + Sync) and used to send
    /// messages to the backing GenServer. The plugin_name is cached for fast
    /// Plugin::name() lookups.
    pub fn new(pid: rustler::LocalPid, plugin_name: String) -> Self {
        Self {
            inner: pid,
            cached_name: plugin_name,
        }
    }
}
"#,
    ),
    (
        "trait_unregistration_fn.rs.jinja",
        r#"#[rustler::nif]
pub fn {{ unregister_fn }}(env: rustler::Env<'_>, name: String) -> rustler::Atom {
    match {{ host_path }}(&name) {
        Ok(_) => rustler::types::atom::Atom::from_str(env, "ok").unwrap(),
        Err(_) => rustler::types::atom::Atom::from_str(env, "error").unwrap(),
    }
}
"#,
    ),
    (
        "trait_clear_fn.rs.jinja",
        r#"#[rustler::nif]
pub fn {{ clear_fn }}(env: rustler::Env<'_>) -> rustler::Atom {
    match {{ host_path }}() {
        Ok(_) => rustler::types::atom::Atom::from_str(env, "ok").unwrap(),
        Err(_) => rustler::types::atom::Atom::from_str(env, "error").unwrap(),
    }
}
"#,
    ),
    (
        "trait_registration_fn.rs.jinja",
        r#"#[rustler::nif]
pub fn {{ register_fn }}(env: rustler::Env<'_>, genserver_pid: rustler::LocalPid, plugin_name: String) -> rustler::Atom {

    let bridge = {{ wrapper_name }}::new(genserver_pid, plugin_name);
    let arc: Arc<dyn {{ trait_path }}> = Arc::new(bridge);

    let registry = {{ registry_getter }}();
    match registry.write().register(arc{{ extra_args }}) {
        Ok(_) => rustler::types::atom::Atom::from_str(env, "ok").unwrap(),
        Err(_) => rustler::types::atom::Atom::from_str(env, "error").unwrap(),
    }
}
"#,
    ),
    (
        "trait_support_nifs.rs.jinja",
        r#"/// Complete a pending trait call with a successful JSON result.
/// Called from Elixir GenServer after handling a trait method call.
#[rustler::nif]
pub fn complete_trait_call(env: rustler::Env, reply_id: u64, result_json: String) -> rustler::Atom {
    if let Some(tx) = TRAIT_REPLY_CHANNELS.lock().unwrap().remove(&reply_id) {
        let _ = tx.send(Ok(result_json));
    }
    rustler::types::atom::ok()
}

/// Fail a pending trait call with an error message.
/// Called from Elixir GenServer if handling fails.
#[rustler::nif]
pub fn fail_trait_call(env: rustler::Env, reply_id: u64, error_message: String) -> rustler::Atom {
    if let Some(tx) = TRAIT_REPLY_CHANNELS.lock().unwrap().remove(&reply_id) {
        let _ = tx.send(Err(error_message));
    }
    rustler::types::atom::ok()
}
"#,
    ),
    (
        "nif_with_visitor_async_body.rs.jinja",
        r#"// Async visitor variant: spawns a system thread, sends result as a message.
#[rustler::nif]
pub fn {{ func_name }}_with_visitor({{ with_params_str }}) -> Result<(), String> {
    let pid = env.pid();
    {{ with_deser }}

    let mut visitor_owned_env = rustler::OwnedEnv::new();
    let visitor_saved = visitor_owned_env.save({{ param_name }});
    {{ clone_stmts }}

    std::thread::spawn(move || {
        let bridge = {{ struct_name }}::new_from_saved(pid, visitor_owned_env, visitor_saved);
        let {{ param_name }}: Option<{{ handle_path }}> = Some(std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {{ handle_path }});
        let mut result_env = rustler::OwnedEnv::new();
        let _ = result_env.send_and_clear(&pid, |env| {
            match {{ core_fn_path }}({{ with_call_args_str }}) {
                Ok(val) => {
                    let result: ConversionResult = val.into();
                    let ok_atom = rustler::types::atom::Atom::from_str(env, "ok").unwrap().to_term(env);
                    let result_term = result.encode(env);
                    rustler::types::tuple::make_tuple(env, &[ok_atom, result_term])
                },
                Err(e) => {
                    let err_atom = rustler::types::atom::Atom::from_str(env, "error").unwrap().to_term(env);
                    let reason = e.to_string().encode(env);
                    rustler::types::tuple::make_tuple(env, &[err_atom, reason])
                },
            }
        });
    });
    Ok(())
}
"#,
    ),
    (
        "nif_with_visitor_field_async_body.rs.jinja",
        r#"// Async visitor variant: pops visitor from options, builds bridge, spawns thread.
#[rustler::nif]
pub fn {{ func_name }}_with_visitor({{ vis_params_str }}) -> Result<(), String> {
    let pid = env.pid();
    let mut visitor_owned_env = rustler::OwnedEnv::new();
    let visitor_saved = visitor_owned_env.save(visitor);
    {{ clone_stmts }}
    std::thread::spawn(move || {
        let _ = visitor_owned_env.send_and_clear(&pid, |env| {
            let visitor_term = visitor_saved.load(env);
            {{ deser_stmts }}
            // Run conversion and return result term to send back to BEAM
            let conversion_result = match {{ core_fn_path }}({{ vis_call_args_str }}) {
                Ok(val) => {
                    let result: ConversionResult = val.into();  // Convert from core::ConversionResult to NIF::ConversionResult
                    Ok(result)
                },
                Err(e) => Err(e.to_string()),
            };
            match conversion_result {
                Ok(result) => {
                    let ok_atom = rustler::types::atom::Atom::from_str(env, "ok").unwrap().to_term(env);
                    rustler::types::tuple::make_tuple(env, &[ok_atom, result.encode(env)])
                },
                Err(reason) => {
                    let err_atom = rustler::types::atom::Atom::from_str(env, "error").unwrap().to_term(env);
                    let reason_term = reason.encode(env);
                    rustler::types::tuple::make_tuple(env, &[err_atom, reason_term])
                },
            }
        });
    });
    Ok(())
}
"#,
    ),
    (
        "visitor_bridge_helper.rs.jinja",
        r#"fn nodecontext_to_elixir_map<'a>(
    env: rustler::Env<'a>,
    ctx: &{{ core_crate }}::visitor::NodeContext,
) -> rustler::Term<'a> {
    let mut pairs: Vec<(rustler::Term<'a>, rustler::Term<'a>)> = Vec::new();
    {
        let node_type_debug = format!("{:?}", ctx.node_type);
        let node_type_snake: String = node_type_debug.chars().enumerate()
            .flat_map(|(i, c)| {
                if c.is_uppercase() && i > 0 { vec!['_', c.to_lowercase().next().unwrap()] }
                else if c.is_uppercase() { vec![c.to_lowercase().next().unwrap()] }
                else { vec![c] }
            }).collect();
        pairs.push((rustler::types::atom::Atom::from_str(env, "node_type").unwrap().to_term(env), rustler::types::atom::Atom::from_str(env, &node_type_snake).unwrap().to_term(env)));
    }
    pairs.push((rustler::types::atom::Atom::from_str(env, "tag_name").unwrap().to_term(env), ctx.tag_name.encode(env)));
    pairs.push((rustler::types::atom::Atom::from_str(env, "depth").unwrap().to_term(env), (ctx.depth as i64).encode(env)));
    pairs.push((rustler::types::atom::Atom::from_str(env, "index_in_parent").unwrap().to_term(env), (ctx.index_in_parent as i64).encode(env)));
    pairs.push((rustler::types::atom::Atom::from_str(env, "is_inline").unwrap().to_term(env), ctx.is_inline.encode(env)));
    let parent_tag_term = match &ctx.parent_tag { Some(s) => s.encode(env), None => rustler::types::atom::Atom::from_str(env, "nil").unwrap().to_term(env) };
    pairs.push((rustler::types::atom::Atom::from_str(env, "parent_tag").unwrap().to_term(env), parent_tag_term));
    let attrs_pairs: Vec<(rustler::Term<'a>, rustler::Term<'a>)> = ctx.attributes.iter().map(|(k, v)| (k.encode(env), v.encode(env))).collect();
    let attrs_map = rustler::Term::map_from_pairs(env, &attrs_pairs).unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, "nil").unwrap().to_term(env));
    pairs.push((rustler::types::atom::Atom::from_str(env, "attributes").unwrap().to_term(env), attrs_map));
    rustler::Term::map_from_pairs(env, &pairs).unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, "nil").unwrap().to_term(env))
}

"#,
    ),
    (
        "visitor_bridge_globals.rs.jinja",
        r#"static VISITOR_REPLY_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static VISITOR_CHANNELS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<u64, std::sync::mpsc::SyncSender<Option<String>>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

"#,
    ),
    (
        "visitor_bridge_struct.rs.jinja",
        r#"pub struct {{ struct_name }} {
    caller_pid: rustler::types::LocalPid,
    visitor_env: rustler::OwnedEnv,
    visitor_saved: rustler::env::SavedTerm,
}

"#,
    ),
    (
        "visitor_bridge_debug.rs.jinja",
        r#"impl std::fmt::Debug for {{ struct_name }} {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ struct_name }}")
    }
}

"#,
    ),
    (
        "visitor_bridge_constructors.rs.jinja",
        r#"impl {{ struct_name }} {
    pub fn new(env: rustler::Env<'_>, caller_pid: rustler::types::LocalPid, visitor_term: rustler::Term<'_>) -> Self {
        let owned = rustler::OwnedEnv::new();
        let saved = owned.save(visitor_term);
        Self { caller_pid, visitor_env: owned, visitor_saved: saved }
    }

    pub fn new_from_saved(caller_pid: rustler::types::LocalPid, visitor_env: rustler::OwnedEnv, visitor_saved: rustler::env::SavedTerm) -> Self {
        Self { caller_pid, visitor_env, visitor_saved }
    }
}

"#,
    ),
    (
        "visitor_send_and_wait.rs.jinja",
        r#"fn visitor_send_and_wait(bridge: &{{ struct_name }}, callback_name: &str, args_json: String) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<String>>(1);
    let ref_id = VISITOR_REPLY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    VISITOR_CHANNELS.lock().unwrap().insert(ref_id, tx);
    let pid = bridge.caller_pid;
    let cb_name = callback_name.to_string();
    let mut msg_env = rustler::OwnedEnv::new();
    let _ = msg_env.send_and_clear(&pid, |env| {
        let tag = rustler::types::atom::Atom::from_str(env, "visitor_callback").unwrap().to_term(env);
        let ref_term = ref_id.encode(env);
        let name_term = rustler::types::atom::Atom::from_str(env, &cb_name).unwrap().to_term(env);
        let args_term = args_json.encode(env);
        rustler::types::tuple::make_tuple(env, &[tag, ref_term, name_term, args_term])
    });
    let result = rx.recv().ok().flatten();
    VISITOR_CHANNELS.lock().unwrap().remove(&ref_id);
    result
}

"#,
    ),
    (
        "visitor_reply_nif.rs.jinja",
        r#"#[rustler::nif]
pub fn visitor_reply(ref_id: u64, result: Option<String>) {
    if let Some(tx) = VISITOR_CHANNELS.lock().unwrap().get(&ref_id) {
        let _ = tx.send(result);
    }
}

"#,
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
        r#"    fn {{ method_name }}({{ sig }}) -> {{ ret_ty }} {
        let mut args_map = serde_json::Map::new();
{%- for arg in args %}
        args_map.insert("{{ arg.key }}".to_string(), {{ arg.expr }});
{%- endfor %}
        let args_json = serde_json::Value::Object(args_map).to_string();
        let result = visitor_send_and_wait(self, "{{ handle_name }}", args_json);
        match result {
            None => {{ ret_ty }}::Continue,
            Some(s) => {
                let lower = s.to_lowercase();
                match lower.as_str() {
                    "continue" => {{ ret_ty }}::Continue,
                    "skip" => {{ ret_ty }}::Skip,
                    "preserve_html" | "preservehtml" => {{ ret_ty }}::PreserveHtml,
                    _ => {{ ret_ty }}::Custom(s),
                }
            }
        }
    }

"#,
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
        "elixir_data_enum_type.jinja",
        "  @type {{ type_name }} :: {%- if is_unit %} {{ variant_atom }}\n{%- else %} %{type: {{ variant_atom }}, {{ field_types | join(\", \") }}}\n{%- endif %}",
    ),
    (
        "elixir_opaque_struct.jinja",
        "pub struct {{ struct_name }} {\n    inner: Arc<{{ core_path }}>,\n}\n\n// SAFETY: See gen_opaque_resource in alef-backend-rustler for rationale.\n\nimpl std::panic::RefUnwindSafe for {{ struct_name }} {}\n\nimpl rustler::Resource for {{ struct_name }} {}\n",
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
    ("elixir_enum_type_arm_first.jinja", "{{ arm }}\n"),
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
        "pub struct {{ struct_name }} {\n    inner: Arc<{{ core_path }}>,\n}\n\n// SAFETY: See gen_opaque_resource in alef-backend-rustler for rationale.\n\nimpl std::panic::RefUnwindSafe for {{ struct_name }} {}\n\nimpl rustler::Resource for {{ struct_name }} {}\n",
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
        r#"  @doc false
  defp do_visitor_receive_loop(visitor) do
    receive do
      {:visitor_callback, ref_id, callback_name, args_json} ->
        result =
          case Map.get(visitor, callback_name) do
            nil -> "continue"
            fun -> apply_visitor_callback(fun, args_json)
          end

        {{ native_mod }}.visitor_reply(ref_id, result)
        do_visitor_receive_loop(visitor)

      {:ok, result} ->
        {:ok, result}

      {:error, reason} ->
        {:error, reason}
    after
      30_000 ->
        {:error, "visitor callback timeout after 30s"}
    end
  end

  @doc false
  defp apply_visitor_callback(fun, args_json) do
    args = Jason.decode!(args_json)
    result = fun.(args)
    case result do
      :continue -> "continue"
      :skip -> "skip"
      :preserve_html -> "preserve_html"
      {:custom, value} -> to_string(value)
      binary when is_binary(binary) -> binary
      _ -> "continue"
    end
  end

"#,
    ),
    (
        "flat_enum_derive.jinja",
        include_str!("../templates/flat_enum_derive.jinja"),
    ),
    (
        "flat_enum_struct_header.jinja",
        include_str!("../templates/flat_enum_struct_header.jinja"),
    ),
    (
        "flat_enum_discriminator_field.jinja",
        include_str!("../templates/flat_enum_discriminator_field.jinja"),
    ),
    (
        "flat_enum_variant_field.jinja",
        include_str!("../templates/flat_enum_variant_field.jinja"),
    ),
    (
        "flat_enum_struct_footer.jinja",
        include_str!("../templates/flat_enum_struct_footer.jinja"),
    ),
    (
        "flat_enum_default_impl.jinja",
        include_str!("../templates/flat_enum_default_impl.jinja"),
    ),
    (
        "flat_enum_default_variant_field.jinja",
        include_str!("../templates/flat_enum_default_variant_field.jinja"),
    ),
    (
        "flat_enum_default_impl_footer.jinja",
        include_str!("../templates/flat_enum_default_impl_footer.jinja"),
    ),
    (
        "flat_enum_from_core_impl.jinja",
        include_str!("../templates/flat_enum_from_core_impl.jinja"),
    ),
    (
        "flat_enum_from_core_variant_unit.jinja",
        include_str!("../templates/flat_enum_from_core_variant_unit.jinja"),
    ),
    (
        "flat_enum_from_core_variant_tuple.jinja",
        include_str!("../templates/flat_enum_from_core_variant_tuple.jinja"),
    ),
    (
        "flat_enum_from_core_impl_footer.jinja",
        include_str!("../templates/flat_enum_from_core_impl_footer.jinja"),
    ),
    (
        "flat_enum_to_core_impl_header.jinja",
        include_str!("../templates/flat_enum_to_core_impl_header.jinja"),
    ),
    (
        "flat_enum_to_core_variant_unit.jinja",
        include_str!("../templates/flat_enum_to_core_variant_unit.jinja"),
    ),
    (
        "flat_enum_to_core_variant_tuple.jinja",
        include_str!("../templates/flat_enum_to_core_variant_tuple.jinja"),
    ),
    (
        "flat_enum_to_core_impl_footer.jinja",
        include_str!("../templates/flat_enum_to_core_impl_footer.jinja"),
    ),
    (
        "default_deser_with_error.rs.jinja",
        include_str!("../templates/default_deser_with_error.rs.jinja"),
    ),
    (
        "default_deser_without_error.rs.jinja",
        include_str!("../templates/default_deser_without_error.rs.jinja"),
    ),
    (
        "named_param_to_json.rs.jinja",
        include_str!("../templates/named_param_to_json.rs.jinja"),
    ),
    (
        "named_param_from_json.rs.jinja",
        include_str!("../templates/named_param_from_json.rs.jinja"),
    ),
    (
        "vec_str_refs_optional.rs.jinja",
        include_str!("../templates/vec_str_refs_optional.rs.jinja"),
    ),
    (
        "vec_str_refs_required.rs.jinja",
        include_str!("../templates/vec_str_refs_required.rs.jinja"),
    ),
    (
        "bytes_to_vec.rs.jinja",
        include_str!("../templates/bytes_to_vec.rs.jinja"),
    ),
    (
        "nif_result_body.rs.jinja",
        include_str!("../templates/nif_result_body.rs.jinja"),
    ),
    (
        "nif_wrapped_body.rs.jinja",
        include_str!("../templates/nif_wrapped_body.rs.jinja"),
    ),
    (
        "async_result_body.rs.jinja",
        include_str!("../templates/async_result_body.rs.jinja"),
    ),
    (
        "async_infallible_body.rs.jinja",
        include_str!("../templates/async_infallible_body.rs.jinja"),
    ),
    (
        "nif_tagged_enum_serde_tag.jinja",
        include_str!("../templates/nif_tagged_enum_serde_tag.jinja"),
    ),
    (
        "nif_tagged_enum_variant_unit.jinja",
        include_str!("../templates/nif_tagged_enum_variant_unit.jinja"),
    ),
    (
        "nif_tagged_enum_variant_struct_header.jinja",
        include_str!("../templates/nif_tagged_enum_variant_struct_header.jinja"),
    ),
    (
        "nif_tagged_enum_variant_field_line.jinja",
        include_str!("../templates/nif_tagged_enum_variant_field_line.jinja"),
    ),
    (
        "nif_tagged_enum_variant_struct_footer.jinja",
        include_str!("../templates/nif_tagged_enum_variant_struct_footer.jinja"),
    ),
    (
        "nif_unit_enum_header.jinja",
        include_str!("../templates/nif_unit_enum_header.jinja"),
    ),
    (
        "nif_enum_variant.jinja",
        include_str!("../templates/nif_enum_variant.jinja"),
    ),
    (
        "nif_enum_default_header.jinja",
        include_str!("../templates/nif_enum_default_header.jinja"),
    ),
    (
        "nif_enum_default_value.jinja",
        include_str!("../templates/nif_enum_default_value.jinja"),
    ),
    (
        "nif_enum_default_with_fields.jinja",
        include_str!("../templates/nif_enum_default_with_fields.jinja"),
    ),
    (
        "nif_enum_default_footer.jinja",
        include_str!("../templates/nif_enum_default_footer.jinja"),
    ),
    (
        "rust_method_instance_call.rs.jinja",
        include_str!("../templates/rust_method_instance_call.rs.jinja"),
    ),
    (
        "rust_method_static_call.rs.jinja",
        include_str!("../templates/rust_method_static_call.rs.jinja"),
    ),
    (
        "rust_method_static_call_with_preamble.rs.jinja",
        include_str!("../templates/rust_method_static_call_with_preamble.rs.jinja"),
    ),
    (
        "elixir_streaming_start_wrapper.jinja",
        include_str!("../templates/elixir_streaming_start_wrapper.jinja"),
    ),
    (
        "elixir_streaming_next_wrapper.jinja",
        include_str!("../templates/elixir_streaming_next_wrapper.jinja"),
    ),
    (
        "elixir_streaming_unfold_wrapper.jinja",
        include_str!("../templates/elixir_streaming_unfold_wrapper.jinja"),
    ),
    (
        "rustler_resource_registration.rs.jinja",
        include_str!("../templates/rustler_resource_registration.rs.jinja"),
    ),
    (
        "rustler_init_with_load.rs.jinja",
        include_str!("../templates/rustler_init_with_load.rs.jinja"),
    ),
    (
        "rustler_init.rs.jinja",
        include_str!("../templates/rustler_init.rs.jinja"),
    ),
    (
        "streaming_default_deser_binding.rs.jinja",
        include_str!("../templates/streaming_default_deser_binding.rs.jinja"),
    ),
    (
        "trait_impl_header.jinja",
        include_str!("../templates/trait_impl_header.jinja"),
    ),
    (
        "nif_tagged_enum_header.jinja",
        include_str!("../templates/nif_tagged_enum_header.jinja"),
    ),
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    for (name, src) in TEMPLATES {
        env.add_template(name, src).expect("built-in template is valid");
    }
    env
}

pub(crate) fn render(template_name: &str, ctx: minijinja::Value) -> String {
    make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"))
}
