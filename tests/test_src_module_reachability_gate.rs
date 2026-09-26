//! Mechanical guard against `.rs` files under `src/` that hold `#[test]` functions but are never
//! wired into the crate's module tree.
//!
//! `mod generators;` was missing from `src/codegen/config_gen/tests.rs`, so
//! `src/codegen/config_gen/tests/generators.rs` and its 18 `#[test]` functions were never part of
//! the crate — `cargo test --lib` never listed them, never ran them, and never warned, because an
//! undeclared module file is simply not source input to `rustc`. This file makes that shape
//! unrepresentable going forward: it re-derives the real module graph from `src/lib.rs` (and
//! `src/main.rs`) by resolving every `mod` item the way `rustc` does, and fails when a source file
//! containing `#[test]` sits outside that graph.
//!
//! ## Why this is not a regex over parent files
//!
//! A naive scan for `mod <stem>;` in the obvious parent file produces dozens of false positives in
//! this tree, because the real module shapes are richer than "sibling file, sibling declaration":
//!
//! - `foo/bar.rs` is declared as `mod bar;` in `foo.rs` **or** in `foo/mod.rs` — both are valid
//!   parents and a scan that only checks one produces a false positive on the other.
//! - Submodules declared inside an inline `#[cfg(test)] mod tests { mod x; }` block resolve to
//!   `foo/tests/x.rs`, not `foo/x.rs` — the inline module name becomes a path segment even though
//!   it has no file of its own.
//! - `#[path = "..."]` attributes redirect resolution entirely, and — per `rustc`'s own rule — are
//!   always resolved relative to the directory of the file that carries the attribute, not to the
//!   implied submodule directory of the module doing the declaring. `types.rs` declaring
//!   `#[path = "types/tests.rs"] mod tests;` and `all_commands.rs` declaring
//!   `#[path = "all_commands_tests.rs"] mod tests;` both exist in this tree and resolve to
//!   different shapes for the same reason.
//!
//! [`reachable_src_files`] performs the same resolution `rustc` performs: it walks `Item::Mod`
//! nodes with `syn`, follows inline module bodies by descending a virtual directory one segment
//! per inline `mod name { .. }`, and treats `#[path]` as relative to the *containing file's*
//! directory rather than the declaring module's submodule directory.
//!
//! Where it runs: nowhere new. An ordinary integration test picked up by `cargo test --workspace`.

