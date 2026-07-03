#!/usr/bin/env python3
"""Post-collapse fixer: rewrite intra-crate `crate::*` references.

The original alef-core/src/* code used `crate::error::Error`, `crate::config`,
etc. to refer to itself. After the collapse, that code lives at
`src/core/*`, where `crate::error` no longer exists — only
`crate::core::error` does.

This script walks every `src/<module>/` directory, discovers the
direct child modules (subdirs and `.rs` files at depth 1), and
rewrites `crate::<child>::` → `crate::<module>::<child>::`.

Known top-level modules — must be left alone so we don't double-prefix:
  adapters, backends, cli, codegen, core, docs, e2e, extract, publish,
  readme, scaffold, snippets

Top-level rewriting only — `crate::backends::pyo3::*` is already a
fully-qualified path and shouldn't be touched.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SRC = REPO / "src"

TOP_LEVEL_MODULES = {
    "adapters",
    "backends",
    "cli",
    "codegen",
    "core",
    "docs",
    "e2e",
    "extract",
    "publish",
    "readme",
    "scaffold",
    "snippets",
}


_PUB_ITEM = re.compile(
    r"^pub(\s*\(\s*crate\s*\))?\s+"
    r"(?:fn|struct|enum|trait|type|const|static|use\s+[A-Za-z_:][\w:]*\s+as\s+)"
    r"\s*([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)
# Also pub use foo::bar; — captures the LAST identifier (re-export name)
_PUB_USE = re.compile(r"^pub\s+use\s+[A-Za-z_:][\w:]*::([A-Za-z_][A-Za-z0-9_]*)(?:\s*;|\s+as\b)", re.MULTILINE)
# pub use self::foo::* — captures the module name
_PUB_USE_GLOB = re.compile(
    r"^pub\s+use\s+(?:self::|crate::)?([A-Za-z_][A-Za-z0-9_]*)(?:::[^;]*)?\s*;",
    re.MULTILINE,
)


def child_modules(module_dir: Path) -> set[str]:
    """Return the names of submodules and pub items at the root of this module.

    Covers:
    - submodule files (foo.rs) and dirs (foo/mod.rs)
    - pub fn/struct/enum/trait/type/const/static at root (in mod.rs)
    - pub use foo::Bar; → Bar (the re-export)
    """
    names: set[str] = set()
    for p in module_dir.iterdir():
        if p.name in {"mod.rs", "lib.rs"}:
            continue
        if p.name == "templates":
            continue
        if p.is_file() and p.suffix == ".rs":
            names.add(p.stem)
        elif p.is_dir() and (p / "mod.rs").exists():
            names.add(p.name)

    mod_rs = module_dir / "mod.rs"
    if mod_rs.exists():
        text = mod_rs.read_text()
        for match in _PUB_ITEM.finditer(text):
            names.add(match.group(2))
        for match in _PUB_USE.finditer(text):
            names.add(match.group(1))
        # `pub use foo::*` — at minimum, foo's submodule is re-exported; the
        # names from foo aren't introspectable cheaply. The submodule scan above
        # handles foo itself.
    return names


def rewrite_module(module_dir: Path, module_name: str) -> int:
    """For every .rs file inside `module_dir`, rewrite intra-module `crate::*`
    references to be qualified through `crate::<module>::`.

    Handles three patterns:
    1. `crate::<child>::...`           → `crate::<module>::<child>::...`
    2. `crate::<child>` (terminal)     → `crate::<module>::<child>`
    3. `use crate::{a, b::c, d};`      → splits and qualifies each entry whose
       leading identifier is a known child.
    """
    children = child_modules(module_dir)
    safe_children = children - TOP_LEVEL_MODULES
    if not safe_children:
        return 0

    pattern_pieces = "|".join(re.escape(c) for c in sorted(safe_children, key=len, reverse=True))
    pattern_path = re.compile(rf"\bcrate::({pattern_pieces})::")
    pattern_end = re.compile(rf"\bcrate::({pattern_pieces})\b(?!::)")

    use_brace_block = re.compile(
        r"^(?P<indent>[ \t]*)use\s+crate::\{(?P<body>[^{}]*)\};",
        re.MULTILINE,
    )

    def rewrite_brace_block(match: re.Match) -> str:
        body = match.group("body")
        # Tokenize by commas at the top level of the brace block. We assume
        # one-line use-statements here (Rust style — multi-line braces also
        # work because we matched on \{[^{}]*\}, no nesting).
        items = [item.strip() for item in body.split(",") if item.strip()]
        out: list[str] = []
        for item in items:
            head = item.split("::", 1)[0].split(" ", 1)[0]  # leading ident
            if head in safe_children:
                out.append(f"{module_name}::{item}")
            else:
                out.append(item)
        new_body = ", ".join(out)
        return f"{match.group('indent')}use crate::{{{new_body}}};"

    count = 0
    for rs in module_dir.rglob("*.rs"):
        text = rs.read_text()
        new = use_brace_block.sub(rewrite_brace_block, text)
        new = pattern_path.sub(rf"crate::{module_name}::\1::", new)
        new = pattern_end.sub(rf"crate::{module_name}::\1", new)
        if new != text:
            rs.write_text(new)
            count += 1
    return count


def main() -> None:
    total = 0
    # Top-level: adapters, codegen, core, docs, e2e, extract, publish,
    # readme, scaffold, snippets, cli
    for entry in sorted(SRC.iterdir()):
        if not entry.is_dir():
            continue
        if entry.name == "backends":
            # Each backend is its own former crate
            for backend in sorted(entry.iterdir()):
                if backend.is_dir():
                    n = rewrite_module(backend, f"backends::{backend.name}")
                    if n:
                        print(f"backends/{backend.name}: rewrote {n} files")
                    total += n
        elif entry.name in TOP_LEVEL_MODULES:
            n = rewrite_module(entry, entry.name)
            if n:
                print(f"{entry.name}: rewrote {n} files")
            total += n
    print(f"\nTOTAL: rewrote {total} files")


if __name__ == "__main__":
    main()
