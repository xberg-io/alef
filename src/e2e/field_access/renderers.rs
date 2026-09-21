use crate::codegen::naming::to_go_name;

use super::optional_renderers::{
    push_key_field_name, push_key_index_suffix, render_c, render_dart, render_kotlin_android, render_pascal_dot,
    render_php, render_r,
};
use super::types::{PathSegment, SwiftFirstClassMap};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::HashSet;

/// Render `key` as a double-quoted string literal.
///
/// `parse_path` yields map keys UNQUOTED, so each renderer adds the quoting its target language
/// wants — and every target alef emits map access for (Rust, Python, TS/JS, Go, Java, Kotlin,
/// Swift, Ruby, Elixir, R, C#, Dart, PHP, Zig, Gleam) accepts the same C-style backslash escapes
/// inside `"`. Interpolating the key raw instead lets a key containing `"` or a newline terminate
/// the literal early and emit code that does not parse. ~keep
pub(super) fn quoted_key_literal(key: &str) -> String {
    let mut literal = String::with_capacity(key.len() + 2);
    literal.push('"');
    for character in key.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            _ => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

pub(super) fn render_accessor(segments: &[PathSegment], language: &str, result_var: &str) -> String {
    match language {
        "rust" => render_rust(segments, result_var),
        "python" => render_dot_access(segments, result_var, "python"),
        "typescript" | "node" => render_typescript(segments, result_var),
        "wasm" => render_wasm(segments, result_var),
        "go" => render_go(segments, result_var),
        "java" => render_java(segments, result_var),
        "kotlin" => render_kotlin(segments, result_var),
        "kotlin_android" => render_kotlin_android(segments, result_var),
        "csharp" => render_pascal_dot(segments, result_var),
        "ruby" => render_dot_access(segments, result_var, "ruby"),
        "php" => render_php(segments, result_var),
        "elixir" => render_dot_access(segments, result_var, "elixir"),
        "r" => render_r(segments, result_var),
        "c" => render_c(segments, result_var),
        "swift" => render_swift(segments, result_var),
        "dart" => render_dart(segments, result_var),
        _ => render_dot_access(segments, result_var, language),
    }
}

/// Generate a Swift accessor expression.
///
/// Alef now emits first-class Swift structs (`public struct Foo: Codable { public let
/// id: String }`) for most DTO types, where fields are properties — property access
/// uses `.id` (no parens). The remaining typealias-to-opaque types (e.g. request
/// types with Vec/Map/Named fields that aren't first-class candidates) are accessed
/// via the swift-bridge-generated method-call syntax `.id()`, but in e2e tests these
/// typealias types are method inputs / streaming outputs rather than parents for
/// field-access chains, so property syntax works in practice. If a future e2e test
/// asserts on a field-access chain rooted in an opaque type, a per-type
/// `SwiftFirstClassMap` (analogous to `PhpGetterMap`) would be needed.
pub(super) fn render_swift(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField { name, index } => {
                if !name.is_empty() {
                    out.push('.');
                    out.push_str(&name.to_lower_camel_case());
                }
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".count");
            }
        }
    }
    out
}

