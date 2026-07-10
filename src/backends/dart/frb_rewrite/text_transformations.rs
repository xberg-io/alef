use regex::Regex;

/// Strip trailing whitespace from every line and ensure a final newline.
pub fn strip_trailing_whitespace(source: &str) -> String {
    let mut result = source.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    result.push('\n');
    result
}

/// Fix FRB-generated Dart code that incorrectly calls `executeSync`/`executeNormal`
/// on callback function parameters.
///
/// When FRB generates service methods that take a callback function parameter
/// (e.g. `handler: FutureOr<String> Function(String)`), it emits code that calls
/// `handler.executeSync(...)` or `handler.executeNormal(...)`, but these methods
/// don't exist on function types. This rewrite strips the erroneous method calls,
/// calling the handler directly as a function.
///
/// FRB 2.x service-API callback parameters are plain `FutureOr<R> Function(T)` types,
/// not executor wrapper objects. The handler must be invoked directly: `await handler(arg)`.
/// This rewrite removes the erroneous `.executeSync()` / `.executeNormal()` method calls
/// that FRB incorrectly emits.
///
/// Additionally, any function/closure that contains `await handler(...)` calls must itself
/// be marked as `async`. This rewrite ensures all containing closures and methods are
/// properly declared as async.
///
/// Example transformation:
/// ```dart
/// // Before (FRB-generated, broken):
/// return handler.executeSync(
///   SyncTask(...),
/// );
///
/// // After (fixed):
/// return await handler(
///   SyncTask(...).request,
/// );
/// ```
///
/// Additionally, fixes FRB 2.x bug where `class RustLibApiImpl implements RustLibApi async`
/// is generated with an invalid `async` keyword in the class declaration. FRB generates this
/// incorrectly when the base class or mixin has async methods. The `async` keyword is only
/// valid on function declarations, not class declarations.
///
/// Also injects a typedef for `BaseHandler` as a function type, allowing handler parameters
/// to be invoked directly as functions in FRB-generated code.
pub fn fix_handler_executor_calls(source: &str) -> String {
    let mut result = source.to_string();

    result = result.replace(" implements RustLibApi async {", " implements RustLibApi {");

    result = rewrite_handler_calls_in_parameterized_functions(&result);

    result = ensure_handler_closures_are_async(&result);

    result
}

/// Rewrite handler.executeSync/executeNormal calls, but ONLY within function/method
/// scopes where `handler` appears as a parameter.
///
/// This prevents rewriting `handler.execute*` on the class field `super.handler`,
/// which is NOT directly callable and should keep its execute* method calls.
fn rewrite_handler_calls_in_parameterized_functions(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut result = String::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        let is_function_start = is_likely_function_start(line);

        if is_function_start {
            let is_handler_parameterized = detect_handler_parameter(&lines, i);

            let mut func_lines = vec![line];
            i += 1;

            let mut depth = count_brace_depth(line);
            let mut saw_body = depth > 0;

            while i < lines.len() && (!saw_body || depth > 0) {
                let curr_line = lines[i];
                func_lines.push(curr_line);
                let line_depth = count_brace_depth(curr_line);
                depth += line_depth;
                saw_body = saw_body || line_depth > 0;
                i += 1;
            }

            let func_text = func_lines.join("\n");
            let rewritten = if is_handler_parameterized {
                rewrite_handler_to_task_executor(&func_text)
            } else {
                func_text
            };

            result.push_str(&rewritten);
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
            i += 1;
        }
    }

    if !source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Quick heuristic to detect if a line likely starts a function definition.
/// Looks for patterns: `Type name(` or `async {...` or `@override`
fn is_likely_function_start(line: &str) -> bool {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with("//") {
        return false;
    }

    if trimmed.starts_with("@") {
        return false;
    }

    if !line.contains('(') {
        return false;
    }

    if trimmed.starts_with("}") || trimmed.starts_with("]") || trimmed.starts_with(")") {
        return false;
    }

    true
}

