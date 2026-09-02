use crate::backends::magnus::type_map::{rbs_marshalled_type, rbs_type};
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{VariantDeclaration, enum_variant_declaration};
use crate::codegen::shared::{binding_fields, substitute_excluded_types, substitute_trait_interfaces};
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, MethodDef, TypeDef, TypeRef};

pub fn gen_stubs(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    gem_name: &str,
    emit_docstrings: bool,
    streaming_return_types: &ahash::AHashMap<String, String>,
    trait_bridges: &[TraitBridgeConfig],
    client_constructor_types: &std::collections::HashSet<&str>,
) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut lines: Vec<String> = header.lines().map(str::to_string).collect();
    lines.push("".to_string());

    let module_name = get_module_name(gem_name);
    lines.push(format!("module {}", module_name));
    lines.push("".to_string());
    lines.push("  VERSION: String".to_string());
    lines.push("".to_string());
    lines.push(
        "  type json_value = Hash[String, untyped] | Array[untyped] | String | Integer | Float | bool | nil"
            .to_string(),
    );
    lines.push("".to_string());

    // Types excluded from the binding surface (opaque `alef(skip)` traits, binding-excluded ~keep
    // structs) have no RBS declaration, so any signature referencing them is substituted to the ~keep
    // declared `json_value` alias — otherwise steep fails with `RBS::UnknownTypeName`. ~keep
    let excluded: std::collections::HashSet<&str> = api
        .excluded_type_paths
        .keys()
        .map(String::as_str)
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect();

    // Trait names exposed as a host-implementable RBS `interface _Name` (trait-bridge traits with a ~keep
    // `register_fn` whose methods are non-empty and whose trait type exists in the API surface — the ~keep
    // same condition `gen_plugin_interface_stub` uses to decide whether it emits the interface). Any ~keep
    // function/method param or return referencing one of these bare trait names must be substituted ~keep
    // to `_Name` via `substitute_trait_interfaces`, otherwise steep fails with `RBS::UnknownTypeName` ~keep
    // (the bare trait name is never declared — only the `_Name` interface is). ~keep
    let trait_interfaces: std::collections::HashSet<&str> = trait_bridges
        .iter()
        .filter(|bridge| bridge.register_fn.is_some())
        .filter(|bridge| !bridge.resolve_methods(api).is_empty())
        .filter(|bridge| api.types.iter().any(|t| t.name == bridge.trait_name))
        .map(|bridge| bridge.trait_name.as_str())
        .collect();

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque {
            lines.push(gen_opaque_type_stub(
                typ,
                emit_docstrings,
                streaming_return_types,
                &excluded,
                &trait_interfaces,
                client_constructor_types,
            ));
            lines.push("".to_string());
        } else {
            lines.push(gen_type_stub(
                typ,
                emit_docstrings,
                streaming_return_types,
                &excluded,
                &trait_interfaces,
            ));
            lines.push("".to_string());
        }
    }

    let core_import = config.core_import_name();
    let enabled_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Ruby);
    let configured_features_set: std::collections::HashSet<&str> =
        enabled_features.iter().map(String::as_str).collect();
    for enum_def in &api.enums {
        let is_host_enum = is_host_owned_rust_path(&core_import, &enum_def.rust_path);
        lines.push(gen_enum_stub(
            enum_def,
            emit_docstrings,
            is_host_enum,
            &configured_features_set,
        ));
        lines.push("".to_string());
    }

    for func in &api.functions {
        lines.push(gen_function_stub(
            func,
            streaming_return_types,
            &excluded,
            &trait_interfaces,
        ));
        lines.push("".to_string());
    }
    for bridge in trait_bridges {
        if bridge.register_fn.is_none() {
            continue;
        }
        if let Some(stub) = gen_plugin_interface_stub(bridge, api) {
            lines.push(stub);
            lines.push("".to_string());
        }
    }

    if !config.components.is_empty() {
        lines.extend([
            "  def self.component_load: (String component) -> nil".to_string(),
            "".to_string(),
            "  def self.component_prefetch: ((Array[String] | nil) components) -> Array[String]".to_string(),
            "".to_string(),
            "  def self.component_status: (String component) -> String".to_string(),
            "".to_string(),
            "  def self.component_cache_path: (String component) -> String".to_string(),
            "".to_string(),
        ]);
    }
    let declared_function_names: std::collections::HashSet<&str> =
        api.functions.iter().map(|f| f.name.as_str()).collect();
    for bridge in trait_bridges {
        // The RBS declares the module functions `gen_module_init` binds, and that emitter skips a
        // bridge this returns `None` for — declaring one it skipped describes a method Ruby never
        // receives. ~keep
        if crate::backends::magnus::trait_bridge::active_bridge_trait(bridge, api).is_none() {
            continue;
        }
        if let Some(register_fn) = bridge.register_fn.as_deref()
            && !declared_function_names.contains(register_fn)
        {
            let backend_type = if trait_interfaces.contains(bridge.trait_name.as_str()) {
                plugin_interface_name(&bridge.trait_name)
            } else {
                "untyped".to_string()
            };
            lines.push(format!(
                "  def self.{register_fn}: ({backend_type} backend, String name) -> nil"
            ));
            lines.push("".to_string());
        }
        if let Some(unregister_fn) = bridge.unregister_fn.as_deref()
            && !declared_function_names.contains(unregister_fn)
        {
            lines.push(format!("  def self.{unregister_fn}: (String name) -> nil"));
            lines.push("".to_string());
        }
        if let Some(clear_fn) = bridge.clear_fn.as_deref()
            && !declared_function_names.contains(clear_fn)
        {
            lines.push(format!("  def self.{clear_fn}: () -> nil"));
            lines.push("".to_string());
        }
    }

    for error in api.errors.iter().filter(|e| !e.methods.is_empty()) {
        let class_name = format!("{}Info", error.name);
        let mut class_lines = vec![format!("  class {class_name}")];
        for method in &error.methods {
            let (rbs_name, rbs_ret): (&str, &str) = match method.name.as_str() {
                "status_code" => ("status_code", "Integer"),
                "is_transient" => ("transient?", "bool"),
                "error_type" => ("error_type", "String"),
                _ => continue,
            };
            class_lines.push(format!("    def {rbs_name}: () -> {rbs_ret}"));
        }
        class_lines.push("  end".to_string());
        lines.push(class_lines.join("\n"));
        lines.push("".to_string());
    }

    lines.push("end".to_string());

    lines.join("\n")
}

