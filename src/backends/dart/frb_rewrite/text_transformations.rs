use regex::Regex;

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
    // Strip the erroneous `.executeSync()` and `.executeNormal()` method calls
    // on callback function parameters. Replace them with direct invocation.
    // IMPORTANT: Only rewrite handler.execute* calls where `handler` is a parameter,
    // not where it's a class field (inherited from super.handler).

    // Inject a typedef for `BaseHandler` to make it a callable function type.
    // FRB may emit BaseHandler as an opaque class, but service methods expect
    // to invoke it directly: `await handler(task)`. Define it as a function typedef.
    let mut result = inject_base_handler_typedef(source);

    // Pattern 4: Fix FRB 2.x bug where class declarations have invalid `async` keyword.
    // `class RustLibApiImpl implements RustLibApi async {` → `class RustLibApiImpl implements RustLibApi {`
    // The `async` keyword is only valid on functions, not class declarations.
    result = result.replace(" implements RustLibApi async {", " implements RustLibApi {");

    // Rewrite handler.execute* calls only in functions/methods where `handler` is a parameter.
    result = rewrite_handler_calls_in_parameterized_functions(&result);

    // Pattern 3: Ensure closures/functions containing `await handler` are marked as async.
    // Fix patterns like: `({...}) {` to `({...}) async {` when body contains `await handler`.
    // This handles synchronous closure signatures that were not originally async.
    result = ensure_handler_closures_are_async(&result);

    result
}

/// Inject a typedef for `BaseHandler` as a function type if not already present.
///
/// FRB may emit `BaseHandler` as an opaque Dart class when the Rust side defines
/// a `DartFnFuture<String>` parameter. However, the generated service methods expect
/// to invoke it directly as a function: `await handler(task)`. This function adds
/// the typedef so `BaseHandler` is a callable function type.
///
/// The typedef is inserted after existing `import` statements to ensure it's available
/// for the RustLib.init() method signature and service API classes.
///
/// Only injects if the source appears to be a full file (contains import statements).
/// This avoids modifying test code snippets or already-modified sources.
fn inject_base_handler_typedef(source: &str) -> String {
    // Check if BaseHandler is already defined (either as a typedef or class).
    // If so, no-op (idempotent).
    if source.contains("typedef BaseHandler") || (source.contains("class BaseHandler") && source.contains("call(")) {
        return source.to_string();
    }

    // Only inject if the source contains import statements (i.e., it's a full file).
    // This avoids modifying code snippets used in tests.
    if !source.contains("\nimport ") && !source.starts_with("import ") {
        return source.to_string();
    }

    // Find the position after the import block to insert the typedef.
    let lines: Vec<&str> = source.lines().collect();
    let mut insert_pos = 0;

    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with("import ") {
            insert_pos = i + 1;
        }
    }

    if insert_pos == 0 {
        // No imports found; don't inject
        return source.to_string();
    }

    // Build the typedef line. BaseHandler should accept a generic request/task object
    // and return a Future<dynamic> since it receives serialized task objects.
    let typedef_line = "\n/// Handler callback type for service methods.\ntypedef BaseHandler = FutureOr<dynamic> Function(dynamic);\n";

    // Insert the typedef after the imports
    let mut result = lines[..insert_pos].join("\n");
    result.push('\n');
    result.push_str(typedef_line);
    result.push_str(&lines[insert_pos..].join("\n"));
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

        // Check if this line starts a function/method definition
        // Look for patterns: `... functionName(...)` or `... functionName(` with multi-line signature
        let is_function_start = is_likely_function_start(line);

        if is_function_start {
            // Check if this function has `handler` as a parameter
            let is_handler_parameterized = detect_handler_parameter(&lines, i);

            // Collect lines until we reach the closing brace of the function body.
            // Function signatures can span multiple lines before the opening `{`,
            // so keep collecting until the body starts and then closes.
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

            // Rewrite if this function has handler parameter
            let func_text = func_lines.join("\n");
            let rewritten = if is_handler_parameterized {
                // When `handler` is a parameter, it's serialized and sent to Rust, where Rust invokes it.
                // The Dart code should NOT invoke the handler directly. Instead, it should invoke
                // the task executor (executeSync/executeNormal) on the task itself.
                //
                // Rewrite: `handler.executeSync(Task(...))` → `Task(...).executeSync()`
                // Rewrite: `handler.executeNormal(Task(...))` → `await Task(...).executeNormal()`
                rewrite_handler_to_task_executor(&func_text)
            } else {
                func_text
            };

            result.push_str(&rewritten);
            result.push('\n');
        } else {
            // Not a function start; just pass through
            result.push_str(line);
            result.push('\n');
            i += 1;
        }
    }

    // Remove the extra trailing newline if the original didn't have it
    if !source.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Quick heuristic to detect if a line likely starts a function definition.
