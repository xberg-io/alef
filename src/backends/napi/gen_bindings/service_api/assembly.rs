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
                        let module_part = modified_line[from_pos..].trim();
                        modified_line = format!("const {imports} = require({}", &module_part[5..]);
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
                        let module_part = modified_line[from_pos..].trim();
                        modified_line = format!("const {imports} = require({}", &module_part[5..]);
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
                        '>' => angle_depth -= 1,
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