/// Count the net braces in a line (positive = more opens than closes)
fn count_brace_depth(line: &str) -> i32 {
    let opens = line.chars().filter(|c| *c == '{').count() as i32;
    let closes = line.chars().filter(|c| *c == '}').count() as i32;
    opens - closes
}

/// Check if the function/method at line `idx` has `handler` as a parameter.
/// Looks for a function signature that includes `handler` in its parameter list.
fn detect_handler_parameter(lines: &[&str], idx: usize) -> bool {
    if idx >= lines.len() {
        return false;
    }

    let line = lines[idx];

    if !line.contains('(') {
        for l in lines.iter().take(std::cmp::min(idx + 20, lines.len())).skip(idx) {
            if l.contains("handler") && l.contains("Function") {
                return true;
            }
            if l.contains(')') && l.contains('{') {
                break;
            }
        }
    } else {
        let mut sig = line.to_string();
        let mut paren_depth = line.chars().filter(|c| *c == '(').count() - line.chars().filter(|c| *c == ')').count();

        let mut j = idx + 1;
        while j < lines.len() && paren_depth > 0 {
            let l = lines[j];
            sig.push(' ');
            sig.push_str(l);
            paren_depth += l.chars().filter(|c| *c == '(').count();
            paren_depth -= l.chars().filter(|c| *c == ')').count();
            j += 1;
        }

        if sig.contains("handler") && sig.contains("Function") {
            return true;
        }
    }

    false
}

/// Rewrite handler.executeSync/executeNormal to move the method call to the task.
/// When handler is a parameter, FRB generates:
///   `return handler.executeSync(SyncTask(...));`
/// But the handler parameter can't be invoked directly (it's serialized and passed to Rust).
/// Instead, invoke the task executor:
///   `return SyncTask(...).executeSync();`
fn rewrite_handler_to_task_executor(source: &str) -> String {
    let mut result = rewrite_handler_executor_wrappers(source);

    let orphaned_paren_sync =
        Regex::new(r"(?s)\),\s*\)\.executeSync\(\)").expect("orphaned paren sync pattern must compile");
    result = orphaned_paren_sync.replace_all(&result, ").executeSync()").into_owned();

    let orphaned_paren_async =
        Regex::new(r"(?s)\),\s*\)\.executeNormal\(\)").expect("orphaned paren async pattern must compile");
    result = orphaned_paren_async
        .replace_all(&result, ").executeNormal()")
        .into_owned();

    result
}

fn rewrite_handler_executor_wrappers(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;

    while let Some((relative_start, method)) = find_next_handler_executor(&source[cursor..]) {
        let start = cursor + relative_start;
        let open_paren = start + format!("handler.{method}").len();
        let Some(close_paren) = find_matching_paren(source, open_paren) else {
            break;
        };

        out.push_str(&source[cursor..start]);
        let task = source[open_paren + 1..close_paren].trim();
        let task = task.strip_suffix(',').map(str::trim_end).unwrap_or(task);
        out.push_str(task);
        out.push('.');
        out.push_str(method);
        out.push_str("()");
        cursor = close_paren + 1;
    }

    out.push_str(&source[cursor..]);
    out
}

fn find_next_handler_executor(source: &str) -> Option<(usize, &'static str)> {
    let sync = source.find("handler.executeSync(");
    let normal = source.find("handler.executeNormal(");

    match (sync, normal) {
        (Some(sync), Some(normal)) if sync <= normal => Some((sync, "executeSync")),
        (Some(_), Some(normal)) => Some((normal, "executeNormal")),
        (Some(sync), None) => Some((sync, "executeSync")),
        (None, Some(normal)) => Some((normal, "executeNormal")),
        (None, None) => None,
    }
}

