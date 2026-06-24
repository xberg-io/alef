pub(super) fn elixir_stub_default(
    return_type: &crate::core::ir::TypeRef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};

    match return_type {
        TypeRef::Primitive(PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64) => {
            defaults.emit_default(return_type)
        }
        TypeRef::Primitive(_) => "1".to_string(),
        _ => defaults.emit_default(return_type),
    }
}

/// Emit an Elixir test backend stub module for a trait bridge.
///
/// Generates a `defmodule TestStub{PascalId}` that implements the trait's required
/// methods using language-appropriate default return values. The stub is registered
/// via the trait bridge's `register_fn`.
/// Emit the Elixir GenServer stub that implements a trait bridge for testing.
///
/// `nif_module` is the Elixir module that exposes `complete_trait_call/2` and
/// `fail_trait_call/2` NIFs (e.g. `"MyApp.Native"` for a crate named `my_app`).
/// Pass an empty string to use the conventional `Native` fallback.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    nif_module: &str,
) -> crate::e2e::codegen::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use heck::ToUpperCamelCase;
    use std::fmt::Write as _;

    let pascal_id = fixture.id.to_upper_camel_case();
    let module_name = format!("TestStub{pascal_id}");

    // Resolve the NIF module that exposes complete_trait_call/2.
    // Falls back to "Native" when no explicit module is provided, which is
    // correct for standalone e2e fixtures not tied to a specific crate namespace.
    let effective_nif_module = if nif_module.is_empty() { "Native" } else { nif_module };

    // Derive the plugin name from the first argument's input field structure.
    // For "register_document_extractor_trait_bridge" with input { extractor: { name: "test-extractor" } },
    // we need to extract input.extractor.name.
    // Pattern: fixture.input has a single key (the argument name), which is an object containing "name".
    let plugin_name = fixture
        .input
        .as_object()
        .and_then(|obj| obj.values().next()) // Get the first value (should be the argument object)
        .and_then(|arg_obj| arg_obj.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let defaults = language_defaults("elixir");

    // Use a fully-qualified E2e.TestStubs namespace so the module name is unique
    // and well-scoped. Guard the definition with Code.ensure_loaded?/1 so that
    // re-running the same compiled test file does not trigger a redefinition
    // warning (which becomes an error under --warnings-as-errors).
    let qualified_module = format!("E2e.TestStubs.{module_name}");
    let genserver_module = format!("{}GenServer", qualified_module);

    // Emit module-level definitions (no leading spaces).
    let mut module_defs = String::new();
    let _ = writeln!(module_defs, "unless Code.ensure_loaded?({qualified_module}) do");
    let _ = writeln!(module_defs, "defmodule {qualified_module} do");

    // If there is a Plugin super-trait, emit `name/0`.
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(module_defs, "  def name, do: \"{plugin_name}\"");
        let _ = writeln!(module_defs, "  def version, do: \"test\"");
        // initialize/0 has a Rust default impl but Rustler calls it unconditionally on
        // every registered plugin object - the Elixir stub must define it.
        let _ = writeln!(module_defs, "  def initialize, do: :ok");
        let _ = writeln!(module_defs, "  def shutdown, do: :ok");
    }

    // Emit every method the bridge may dispatch, including lifecycle/default
    // methods. The stub implementations remain no-ops/defaults when Rust would
    // normally provide a default body.
    for method in methods {
        // Build parameter list: skip `self` receiver, emit param names.
        let params: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
        let params_str = params.join(", ");

        let default_val = elixir_stub_default(&method.return_type, &*defaults);

        // Elixir NIFs that may error wrap the result in `{:ok, value}`.
        let return_expr = if method.error_type.is_some() {
            format!("{{:ok, {default_val}}}")
        } else {
            default_val
        };

        if params_str.is_empty() {
            let _ = writeln!(module_defs, "  def {}, do: {return_expr}", method.name);
        } else {
            let _ = writeln!(module_defs, "  def {}({params_str}), do: {return_expr}", method.name);
        }
    }

    let _ = writeln!(module_defs, "end");
    let _ = writeln!(module_defs, "end");

    // Emit the GenServer wrapper that Rustler NIFs can call via PID message passing.
    // Messages arrive as {:trait_call, method_atom, args, reply_id}, where `args` is a native
    // Erlang map (the callback args are sent as native terms, not a JSON string).
    // The GenServer calls the stub module method, serializes the result to JSON, and
    // passes it back to the NIF's complete_trait_call/2 which unblocks the waiting Rust thread.
    let _ = writeln!(module_defs, "unless Code.ensure_loaded?({genserver_module}) do");
    let _ = writeln!(module_defs, "defmodule {genserver_module} do");
    let _ = writeln!(module_defs, "  use GenServer");
    let _ = writeln!(module_defs);
    let _ = writeln!(module_defs, "  def start_link(_opts) do");
    let _ = writeln!(module_defs, "    GenServer.start_link(__MODULE__, nil)");
    let _ = writeln!(module_defs, "  end");
    let _ = writeln!(module_defs);
    let _ = writeln!(module_defs, "  @impl true");
    let _ = writeln!(module_defs, "  def init(_), do: {{:ok, nil}}");
    let _ = writeln!(module_defs);
    let _ = writeln!(module_defs, "  @impl true");
    let _ = writeln!(
        module_defs,
        "  def handle_info({{:trait_call, method_atom, args, reply_id}}, state) do"
    );
    let _ = writeln!(module_defs, "    method_name = to_string(method_atom)");
    let _ = writeln!(
        module_defs,
        "    ordered_args = __alef_ordered_args__(method_name, args)"
    );
    let _ = writeln!(
        module_defs,
        "    result = apply({qualified_module}, String.to_existing_atom(method_name), ordered_args)"
    );
    let _ = writeln!(module_defs, "    result_json = Jason.encode!(result)");
    let _ = writeln!(
        module_defs,
        "    {effective_nif_module}.complete_trait_call(reply_id, result_json)"
    );
    let _ = writeln!(module_defs, "    {{:noreply, state}}");
    let _ = writeln!(module_defs, "  end");
    let _ = writeln!(module_defs);
    for method in methods {
        let args = method
            .params
            .iter()
            .map(|p| format!("args[\"{}\"]", p.name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            module_defs,
            "  defp __alef_ordered_args__(\"{}\", args), do: [{}]",
            method.name, args
        );
    }
    if trait_bridge.super_trait.is_some() {
        let _ = writeln!(module_defs, "  defp __alef_ordered_args__(\"version\", _args), do: []");
        let _ = writeln!(
            module_defs,
            "  defp __alef_ordered_args__(\"initialize\", _args), do: []"
        );
        let _ = writeln!(module_defs, "  defp __alef_ordered_args__(\"shutdown\", _args), do: []");
    }
    let _ = writeln!(
        module_defs,
        "  defp __alef_ordered_args__(_method, args) when map_size(args) == 0, do: []"
    );
    let _ = writeln!(module_defs, "end");
    let _ = writeln!(module_defs, "end");

    // Emit the test-function-level code: start the GenServer and capture its PID.
    // This will be indented when rendered inside the test function.
    let pid_var = format!("{}_pid", pascal_id.to_lowercase());
    let mut test_setup = String::new();
    let _ = writeln!(test_setup, "{{:ok, {pid_var}}} = {genserver_module}.start_link(nil)");

    // Combine both parts with a separator so we can split them during rendering.
    // Use `\n__TRAIT_BRIDGE_MODULE_DEFS_END__\n` as a marker.
    let mut combined_setup = module_defs;
    combined_setup.push_str("\n__TRAIT_BRIDGE_MODULE_DEFS_END__\n");
    combined_setup.push_str(&test_setup);

    crate::e2e::codegen::TestBackendEmission {
        setup_block: combined_setup,
        arg_expr: pid_var,
        type_imports: Vec::new(),
        teardown_block: String::new(),
    }
}

