use crate::type_map::rbs_type;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, EnumDef, FunctionDef, MethodDef, TypeDef};

pub fn gen_stubs(api: &ApiSurface, gem_name: &str) -> String {
    let header = hash::header(CommentStyle::Hash);
    let mut lines: Vec<String> = header.lines().map(str::to_string).collect();
    lines.push("".to_string());

    let module_name = get_module_name(gem_name);
    lines.push(format!("module {}", module_name));
    lines.push("".to_string());
    lines.push("  VERSION: String".to_string());
    lines.push("".to_string());

    // Generate type stubs
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque {
            lines.push(gen_opaque_type_stub(typ));
            lines.push("".to_string());
        } else {
            lines.push(gen_type_stub(typ));
            lines.push("".to_string());
        }
    }

    // Generate enum stubs
    for enum_def in &api.enums {
        lines.push(gen_enum_stub(enum_def));
        lines.push("".to_string());
    }

    // Generate function stubs (module methods)
    for func in &api.functions {
        lines.push(gen_function_stub(func));
        lines.push("".to_string());
    }

    lines.push("end".to_string());

    lines.join("\n")
}

/// Convert crate name to PascalCase module name.
fn get_module_name(crate_name: &str) -> String {
    crate_name
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

/// Generate a Ruby type stub for an opaque type (no fields, only methods).
fn gen_opaque_type_stub(typ: &TypeDef) -> String {
    let mut lines = vec![];

    lines.push(format!("  class {}", typ.name));

    if !typ.doc.is_empty() {
        for doc_line in typ.doc.lines() {
            lines.push(format!("    # {doc_line}"));
        }
        lines.push("".to_string());
    }

    // Instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(method, false));
        }
    }

    // Static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(method, true));
        }
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Generate a Ruby type stub for a struct.
fn gen_type_stub(typ: &TypeDef) -> String {
    let mut lines = vec![];

    lines.push(format!("  class {}", typ.name));

    // Add docstring if present
    if !typ.doc.is_empty() {
        for doc_line in typ.doc.lines() {
            lines.push(format!("    # {doc_line}"));
        }
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
    for f in &typ.fields {
        let mut field_type = rbs_type(&f.ty);
        // Builder types have optional fields (attr_accessor allows setting/getting nil)
        if typ.has_default && !field_type.ends_with('?') {
            field_type.push('?');
        }
        lines.push(format!(r#"    {accessor} {}: {field_type}"#, f.name));
    }

    if !typ.fields.is_empty() {
        lines.push("".to_string());
    }

    // Add initialize method
    let init_params: Vec<String> = typ
        .fields
        .iter()
        .map(|f| {
            let field_type = rbs_type(&f.ty);
            if f.optional {
                format!("?{}: {}", f.name, field_type)
            } else {
                format!("{}: {}", f.name, field_type)
            }
        })
        .collect();

    lines.push(format!("    def initialize: ({}) -> void", init_params.join(", ")));

    // Add instance methods
    for method in &typ.methods {
        if !method.is_static {
            lines.push(gen_method_stub(method, false));
        }
    }

    // Add static methods
    for method in &typ.methods {
        if method.is_static {
            lines.push(gen_method_stub(method, true));
        }
    }

    lines.push("  end".to_string());

    lines.join("\n")
}

/// Generate a method stub using RBS declaration syntax.
fn gen_method_stub(method: &MethodDef, is_static: bool) -> String {
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
    let param_list = format!("({})", params.join(", "));

    if is_static {
        format!("    def self.{}: {} -> {}", method.name, param_list, return_type)
    } else {
        format!("    def {}: {} -> {}", method.name, param_list, return_type)
    }
}

/// Generate a Ruby enum stub.
/// Unit-variant enums are represented as Ruby Symbols (e.g., :left_to_right).
/// RBS stubs are minimal — actual return types use symbol unions in method signatures.
fn gen_enum_stub(enum_def: &EnumDef) -> String {
    // Empty class stub — the actual type is expressed where it's used (method returns, fields).
    // RBS does not support standalone type declarations for symbol unions; they must be
    // inline in type annotations or use class hierarchy.
    format!("  class {}\n  end", enum_def.name)
}

/// Convert PascalCase to snake_case for symbol names.
fn pascal_to_snake(name: &str) -> String {
    let mut result = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_lowercase().next().unwrap_or(ch));
    }
    result
}

/// Generate a function stub (module method) using RBS declaration syntax.
fn gen_function_stub(func: &FunctionDef) -> String {
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

    let return_type = rbs_type(&func.return_type);
    let param_list = format!("({})", params.join(", "));

    format!("  def self.{}: {} -> {}", func.name, param_list, return_type)
}
