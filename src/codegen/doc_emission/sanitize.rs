use super::sections::is_rust_fence_tag;

mod escaping;
mod prose;
mod references;
mod type_wrappers;
mod utf8;

use escaping::{escape_jsdoc_block_close, xml_escape_for_csharp};
use prose::apply_prose_transforms;
pub(crate) use references::wrap_bare_bracket_references;

/// Target language for [`sanitize_rust_idioms`].
///
/// Each variant selects the idiomatic mapping for Rust constructs that do not
/// translate directly to foreign-language doc syntax.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DocTarget {
    /// PHPDoc (`/** ... */`), e.g. phpstan-typed prose.
    PhpDoc,
    /// Javadoc (`/** ... */`), e.g. OpenJDK-style annotations.
    JavaDoc,
    /// TSDoc (`/** ... */`), TypeScript variant of JSDoc.
    TsDoc,
    /// JSDoc (`/** ... */`), JavaScript variant.
    JsDoc,
    /// C# XML doc (`/// <summary>...</summary>`).
    ///
    /// Strips Rust code fences and section headings (`# Examples`,
    /// `# Arguments`, `# Returns`, etc.), drops Rust trait-bound prose,
    /// and XML-escapes any remaining `<` / `>` / `&` so the result is
    /// safe to embed inside a `<summary>` element.
    CSharpDoc,
}

/// Sanitize Rust-specific idioms in a prose string for the given foreign-language
/// documentation target.
///
/// Transformations are applied **outside** backtick spans and code fences only,
/// so inline code examples and fenced blocks are never mutated (except that
/// ` ```rust ` fences and unmarked ` ``` ` code blocks are dropped entirely
/// for all targets [`DocTarget::TsDoc`], [`DocTarget::JsDoc`], [`DocTarget::PhpDoc`],
/// [`DocTarget::JavaDoc`], and [`DocTarget::CSharpDoc`]).
///
/// # Transformations
///
/// - Intra-doc links `` [`Type::method`] `` → `` `Type.method` ``.
/// - `[`Foo`]` (backtick inside square brackets) → `` `Foo` ``.
/// - `None` (word boundary) → `null` (PHP/Java) or `undefined` (TS/JS).
/// - `Some(x)` → `the value (x)`.
/// - `Option<T>` → `T?` (PHP) / `T | null` (Java) / `T | undefined` (TS/JS).
/// - `Vec<u8>` → `string` (PHP) / `byte[]` (Java) / `Uint8Array` (TS/JS).
/// - `Vec<T>` → `T[]` (all targets).
/// - `HashMap<K, V>` → `array<K, V>` (PHP) / `Map<K, V>` (Java) / `Record<K, V>` (TS/JS).
/// - `Arc<T>`, `Box<T>`, `Mutex<T>`, `RwLock<T>`, `Rc<T>`, `Cell<T>`, `RefCell<T>` → `T`.
/// - `Send + Sync`, `Send`, `Sync`, `'static` → stripped.
/// - Standalone `::` between identifiers → `.`.
/// - `pub fn `, `crate::`, `&self`, `&mut self` → stripped.
/// - `#[…]` attribute macros on their own line or inline → stripped.
/// - `.unwrap()`, `.expect("…")` → stripped.
/// - ` ```rust ` and unmarked ` ``` ` code fences → dropped entirely.
pub fn sanitize_rust_idioms(text: &str, target: DocTarget) -> String {
    // For C# XML doc the default is to drop rustdoc section headings
    // (`# Examples`, `# Arguments`, …) and the remainder of the comment,
    // because those bodies routinely contain content that cannot be embedded
    // safely inside `<summary>`. Callers that have already extracted sections
    // (`emit_csharp_doc`) sanitise each section body via [`sanitize_rust_idioms_keep_sections`].
    sanitize_rust_idioms_inner(text, target, true)
}

/// Same as [`sanitize_rust_idioms`] but never drops rustdoc section headings.
///
/// Used by emitters that have already split the doc into sections and need to
/// sanitise each body fragment independently (e.g. C# XML doc emission with
/// per-section `<param>` / `<returns>` / `<exception>` tags).
pub fn sanitize_rust_idioms_keep_sections(text: &str, target: DocTarget) -> String {
    sanitize_rust_idioms_inner(text, target, false)
}

