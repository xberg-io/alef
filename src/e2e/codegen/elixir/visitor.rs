use crate::e2e::escape::escape_elixir;
use crate::e2e::fixture::CallbackAction;
use std::fmt::Write as FmtWrite;

/// Build an Elixir visitor map and add setup line. Returns the visitor variable name.
pub(super) fn build_elixir_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
) -> String {
    let mut visitor_obj = String::new();
    let _ = writeln!(visitor_obj, "%{{");
    for (method_name, action) in &visitor_spec.callbacks {
        emit_elixir_visitor_method(&mut visitor_obj, method_name, action);
    }
    let _ = writeln!(visitor_obj, "    }}");

    setup_lines.push(format!("visitor = {visitor_obj}"));
    "visitor".to_string()
}

/// Emit an Elixir visitor method for a callback action.
pub(super) fn emit_elixir_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    // Elixir uses atom keys and handle_ prefix
    let handle_method = format!("handle_{}", &method_name[6..]); // strip "visit_" prefix
    // The Rust NIF bridge packages every visitor argument (`_ctx`, `_text`, ...) into a
    // single map and invokes the user's anonymous function with that map. Generating
    // multi-arity functions like `fn(_ctx, _text) ->` therefore raised BadArityError
    // ("arity 2 called with 1 argument") at runtime. Generate arity-1 functions that
    // accept the args map (and ignore it) to match the bridge's calling convention.

    // CustomTemplate needs to read from args; other actions can ignore it.
    let arg_binding = match action {
        CallbackAction::CustomTemplate { .. } => "args",
        _ => "_args",
    };
    let _ = writeln!(out, "      :{handle_method} => fn({arg_binding}) ->");
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "        :skip");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "        :continue");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "        :preserve_html");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_elixir(output);
            let _ = writeln!(out, "        {{:custom, \"{escaped}\"}}");
        }
        CallbackAction::CustomTemplate { template, .. } => {
            // Build a <> concatenation expression so {key} placeholders are substituted
            // from the args map at runtime without embedding double-quoted strings inside
            // a double-quoted string literal.
            let expr = template_to_elixir_concat(template);
            let _ = writeln!(out, "        {{:custom, {expr}}}");
        }
    }
    let _ = writeln!(out, "      end,");
}

/// Convert a template like `"_{text}_"` into an Elixir `<>` concat expression:
/// `"_" <> Map.get(args, "text", "") <> "_"`.
/// Static parts are escaped via `escape_elixir`; `{key}` placeholders become
/// `Map.get(args, "key", "")` lookups into the JSON-decoded args map.
pub(super) fn template_to_elixir_concat(template: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut static_buf = String::new();
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            let mut closed = false;
            for kc in chars.by_ref() {
                if kc == '}' {
                    closed = true;
                    break;
                }
                key.push(kc);
            }
            if closed && !key.is_empty() {
                if !static_buf.is_empty() {
                    let escaped = escape_elixir(&static_buf);
                    parts.push(format!("\"{escaped}\""));
                    static_buf.clear();
                }
                let escaped_key = escape_elixir(&key);
                parts.push(format!("Map.get(args, \"{escaped_key}\", \"\")"));
            } else {
                static_buf.push('{');
                static_buf.push_str(&key);
                if !closed {
                    // unclosed brace - treat remaining as literal
                }
            }
        } else {
            static_buf.push(ch);
        }
    }

    if !static_buf.is_empty() {
        let escaped = escape_elixir(&static_buf);
        parts.push(format!("\"{escaped}\""));
    }

    if parts.is_empty() {
        return "\"\"".to_string();
    }
    parts.join(" <> ")
}
