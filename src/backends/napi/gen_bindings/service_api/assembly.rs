use std::path::PathBuf;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;

use super::{gen_service_rs, gen_service_ts};

pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("node"), &config.name, "crates/{name}-node/src/");
    let crate_root = {
        let p = PathBuf::from(&output_dir);
        match p.file_name().and_then(|n| n.to_str()) {
            Some("src") => p.parent().map(|parent| parent.to_path_buf()).unwrap_or(p),
            _ => p,
        }
    };
    let package_name = config.name.replace('-', "_");

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // TypeScript wrapper
    let service_ts = gen_service_ts(api, &package_name, config);

    // JavaScript version (TypeScript with types stripped)
    let service_cjs = strip_typescript_annotations(&service_ts);

    // Node package output base: derive from package_name or use default
    let output_base = config
        .node
        .as_ref()
        .and_then(|n| n.package_name.as_ref())
        .map(|p| PathBuf::from(format!("packages/node/{}", p)))
        .unwrap_or_else(|| PathBuf::from(format!("packages/node/{}", package_name)));

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("service.ts"),
            content: service_ts.clone(),
            generated_header: true,
        },
        GeneratedFile {
            path: crate_root.join("service.ts"),
            content: service_ts,
            generated_header: true,
        },
        // Emit CommonJS version for runtime require() in index.js
        GeneratedFile {
            path: crate_root.join("service.cjs"),
            content: service_cjs,
            generated_header: true,
        },
    ])
}