/// RBS interface name for a plugin-bridge trait. RBS requires interface names to begin with an
/// underscore (e.g. trait `Greeter` → interface `_Greeter`).
fn plugin_interface_name(trait_name: &str) -> String {
    format!("_{trait_name}")
}

/// Generate a host-implementable RBS `interface` for a plugin-pattern trait bridge.
///
/// Returns `None` when the bridge's trait (or its methods) is absent from the API surface — the
/// caller then falls back to `untyped` for the `register_*` backend param so the stub still
/// type-checks.
///
/// Method signatures come from [`TraitBridgeConfig::resolve_methods`], the same source the
/// trait-bridge code generator uses to emit the runtime vtable, so the interface surface matches
/// the methods the bridge actually forwards through Magnus. Each param's RBS type is its native
/// type via [`rbs_type`]: known serde structs surface as their struct type (matching the native
/// Ruby value the runtime now passes), and the return is the method's result type.
fn gen_plugin_interface_stub(bridge: &TraitBridgeConfig, api: &ApiSurface) -> Option<String> {
    let methods = bridge.resolve_methods(api);
    if methods.is_empty() {
        return None;
    }
    api.types.iter().find(|t| t.name == bridge.trait_name)?;

    let excluded: std::collections::HashSet<&str> = api
        .excluded_type_paths
        .keys()
        .map(String::as_str)
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect();

    let interface_name = plugin_interface_name(&bridge.trait_name);
    let mut lines = vec![format!("  interface {interface_name}")];

    let (required, optional): (Vec<&crate::core::ir::MethodDef>, Vec<&crate::core::ir::MethodDef>) =
        methods.iter().partition(|m| !m.has_default_impl);
    if !optional.is_empty() {
        let names: Vec<&str> = optional.iter().map(|m| m.name.as_str()).collect();
        lines.push(format!(
            "    # Optional methods the bridge calls when the object defines them (the
    # trait's Rust default behavior applies otherwise): {}.
    # The lifecycle hooks `initialize`/`shutdown` are likewise optional (note:
    # Ruby constructors are private and never count as the lifecycle hook).",
            names.join(", ")
        ));
    }

    for method in required {
        if method.binding_excluded {
            continue;
        }
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let param_type = rbs_type(&substitute_excluded_types(&p.ty, &excluded));
                if p.optional {
                    format!("?{} {}", param_type, p.name)
                } else {
                    format!("{} {}", param_type, p.name)
                }
            })
            .collect();
        let return_type = rbs_type(&substitute_excluded_types(&method.return_type, &excluded));
        lines.push(format!(
            "    def {}: ({}) -> {}",
            method.name,
            params.join(", "),
            return_type
        ));
    }

    lines.push("  end".to_string());
    Some(lines.join("\n"))
}

