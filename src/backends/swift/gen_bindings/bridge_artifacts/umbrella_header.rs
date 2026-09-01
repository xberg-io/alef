//! Assembly and validation of `RustBridgeC.h`, the C umbrella header that concatenates
//! swift-bridge's `SwiftBridgeCore.h` and `{binding_crate}.h`.
//!
//! Two independent call sites write this one file — the Swift scaffold
//! (`scaffold::languages::swift`) and the `MaterializeSwiftBridge` post-build step
//! (`emit_swift_bridge_files` in the parent module) — so the decision of *what bytes belong
//! in it* lives here once, and both call sites route through [`resolve_fresh`].
//!
//! Two properties are enforced here that neither call site enforced before:
//!
//! 1. **Completeness.** A concatenation of two files that both merely *exist* is not
//!    automatically a valid header: a degenerate or partial `SwiftBridgeCore.h` still
//!    satisfies an exists-check while dropping the `RustStr` / `__private__*` typedefs every
//!    `__swift_bridge__$*` declaration in `{binding_crate}.h` refers to. The result is a
//!    plausible-looking header that cannot compile. [`undefined_core_types`] checks the
//!    assembled bytes for that shape directly — which core types the body *references*
//!    versus which it *defines* — rather than trusting the two inputs because they were
//!    present.
//! 2. **Byte stability across runs.** The bytes swift-bridge emits are unformatted; the
//!    bytes that ship are formatted (`poly fmt` delegates `.h` to clang-format). Because
//!    `poly` unconditionally skips any file carrying an `alef:hash:` line, a run that stamps
//!    the raw concatenation before the format pass reaches it locks the unformatted bytes in
//!    permanently, and a run that does not produces the formatted ones — the same input
//!    yielding two different shipped files, with alef exiting 0 either way. [`declares_the_same_c`]
//!    compares the freshly assembled header against the committed one as C rather than as
//!    text, so a header whose declarations are unchanged keeps its committed bytes and the
//!    formatted/unformatted race has nothing left to flip. ~keep

use std::collections::BTreeMap;

/// A `RustBridgeC.h` is "populated" once it carries swift-bridge's generated C function
/// declarations. The placeholder only defines base typedefs and never references a
/// `__swift_bridge__$` symbol, so the presence of that prefix is a reliable
/// populated/placeholder discriminator — independent of whether the header was produced by
/// alef's umbrella or by a consumer's own concat script.
pub(crate) fn is_populated(header: &str) -> bool {
    header.contains("__swift_bridge__$")
}

/// Concatenate swift-bridge's two generated C headers into the umbrella header.
pub(crate) fn assemble(binding_crate_name: &str, core_h: &str, crate_h: &str) -> String {
    let marker = crate::core::hash::SWIFT_C_UMBRELLA_HEADER_MARKER;
    format!(
        "#ifndef RUST_BRIDGE_C_H\n\
         #define RUST_BRIDGE_C_H\n\
         \n\
         {marker}\n\
         // Concatenates SwiftBridgeCore.h and {binding_crate_name}.h produced by\n\
         // `cargo build -p {binding_crate_name}` via swift_bridge_build.\n\
         \n\
         {core_h}\n\
         {crate_h}\n\
         #endif /* RUST_BRIDGE_C_H */\n"
    )
}

/// Decide the umbrella header's content when swift-bridge build output *is* available.
///
/// Errors rather than returning a degraded header: a partial assembly with no committed
/// header to fall back on is exactly the case that must not ship silently. When a populated
/// header *is* committed, that file is preserved and the failure is reported as a warning —
/// preserving known-good declarations beats failing a whole regeneration over a stale
/// `target/` directory.
///
/// The undefined types come from the crate's own build directory: an empty or partial
/// `SwiftBridgeCore.h` there is what leaves the `__swift_bridge__$` declarations without
/// definitions. ~keep
pub(crate) fn resolve_fresh(
    binding_crate_name: &str,
    core_h: &str,
    crate_h: &str,
    existing: Option<&str>,
) -> anyhow::Result<String> {
    let assembled = assemble(binding_crate_name, core_h, crate_h);
    let committed = existing.filter(|header| is_populated(header));

    let undefined = undefined_core_types(&assembled);
    if !undefined.is_empty() {
        let missing = undefined.join(", ");
        if let Some(committed) = committed {
            tracing::warn!(
                "swift-bridge build output for `{binding_crate_name}` uses {missing} but never defines \
                 them; keeping the committed RustBridgeC.h. Re-run `cargo build -p {binding_crate_name}` \
                 to refresh it."
            );
            return Ok(committed.to_string());
        }
        anyhow::bail!(
            "refusing to write an incomplete RustBridgeC.h: the swift-bridge build output for \
             `{binding_crate_name}` uses {missing} but never defines them, and no populated header is \
             committed to fall back on. Re-run `cargo build -p {binding_crate_name}`"
        );
    }

    // Byte-preserve a committed header whose C is unchanged. `assembled` is swift-bridge's raw
    // output; what is committed has been through the formatter. Rewriting one into the other on
    // every run is pure churn, and (because `poly` skips hash-stamped files) which of the two
    // ships is decided by run ordering rather than by any input. ~keep
    if let Some(committed) = committed
        && declares_the_same_c(committed, &assembled)
    {
        return Ok(committed.to_string());
    }

    Ok(assembled)
}