#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use syn::visit::Visit;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Repo-relative, `/`-separated paths of every Rust source file under `src/`.
///
/// `git ls-files` is authoritative because the rule governs committed content.
fn all_src_sources() -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z", "--", "src/*.rs"])
        .output()
        .unwrap_or_else(|error| panic!("git ls-files: {error}"));
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut paths: Vec<String> = String::from_utf8(output.stdout)
        .expect("git ls-files output must be UTF-8")
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    paths.sort();
    assert!(
        paths.len() > 500,
        "expected src/ to hold hundreds of source files; found {} — the enumeration is broken, \
         not the tree",
        paths.len()
    );
    paths
}

fn parse(repo_root: &Path, relative: &Path) -> syn::File {
    let absolute = repo_root.join(relative);
    let source =
        std::fs::read_to_string(&absolute).unwrap_or_else(|error| panic!("read {}: {error}", relative.display()));
    syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {}: {error}", relative.display()))
}

fn is_test_fn(function: &syn::ItemFn) -> bool {
    function.attrs.iter().any(|attribute| attribute.path().is_ident("test"))
}

/// True if `file`, parsed on its own (including any inline `mod { .. }` bodies it contains),
/// holds at least one `#[test]` function.
struct HasTestFn(bool);

impl<'ast> Visit<'ast> for HasTestFn {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if is_test_fn(function) {
            self.0 = true;
        }
        syn::visit::visit_item_fn(self, function);
    }
}

fn file_contains_test_fn(file: &syn::File) -> bool {
    let mut finder = HasTestFn(false);
    finder.visit_file(file);
    finder.0
}

/// The string value of a `#[path = "..."]` attribute, if present.
fn path_attr_value(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attribute| {
        if !attribute.path().is_ident("path") {
            return None;
        }
        let syn::Meta::NameValue(name_value) = &attribute.meta else {
            return None;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(literal),
            ..
        }) = &name_value.value
        else {
            return None;
        };
        Some(literal.value())
    })
}

/// Collapse `..` and `.` components the way a filesystem path resolver would, without touching
/// the filesystem. `#[path]` values in this tree are all simple relative subpaths, but this keeps
/// the resolver correct if one ever climbs a directory.
fn normalize(path: PathBuf) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out.into_iter().collect()
}

/// Resolve a `mod name;` (no inline body) declaration to the real file it names, `rustc`-style.
///
/// `file_dir` is the directory of the file that *contains* the `mod` item — the anchor for
/// `#[path]`. `children_dir` is that file's own submodule directory — `foo/` for both `foo.rs`
/// and `foo/mod.rs`, and `foo/tests/` for a `mod x;` written inside an inline
/// `mod tests { .. }` block in `foo.rs`. Ordinary (non-`#[path]`) resolution uses `children_dir`.
///
/// `path_anchor` is where a `#[path]` value is resolved from, and it is NOT simply the containing
/// file's directory. Per `rustc`, in a non-`mod-rs` file the attribute anchors to that file's
/// directory only at the file's top level; inside an inline `mod name { .. }` the inline segments
/// count as directories, so `#[path = "x.rs"]` written in `mod tests` inside `foo.rs` resolves to
/// `foo/tests/x.rs`. Anchoring unconditionally to the file's directory made this gate report
/// three genuinely-compiled test files as dead. ~keep
fn resolve_external_mod(
    repo_root: &Path,
    path_anchor: &Path,
    children_dir: &Path,
    ident: &str,
    explicit_path: Option<String>,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit_path {
        let candidate = normalize(path_anchor.join(explicit));
        return repo_root.join(&candidate).is_file().then_some(candidate);
    }
    let as_file = normalize(children_dir.join(format!("{ident}.rs")));
    if repo_root.join(&as_file).is_file() {
        return Some(as_file);
    }
    let as_mod_rs = normalize(children_dir.join(ident).join("mod.rs"));
    if repo_root.join(&as_mod_rs).is_file() {
        return Some(as_mod_rs);
    }
    None
}

/// The repo-relative path an `include!("...")` item-position macro invocation names, resolved
/// relative to the directory of the file containing the invocation (the same anchor `#[path]`
/// uses). Any macro shape other than a single string-literal argument is not `include!` as used
/// for module splicing and is ignored.
fn include_target(mac: &syn::Macro, file_dir: &Path) -> Option<PathBuf> {
    let literal: syn::LitStr = mac.parse_body().ok()?;
    Some(normalize(file_dir.join(literal.value())))
}

/// Walk one parsed file's items, recording every file `mod` declaration reaches (directly or
/// through further nested modules) into `reachable`.
///
/// `file_dir` is fixed for the whole file (used only to anchor `include!`). `path_anchor` starts
/// as `file_dir` and switches to the inline module's directory once inside one. `children_dir` starts
/// as the file's own submodule directory and descends one segment per inline `mod name { .. }`
/// nesting level, mirroring where `rustc` would look for `mod x;` written inside that block.
fn walk_items(
    repo_root: &Path,
    items: &[syn::Item],
    file_dir: &Path,
    path_anchor: &Path,
    children_dir: &Path,
    reachable: &mut BTreeSet<PathBuf>,
) {
    for item in items {
        if let syn::Item::Macro(item_macro) = item {
            // `include!("path.rs")` splices the target file's tokens in place — it is a distinct
            // reachability mechanism from `mod`, resolved relative to the *current file's own*
            // directory (like `#[path]`), and it does not open a new module: any `mod` items the
            // spliced file goes on to declare are resolved as if written at this exact point, so
            // recursion keeps the same `file_dir`/`children_dir`. `alef` uses this to split large
            // generator files (e.g. `backends/jni/gen_shims.rs`) into pieces that stay in one
            // module. ~keep
            if item_macro.mac.path.is_ident("include")
                && let Some(included) = include_target(&item_macro.mac, file_dir)
                && repo_root.join(&included).is_file()
                && reachable.insert(included.clone())
            {
                let file = parse(repo_root, &included);
                walk_items(repo_root, &file.items, file_dir, path_anchor, children_dir, reachable);
            }
            continue;
        }
        let syn::Item::Mod(module) = item else { continue };
        let explicit_path = path_attr_value(&module.attrs);
        let ident = module.ident.to_string();
        match &module.content {
            Some((_, inline_items)) => {
                // An inline body has no file of its own; its submodules resolve one directory
                // segment deeper than the enclosing file's own submodule directory.
                let inline_children_dir = children_dir.join(&ident);
                walk_items(
                    repo_root,
                    inline_items,
                    file_dir,
                    &inline_children_dir,
                    &inline_children_dir,
                    reachable,
                );
            }
            None => {
                let used_explicit_path = explicit_path.is_some();
                let Some(resolved) = resolve_external_mod(repo_root, path_anchor, children_dir, &ident, explicit_path)
                else {
                    // Unresolvable mod declaration is a different bug (a broken build), not this
                    // gate's concern; rustc itself will refuse to compile it.
                    continue;
                };
                if reachable.insert(resolved.clone()) {
                    let file = parse(repo_root, &resolved);
                    let new_file_dir = resolved
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from(""));
                    // rustc rule, confirmed empirically against two nesting depths (E0583's
                    // suggested paths in both cases): a module resolved via an explicit `#[path]`
                    // treats the resolved file exactly like a `mod.rs` — its own further plain
                    // `mod z;` children live in the SAME directory as the resolved file, not in a
                    // subdirectory named after the resolved file's stem or the module's own name.
                    // Without `#[path]`, the ordinary `mod.rs` vs `foo.rs` stem rule applies.
                    let new_children_dir =
                        if used_explicit_path || resolved.file_name().and_then(|n| n.to_str()) == Some("mod.rs") {
                            new_file_dir.clone()
                        } else {
                            resolved.with_extension("")
                        };
                    walk_items(
                        repo_root,
                        &file.items,
                        &new_file_dir,
                        &new_file_dir,
                        &new_children_dir,
                        reachable,
                    );
                }
            }
        }
    }
}

/// Repo-relative paths of `.rs` files pulled in as source *text* by `include_str!`, resolved
/// against the directory of the file carrying the macro (the way `rustc` resolves it).
///
/// A file under a `testdata/` directory is not a crate module: it is the source of a throwaway
/// cargo project this crate generates, compiles and runs at test time, so its `#[test]`
/// functions run in that project rather than in this one. Reachability via `mod` is the wrong
/// question for it. Being `include_str!`d is the evidence that it is live — a stray file in
/// `testdata/` that nothing includes still fails the gate. ~keep
fn include_str_targets(repo_root: &Path, sources: &[String]) -> BTreeSet<PathBuf> {
    let mut targets = BTreeSet::new();
    for source in sources {
        let relative = PathBuf::from(source);
        let Some(dir) = relative.parent().map(Path::to_path_buf) else {
            continue;
        };
        let text =
            std::fs::read_to_string(repo_root.join(&relative)).unwrap_or_else(|error| panic!("read {source}: {error}"));
        for capture in text.split("include_str!(\"").skip(1) {
            let Some((literal, _)) = capture.split_once('"') else {
                continue;
            };
            if literal.ends_with(".rs") {
                targets.insert(normalize(dir.join(literal)));
            }
        }
    }
    targets
}

/// Every file reachable from a crate root (`src/lib.rs`, `src/main.rs`), by real `mod`
/// resolution.
fn reachable_src_files(repo_root: &Path) -> BTreeSet<PathBuf> {
    let mut reachable = BTreeSet::new();
    for root in ["src/lib.rs", "src/main.rs"] {
        let root_path = repo_root.join(root);
        if !root_path.is_file() {
            continue;
        }
        let relative = PathBuf::from(root);
        reachable.insert(relative.clone());
        let file = parse(repo_root, &relative);
        // A crate root's own submodule directory is `src/`, exactly like a `mod.rs`.
        walk_items(
            repo_root,
            &file.items,
            Path::new("src"),
            Path::new("src"),
            Path::new("src"),
            &mut reachable,
        );
    }
    reachable
}

/// A `.rs` file under `src/` that holds a `#[test]` function but is not reachable via `mod` from
/// any crate root never compiles, so `cargo test` silently never lists or runs those tests.
#[test]
fn every_src_test_file_is_reachable_via_mod_declaration() {
    let repo_root = repo_root();
    let reachable = reachable_src_files(&repo_root);
    let sources = all_src_sources();
    let included = include_str_targets(&repo_root, &sources);
    assert!(
        !included.is_empty(),
        "no `include_str!(\"*.rs\")` target resolved — the exemption below would be vacuous and \
         this gate would be asserting nothing about testdata"
    );

    let mut offenders = Vec::new();
    for path in sources {
        let relative = PathBuf::from(&path);
        let is_included_testdata = included.contains(&relative)
            && relative
                .components()
                .any(|component| component.as_os_str() == "testdata");
        if is_included_testdata {
            continue;
        }
        let file = parse(&repo_root, &relative);
        if file_contains_test_fn(&file) && !reachable.contains(&relative) {
            offenders.push(path);
        }
    }

    assert!(
        offenders.is_empty(),
        "{} `.rs` file(s) under src/ hold a `#[test]` function but are not reachable via any \
         `mod` declaration from src/lib.rs or src/main.rs, so cargo never compiles them and the \
         tests inside never run:\n{}\n\n\
         Add the missing `mod <name>;` declaration in the parent module, or delete the file if \
         it is genuinely dead.",
        offenders.len(),
        offenders
            .iter()
            .map(|p| format!("  {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
