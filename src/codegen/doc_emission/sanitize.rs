use super::sections::is_rust_fence_tag;

mod escaping;
mod prose;
mod references;
mod type_wrappers;
mod utf8;

use escaping::{escape_jsdoc_block_close, xml_escape_for_csharp};
use prose::apply_prose_transforms;
pub(crate) use references::{unlink_intradoc_references, wrap_bare_bracket_references};

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

        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_rust_fence {
                in_rust_fence = false;
                match target {
                    DocTarget::TsDoc
                    | DocTarget::JsDoc
                    | DocTarget::CSharpDoc
                    | DocTarget::PhpDoc
                    | DocTarget::JavaDoc => {}
                }
                continue;
            }
            if in_other_fence {
                in_other_fence = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            let lang = rest.split(',').next().unwrap_or("").trim();
            let is_rust = is_rust_fence_tag(lang);
            if is_rust {
                in_rust_fence = true;
                match target {
                    DocTarget::TsDoc
                    | DocTarget::JsDoc
                    | DocTarget::CSharpDoc
                    | DocTarget::PhpDoc
                    | DocTarget::JavaDoc => {}
                }
                continue;
            }
            in_other_fence = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_rust_fence {
            match target {
                DocTarget::TsDoc | DocTarget::JsDoc | DocTarget::CSharpDoc | DocTarget::PhpDoc | DocTarget::JavaDoc => {
                }
            }
            continue;
        }

        if in_other_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Check if this line is a bare `#[...]` attribute line.
        let stripped_indent = line.trim_start();
        if stripped_indent.starts_with("#[") && stripped_indent.ends_with(']') {
            continue;
        }

        let sanitized = apply_prose_transforms(line, target);
        out.push_str(&sanitized);
        out.push('\n');
    }

    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }

    if matches!(target, DocTarget::TsDoc | DocTarget::JsDoc) {
        out = escape_jsdoc_block_close(&out);
    }

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