fn find_matching_paren(source: &str, open_paren: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in source[open_paren..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open_paren + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Ensure all closures and anonymous functions that contain `await handler` calls
/// are declared as `async`. This fixes the Dart compile error where `await` is used
/// in a non-async context.
fn ensure_handler_closures_are_async(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();

    let mut lines_to_fix: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        let trimmed_line = line.trim();
        if trimmed_line.starts_with("//")
            || line.contains("async")
            || trimmed_line.starts_with("class ")
            || trimmed_line.starts_with("abstract class ")
            || trimmed_line.starts_with("mixin ")
        {
            i += 1;
            continue;
        }

        let contains_await_handler =
            (i..std::cmp::min(i + 30, lines.len())).any(|j| lines[j].contains("await handler("));

        if contains_await_handler {
            let parens_balanced =
                line.chars().filter(|c| *c == '(').count() == line.chars().filter(|c| *c == ')').count();

            if parens_balanced && line.contains('{') {
                lines_to_fix.insert(i);
            } else if !parens_balanced {
                for (j, check_line) in lines
                    .iter()
                    .enumerate()
                    .take(std::cmp::min(i + 30, lines.len()))
                    .skip(i + 1)
                {
                    if check_line.contains(')') && check_line.contains('{') && !check_line.trim().starts_with("//") {
                        if !check_line.contains("async") {
                            lines_to_fix.insert(j);
                        }
                        break;
                    }
                }
            }
        }

        i += 1;
    }

    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if lines_to_fix.contains(&i) {
            let fixed = if line.contains(") {") {
                line.replace(") {", ") async {")
            } else {
                let trimmed = line.trim_end();
                if trimmed.ends_with("{") {
                    format!("{} async {{", trimmed.trim_end_matches('{').trim_end())
                } else {
                    line.to_string()
                }
            };
            result.push_str(&fixed);
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }

    if !source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Filter out function definitions for excluded function names from FRB-generated `lib.dart`.
///
/// FRB generates public `Future<T> functionName(...)` wrappers for all public functions
/// in the Rust API. This function removes lines that define functions whose names match
/// the provided exclude set, allowing the Dart wrapper class to honor `exclude_functions`
/// config without re-parsing the FRB output.
///
/// The function is idempotent: running it multiple times produces the same result.
pub fn filter_excluded_functions(source: &str, exclude_functions: &std::collections::HashSet<&str>) -> String {
    if exclude_functions.is_empty() {
        return source.to_string();
    }

    let lines: Vec<&str> = source.lines().collect();
    let mut result = String::with_capacity(source.len());
    let mut i = 0;
    let mut doc_buffer: Vec<&str> = Vec::new();

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.starts_with("///")
            || trimmed.starts_with("//")
            || (trimmed.starts_with("*") && !trimmed.starts_with("**/"))
        {
            doc_buffer.push(line);
            i += 1;
            continue;
        }

        let mut should_skip_function = false;
        if !trimmed.is_empty() && !trimmed.starts_with("class") && !trimmed.starts_with("enum") {
            should_skip_function = exclude_functions.iter().any(|&excluded| {
                let camel_excluded = snake_to_camel(excluded);

                let pattern = format!(" {}(", camel_excluded);
                line.contains(&pattern)
            });
        }

        if should_skip_function {
            doc_buffer.clear();
            loop {
                i += 1;
                if i >= lines.len() {
                    break;
                }
                let check_line = lines[i];
                if check_line.contains(';') {
                    i += 1;
                    break;
                }
            }
        } else {
            for doc_line in &doc_buffer {
                result.push_str(doc_line);
                result.push('\n');
            }
            doc_buffer.clear();
            result.push_str(line);
            result.push('\n');
            i += 1;
        }
    }

    for doc_line in &doc_buffer {
        result.push_str(doc_line);
        result.push('\n');
    }

    result
}

/// Convert Rust snake_case to Dart lowerCamelCase
fn snake_to_camel(name: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for c in name.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            for upper_c in c.to_uppercase() {
                result.push(upper_c);
            }
            capitalize_next = false;
        } else if result.is_empty() {
            for lower_c in c.to_lowercase() {
                result.push(lower_c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Inject `text()` methods on sealed classes that are display-as-text types.
///
/// For sealed classes representing untagged unions (e.g., `AssistantContent`),
/// inject a `text()` getter that returns the plain-text display value by pattern-matching
/// the sealed class variants. The semantics mirror Rust Display:
///   - JSON string → verbatim
///   - array → concat `type=="text"` parts
///   - else → ""
///
/// This function is idempotent and safe to run multiple times.
pub fn inject_display_as_text_methods(source: &str, text_type_names: &[String]) -> String {
    if text_type_names.is_empty() {
        return source.to_string();
    }

    let mut result = source.to_string();

    for type_name in text_type_names {
        let pattern = format!(r"sealed\s+class\s+{}\b[^{{}}\n]*\{{", regex::escape(type_name));
        let sealed_class_regex = Regex::new(&pattern).expect("sealed class regex should be valid");
        let found = sealed_class_regex.is_match(&result);

        if found {
            let ext_name = format!("extension {}TextExt", type_name);
            if !result.contains(&ext_name) {
                let text_method = generate_text_method_extension(type_name);
                result.push_str("\n\n");
                result.push_str(&text_method);
            }
        }
    }

    result
}

/// Generate the text() extension method for a display-as-text sealed class.
fn generate_text_method_extension(type_name: &str) -> String {
    let ext_name = format!("{}TextExt", type_name);
    format!(
        r#"extension {} on {} {{
  /// Returns the plain-text display value of this content.
  ///
  /// - If this is a Text variant, returns its string content verbatim.
  /// - If this is a Parts variant, concatenates the text fields from all text-type
  ///   parts, skipping non-text parts like images, audio, or refusals.
  /// - Otherwise returns an empty string.
  String text() {{
    return switch (this) {{
      {}_Text(:final field0) => field0,
      {}_Parts(:final field0) => _extractTextFromContentParts(field0),
      _ => '',
    }};
  }}
}}

/// Helper: Extract and concatenate text from content parts.
/// Expects parts to be a List of sealed class instances (typically ContentPart
/// or similar union types where each variant is a sealed class, e.g. ContentPart_Text).
/// Only instances of *_Text variants (containing a text field) contribute to output.
String _extractTextFromContentParts(List<dynamic> parts) {{
  final sb = StringBuffer();
  for (final part in parts) {{
    // For FRB-generated sealed class instances, we can check the runtime type
    // and access properties dynamically. Sealed class variants follow the pattern
    // TypeName_VariantName, and text-type variants have a 'text' property.
    final typeString = part.runtimeType.toString();
    // Check if this is a text-type variant (e.g., 'ContentPart_Text(...)' or similar)
    if (typeString.contains('_Text')) {{
      try {{
        // Use dynamic access to get the 'text' field from the variant instance
        final text = (part as dynamic).text;
        if (text is String) {{
          sb.write(text);
        }}
      }} catch (_) {{
        // If field access fails, skip this part (it's not actually a text variant)
        continue;
      }}
    }}
  }}
  return sb.toString();
}}"#,
        ext_name, type_name, type_name, type_name
    )
}

/// Legacy identity transform kept for old post-build processor references.
///
/// Dart default handling is emitted from IR metadata in the generated wrapper
/// layer. This post-FRB source rewriter has no API metadata, so it must not infer
/// defaults from product-specific class or field names.
pub fn make_struct_fields_with_defaults_optional(source: &str) -> String {
    source.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_display_as_text_methods_empty_types() {
        let source = "sealed class AssistantContent {}";
        let result = inject_display_as_text_methods(source, &[]);
        assert_eq!(result, source);
    }

    #[test]
    fn inject_display_as_text_methods_single_type() {
        let source = "sealed class AssistantContent {}\n";
        let types = vec!["AssistantContent".to_string()];
        let result = inject_display_as_text_methods(source, &types);

        assert!(result.contains("extension AssistantContentTextExt on AssistantContent"));
        assert!(result.contains("String text()"));
        assert!(result.contains("AssistantContent_Text"));
        assert!(result.contains("AssistantContent_Parts"));
    }

    #[test]
    fn inject_display_as_text_methods_with_freezed_mixin() {
        let source = "sealed class AssistantContent with _$AssistantContent {}\n";
        let types = vec!["AssistantContent".to_string()];
        let result = inject_display_as_text_methods(source, &types);

        assert!(
            result.contains("extension AssistantContentTextExt on AssistantContent"),
            "must inject text() extension even when the class has a freezed mixin clause; got:\n{result}"
        );
        assert!(result.contains("String text()"));
        assert!(
            result.contains("AssistantContent_Text(:final field0) => field0"),
            "Text variant must destructure the freezed `field0` getter; got:\n{result}"
        );
        assert!(
            result.contains("AssistantContent_Parts(:final field0)"),
            "Parts variant must destructure the freezed `field0` getter; got:\n{result}"
        );
        assert!(
            !result.contains(":final parts") && !result.contains(":final text)"),
            "must not reference the non-existent `text`/`parts` getters; got:\n{result}"
        );
    }

    #[test]
    fn inject_display_as_text_methods_idempotent() {
        let source = "sealed class AssistantContent {}\n";
        let types = vec!["AssistantContent".to_string()];
        let once = inject_display_as_text_methods(source, &types);
        let _twice = inject_display_as_text_methods(&once, &types);

        let count_once = once.matches("extension AssistantContentTextExt").count();

        assert!(count_once > 0);
    }

    #[test]
    fn inject_display_as_text_methods_multiple_types() {
        let source = "sealed class AssistantContent {}\nsealed class ContentPart {}\n";
        let types = vec!["AssistantContent".to_string(), "ContentPart".to_string()];
        let result = inject_display_as_text_methods(source, &types);

        assert!(result.contains("extension AssistantContentTextExt on AssistantContent"));
        assert!(result.contains("extension ContentPartTextExt on ContentPart"));
    }

    #[test]
    fn inject_display_as_text_methods_preserves_original() {
        let source = "sealed class AssistantContent {}\nfinal x = 5;";
        let types = vec!["AssistantContent".to_string()];
        let result = inject_display_as_text_methods(source, &types);

        assert!(result.contains("sealed class AssistantContent"));
        assert!(result.contains("final x = 5;"));
    }

    #[test]
    fn inject_display_as_text_methods_assistant_content_pattern() {
        let source = r#"sealed class AssistantContent {}
final class AssistantContent_Text extends AssistantContent {
  final String text;
}
final class AssistantContent_Parts extends AssistantContent {
  final List<AssistantPart> parts;
}
sealed class AssistantPart {}
final class AssistantPart_Text extends AssistantPart {
  final String text;
}
final class AssistantPart_Image extends AssistantPart {
  final String imageUrl;
}"#;
        let types = vec!["AssistantContent".to_string()];
        let result = inject_display_as_text_methods(source, &types);

        assert!(result.contains("extension AssistantContentTextExt on AssistantContent"));
        assert!(result.contains("String text()"));
        assert!(result.contains("AssistantContent_Text(:final field0) => field0"));
        assert!(result.contains("AssistantContent_Parts(:final field0)"));
        assert!(result.contains("_extractTextFromContentParts"));
        assert!(result.contains("sealed class AssistantContent {}"));
        assert!(result.contains("sealed class AssistantPart {}"));
    }

    #[test]
    fn inject_display_as_text_methods_config_gated() {
        let source = "sealed class AssistantContent {}\nfinal class AssistantContent_Text {}";
        let result = inject_display_as_text_methods(source, &[]);

        assert_eq!(result, source);
        assert!(!result.contains("extension AssistantContentTextExt"));
    }
}