/// Convert crate name to PascalCase module name. Handles both kebab- and
/// snake_case (matches `gen_bindings::get_module_name`).
fn get_module_name(crate_name: &str) -> String {
    use heck::ToUpperCamelCase;
    crate_name.to_upper_camel_case()
}

/// Generate a Ruby type stub for an opaque type (no fields, only methods).
fn gen_opaque_type_stub(
    typ: &TypeDef,
    emit_docstrings: bool,
    streaming_return_types: &ahash::AHashMap<String, String>,
    excluded: &std::collections::HashSet<&str>,
    trait_interfaces: &std::collections::HashSet<&str>,
    client_constructor_types: &std::collections::HashSet<&str>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("  class {}", typ.name));

    if emit_docstrings && !typ.doc.is_empty() {
        let doc_lines: Vec<String> = typ.doc.lines().map(ToString::to_string).collect();
        lines.push(crate::backends::magnus::template_env::render(
            "rbs_doc_block.jinja",
            minijinja::context! { doc_lines },
        ));
        lines.push("".to_string());
    }

    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(
                method,
                false,
                emit_docstrings,
                streaming_return_types,
                excluded,
                trait_interfaces,
                &typ.name,
            ));
        }
    }

    // An opaque type's static methods have no generated wrapper fn in general —
    // `gen_opaque_struct_methods` (classes/mod.rs) skips every `is_static` method with no
    // else branch. The sole exception is the `new` constructor of a variant-wrapper type:
    // `magnus_variant_wrapper_constructor` (tagged_enums.rs) emits a real `pub fn new(...)`
    // for it, and `module_init::gen_module_init`'s `has_variant_wrapper_ctor` branch is the
    // only path that registers a static method here. Mirror that exact condition — stubbing
    // any other static method promises a Ruby method the runtime binding never defines
    // (steep type-checks against a method that raises `NoMethodError` at runtime). ~keep
    let has_variant_wrapper_ctor = typ.is_variant_wrapper
        && !client_constructor_types.contains(typ.name.as_str())
        && typ.methods.iter().any(|m| m.name == "new" && m.receiver.is_none());
    if has_variant_wrapper_ctor
        && let Some(ctor_method) = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())
    {
        lines.push(gen_method_stub(
            ctor_method,
            true,
            emit_docstrings,
            streaming_return_types,
            excluded,
            trait_interfaces,
            &typ.name,
        ));
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Generate a Ruby type stub for a struct.
/// The RBS type of a generated field accessor, mirroring `classes::gen_field_accessor`.
///
/// ~keep Nullability follows the accessor, not the owning type's `has_default`. The stub used to
/// append `?` to every field of a defaulted struct, on the reasoning that such a struct can be
/// built empty — but the accessor derives its own optionality from `field.optional`, so a
/// non-optional collection on a defaulted struct really does return a bare `Vec<String>` that can
/// never be nil. Declaring it nilable forces callers into a nil branch that cannot be reached, and
/// steep flags the unreachable code they write to satisfy it.
fn rbs_accessor_type(field: &crate::core::ir::FieldDef) -> String {
    let base = match &field.ty {
        TypeRef::Optional(inner) => rbs_marshalled_type(inner),
        ty => rbs_marshalled_type(ty),
    };
    let is_optional = field.optional || matches!(field.ty, TypeRef::Optional(_));
    if is_optional && !base.ends_with('?') {
        format!("{base}?")
    } else {
        base
    }
}

fn gen_type_stub(
    typ: &TypeDef,
    emit_docstrings: bool,
    streaming_return_types: &ahash::AHashMap<String, String>,
    excluded: &std::collections::HashSet<&str>,
    trait_interfaces: &std::collections::HashSet<&str>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("  class {}", typ.name));

    if emit_docstrings && !typ.doc.is_empty() {
        let doc_lines: Vec<String> = typ.doc.lines().map(ToString::to_string).collect();
        lines.push(crate::backends::magnus::template_env::render(
            "rbs_doc_block.jinja",
            minijinja::context! { doc_lines },
        ));
        lines.push("".to_string());
    }

    // ~keep Always a reader. `attr_accessor` additionally declares `name=`, and the binding
    // defines no writer for any field: `gen_field_accessor` emits one zero-arity getter, and
    // `module_class_method_register` is the only registration template, so nothing ever reaches
    // Ruby as a setter. Keying this off `has_default` declared a writer on every field of every
    // defaulted struct — steep then accepts an assignment that raises `NoMethodError` at runtime,
    // which is exactly the stub-vs-extension drift these files exist to prevent.
    let accessor = "attr_reader";
    let mut emitted_attr_names: ahash::AHashSet<&str> = ahash::AHashSet::default();
    for f in binding_fields(&typ.fields) {
        let field_type = rbs_accessor_type(f);
        if emit_docstrings && !f.doc.is_empty() {
            for line in f.doc.lines() {
                let line = line.trim();
                if line.is_empty() {
                    lines.push("    #".to_string());
                } else {
                    lines.push(format!("    # {line}"));
                }
            }
        }
        emitted_attr_names.insert(f.name.as_str());
        lines.push(format!(r#"    {accessor} {}: {field_type}"#, f.name));
    }

    if binding_fields(&typ.fields).next().is_some() {
        lines.push("".to_string());
    }

    let init_params: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| {
            // ~keep Same marshalling as the accessors. The kwargs constructor converts each field
            // with `<mapped type>::try_convert`, and `MagnusMapper` maps Json to String, so a Json
            // keyword takes a serialized String. Declaring `json_value` here let steep accept
            // `Foo.new(data: {a: 1})`, where `String::try_convert` fails and the field silently
            // falls back to its default instead of raising — a wrong value, not an error.
            let field_type = rbs_marshalled_type(&f.ty);
            if typ.has_default {
                format!("?{}: {}", f.name, field_type)
            } else if f.optional {
                format!("?{}: {}", f.name, field_type)
            } else {
                format!("{}: {}", f.name, field_type)
            }
        })
        .collect();

    lines.push(format!("    def initialize: ({}) -> void", init_params.join(", ")));

    for method in &typ.methods {
        if !method.is_static {
            // `attr_accessor providers` already declares `providers`, so a same-named method
            // stub makes the class carry two definitions of it — RBS rejects that with
            // `RBS::DuplicatedMethodDefinition`. The attribute is declared first and wins,
            // matching what the magnus binding emits (see `classes::gen_struct_methods`). ~keep
            if emitted_attr_names.contains(method.name.as_str()) {
                continue;
            }
            lines.push(gen_method_stub(
                method,
                false,
                emit_docstrings,
                streaming_return_types,
                excluded,
                trait_interfaces,
                &typ.name,
            ));
        }
    }

    // A non-opaque struct's static methods have no generated wrapper fn: `gen_struct_methods`
    // (classes/mod.rs) skips every `is_static` method with no else branch. Its `new` is a
    // synthesized kwargs constructor built from the struct's fields, unconditionally emitted
    // and already declared above via `def initialize` — it is never sourced from `typ.methods`.
    // So no static method here is ever backed by a registered runtime binding; stubbing one
    // would promise a Ruby method the runtime never defines (steep type-checks against a
    // method that raises `NoMethodError` at runtime). ~keep

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Like [`substitute_excluded_types`], but never substitutes `Named(owner_type_name)`.
///
/// A type is only added to `excluded` (in [`gen_stubs`]) when it has no RBS class
/// declaration of its own — e.g. an `alef(skip)` trait, or a struct managed entirely by
/// another codegen pass (services). But a service owner type still gets a `class Owner`
/// stub emitted by [`gen_opaque_type_stub`]/[`gen_type_stub`] right here, so a method
/// declared *on* that class returning `Self` (already resolved to `Named(owner_type_name)`
/// during extraction — see `resolve_self_refs`) must reference the real, just-declared
/// class rather than fall back to `json_value`.
fn substitute_excluded_types_except_owner(
    ty: &TypeRef,
    excluded: &std::collections::HashSet<&str>,
    owner_type_name: &str,
) -> TypeRef {
    match ty {
        TypeRef::Named(name) if name == owner_type_name => ty.clone(),
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_excluded_types_except_owner(
            inner,
            excluded,
            owner_type_name,
        ))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_excluded_types_except_owner(
            inner,
            excluded,
            owner_type_name,
        ))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(substitute_excluded_types_except_owner(k, excluded, owner_type_name)),
            Box::new(substitute_excluded_types_except_owner(v, excluded, owner_type_name)),
        ),
        other => substitute_excluded_types(other, excluded),
    }
}

