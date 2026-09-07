//! Where `TypeScriptValidator` allocates scratch for a snippet that has no configured session.
//!
//! ## Background: why "isolated" used to mean "can never see real types"
//!
//! Before this module existed, a session-less TypeScript snippet always validated in a fresh OS
//! temp directory ([`ScratchDir::isolated`]) -- deliberately unreachable from any real project, so
//! alef's own hand-written ambient declaration for constructs alef itself generates (see
//! `NODE_AMBIENT_DECLARATION_CONTENT` in `typescript.rs`) could be proven to work without a real
//! `@types/node` anywhere on the resolution path. That property is correct and stays correct here
//! -- see `node_ambient_declaration_tsc_tests.rs`.
//!
//! It stopped being the whole story once a *hand-written* doc snippet entered the picture. A
//! human can legitimately write `import { readFileSync } from "node:fs"` in a markdown doc under a
//! consumer's own snippet directory (as opposed to a generated snippet under
//! `snippets-generated/`, which only ever uses the finite set of Node constructs the ambient
//! declaration already covers). The consumer's real project can have `@types/node` installed and
//! committed -- but the OS temp directory alef validated that snippet in has no relationship to
//! that project at all, so the install is never seen. Extending the hand-written ambient
//! declaration cannot fix this in general: it only ever covers the finite set of constructs alef's
//! own templates emit, not whatever builtin a human happens to import next.
//!
//! ## The fix: nest scratch inside the real project, and name the types explicitly
//!
//! `resolve_isolated_scratch` is the "ask the real project instead of guessing" counterpart to
//! `kotlin::gradle_classpath::resolve_class_path`, which asks Gradle for a project's real
//! classpath rather than hand-maintaining one. There is no single tool to invoke for TypeScript
//! the way Gradle is invoked for Kotlin, but there is an equivalent concrete, checkable fact: an
//! ancestor `node_modules/@types/node` directory of the snippet's own real file.
//!
//! An earlier version of this module stopped at nesting scratch inside that ancestor and relied on
//! `tsc`'s *default automatic type-acquisition* (the behavior that includes every visible
//! `@types/*` package when a tsconfig sets no explicit `"types"` array) to notice it was there.
//! That was wrong, and provably so: a hand-probed `tsc` run against exactly this scratch layout
//! (`.alef/snippets/tmp/<random>/tsconfig.json` several levels below a real
//! `node_modules/@types/node`) failed to pick up the install at all under the `tsc` this machine's
//! `PATH` resolves to -- a TypeScript 7 native-compiler build, `typescript@7.0.2`, which does not
//! implement automatic type-acquisition the way the classic `typescript@5.x` compiler does (the
//! same install resolved perfectly under a locally installed classic 5.9.3 `tsc`, confirmed with
//! `--listFiles`: the classic compiler loaded the `@types/node` declaration file into the program,
//! the native one never did). `alef` shells out to whatever `tsc` a `PATH` lookup finds
//! (a `tool_command("tsc")` PATH resolve, no project-local `node_modules/.bin/tsc` preference), so it
//! cannot assume a classic compiler is what actually runs a given check.
//!
//! The fix that is portable across both: once [`nearest_ancestor_with_types_node`] has already
//! confirmed a real install exists, `TypeScriptValidator` writes `"types": ["node"]` into the
//! isolated tsconfig instead of leaving `"types"` unset. Naming a type-reference-directive
//! explicitly resolves via `typeRoots`, which both compiler generations implement identically
//! (confirmed the same way, against the same nested layout, under both `tsc` versions) --
//! automatic *discovery* of what to name is the part that differs between them, not resolution of
//! a name once given. This still hands "what `@types/node` actually contains" entirely to `tsc`;
//! alef only ever decides *whether* to name `"node"` and *where* to point `tsc` at, never what the
//! package looks like.
//!
//! That conditionality is load-bearing, not cosmetic: `"types": ["node"]` on a tsconfig with no
//! resolvable `@types/node` anywhere on its `typeRoots` search path fails with
//! `TS2688: Cannot find type definition file for 'node'` -- confirmed by hand-probing the same
//! `tsc` against a tsconfig naming `"node"` with no install in reach at all. That is, byte for
//! byte, the failure this whole feature exists to fix, so this module must never name `"node"`
//! except when [`resolve_isolated_scratch`]'s own ancestor check already found it. When no such
//! ancestor exists, `resolve_isolated_scratch` falls back to exactly the previous behavior:
//! [`ScratchDir::isolated`], the OS temp directory, unreachable from any real project, with no
//! `"types"` array named. Nothing about how a failure is classified or reported changes either way
//! -- a snippet that imports a builtin neither the ambient declaration nor a resolved
//! `@types/node` covers still fails with `tsc`'s own diagnostic (which, for the `TS2591` family,
//! already names the missing package and the install command). A silent downgrade to
//! "unavailable" or a skipped check would be worse than today's hard failure; this module never
//! does that. ~keep