fn sanitize_rust_idioms_inner(text: &str, target: DocTarget, drop_csharp_sections: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_rust_fence = false;
    let mut in_other_fence = false;
    // For C# XML doc: once a `# Examples` / `# Arguments` / etc. heading is
    // encountered, drop the entire remainder of the comment. Rustdoc section
    // headings cannot be safely embedded inside `<summary>` and the per-section
    // content (code fences, intra-doc links, generics) is the leading cause
    // of CS1002/CS1519 leakage. The plain `<summary>` path collapses to the
    // top-level prose only.
    let mut csharp_section_dropped = false;

    for line in text.lines() {
        if csharp_section_dropped {
            continue;
        }
        let trimmed = line.trim_start();
        if drop_csharp_sections
            && matches!(target, DocTarget::CSharpDoc)
            && !in_rust_fence
            && !in_other_fence
            && is_rustdoc_section_heading(trimmed)
        {
            csharp_section_dropped = true;
            continue;
        }

        // Detect code fence boundaries.
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_rust_fence {
                // Closing fence of a rust block.
                in_rust_fence = false;
                match target {
                    DocTarget::TsDoc
                    | DocTarget::JsDoc
                    | DocTarget::CSharpDoc
                    | DocTarget::PhpDoc
                    | DocTarget::JavaDoc => {
                        // Entire rust block dropped — don't emit closing fence.
                    }
                }
                continue;
            }
            if in_other_fence {
                // Closing fence of a non-rust block.
                in_other_fence = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            // Opening fence — determine language.
            let lang = rest.split(',').next().unwrap_or("").trim();
            let is_rust = is_rust_fence_tag(lang);
            if is_rust {
                in_rust_fence = true;
                match target {
                    DocTarget::TsDoc
                    | DocTarget::JsDoc
                    | DocTarget::CSharpDoc
                    | DocTarget::PhpDoc
                    | DocTarget::JavaDoc => {
                        // Drop the entire rust fence block — skip opening line.
                        // Rust code examples are not portable to any of the target languages.
                    }
                }
                continue;
            }
            // Non-rust fence: pass through verbatim.
            in_other_fence = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Inside a rust fence.
        if in_rust_fence {
            match target {
                DocTarget::TsDoc | DocTarget::JsDoc | DocTarget::CSharpDoc | DocTarget::PhpDoc | DocTarget::JavaDoc => {
                    // Drop content of rust fences — all targets filter out Rust code examples.
                }
            }
            continue;
        }

        // Inside a non-rust fence: pass through verbatim.
        if in_other_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Check if this line is a bare `#[...]` attribute line.
        let stripped_indent = line.trim_start();
        if stripped_indent.starts_with("#[") && stripped_indent.ends_with(']') {
            // Attribute-only line — drop entirely.
            continue;
        }

        // Normal prose line: apply token-level transformations.
        let sanitized = apply_prose_transforms(line, target);
        out.push_str(&sanitized);
        out.push('\n');
    }

    // Trim trailing newline added by the loop (preserve internal newlines).
    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }

    // For JSDoc and TSDoc, escape any `*/` sequences so they don't prematurely
    // close the /** ... */ block. Replace `*/` with `* /` (backslash prevents
    // JS/TS comment-terminator matching but renders identically in docs).
    if matches!(target, DocTarget::TsDoc | DocTarget::JsDoc) {
        out = escape_jsdoc_block_close(&out);
    }

    // For C# XML doc, escape any remaining `<`, `>`, `&` so the result is
    // safe to embed inside `<summary>...</summary>`. By this point the
    // Rust-idiom substitutions have replaced `Vec<T>` / `Option<T>` /
    // `HashMap<K, V>` / `Result<T, E>` with their idiomatic forms, but
    // unrecognised generic constructs (e.g. trait-object references) may
    // still contain raw angle brackets that would break C# XML parsing.
    if matches!(target, DocTarget::CSharpDoc) {
        out = xml_escape_for_csharp(&out);
    }

    out
}

/// Return `true` if `line` (already left-trimmed) is a Rustdoc section heading
/// such as `# Examples`, `# Arguments`, `# Returns`, `# Errors`, `# Panics`,
/// or `# Safety`. Case-insensitive on the heading name.
fn is_rustdoc_section_heading(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("# ") else {
        return false;
    };
    let head = rest.trim().to_ascii_lowercase();
    matches!(
        head.as_str(),
        "arguments" | "args" | "returns" | "errors" | "panics" | "safety" | "example" | "examples"
    )
}
