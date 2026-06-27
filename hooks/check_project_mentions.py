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

BASE_SKIP_PATH_PARTS = {
    ".git",
    ".alef",
    ".cursor",
    "__pycache__",
    "target",
}

DEFAULT_SKIP_PATH_PARTS = {
    "fixtures",
    "snapshots",
    "tests",
}

# This hook is intentionally opinionated about production code: generic capabilities
# such as package naming, hosted documentation, or repository metadata belong in
# config-driven Alef infrastructure, while downstream/product-specific names do not.
POLICY_FILES = {
    "hooks/check_project_mentions.py",
    "tests/project_mentions_hook.rs",
    "tests/cli_no_project_special_casing.rs",
}

PROJECT_NAMES = {
    "kreuzberg": ("kreuz", "berg"),
    "xberg": ("xberg",),
    "kreuzcrawl": ("kreuzcrawl",),
    "crawlberg": ("crawl", "berg"),
    "tree-sitter-language-pack": ("tree", "sitter", "language", "pack"),
    "ts-pack": ("ts", "pack"),
    "html-to-markdown": ("html", "to", "markdown"),
    "h2m": ("h2m",),
    "spikard": ("spikard",),
    "liter-llm": ("liter", "llm"),
    "lllm": ("lllm",),
}

DOWNSTREAM_SAMPLE_NAMES = {
    "sample-llm": ("sample", "llm"),
    "sample-markdown": ("sample", "markdown"),
    "sample-crawler": ("sample", "crawler"),
}

# Alef's own brand/org infrastructure is allowed; downstream product names are not.
# Masking the `xberg-io` org namespace and the `xberg.io` domain (and every
# `docs.<repo>.xberg.io` subdomain) keeps Alef's own actions, repos, package owners,
# and brand references clean while still catching the `xberg` product name itself —
# e.g. `xberg-io/xberg` masks to `/xberg`, which remains a forbidden mention.
ALEF_INFRASTRUCTURE_ALLOWLIST = (
    "xberg-io",
    "xberg.io",
    "owner: kreuzberg-dev",
    "kreuzberg-bot",
    "kreuzberg, inc.",
)

DOWNSTREAM_DOMAIN_TYPES = (
    "InternalDocument",
    "ExtractionConfig",
    "ExtractionResult",
    "EmbeddingConfig",
    "ChunkingConfig",
    "BatchBytesItem",
    "BatchFileItem",
    "ConversionOptions",
    "ConversionResult",
    "HtmlVisitor",
    "IHtmlVisitor",
    "OcrBackend",
    "VisitorHandle",
)


def build_pattern(parts: tuple[str, ...]) -> Pattern[str]:
    body = r"[\s_-]*".join(re.escape(part) for part in parts)
    return re.compile(rf"(?<![a-z0-9]){body}(?![a-z0-9])")


def split_camel_name(name: str) -> tuple[str, ...]:
    return tuple(re.findall(r"[A-Z]+(?=[A-Z][a-z]|\b)|[A-Z]?[a-z]+|[0-9]+", name))


def build_split_domain_type_pattern(name: str) -> Pattern[str]:
    parts = split_camel_name(name)
    if len(parts) <= 1:
        return re.compile(r"$^")
    joiner = r"""["'`]*\s*(?:~|\+|,)\s*["'`]*"""
    body = joiner.join(re.escape(part) for part in parts)
    return re.compile(rf"(?<![A-Za-z0-9_]){body}(?![A-Za-z0-9_])")


def build_embedded_domain_type_pattern(name: str) -> Pattern[str]:
    return re.compile(rf"(?<![A-Za-z0-9_])[A-Z][A-Za-z0-9_]*{name}[A-Za-z0-9_]*(?![A-Za-z0-9_])")