#[cfg(test)]
mod test_backend_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str, required: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: !required,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    /// Verify that no sample_core-domain names leak into the generated output when
    /// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
    #[test]
    fn elixir_stub_contains_no_sample_crate_domain_names() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            !output.contains("SampleCrate"),
            "must not contain literal 'SampleCrate', got:\n{output}"
        );
        assert!(
            !output.contains("sample_crate::"),
            "must not contain 'sample_crate::', got:\n{output}"
        );
        assert!(
            !output.contains("SampleCrateBridge"),
            "must not contain 'SampleCrateBridge', got:\n{output}"
        );
        assert!(
            output.contains("TestStubMyTestFixture"),
            "module name must be derived from fixture id, got:\n{output}"
        );
        assert!(
            output.contains("def process"),
            "required method 'process' must be emitted, got:\n{output}"
        );
    }

    /// Verify that the defmodule is guarded with `Code.ensure_loaded?` to prevent
    /// redefinition warnings when the same compiled test file is loaded multiple times.
    #[test]
    fn elixir_stub_defmodule_guarded_against_redefinition() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("unless Code.ensure_loaded?"),
            "defmodule must be guarded with `unless Code.ensure_loaded?` to prevent redefine warnings, got:\n{output}"
        );
        // The module atom in the `unless` guard must match the arg_expr.
        assert!(
            emission.setup_block.contains(&emission.arg_expr),
            "setup_block must reference the same module atom as arg_expr, got:\narg_expr={}\nsetup_block={}",
            emission.arg_expr,
            emission.setup_block
        );
    }

    /// Verify that `fixture.input.<arg>.name` is used as the plugin name when present.
    /// Fixture structure: { "backend": { "name": "my-backend-name" } }
    #[test]
    fn elixir_stub_uses_fixture_input_name_for_plugin_name() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let mut fixture = make_fixture("my_fixture_id");
        fixture.input = serde_json::json!({ "backend": { "name": "my-backend-name" } });

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");
        let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

        assert!(
            output.contains("\"my-backend-name\""),
            "plugin name must come from fixture.input.<arg>.name, got:\n{output}"
        );
    }

    /// Verify that the module is emitted under the E2e.TestStubs namespace so it is
    /// well-scoped and does not pollute the top-level Elixir module namespace.
    #[test]
    fn elixir_stub_uses_scoped_namespace() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        assert!(
            emission.setup_block.contains("E2e.TestStubs."),
            "setup_block must reference E2e.TestStubs namespace, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that a GenServer is emitted to wrap the stub module so Rustler NIFs
    /// can call trait methods via PID message passing.
    #[test]
    fn elixir_stub_emits_genserver_wrapper() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        assert!(
            emission.setup_block.contains("defmodule") && emission.setup_block.contains("GenServer"),
            "setup_block must define a GenServer module, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("handle_info"),
            "GenServer must implement handle_info for trait_call messages, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("complete_trait_call"),
            "GenServer must reply via the NIF complete_trait_call/2, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission
                .setup_block
                .contains("ordered_args = __alef_ordered_args__(method_name, args)")
                && emission.setup_block.contains(
                    "apply(E2e.TestStubs.TestStubMyTestFixture, String.to_existing_atom(method_name), ordered_args)"
                ),
            "GenServer must convert the native args map into ordered apply/3 args, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission
                .setup_block
                .contains("{:trait_call, method_atom, args, reply_id}")
                && !emission.setup_block.contains("Jason.decode!(args_json)"),
            "GenServer must receive callback args as a native map, not Jason.decode! a JSON string, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn elixir_stub_orders_callback_args_by_method_signature() {
        let bridge = make_trait_bridge("TestTrait");
        let mut required_method = make_method("process", true);
        required_method.params = vec![
            crate::core::ir::ParamDef {
                name: "first".to_string(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            },
            crate::core::ir::ParamDef {
                name: "second".to_string(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            },
        ];
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        assert!(
            emission
                .setup_block
                .contains("defp __alef_ordered_args__(\"process\", args), do: [args[\"first\"], args[\"second\"]]"),
            "GenServer must emit method-specific ordered args, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that arg_expr is a PID variable, not a module name.
    /// This allows Rustler NIFs to receive the PID and send messages to it.
    #[test]
    fn elixir_stub_arg_expr_is_pid_variable() {
        let bridge = make_trait_bridge("TestTrait");
        let required_method = make_method("process", true);
        let methods = [&required_method];
        let fixture = make_fixture("my_test_fixture");

        let emission = emit_test_backend(&bridge, &methods, &fixture, "");

        // arg_expr should be a lowercase variable name like "my_test_fixture_pid", not a module atom
        assert!(
            !emission.arg_expr.contains("."),
            "arg_expr must be a PID variable (not a module atom), got:\n{}",
            emission.arg_expr
        );
        assert!(
            emission.arg_expr.ends_with("_pid"),
            "arg_expr must end with _pid to indicate it is a process identifier, got:\n{}",
            emission.arg_expr
        );
        assert!(
            emission
                .setup_block
                .contains(&format!("{{:ok, {}}}", emission.arg_expr)),
            "setup_block must start GenServer and assign its PID to the arg_expr variable, got:\n{}",
            emission.setup_block
        );
    }
}