/// Generate a method stub using RBS declaration syntax.
/// Streaming methods return Enumerator[ItemType] instead of String.
///
/// `owner_type_name` is the name of the class this method is declared on. It is never
/// substituted via `excluded`, even when the owning type is `binding_excluded` (e.g. a
/// service owner type managed by the services extraction pass): a class stub for it is
/// being emitted right here, so `Self`-returning methods (already resolved to the owning
/// type's name during extraction) must reference that real, declared class rather than
/// falling back to `json_value`.
fn gen_method_stub(
    method: &MethodDef,
    is_static: bool,
    emit_docstrings: bool,
    streaming_return_types: &ahash::AHashMap<String, String>,
    excluded: &std::collections::HashSet<&str>,
    trait_interfaces: &std::collections::HashSet<&str>,
    owner_type_name: &str,
) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type = rbs_type(&substitute_trait_interfaces(
                &substitute_excluded_types_except_owner(&p.ty, excluded, owner_type_name),
                trait_interfaces,
            ));
            if p.optional {
                format!("?{} {}", param_type, p.name)
            } else {
                format!("{} {}", param_type, p.name)
            }
        })
        .collect();

    let return_type = if let Some(item_type) = streaming_return_types.get(&method.name) {
        format!("Enumerator[{item_type}]")
    } else {
        rbs_type(&substitute_trait_interfaces(
            &substitute_excluded_types_except_owner(&method.return_type, excluded, owner_type_name),
            trait_interfaces,
        ))
    };

    let param_list = format!("({})", params.join(", "));

    let sig_line = if is_static {
        format!("    def self.{}: {} -> {}", method.name, param_list, return_type)
    } else {
        format!("    def {}: {} -> {}", method.name, param_list, return_type)
    };

    if !emit_docstrings || method.doc.is_empty() {
        return sig_line;
    }
    let mut out = String::new();
    let doc_lines = method.doc.lines().map(str::trim).collect::<Vec<_>>();
    out.push_str(&crate::backends::magnus::template_env::render(
        "rbs_doc_block.jinja",
        minijinja::context! {
            doc_lines => doc_lines,
        },
    ));
    out.push_str(&sig_line);
    out
}