/// Dispatches per-segment between property access (first-class Codable struct)
/// and method-call access (typealias-to-opaque RustBridge class). Uses the
/// `SwiftFirstClassMap` to track the current type as the path advances.
pub(super) fn render_swift_with_first_class_map(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    map: &SwiftFirstClassMap,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    let mut current_type: Option<String> = map.root_type.clone();
    // Once a chain crosses an `ArrayField` segment, every subsequent segment
    // operates on an element pulled from a `RustVec<T>` — and `RustVec[i]`
    // yields the OPAQUE `RustBridge.T` (whose fields are swift-bridge methods),
    // never the first-class Codable Swift struct `T`. swift-bridge generates
    // `RustVec` as a thin wrapper around the Rust vector, not as a converter
    // to the binding's first-class struct. Pin opaque (method-call) syntax
    // after the first index step so paths like `data[0].id` emit `.id()` even
    // when the Codable `Model` first-class struct also exists.
    let mut via_rust_vec = false;
    // Once a chain crosses an opaque (typealias-to-`RustBridge.X`) segment,
    // every subsequent accessor must also be opaque (method-call syntax). The
    // backend emits `public typealias X = RustBridge.X` when `X` fails the
    // `can_emit_first_class_struct` check (e.g. contains a non-unit enum, or a
    // field of a still-opaque type). Calling a method on `RustBridge.X` returns
    // the OPAQUE wrapper of the next type, never the first-class Codable
    // struct — even when that next type IS independently eligible for
    // first-class emission. Pin method-call syntax after the first opaque step
    // so paths like `metrics.total_lines` on opaque `ProcessResult` emit
    // `.metrics().totalLines()` (not `.metrics().totalLines`).
    let mut via_opaque = false;
    let total = segments.len();
    for (i, seg) in segments.iter().enumerate() {
        let is_leaf = i == total - 1;
        let property_syntax = !via_rust_vec && !via_opaque && map.is_first_class(current_type.as_deref());
        if !property_syntax {
            via_opaque = true;
        }
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                // Swift bindings (both first-class `public let` props and
                // swift-bridge method names) always use lowerCamelCase.
                out.push_str(&f.to_lower_camel_case());
                if !property_syntax {
                    out.push_str("()");
                }
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push('?');
                }
                current_type = map.advance(current_type.as_deref(), f);
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                let is_optional = optional_fields.contains(&path_so_far);
                // A root-array segment (`name` empty — the path itself starts with `[N]`,
                // e.g. `[0].id`) has nothing to name: `result_var` IS the array, so there is
                // no field/getter to emit before the index. ~keep
                if !name.is_empty() {
                    out.push('.');
                    out.push_str(&name.to_lower_camel_case());
                }
                let access = if property_syntax || name.is_empty() { "" } else { "()" };
                if is_optional {
                    out.push_str(&format!("{access}?[{index}]"));
                } else {
                    out.push_str(&format!("{access}[{index}]"));
                }
                push_key_index_suffix(&mut path_so_far, seg);
                // Indexing into a Vec<Named> yields a Named element — advance current_type.
                // Only pin opaque syntax when the array field was itself emitted in
                // method-call mode (i.e. it's a RustVec accessor). When the owning
                // type is first-class, the array IS a Swift `[T]` and indexing yields
                // the first-class `T` directly (also a Codable struct → property access).
                current_type = map.advance(current_type.as_deref(), name);
                if !property_syntax && !name.is_empty() {
                    via_rust_vec = true;
                }
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                let access = if property_syntax { "" } else { "()" };
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("{access}[{key}]"));
                } else {
                    out.push_str(&format!("{access}[{}]", quoted_key_literal(key)));
                }
                push_key_index_suffix(&mut path_so_far, seg);
                current_type = map.advance(current_type.as_deref(), field);
            }
            PathSegment::Length => {
                out.push_str(".count");
            }
        }
    }
    out
}

pub(super) fn render_rust(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_snake_case());
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&name.to_snake_case());
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_snake_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!(".get({}).map(|s| s.as_str())", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".len()");
            }
        }
    }
    out
}

