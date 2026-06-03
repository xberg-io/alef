#!/usr/bin/env python3
"""Reject downstream project-name special casing in enforced text files."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from re import Pattern

PROJECT_MENTION_MESSAGE = (
    "Alef must stay project-agnostic; model downstream differences through "
    "`alef.toml` or generic config, not project-name special cases."
)
DOMAIN_TYPE_MESSAGE = (
    "Alef generator code must not hard-code downstream domain types; model these "
    "branches through generic IR metadata or backend configuration."
)

DOC_EXTENSIONS = {
    ".md",
    ".markdown",
    ".mdx",
    ".mdc",
    ".rst",
    ".txt",
}

SKIP_PATH_PARTS = {
    ".git",
    ".alef",
    ".cursor",
    "__pycache__",
    "fixtures",
    "snapshots",
    "target",
    "tests",
}

# This hook is intentionally opinionated about production code: generic capabilities
# such as package naming, hosted documentation, or repository metadata belong in
# config-driven Alef infrastructure, while downstream/product-specific names do not.
POLICY_FILES = {
    "hooks/check_project_mentions.py",
    "tests/project_mentions_hook.rs",
}

PROJECT_NAMES = {
    "kreuzberg": ("kreuz", "berg"),
    "kreuzcrawl": ("kreuzcrawl",),
    "tree-sitter-language-pack": ("tree", "sitter", "language", "pack"),
    "ts-pack": ("ts", "pack"),
    "html-to-markdown": ("html", "to", "markdown"),
    "h2m": ("h2m",),
    "spikard": ("spikard",),
    "liter-llm": ("liter", "llm"),
    "lllm": ("lllm",),
}

ALEF_INFRASTRUCTURE_ALLOWLIST = (
    "kreuzberg-dev/actions",
    "kreuzberg-dev/alef",
    "kreuzberg-dev/ai-rulez",
    "kreuzberg-dev/homebrew-tap",
    "github.com/kreuzberg-dev/pre-commit-hooks",
    "owner: kreuzberg-dev",
    "kreuzberg-bot",
    "bot@kreuzberg.dev",
    "docs.<repo>.kreuzberg.dev",
    "kreuzberg, inc.",
)

DOWNSTREAM_DOMAIN_TYPES = (
    "InternalDocument",
    "ExtractionConfig",
    "EmbeddingConfig",
    "ChunkingConfig",
    "BatchBytesItem",
    "BatchFileItem",
    "ConversionOptions",
    "HtmlVisitor",
    "IHtmlVisitor",
    "OcrBackend",
    "VisitorBridge",
    "VisitorHandle",
)


def build_pattern(parts: tuple[str, ...]) -> Pattern[str]:
    body = r"[\s_-]*".join(re.escape(part) for part in parts)
    return re.compile(rf"(?<![a-z0-9]){body}(?![a-z0-9])")


PATTERNS = {name: build_pattern(parts) for name, parts in PROJECT_NAMES.items()}
INFRASTRUCTURE_PATTERNS = tuple(
    re.compile(re.escape(allowed), re.IGNORECASE) for allowed in ALEF_INFRASTRUCTURE_ALLOWLIST
)
DOMAIN_TYPE_PATTERNS = tuple(
    (name, re.compile(rf"(?<![A-Za-z0-9_]){name}(?![A-Za-z0-9_])")) for name in DOWNSTREAM_DOMAIN_TYPES
)
DOMAIN_TYPE_SPECIAL_CASE_MARKERS = (
    "==",
    "!=",
    "match ",
    "matches!",
    "ends_with(",
    "unwrap_or(",
)
DOMAIN_TYPE_EMBEDDED_BEHAVIOR_MARKERS = (
    "class ",
    "interface ",
    "new ",
    "struct ",
    "type ",
    "format!(",
    "const ",
    "public ",
    "private ",
    "internal ",
)


def is_enforced_path(path: Path) -> bool:
    normalized = path.as_posix()
    if any(normalized == policy_file or normalized.endswith(f"/{policy_file}") for policy_file in POLICY_FILES):
        return False
    if any(part in SKIP_PATH_PARTS for part in path.parts):
        return False
    if ".ai-rulez" in path.parts:
        return True
    return path.suffix.lower() not in DOC_EXTENSIONS


def is_production_generator_path(path: Path) -> bool:
    parts = path.parts
    if "src" not in parts:
        return False
    src_index = parts.index("src")
    if len(parts) <= src_index + 1:
        return False
    if parts[src_index + 1] in {"backends", "codegen"}:
        return True
    return len(parts) > src_index + 2 and parts[src_index + 1] == "e2e" and parts[src_index + 2] == "codegen"


def read_text(path: Path) -> str | None:
    try:
        data = path.read_bytes()
    except OSError:
        return None
    if b"\x00" in data:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


def mask_allowed_infrastructure(line: str) -> str:
    masked = line
    for pattern in INFRASTRUCTURE_PATTERNS:
        masked = pattern.sub("", masked)
    return masked


def normalize_for_project_mentions(line: str) -> str:
    with_acronym_boundaries = re.sub(r"(?<=[A-Z])(?=[A-Z][a-z])", " ", line)
    with_camel_boundaries = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", " ", with_acronym_boundaries)
    return with_camel_boundaries.lower()


def is_domain_type_special_case(line: str) -> bool:
    stripped = line.strip()
    if stripped.startswith(("//", "///", "#")):
        return False
    if "assert" in stripped:
        return False
    return any(marker in stripped for marker in DOMAIN_TYPE_SPECIAL_CASE_MARKERS) or any(
        marker in stripped for marker in DOMAIN_TYPE_EMBEDDED_BEHAVIOR_MARKERS
    )


def violations_for_file(path: Path) -> list[str]:
    if not is_enforced_path(path):
        return []

    content = read_text(path)
    if content is None:
        return []

    violations: list[str] = []
    for line_number, line in enumerate(content.splitlines(), start=1):
        masked_line = normalize_for_project_mentions(mask_allowed_infrastructure(line))
        for name, pattern in PATTERNS.items():
            if pattern.search(masked_line):
                violations.append(f"{path}:{line_number}: forbidden project mention `{name}`")
        if is_production_generator_path(path) and is_domain_type_special_case(line):
            for name, pattern in DOMAIN_TYPE_PATTERNS:
                if pattern.search(line):
                    violations.append(f"{path}:{line_number}: forbidden downstream domain type `{name}`")
    return violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*", help="Files to scan")
    args = parser.parse_args(argv)

    violations: list[str] = []
    for raw in args.files:
        path = Path(raw)
        if path.is_file():
            violations.extend(violations_for_file(path))

    if violations:
        for violation in violations:
            print(violation, file=sys.stderr)
        print(f"\n{PROJECT_MENTION_MESSAGE}", file=sys.stderr)
        print(DOMAIN_TYPE_MESSAGE, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