/// Generate a Ruby enum stub.
/// Unit-variant enums are represented as Ruby Symbols (e.g., :left_to_right).
/// RBS stubs are minimal — actual return types use symbol unions in method signatures.
fn gen_enum_stub(
    enum_def: &EnumDef,
    emit_docstrings: bool,
    is_host_enum: bool,
    configured_features: &std::collections::HashSet<&str>,
) -> String {
    let mut lines = vec![];

    lines.push(format!("  class {}", enum_def.name));

    if emit_docstrings && !enum_def.doc.is_empty() {
        let doc_lines: Vec<String> = enum_def.doc.lines().map(ToString::to_string).collect();
        lines.push(crate::backends::magnus::template_env::render(
            "rbs_doc_block.jinja",
            minijinja::context! { doc_lines },
        ));
    }

    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if !has_data {
        let symbol_variants: Vec<String> = enum_def
            .variants
            .iter()
            .map(|v| {
                let wire_name = crate::codegen::naming::wire_variant_value(
                    &v.name,
                    v.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                );
                crate::backends::magnus::ruby_symbol_literal(&wire_name)
            })
            .collect();
        lines.push(format!("    type value = {}", symbol_variants.join(" | ")));
    } else if enum_def.serde_tag.is_none() {
        gen_data_enum_variant_constructor_stubs(&mut lines, enum_def, is_host_enum, configured_features);
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Emit an RBS singleton-method declaration for each per-variant constructor the magnus binding
/// registers (`def self.<snake>: (<Type> <name>, ...) -> <Enum>`).
///
/// The runtime binding registers these under the bare snake_case host name, so the stub declares the
/// same name. Each param type maps through [`rbs_type`] — the same mapper the surrounding stub uses —
/// and the return type is the enum. `collect_all_variant_constructors` owns the skip rules (unit /
/// tuple / `binding_excluded` / sanitized-field variants) so the stub and runtime binding stay
/// aligned — including for a variant whose snake_case name collides with a hand-written
/// `impl EnumType { .. }` method, since the runtime binding never forwards that method either.
///
/// `is_host_enum`/`configured_features` additionally narrow the stub to the SAME
/// [`enum_variant_declaration`] verdict `classes::gen_enum::gen_data_enum_variant_constructors`
/// consults for the runtime factory: a FOREIGN cfg-gated variant the runtime drops must not stay
/// documented here, or steep would accept a call the extension never registers.
fn gen_data_enum_variant_constructor_stubs(
    lines: &mut Vec<String>,
    enum_def: &EnumDef,
    is_host_enum: bool,
    configured_features: &std::collections::HashSet<&str>,
) {
    use crate::codegen::generators::collect_all_variant_constructors;

    let declared_names: std::collections::HashSet<&str> = enum_def
        .variants
        .iter()
        .filter(|v| {
            let verdict = enum_variant_declaration(v, is_host_enum, Some(configured_features));
            !matches!(verdict, VariantDeclaration::Drop)
        })
        .map(|v| v.name.as_str())
        .collect();

    for ctor in collect_all_variant_constructors(enum_def)
        .into_iter()
        .filter(|ctor| declared_names.contains(ctor.variant_name))
    {
        let params: Vec<String> = ctor
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let optional = p.optional || crate::codegen::shared::is_promoted_optional(&ctor.params, idx);
                crate::backends::magnus::template_env::render(
                    "rbs_enum_variant_constructor_param.jinja",
                    minijinja::context! {
                        rbs_type => rbs_type(&p.ty),
                        name => &p.name,
                        optional => optional,
                    },
                )
                .trim_end()
                .to_string()
            })
            .collect();
        lines.push(crate::backends::magnus::template_env::render(
            "rbs_enum_variant_constructor.jinja",
            minijinja::context! {
                method_name => &ctor.snake_name,
                params => params.join(", "),
                return_type => &enum_def.name,
            },
        ));
    }
}

/// Generate a function stub (module method) using RBS declaration syntax.
fn gen_function_stub(
    func: &FunctionDef,
    streaming_return_types: &ahash::AHashMap<String, String>,
    excluded: &std::collections::HashSet<&str>,
    trait_interfaces: &std::collections::HashSet<&str>,
) -> String {
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let param_type = rbs_type(&substitute_trait_interfaces(
                &substitute_excluded_types(&p.ty, excluded),
                trait_interfaces,
            ));
            if p.optional {
                format!("?{} {}", param_type, p.name)
            } else {
                format!("{} {}", param_type, p.name)
            }
        })
        .collect();

    let return_type = if let Some(item_type) = streaming_return_types.get(&func.name) {
        format!("Enumerator[{item_type}]")
    } else {
        rbs_type(&substitute_trait_interfaces(
            &substitute_excluded_types(&func.return_type, excluded),
            trait_interfaces,
        ))
    };

    let param_list = format!("({})", params.join(", "));

    let function_name = crate::backends::magnus::ruby_public_function_name(func);
    format!("  def self.{}: {} -> {}", function_name, param_list, return_type)
}

#[cfg(test)]
mod tests;