pub(super) fn render_dot_access(segments: &[PathSegment], result_var: &str, language: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(f);
            }
            PathSegment::ArrayField { name, index } => {
                if language == "elixir" {
                    let current = std::mem::take(&mut out);
                    out = format!("Enum.at({current}.{name}, {index})");
                } else {
                    if !name.is_empty() {
                        out.push('.');
                        out.push_str(name);
                    }
                    out.push_str(&format!("[{index}]"));
                }
            }
            PathSegment::MapAccess { field, key } => {
                let is_numeric = key.chars().all(|c| c.is_ascii_digit());
                if is_numeric && language == "elixir" {
                    let current = std::mem::take(&mut out);
                    out = format!("Enum.at({current}.{field}, {key})");
                } else {
                    out.push('.');
                    out.push_str(field);
                    if is_numeric {
                        let idx: usize = key.parse().unwrap_or(0);
                        out.push_str(&format!("[{idx}]"));
                    } else if language == "elixir" || language == "ruby" {
                        // Ruby/Elixir hashes use `["key"]` bracket access (Ruby's Hash has
                        // no `get` method; Elixir maps use bracket access too).
                        out.push_str(&format!("[{}]", quoted_key_literal(key)));
                    } else {
                        out.push_str(&format!(".get({})", quoted_key_literal(key)));
                    }
                }
            }
            PathSegment::Length => match language {
                "ruby" => out.push_str(".length"),
                "elixir" => {
                    let current = std::mem::take(&mut out);
                    out = format!("length({current})");
                }
                "gleam" => {
                    let current = std::mem::take(&mut out);
                    out = format!("list.length({current})");
                }
                _ => {
                    let current = std::mem::take(&mut out);
                    out = format!("len({current})");
                }
            },
        }
    }
    out
}

pub(super) fn render_typescript(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField { name, index } => {
                if !name.is_empty() {
                    out.push('.');
                    out.push_str(&name.to_lower_camel_case());
                }
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                // Numeric (digit-only) keys index into arrays as integers, not as
                // string-keyed object properties; emit `[0]` not `["0"]`.
                if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".length");
            }
        }
    }
    out
}

pub(super) fn render_wasm(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField { name, index } => {
                if !name.is_empty() {
                    out.push('.');
                    out.push_str(&name.to_lower_camel_case());
                }
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                out.push_str(&format!(".get({})", quoted_key_literal(key)));
            }
            PathSegment::Length => {
                out.push_str(".length");
            }
        }
    }
    out
}

pub(super) fn render_go(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&to_go_name(f));
            }
            PathSegment::ArrayField { name, index } => {
                if !name.is_empty() {
                    out.push('.');
                    out.push_str(&to_go_name(name));
                }
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&to_go_name(field));
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("len({current})");
            }
        }
    }
    out
}

pub(super) fn render_java(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
            }
            PathSegment::ArrayField { name, index } => {
                if name.is_empty() {
                    out.push_str(&format!(".get({index})"));
                } else {
                    out.push('.');
                    out.push_str(&name.to_lower_camel_case());
                    out.push_str(&format!("().get({index})"));
                }
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                // Numeric keys index into List<T> (.get(int)); string keys index into Map<String, V>.
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    out.push_str(&format!("().get({key})"));
                } else {
                    out.push_str(&format!("().get({})", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".size()");
            }
        }
    }
    out
}

/// Wrap a Kotlin getter name in backticks when it collides with a Kotlin hard keyword.
///
/// Hard keywords cannot be used as identifiers without escaping, so `result.object()`
/// is a syntax error; `` result.`object`() `` is the legal form.
pub(super) fn kotlin_getter(name: &str) -> String {
    let camel = name.to_lower_camel_case();
    match camel.as_str() {
        "as" | "break" | "class" | "continue" | "do" | "else" | "false" | "for" | "fun" | "if" | "in" | "interface"
        | "is" | "null" | "object" | "package" | "return" | "super" | "this" | "throw" | "true" | "try"
        | "typealias" | "typeof" | "val" | "var" | "when" | "while" => format!("`{camel}`"),
        _ => camel,
    }
}

pub(super) fn render_kotlin(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&kotlin_getter(f));
                out.push_str("()");
            }
            PathSegment::ArrayField { name, index } => {
                if name.is_empty() {
                    if *index == 0 {
                        out.push_str(".first()");
                    } else {
                        out.push_str(&format!(".get({index})"));
                    }
                } else {
                    out.push('.');
                    out.push_str(&kotlin_getter(name));
                    if *index == 0 {
                        out.push_str("().first()");
                    } else {
                        out.push_str(&format!("().get({index})"));
                    }
                }
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&kotlin_getter(field));
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    out.push_str(&format!("().get({key})"));
                } else {
                    out.push_str(&format!("().get({})", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".size");
            }
        }
    }
    out
}