/// Looks for patterns: `Type name(` or `async {...` or `@override`
fn is_likely_function_start(line: &str) -> bool {
    let trimmed = line.trim();

    // Skip comments and empty lines
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return false;
    }

    // @override always precedes function definitions
    if trimmed.starts_with("@") {
        return false; // The actual function is on the next line
    }

    // Function signatures typically have an opening paren
    if !line.contains('(') {
        return false;
    }

    // Exclude closing braces, field assignments, etc.
    if trimmed.starts_with("}") || trimmed.starts_with("]") || trimmed.starts_with(")") {
        return false;
    }

    // Check if line contains `{` or ends with the start of signature (maybe multi-line)
    // This is a heuristic and might match non-function-starting lines, which is OK
    // because we'll later filter by whether handler is a parameter
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

    // Quick check: does this line contain both `(` and potentially the start of a parameter list?
    if !line.contains('(') {
        // This might be a multi-line signature; check the next few lines
        for l in lines.iter().take(std::cmp::min(idx + 20, lines.len())).skip(idx) {
            if l.contains("handler") && l.contains("Function") {
                // Likely contains `handler: ... Function(...)` parameter
                return true;
            }
            if l.contains(')') && l.contains('{') {
                // Reached the end of the signature; stop searching
                break;
            }
        }
    } else {
        // Single-line or start of multi-line signature on this line
        // Collect lines until we close the parameter list
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

        // Check if the signature contains `handler` as a parameter
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
    // Fix the pattern where FRB generates a stray closing paren before .executeSync()/.executeNormal()
    //
    // Pattern (raw from FRB with (?s) for dot matching newlines):
    //   ),\n  <-- Task constructor closing paren + comma
    //   ).executeSync();  <-- orphaned closing paren before the method call
    //
    // The `)` before `.executeSync()` is orphaned and should be removed.
    // Fix: Strip the orphaned `)` on the line before `.executeSync()` / `.executeNormal()`

    let mut result = rewrite_handler_executor_wrappers(source);

    // Match `),` followed by any whitespace (including newlines), then orphaned `)` before `.executeSync()` or `.executeNormal()`
    // Pattern: `),` + newline + indent + `)` + `.execute(Sync|Normal)()`
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

    // First pass: identify which lines need `async` injected. For each line that
    // starts a function or closure, check if any of the next ~30 lines contain
    // `await handler`. If so, mark the closing brace line for mutation.
    let mut lines_to_fix: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Skip comments, class/mixin declarations, and lines that already have async.
        // Class declarations (starting with `class `, `abstract class`, `mixin `, etc.) must
        // never receive an `async` keyword — `async` is only valid on function declarations.
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

        // Check if any of the next ~30 lines contain `await handler`
        let contains_await_handler =
            (i..std::cmp::min(i + 30, lines.len())).any(|j| lines[j].contains("await handler("));

        if contains_await_handler {
            let parens_balanced =
                line.chars().filter(|c| *c == '(').count() == line.chars().filter(|c| *c == ')').count();

            // Case 1: Single-line signature with balanced parens and opening brace
            if parens_balanced && line.contains('{') {
                lines_to_fix.insert(i);
            }
            // Case 2: Multi-line signature (unbalanced parens) — find the closing brace line
            else if !parens_balanced {
                for (j, check_line) in lines
                    .iter()
                    .enumerate()
                    .take(std::cmp::min(i + 30, lines.len()))
                    .skip(i + 1)
                {
                    // Look for a line that has `)` (closing paren) and `{` (opening brace).
                    // This is typically the closing line of a multi-line function signature.
                    // Skip lines that already have `async` — adding it again would duplicate the keyword.
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

    // Second pass: apply the fixes
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        if lines_to_fix.contains(&i) {
            let fixed = if line.contains(") {") {
                line.replace(") {", ") async {")
            } else {
                // Insert `async` before `{`
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

    // Remove the extra trailing newline if the original didn't have it
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
    let mut doc_buffer: Vec<&str> = Vec::new(); // Buffer for doc comments

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Check if this is a doc/comment line
        if trimmed.starts_with("///")
            || trimmed.starts_with("//")
            || (trimmed.starts_with("*") && !trimmed.starts_with("**/"))
        {
            // Buffer the comment line
            doc_buffer.push(line);
            i += 1;
            continue;
        }

        // Check if this is the start of a function definition we should exclude.
        // Match function signature lines that contain a function name followed by `(`
        let mut should_skip_function = false;
        if !trimmed.is_empty() && !trimmed.starts_with("class") && !trimmed.starts_with("enum") {
            should_skip_function = exclude_functions.iter().any(|&excluded| {
                // Convert snake_case to lowerCamelCase to match Dart's function naming
                let camel_excluded = snake_to_camel(excluded);

                // Match patterns like:
                // - `Future<double> functionName({`
                // - `void functionName(`
                // - `String functionName(`
                let pattern = format!(" {}(", camel_excluded);
                line.contains(&pattern)
            });
        }

        if should_skip_function {
            // Clear the buffered doc comments since we're skipping this function
            doc_buffer.clear();
            // Skip this line and all continuation lines until we find a line ending with `;`
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
            // Keep all buffered doc comments and this line
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

    // Append any remaining buffered comments (shouldn't happen, but be safe)
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

/// Legacy identity transform kept for old post-build processor references.
///
/// Dart default handling is emitted from IR metadata in the generated wrapper
/// layer. This post-FRB source rewriter has no API metadata, so it must not infer
/// defaults from product-specific class or field names.
pub fn make_struct_fields_with_defaults_optional(source: &str) -> String {
    source.to_string()
}