PATTERNS = {name: build_pattern(parts) for name, parts in PROJECT_NAMES.items()}
SAMPLE_PATTERNS = {name: build_pattern(parts) for name, parts in DOWNSTREAM_SAMPLE_NAMES.items()}
INFRASTRUCTURE_PATTERNS = tuple(
    re.compile(re.escape(allowed), re.IGNORECASE) for allowed in ALEF_INFRASTRUCTURE_ALLOWLIST
)
DOMAIN_TYPE_PATTERNS = tuple(
    (name, re.compile(rf"(?<![A-Za-z0-9_]){name}(?![A-Za-z0-9_])")) for name in DOWNSTREAM_DOMAIN_TYPES
)
SPLIT_DOMAIN_TYPE_PATTERNS = tuple((name, build_split_domain_type_pattern(name)) for name in DOWNSTREAM_DOMAIN_TYPES)
EMBEDDED_DOMAIN_TYPE_PATTERNS = tuple(
    (name, build_embedded_domain_type_pattern(name)) for name in DOWNSTREAM_DOMAIN_TYPES
)
DOMAIN_TYPE_SPECIAL_CASE_MARKERS = (
    "==",
    "!=",
    "match ",
    "matches!",
    "ends_with(",
    "unwrap_or(",
    "unwrap_or_else(",
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
COMMENT_PREFIXES = ("//", "///", "//!", "#", "<!--", "*")
SAMPLE_PROJECT_BEHAVIOR_MARKERS = (
    *DOMAIN_TYPE_SPECIAL_CASE_MARKERS,
    "push_str(",
    "write!(",
    "writeln!(",
    "format!(",
    "let ",
    "const ",
    "static ",
    "return ",
    "=>",
    "insert(",
)


def is_enforced_path(path: Path, *, strict: bool = False) -> bool:
    normalized = path.as_posix()
    if any(normalized == policy_file or normalized.endswith(f"/{policy_file}") for policy_file in POLICY_FILES):
        return False
    skip_path_parts = BASE_SKIP_PATH_PARTS if strict else BASE_SKIP_PATH_PARTS | DEFAULT_SKIP_PATH_PARTS
    if any(part in skip_path_parts for part in path.parts):
        return False
    if ".ai-rulez" in path.parts:
        return True
    if strict:
        return is_strict_domain_surface_path(path) or is_production_generator_path(path)
    return path.suffix.lower() not in DOC_EXTENSIONS


def is_strict_domain_surface_path(path: Path) -> bool:
    normalized = path.as_posix()
    if normalized in {"AGENTS.md", ".github/copilot-instructions.md"}:
        return True
    if ".ai-rulez" in path.parts:
        return True
    if path.parts and path.parts[0] == "docs":
        return True
    if "snapshots" in path.parts:
        return True
    return any(
        template_root in normalized
        for template_root in (
            "src/codegen/templates/",
            "src/docs/templates/",
            "src/e2e/templates/",
            "src/readme/templates/",
            "src/scaffold/templates/",
        )
    )


def is_production_generator_path(path: Path) -> bool:
    parts = path.parts
    if "src" not in parts:
        return False
    src_index = parts.index("src")
    if len(parts) <= src_index + 1:
        return False
    if parts[src_index + 1] in {"backends", "codegen", "publish", "scaffold"}:
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


def is_split_domain_type_literal(line: str) -> bool:
    stripped = line.strip()
    if stripped.startswith(("//", "///", "#")):
        return False
    if "assert" in stripped:
        return False
    return any(pattern.search(line) for _, pattern in SPLIT_DOMAIN_TYPE_PATTERNS)


def is_sample_project_behavior_line(line: str) -> bool:
    stripped = line.strip()
    if stripped.startswith(("//", "///", "#")):
        return False
    if "assert" in stripped:
        return False
    return any(marker in stripped for marker in SAMPLE_PROJECT_BEHAVIOR_MARKERS)


def is_strict_prose_line(path: Path, line: str) -> bool:
    stripped = line.strip()
    if is_strict_domain_surface_path(path):
        return True
    return stripped.startswith(COMMENT_PREFIXES)


def update_rust_cfg_test_region(
    path: Path,
    line: str,
    pending: bool,
    depth: int | None,
) -> tuple[bool, int | None, bool, bool]:
    started = False
    if path.suffix == ".rs" and line.strip() == "#[cfg(test)]":
        pending = True
    elif pending and path.suffix == ".rs" and "{" in line:
        depth = line.count("{") - line.count("}")
        pending = False
        started = True
    return pending, depth, depth is not None, started


def advance_rust_cfg_test_region(line: str, depth: int | None, started: bool) -> int | None:
    if depth is None or started:
        return depth
    depth += line.count("{") - line.count("}")
    return None if depth <= 0 else depth


def project_violations_for_line(path: Path, line_number: int, line: str) -> list[str]:
    masked_line = normalize_for_project_mentions(mask_allowed_infrastructure(line))
    return [
        f"{path}:{line_number}: forbidden project mention `{name}`"
        for name, pattern in PATTERNS.items()
        if pattern.search(masked_line)
    ]


def sample_project_violations_for_line(path: Path, line_number: int, line: str) -> list[str]:
    masked_line = normalize_for_project_mentions(mask_allowed_infrastructure(line))
    return [
        f"{path}:{line_number}: forbidden downstream sample fixture mention `{name}`"
        for name, pattern in SAMPLE_PATTERNS.items()
        if pattern.search(masked_line)
    ]


def domain_type_violations_for_line(path: Path, line_number: int, line: str) -> list[str]:
    violations: list[str] = []
    for patterns in (DOMAIN_TYPE_PATTERNS, SPLIT_DOMAIN_TYPE_PATTERNS, EMBEDDED_DOMAIN_TYPE_PATTERNS):
        violations.extend(
            f"{path}:{line_number}: forbidden downstream domain type `{name}`"
            for name, pattern in patterns
            if pattern.search(line)
        )
    return violations


def violations_for_file(path: Path, *, strict: bool = False) -> list[str]:
    if not is_enforced_path(path, strict=strict):
        return []

    content = read_text(path)
    if content is None:
        return []

    violations: list[str] = []
    pending_rust_cfg_test = False
    rust_cfg_test_depth: int | None = None
    for line_number, line in enumerate(content.splitlines(), start=1):
        pending_rust_cfg_test, rust_cfg_test_depth, in_rust_cfg_test_region, started = update_rust_cfg_test_region(
            path,
            line,
            pending_rust_cfg_test,
            rust_cfg_test_depth,
        )
        if not strict or is_strict_domain_surface_path(path) or is_sample_project_behavior_line(line):
            violations.extend(project_violations_for_line(path, line_number, line))
        if strict and is_strict_domain_surface_path(path):
            violations.extend(sample_project_violations_for_line(path, line_number, line))
            if is_strict_prose_line(path, line):
                violations.extend(domain_type_violations_for_line(path, line_number, line))
        if (
            is_production_generator_path(path)
            and not in_rust_cfg_test_region
            and (is_domain_type_special_case(line) or is_split_domain_type_literal(line))
        ):
            violations.extend(domain_type_violations_for_line(path, line_number, line))
        if (
            not strict
            and is_production_generator_path(path)
            and not in_rust_cfg_test_region
            and is_sample_project_behavior_line(line)
        ):
            violations.extend(sample_project_violations_for_line(path, line_number, line))
        rust_cfg_test_depth = advance_rust_cfg_test_region(line, rust_cfg_test_depth, started)
    return violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Scan documentation, fixtures, snapshots, and tests for meta-enforcement.",
    )
    parser.add_argument("files", nargs="*", help="Files to scan")
    args = parser.parse_args(argv)

    violations: list[str] = []
    for raw in args.files:
        path = Path(raw)
        if path.is_file():
            violations.extend(violations_for_file(path, strict=args.strict))

    if violations:
        for violation in violations:
            print(violation, file=sys.stderr)
        print(f"\n{PROJECT_MENTION_MESSAGE}", file=sys.stderr)
        print(DOMAIN_TYPE_MESSAGE, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
