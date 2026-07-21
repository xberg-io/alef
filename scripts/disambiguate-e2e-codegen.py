#!/usr/bin/env python3
"""Disambiguate `crate::codegen::<X>` references inside src/e2e/.

There are two `codegen` modules after the collapse:
- src/codegen/   (former alef-codegen — shared codegen utilities)
- src/e2e/codegen/ (former alef-e2e::codegen — per-language test emitters)

Both end up reachable via `crate::codegen::*`, which is ambiguous. We split them
by which submodule names exist in each: if `crate::codegen::<X>` references an
X present in src/codegen/, it's the external one (leave alone). If X is only in
src/e2e/codegen/, it's the e2e-internal one — rewrite to
`crate::e2e::codegen::<X>::`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SRC = REPO / "src"

_PUB_ITEM_RE = re.compile(
    r"^pub(\s*\(\s*crate\s*\))?\s+"
    r"(?:fn|struct|enum|trait|type|const|static)"
    r"\s*([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)


def collect_module_names(mod_dir: Path) -> set[str]:
    """Submodule names + pub items defined at mod_dir/mod.rs root."""
    names: set[str] = {
        p.stem if p.suffix == ".rs" else p.name for p in mod_dir.iterdir() if p.name not in {"mod.rs", "templates"}
    }
    mod_rs = mod_dir / "mod.rs"
    if mod_rs.exists():
        for m in _PUB_ITEM_RE.finditer(mod_rs.read_text()):
            names.add(m.group(2))
    return names


TOP_CODEGEN = collect_module_names(SRC / "codegen")
E2E_CODEGEN = collect_module_names(SRC / "e2e" / "codegen")

E2E_ONLY = E2E_CODEGEN - TOP_CODEGEN

print(f"top codegen children: {sorted(TOP_CODEGEN)}")
print(f"e2e codegen children: {sorted(E2E_CODEGEN)}")
print(f"e2e-only (rewrite targets): {sorted(E2E_ONLY)}")

if not E2E_ONLY:
    print("Nothing to rewrite.")
    sys.exit(0)

pattern_pieces = "|".join(re.escape(c) for c in sorted(E2E_ONLY, key=len, reverse=True))
pattern = re.compile(rf"\bcrate::codegen::({pattern_pieces})\b")
replacement = r"crate::e2e::codegen::\1"

count = 0
for rs in (SRC / "e2e").rglob("*.rs"):
    text = rs.read_text()
    new = pattern.sub(replacement, text)
    if new != text:
        rs.write_text(new)
        count += 1
        print(f"rewrote {rs.relative_to(REPO)}")

print(f"\nTOTAL: {count} files rewritten")