use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use std::path::{Path, PathBuf};

/// The one filesystem fact that answers "does a real, already-installed `@types/node` exist
/// somewhere this snippet's own project can see" -- checked the same way `node_modules/@types/*`
/// resolution itself would, so a pnpm/yarn workspace's symlinked `node_modules/@types/node` still
/// resolves correctly (`Path::is_file` follows symlinks). ~keep
const TYPES_NODE_MARKER: &str = "node_modules/@types/node/package.json";

/// Walks upward from `directory` (inclusive) for the nearest ancestor with `@types/node`
/// installed. Returns `None` when no ancestor has it, all the way to the filesystem root.
fn nearest_ancestor_with_types_node(directory: &Path) -> Option<PathBuf> {
    directory
        .ancestors()
        .find(|candidate| candidate.join(TYPES_NODE_MARKER).is_file())
        .map(Path::to_path_buf)
}

/// Resolves a real, on-disk anchor directory for `snippet_path`, or `None` when it does not
/// correspond to a real file -- which is deliberate, not a gap: a synthetic path (every
/// hand-constructed `Snippet` fixture in this crate's own tests uses a bare `"snippet.ts"`, which
/// resolves relative to the *test process's* working directory if treated as real) must never be
/// treated as a real project location, or a unit test could accidentally probe this repository's
/// own tree. Only a snippet whose `path` [`Path::canonicalize`]s successfully -- i.e. one
/// `discover_snippets` actually read off disk -- is eligible. ~keep
fn real_snippet_directory(snippet_path: &Path) -> Option<PathBuf> {
    snippet_path
        .canonicalize()
        .ok()
        .and_then(|canonical| canonical.parent().map(Path::to_path_buf))
}