/// Convert TypeScript service wrapper to CommonJS JavaScript.
/// Simple approach: convert imports and output with // type comments removed.
/// Uses a brute-force regex-like approach to handle `: Type` patterns.
fn strip_typescript_annotations(ts_code: &str) -> String {
    let mut result = String::new();

    for line in ts_code.lines() {
        let mut modified_line = line.to_string();

        // Convert `import type { ... } from 'module'` → `const { ... } = require('module')`
        if modified_line.trim().starts_with("import type {") && modified_line.contains("from") {
            if let Some(start_brace) = modified_line.find('{') {
                if let Some(end_brace) = modified_line.rfind('}') {
                    if let Some(from_pos) = modified_line.find("from") {
                        let imports = modified_line[start_brace..=end_brace].to_string();
                        let module_spec = modified_line[from_pos + 4..].trim(); // Skip "from"
                        // module_spec is something like `"./index";` — strip trailing semicolon
                        let module_spec = module_spec.trim_end_matches(';');
                        modified_line = format!("const {imports} = require({module_spec});");
                    }
                }
            }
            result.push_str(&modified_line);
            result.push('\n');
            continue;
        }

        // Convert `import { ... } from 'module'` → `const { ... } = require('module')`
        if modified_line.trim().starts_with("import {") && modified_line.contains("from") {
            if let Some(start_brace) = modified_line.find('{') {
                if let Some(end_brace) = modified_line.rfind('}') {
                    if let Some(from_pos) = modified_line.find("from") {
                        let imports = modified_line[start_brace..=end_brace].to_string();
                        let module_spec = modified_line[from_pos + 4..].trim(); // Skip "from"
                        // module_spec is something like `"./index";` — strip trailing semicolon
                        let module_spec = module_spec.trim_end_matches(';');
                        modified_line = format!("const {imports} = require({module_spec});");
                    }
                }
            }
            result.push_str(&modified_line);
            result.push('\n');
            continue;
        }

        // Remove `export` keyword from class declarations (they're not needed in CommonJS)
        if modified_line.trim().starts_with("export class") {
            modified_line = modified_line.replace("export class", "class");
        }

        // Remove `private` keyword
        if modified_line.contains("private ") {
            modified_line = modified_line.replace("private ", "");
        }

        // Remove `: Type` where Type is anything up to ) or , or = or {
        // Using a character-by-character approach
        let mut output = String::new();
        let chars: Vec<char> = modified_line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if i < chars.len() - 1 && chars[i] == ':' && !modified_line[..i].ends_with("://") {
                // Found a potential type annotation. Skip to the next ) , { or =
                let mut j = i + 1;
                // Skip whitespace
                while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                    j += 1;
                }
                // Skip the type (everything until we hit a boundary)
                let mut paren_depth = 0;
                let mut angle_depth = 0;
                while j < chars.len() {
                    match chars[j] {
                        '(' => paren_depth += 1,
                        ')' => {
                            if paren_depth == 0 {
                                break;
                            }
                            paren_depth -= 1;
                        }
                        '<' => angle_depth += 1,
                        // Only decrement `angle_depth` when there's actually an open
                        // generic — a `>` from `=>` (function-type arrow) is not a
                        // generic close and must not push the counter negative.
                        '>' if angle_depth > 0 => angle_depth -= 1,
                        // `=>` inside a type annotation is a function-type arrow, not an
                        // assignment terminator — skip past both chars and keep scanning.
                        '=' if paren_depth == 0
                            && angle_depth == 0
                            && j + 1 < chars.len()
                            && chars[j + 1] == '>' =>
                        {
                            j += 1;
                        }
                        ',' | '=' | '{' | ';' if paren_depth == 0 && angle_depth == 0 => {
                            break;
                        }
                        _ => {}
                    }
                    j += 1;
                }
                // We've found the end of the type annotation. Skip to j.
                i = j;
                // Trim trailing space from output if present
                while !output.is_empty() && output.ends_with(' ') {
                    output.pop();
                }
                // Don't add extra spaces
                if i < chars.len() && chars[i] != ',' && chars[i] != ')' && !output.is_empty() {
                    output.push(' ');
                }
                continue;
            }

            output.push(chars[i]);
            i += 1;
        }

        modified_line = output;

        result.push_str(&modified_line);
        result.push('\n');
    }

    result
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_typescript_annotations_converts_imports() {
        let ts_input = r#"import type { ServerConfig } from "./index";
import { Method, RouteBuilder } from "./index";
import { appIntoRouter } from "./index";"#;

        let js_output = strip_typescript_annotations(ts_input);

        // Check that import type converts to const
        assert!(
            js_output.contains(r#"const { ServerConfig } = require("./index");"#),
            "import type should convert to const with require, got: {}",
            js_output
        );

        // Check that import converts to const with closing paren
        assert!(
            js_output.contains(r#"const { Method, RouteBuilder } = require("./index");"#),
            "import should convert to const with require, got: {}",
            js_output
        );

        // Check that appIntoRouter import is correct
        assert!(
            js_output.contains(r#"const { appIntoRouter } = require("./index");"#),
            "appIntoRouter import should convert correctly, got: {}",
            js_output
        );
    }

    #[test]
    fn strip_typescript_annotations_removes_export_class() {
        let ts_input = "export class App {";
        let js_output = strip_typescript_annotations(ts_input);

        assert!(js_output.contains("class App {"), "export class should become class");
        assert!(!js_output.contains("export class"), "export should be removed");
    }

    #[test]
    fn strip_typescript_annotations_removes_private() {
        let ts_input = "  private _registrations: Array<[string, any[]]> = [];";
        let js_output = strip_typescript_annotations(ts_input);

        assert!(
            !js_output.contains("private"),
            "private keyword should be removed, got: {}",
            js_output
        );
        assert!(js_output.contains("_registrations"), "field name should be preserved");
    }

    #[test]
    fn strip_typescript_annotations_removes_type_annotations() {
        let ts_input = "  route(builder: RouteBuilder): (fn: (...args: any[]) => any) => (...args: any[]) => any {";
        let js_output = strip_typescript_annotations(ts_input);

        // Should remove the type annotations but keep structure
        assert!(
            js_output.contains("route(builder)"),
            "type annotations should be stripped, got: {}",
            js_output
        );
        assert!(!js_output.contains("RouteBuilder"), "type name should be removed");
        // Critical: the method-body opener `{` must survive the strip.
        // Earlier the function-type return annotation `=> any =>` confused the
        // boundary scanner into stopping at the first `=`, leaving `{` consumed.
        assert!(
            js_output.trim_end().ends_with('{'),
            "method-body opener `{{` must be preserved, got: {}",
            js_output
        );
        assert!(
            !js_output.contains("fn =>"),
            "stripping must not leave fragmentary arrow-type pieces like `fn => any`, got: {}",
            js_output
        );
    }

    #[test]
    fn strip_typescript_annotations_preserves_brace_on_method_with_arrow_return_type() {
        // Regression test for service.cjs being unparseable: a method whose return type
        // is itself a function type (`(...) => (...) => T`) used to drop the `{` body
        // opener because the boundary scanner broke on the `=` from the first `=>`.
        let ts_input = "  register_route(builder: RouteBuilder, handler: (...args: any[]) => any): this {";
        let js_output = strip_typescript_annotations(ts_input);
        assert!(
            js_output.trim_end().ends_with('{'),
            "method-body opener must survive when params contain arrow-type annotations, got: {}",
            js_output
        );
        assert!(
            js_output.contains("register_route(builder, handler)"),
            "param-list type annotations should be stripped cleanly, got: {}",
            js_output
        );
        // No stray `=> any` fragment must leak through.
        assert!(
            !js_output.contains("=> any"),
            "arrow-type return fragments must be fully stripped, got: {}",
            js_output
        );
    }
}