/// Core swift-bridge types the assembled body uses but never defines.
///
/// Scoped to the types swift-bridge's own `SwiftBridgeCore.h` is responsible for (`RustStr`
/// and the `__private__*` family) rather than to a hard-coded list of names, so a
/// swift-bridge release that adds, drops or renames one of them changes nothing here. Types
/// referenced by no declaration are not demanded at all, which is why this stays quiet on a
/// binding crate whose surface happens to use none of them.
pub(crate) fn undefined_core_types(header: &str) -> Vec<String> {
    let tokens = c_tokens(header);
    let defined = defined_type_names(&tokens);
    let mut undefined: Vec<String> = tokens
        .iter()
        .filter(|token| is_core_type_name(token) && !defined.contains(token.as_str()))
        .cloned()
        .collect();
    undefined.sort();
    undefined.dedup();
    undefined
}

fn is_core_type_name(token: &str) -> bool {
    token == "RustStr" || token.starts_with("__private__")
}

/// Names a header defines a body for, in either shape swift-bridge uses: the typedef form
/// `typedef struct NAME { ... } NAME;` (matched on its `} NAME ;` tail) and the plain tag
/// form `struct NAME { ... };` (matched on its `struct NAME {` head). Both survive any
/// reformatting, since neither depends on line breaks or spacing.
fn defined_type_names(tokens: &[String]) -> std::collections::HashSet<&str> {
    let mut defined = std::collections::HashSet::new();
    for window in tokens.windows(3) {
        if window[0] == "}" && window[2] == ";" {
            defined.insert(window[1].as_str());
        }
        if window[0] == "struct" && window[2] == "{" {
            defined.insert(window[1].as_str());
        }
    }
    defined
}

/// Whether two headers declare the same C, ignoring comments, whitespace and the relative
/// order of top-level declarations.
///
/// Order-insensitivity across declarations is required, not incidental: clang-format's
/// `SortIncludes` reorders the two `#include` lines swift-bridge emits. Order *within* a
/// declaration is still compared, so this cannot call two headers equal because they happen
/// to use the same tokens in a different arrangement. ~keep
pub(crate) fn declares_the_same_c(left: &str, right: &str) -> bool {
    declaration_counts(left) == declaration_counts(right)
}

/// Multiset of declarations, each normalized to its space-joined token sequence.
///
/// Preprocessor directives are split off by line and everything else by `;`. A directive has
/// no terminator, so line structure is the only thing that ends one — without that split, the
/// last `#include` would merge into the declaration that follows it and `SortIncludes`
/// swapping the two `#include` lines would register as a change to that declaration. ~keep
fn declaration_counts(header: &str) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut body = String::new();
    for line in strip_comments(header).lines() {
        if !line.trim_start().starts_with('#') {
            body.push_str(line);
            body.push('\n');
            continue;
        }
        let directive = c_tokens(line).join(" ");
        if !directive.is_empty() {
            *counts.entry(directive).or_insert(0) += 1;
        }
    }
    for declaration in split_statements(&c_tokens(&body)) {
        *counts.entry(declaration).or_insert(0) += 1;
    }
    counts
}

/// Split a token stream into statements terminated by `;`, keeping any unterminated tail.
fn split_statements(tokens: &[String]) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for token in tokens {
        current.push(token);
        if token == ";" {
            statements.push(current.join(" "));
            current.clear();
        }
    }
    if !current.is_empty() {
        statements.push(current.join(" "));
    }
    statements
}

/// Tokenize C into identifiers/numbers and single punctuation characters, dropping comments
/// and all whitespace. `$` is part of an identifier here because every generated swift-bridge
/// symbol contains it.
fn c_tokens(header: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut identifier = String::new();
    for character in strip_comments(header).chars() {
        if character.is_alphanumeric() || character == '_' || character == '$' {
            identifier.push(character);
            continue;
        }
        if !identifier.is_empty() {
            tokens.push(std::mem::take(&mut identifier));
        }
        if !character.is_whitespace() {
            tokens.push(character.to_string());
        }
    }
    if !identifier.is_empty() {
        tokens.push(identifier);
    }
    tokens
}

/// Drop `//` and `/* */` comments. The `alef:hash:` provenance line a committed header
/// carries — and the marker preamble a freshly assembled one does not yet carry — are both
/// comments, so removing them is what lets the two be compared at all.
fn strip_comments(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    let mut characters = header.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '/' {
            out.push(character);
            continue;
        }
        match characters.peek() {
            Some('/') => {
                for inner in characters.by_ref() {
                    if inner == '\n' {
                        break;
                    }
                }
                out.push('\n');
            }
            Some('*') => {
                characters.next();
                let mut previous = '\0';
                for inner in characters.by_ref() {
                    if previous == '*' && inner == '/' {
                        break;
                    }
                    previous = inner;
                }
                out.push(' ');
            }
            _ => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests;