/// Allocates scratch for a session-less TypeScript check: nested inside the nearest real ancestor
/// project that already has `@types/node` installed, when `anchor` names one, or in the OS temp
/// directory otherwise. The second element reports whether that ancestor was found -- the caller
/// must use it to decide whether the tsconfig it writes may name `"node"` in `types` (see the
/// module docs' "explicit, not implicit" section: naming it when nothing resolves reproduces the
/// exact `TS2688` failure this whole mechanism exists to prevent). See the module docs for why
/// placement alone is not the fix and not a guess at any particular consumer's install layout.
///
/// # Errors
///
/// Returns an error under the same conditions [`ScratchDir::rooted`] and [`ScratchDir::isolated`]
/// do: the resolved scratch root cannot be swept or created, or a unique directory cannot be
/// allocated inside it.
pub(super) fn resolve_isolated_scratch(anchor: Option<&Path>, timeout_secs: u64) -> Result<(ScratchDir, bool)> {
    let project_root = anchor
        .and_then(real_snippet_directory)
        .and_then(|directory| nearest_ancestor_with_types_node(&directory));
    match project_root {
        Some(root) => Ok((ScratchDir::rooted(&root, timeout_secs)?, true)),
        None => Ok((ScratchDir::isolated()?, false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_types_node_at_the_directory_itself() {
        let root = tempfile::tempdir().expect("project root");
        std::fs::create_dir_all(root.path().join("node_modules/@types/node")).expect("types/node directory");
        std::fs::write(root.path().join("node_modules/@types/node/package.json"), "{}").expect("package.json");

        assert_eq!(
            nearest_ancestor_with_types_node(root.path()),
            Some(root.path().to_path_buf())
        );
    }

    #[test]
    fn finds_types_node_several_levels_up() {
        let root = tempfile::tempdir().expect("project root");
        std::fs::create_dir_all(root.path().join("node_modules/@types/node")).expect("types/node directory");
        std::fs::write(root.path().join("node_modules/@types/node/package.json"), "{}").expect("package.json");
        let nested = root.path().join("docs-site/src/snippets/typescript/api");
        std::fs::create_dir_all(&nested).expect("nested snippet directory");

        assert_eq!(
            nearest_ancestor_with_types_node(&nested),
            Some(root.path().to_path_buf())
        );
    }

    /// The nearest ancestor wins, not the topmost one -- a monorepo can have `@types/node`
    /// installed at more than one level (a root install and a package-local override), and the
    /// closer one is the one the snippet's own directory would actually resolve. ~keep
    #[test]
    fn the_nearest_ancestor_wins_over_a_more_distant_one() {
        let root = tempfile::tempdir().expect("workspace root");
        for level in ["", "packages/docs-site"] {
            let types_dir = root.path().join(level).join("node_modules/@types/node");
            std::fs::create_dir_all(&types_dir).expect("types/node directory");
            std::fs::write(types_dir.join("package.json"), "{}").expect("package.json");
        }
        let nested = root.path().join("packages/docs-site/src/snippets");
        std::fs::create_dir_all(&nested).expect("nested snippet directory");

        assert_eq!(
            nearest_ancestor_with_types_node(&nested),
            Some(root.path().join("packages/docs-site"))
        );
    }

    #[test]
    fn no_ancestor_with_types_node_resolves_to_none() {
        let root = tempfile::tempdir().expect("project root");
        let nested = root.path().join("src/snippets");
        std::fs::create_dir_all(&nested).expect("nested snippet directory");

        assert_eq!(nearest_ancestor_with_types_node(&nested), None);
    }

    /// A directory entry named `@types/node` that is not actually a resolvable package (no
    /// `package.json`) must not be mistaken for a real install -- the marker is the file, not the
    /// directory name.
    #[test]
    fn an_empty_types_node_directory_without_a_package_json_does_not_count() {
        let root = tempfile::tempdir().expect("project root");
        std::fs::create_dir_all(root.path().join("node_modules/@types/node")).expect("types/node directory");

        assert_eq!(nearest_ancestor_with_types_node(root.path()), None);
    }

    /// The safety property the module docs promise: a synthetic, non-existent snippet path (every
    /// hand-constructed fixture in this crate's own tests) must never resolve to a real ancestor,
    /// however many `@types/node` installs happen to sit above the test process's own working
    /// directory. `resolve_isolated_scratch` must fall through to `ScratchDir::isolated` for it,
    /// exactly like the pre-existing "no anchor at all" (`None`) case. ~keep
    #[test]
    fn a_synthetic_snippet_path_never_resolves_to_a_real_project_root() {
        let synthetic = Path::new("snippet.ts");

        assert_eq!(real_snippet_directory(synthetic), None);
    }

    #[test]
    fn resolve_isolated_scratch_falls_back_to_the_os_temp_directory_with_no_anchor() {
        let (scratch, types_node_resolved) = resolve_isolated_scratch(None, 5).expect("isolated scratch directory");

        assert!(
            scratch.path().starts_with(std::env::temp_dir()),
            "no anchor must fall back to the OS temp directory: {}",
            scratch.path().display()
        );
        assert!(
            !types_node_resolved,
            "no anchor must never report a resolved @types/node -- the caller uses this to decide \
             whether `\"types\": [\"node\"]` is safe to name"
        );
    }

    #[test]
    fn resolve_isolated_scratch_nests_inside_a_resolved_real_project_root() {
        let root = tempfile::tempdir().expect("project root");
        let canonical_root = root.path().canonicalize().expect("canonical project root");
        std::fs::create_dir_all(canonical_root.join("node_modules/@types/node")).expect("types/node directory");
        std::fs::write(canonical_root.join("node_modules/@types/node/package.json"), "{}").expect("package.json");
        let snippet_file = canonical_root.join("docs/example.md");
        std::fs::create_dir_all(snippet_file.parent().unwrap()).expect("docs directory");
        std::fs::write(&snippet_file, "example").expect("snippet source file");

        let (scratch, types_node_resolved) =
            resolve_isolated_scratch(Some(snippet_file.as_path()), 5).expect("rooted scratch directory");

        assert!(
            scratch
                .path()
                .starts_with(canonical_root.join(crate::snippets::scratch::SNIPPET_SCRATCH_ROOT)),
            "a real project root with @types/node must nest scratch under its own {} convention, got {}",
            crate::snippets::scratch::SNIPPET_SCRATCH_ROOT,
            scratch.path().display()
        );
        assert!(
            types_node_resolved,
            "a resolved ancestor @types/node must be reported so the caller can safely name \
             `\"types\": [\"node\"]`"
        );
    }
}
