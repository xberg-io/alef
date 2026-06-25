use crate::backends::magnus::type_map::rbs_type;
use crate::codegen::shared::binding_fields;
use crate::core::config::TraitBridgeConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, MethodDef, TypeDef};

pub fn gen_stubs(
    api: &ApiSurface,
    gem_name: &str,
    emit_docstrings: bool,
    streaming_method_names: &ahash::AHashSet<String>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut lines: Vec<String> = header.lines().map(str::to_string).collect();
    lines.push("".to_string());

    let module_name = get_module_name(gem_name);
    lines.push(format!("module {}", module_name));
    lines.push("".to_string());
    lines.push("  VERSION: String".to_string());
    lines.push("".to_string());
    // Type alias for JSON values: any JSON-compatible type
    lines.push(
        "  type json_value = Hash[String, untyped] | Array[untyped] | String | Integer | Float | bool | nil"
            .to_string(),
    );
    lines.push("".to_string());

    // Generate type stubs
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque {
            lines.push(gen_opaque_type_stub(typ, emit_docstrings, streaming_method_names));
            lines.push("".to_string());
        } else {
            lines.push(gen_type_stub(typ, emit_docstrings, streaming_method_names));
            lines.push("".to_string());
        }
    }

    // Generate enum stubs
    for enum_def in &api.enums {
        lines.push(gen_enum_stub(enum_def, emit_docstrings));
        lines.push("".to_string());
    }

    // Generate function stubs (module methods)
    for func in &api.functions {
        lines.push(gen_function_stub(func, streaming_method_names));
        lines.push("".to_string());
    }
    // Emit a host-implementable RBS `interface` for each plugin-pattern trait bridge (those with
    // a `register_*` function) whose trait is resolvable in the API surface. This surfaces the
    // typed protocol a host backend must implement to be registered, rather than leaving callers
    // with an untyped `backend`. Interface method params that are known serde structs are typed as
    // their native struct type and returns as the result type, matching the native Ruby values the
    // runtime bridge now passes/expects.
    //
    // Track the trait names that received an interface so the `register_*` signature below can type
    // its `backend` parameter against the interface instead of `untyped`.
    let mut interface_trait_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for bridge in trait_bridges {
        if bridge.register_fn.is_none() {
            continue;
        }
        if let Some(stub) = gen_plugin_interface_stub(bridge, api) {
            lines.push(stub);
            lines.push("".to_string());
            interface_trait_names.insert(bridge.trait_name.clone());
        }
    }
    for bridge in trait_bridges {
        if let Some(register_fn) = bridge.register_fn.as_deref() {
            // Type the `backend` param against the host-implementable interface when one was
            // emitted for this bridge's trait; otherwise fall back to `untyped`.
            let backend_type = if interface_trait_names.contains(&bridge.trait_name) {
                plugin_interface_name(&bridge.trait_name)
            } else {
                "untyped".to_string()
            };
            lines.push(format!(
                "  def self.{register_fn}: ({backend_type} backend, String name) -> nil"
            ));
            lines.push("".to_string());
        }
        if let Some(unregister_fn) = bridge.unregister_fn.as_deref() {
            lines.push(format!("  def self.{unregister_fn}: (String name) -> nil"));
            lines.push("".to_string());
        }
        if let Some(clear_fn) = bridge.clear_fn.as_deref() {
            lines.push(format!("  def self.{clear_fn}: () -> nil"));
            lines.push("".to_string());
        }
    }

    // Generate error info class stubs for errors with introspection methods.
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

    let interface_name = plugin_interface_name(&bridge.trait_name);
    let mut lines = vec![format!("  interface {interface_name}")];

    for method in methods {
        if method.binding_excluded {
            continue;
        }
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let param_type = rbs_type(&p.ty);
                if p.optional {
                    format!("?{} {}", param_type, p.name)
                } else {
                    format!("{} {}", param_type, p.name)
                }
            })
            .collect();
        let return_type = rbs_type(&method.return_type);
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
    streaming_method_names: &ahash::AHashSet<String>,
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

    // Instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(method, false, emit_docstrings, streaming_method_names));
        }
    }

    // Static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(method, true, emit_docstrings, streaming_method_names));
        }
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Generate a Ruby type stub for a struct.
fn gen_type_stub(typ: &TypeDef, emit_docstrings: bool, streaming_method_names: &ahash::AHashSet<String>) -> String {
    let mut lines = vec![];

    lines.push(format!("  class {}", typ.name));

    // Add docstring if present
    if emit_docstrings && !typ.doc.is_empty() {
        let doc_lines: Vec<String> = typ.doc.lines().map(ToString::to_string).collect();
        lines.push(crate::backends::magnus::template_env::render(
            "rbs_doc_block.jinja",
            minijinja::context! { doc_lines },
        ));
        lines.push("".to_string());
    }

    // Add field attr declarations — use attr_accessor for config types (has_default),
    // attr_reader for immutable result types.
    // For config types, all fields are optional (builder pattern).
    let accessor = if typ.has_default {
        "attr_accessor"
    } else {
        "attr_reader"
    };
    for f in binding_fields(&typ.fields) {
        let mut field_type = rbs_type(&f.ty);
        // Builder types have optional fields (attr_accessor allows setting/getting nil)
        if typ.has_default && !field_type.ends_with('?') {
            field_type.push('?');
        }
        // Field-level doc comment from the Rust source. Gated behind emit_docstrings.
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
        lines.push(format!(r#"    {accessor} {}: {field_type}"#, f.name));
    }

    if binding_fields(&typ.fields).next().is_some() {
        lines.push("".to_string());
    }

    // Add initialize method
    // For has_default types (config/builder), all fields are optional kwargs.
    // For result types, required fields are required kwargs, optional fields are optional.
    let init_params: Vec<String> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| {
            let field_type = rbs_type(&f.ty);
            if typ.has_default {
                // Config types: all fields are optional kwargs in Ruby (defaults applied in Rust)
                format!("?{}: {}", f.name, field_type)
            } else if f.optional {
                // Result types: optional fields are optional kwargs
                format!("?{}: {}", f.name, field_type)
            } else {
                // Result types: required fields are required kwargs
                format!("{}: {}", f.name, field_type)
            }
        })
        .collect();

    lines.push(format!("    def initialize: ({}) -> void", init_params.join(", ")));

    // Add instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(method, false, emit_docstrings, streaming_method_names));
        }
    }

    // Add static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(method, true, emit_docstrings, streaming_method_names));
        }
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Generate a method stub using RBS declaration syntax.
/// Streaming methods return Enumerator[ItemType] instead of String.
fn gen_method_stub(
    method: &MethodDef,
    is_static: bool,
    emit_docstrings: bool,
    streaming_method_names: &ahash::AHashSet<String>,
) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type = rbs_type(&p.ty);
            if p.optional {
                format!("?{} {}", param_type, p.name)
            } else {
                format!("{} {}", param_type, p.name)
            }
        })
        .collect();

    let return_type = if streaming_method_names.contains(&method.name) {
        // For streaming methods like crawl_stream, derive the iterator type name
        // from the method name (e.g., crawl_stream → CrawlStreamIterator)
        let pascal_name = method
            .name
            .split('_')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<String>();
        format!("Enumerator[{}Iterator]", pascal_name)
    } else {
        rbs_type(&method.return_type)
    };

    let param_list = format!("({})", params.join(", "));

    let sig_line = if is_static {
        format!("    def self.{}: {} -> {}", method.name, param_list, return_type)
    } else {
        format!("    def {}: {} -> {}", method.name, param_list, return_type)
    };

    // Prefix with the method's Rust doc comment, line by line. RBS allows free-form
    // comments preceding method declarations. Gated behind emit_docstrings.
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
fn gen_enum_stub(enum_def: &EnumDef, emit_docstrings: bool) -> String {
    let mut lines = vec![];

    // Always emit class stub (even for unit enums, for Ruby introspection)
    lines.push(format!("  class {}", enum_def.name));

    // Add docstring if present — gated behind emit_docstrings.
    if emit_docstrings && !enum_def.doc.is_empty() {
        let doc_lines: Vec<String> = enum_def.doc.lines().map(ToString::to_string).collect();
        lines.push(crate::backends::magnus::template_env::render(
            "rbs_doc_block.jinja",
            minijinja::context! { doc_lines },
        ));
    }

    // Check if enum has data (non-unit variants)
    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    if !has_data {
        // Unit enum: also emit as type alias with symbol union inside the class
        let symbol_variants: Vec<String> = enum_def
            .variants
            .iter()
            .map(|v| format!(":{}", crate::codegen::naming::pascal_to_snake(&v.name)))
            .collect();
        lines.push(format!("    type value = {}", symbol_variants.join(" | ")));
    } else {
        // Data enum: declare a singleton constructor per data-carrying variant so RBS sees the
        // `Shape.circle(...)` factories the runtime binding registers via define_singleton_method.
        gen_data_enum_variant_constructor_stubs(&mut lines, enum_def);
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Emit an RBS singleton-method declaration for each per-variant constructor the magnus binding
/// registers (`def self.<snake>: (<Type> <name>, ...) -> <Enum>`).
///
/// The runtime binding registers these under the bare snake_case host name, so the stub declares the
/// same name. Each param type maps through [`rbs_type`] — the same mapper the surrounding stub uses —
/// and the return type is the enum. `collect_variant_constructors` owns the skip rules (unit / tuple /
/// `binding_excluded` / sanitized-field variants and hand-written method collisions) so the stub and
/// runtime binding stay aligned.
fn gen_data_enum_variant_constructor_stubs(lines: &mut Vec<String>, enum_def: &EnumDef) {
    use crate::codegen::generators::collect_variant_constructors;

    for ctor in collect_variant_constructors(enum_def) {
        let params: Vec<String> = ctor
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                // A param is nilable in the emitted RBS signature when it is naturally optional OR was
                // promoted because it follows an optional param — the same rule the runtime magnus
                // binding applies (`is_promoted_optional`), which wraps such params in `Option<T>`.
                // Mirroring it keeps the stub's required/optional split identical to the runtime
                // constructor, and matches how `gen_function_stub` renders optional params (`?T name`).
                let optional = p.optional || crate::codegen::shared::is_promoted_optional(&ctor.params, idx);
                crate::backends::magnus::template_env::render(
                    "rbs_enum_variant_constructor_param.jinja",
                    minijinja::context! {
                        rbs_type => rbs_type(&p.ty),
                        name => &p.name,
                        optional => optional,
                    },
                )
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
fn gen_function_stub(func: &FunctionDef, streaming_method_names: &ahash::AHashSet<String>) -> String {
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let param_type = rbs_type(&p.ty);
            if p.optional {
                format!("?{} {}", param_type, p.name)
            } else {
                format!("{} {}", param_type, p.name)
            }
        })
        .collect();

    let return_type = if streaming_method_names.contains(&func.name) {
        // For streaming methods like batch_crawl_stream
        let pascal_name = func
            .name
            .split('_')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<String>();
        format!("Enumerator[{}Iterator]", pascal_name)
    } else {
        rbs_type(&func.return_type)
    };

    let param_list = format!("({})", params.join(", "));

    format!("  def self.{}: {} -> {}", func.name, param_list, return_type)
}

#[cfg(test)]
mod tests;
