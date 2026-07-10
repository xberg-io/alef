#!/usr/bin/env python3
"""Collapse the alef workspace (30 crates) into a single root-flat `alef` crate.

One-shot migration script for v0.18.0. Replayable on a clean checkout.

Operations:
1. Move each `crates/alef-<name>/src/` to `src/<module>/` (backends nested
   under `src/backends/<lang>/`).
2. Move templates colocated with their owning module (templates already
   live at `crates/alef-<name>/templates/`).
3. Move tests to root `tests/` with module-prefixed filenames.
4. Move benches to root `benches/`.
5. Rewrite `use alef_<crate>::` → `use crate::<module>::` in src/, and
   `use alef::<module>::` in tests/.
6. Merge per-crate `[dependencies]` into a single root `Cargo.toml`,
   dropping internal alef-* entries.
7. Delete `crates/`.
8. Rename `lib.rs` of each former crate to `mod.rs` after move.
9. Synthesize `src/lib.rs` re-exporting every top-level module.
10. Synthesize `src/backends/mod.rs` re-exporting every backend.
11. Keep `crates/alef-cli/src/main.rs` as `src/main.rs`; move other
    alef-cli sources under `src/cli/`.

Aborts if the working tree has uncommitted changes (other than its own
output). Idempotent on a fresh `git stash`.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CRATES_DIR = REPO / "crates"
SRC_DIR = REPO / "src"
TESTS_DIR = REPO / "tests"
BENCHES_DIR = REPO / "benches"

MODULE_MOVES: list[tuple[str, str]] = [
    ("alef-core", "core"),
    ("alef-codegen", "codegen"),
    ("alef-adapters", "adapters"),
    ("alef-extract", "extract"),
    ("alef-docs", "docs"),
    ("alef-e2e", "e2e"),
    ("alef-readme", "readme"),
    ("alef-scaffold", "scaffold"),
    ("alef-snippets", "snippets"),
    ("alef-publish", "publish"),
    ("alef-backend-csharp", "backends/csharp"),
    ("alef-backend-dart", "backends/dart"),
    ("alef-backend-extendr", "backends/extendr"),
    ("alef-backend-ffi", "backends/ffi"),
    ("alef-backend-gleam", "backends/gleam"),
    ("alef-backend-go", "backends/go"),
    ("alef-backend-java", "backends/java"),
    ("alef-backend-jni", "backends/jni"),
    ("alef-backend-kotlin", "backends/kotlin"),
    ("alef-backend-kotlin-android", "backends/kotlin_android"),
    ("alef-backend-magnus", "backends/magnus"),
    ("alef-backend-napi", "backends/napi"),
    ("alef-backend-php", "backends/php"),
    ("alef-backend-pyo3", "backends/pyo3"),
    ("alef-backend-rustler", "backends/rustler"),
    ("alef-backend-swift", "backends/swift"),
    ("alef-backend-wasm", "backends/wasm"),
    ("alef-backend-zig", "backends/zig"),
]

CLI_CRATE = "alef-cli"
CLI_MODULE = "cli"

CRATE_TO_USE_PATH: dict[str, str] = {}
for crate, dest in MODULE_MOVES:
    rust_name = crate.replace("-", "_")
    if dest.startswith("backends/"):
        backend = dest.removeprefix("backends/")
        CRATE_TO_USE_PATH[rust_name] = f"crate::backends::{backend}"
    else:
        CRATE_TO_USE_PATH[rust_name] = f"crate::{dest}"

LIB_PATH_REWRITES: dict[str, str] = {
    rust_name: path.replace("crate::", "alef::") for rust_name, path in CRATE_TO_USE_PATH.items()
}


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"$ {' '.join(cmd)}")
    result = subprocess.run(cmd, cwd=REPO, check=True, **kwargs)
    return result


def assert_clean_tree() -> None:
    """Block only if the working tree has changes outside this migration's domain.

    Allowed (our own in-flight output):
      - any change under crates/ (we're emptying it)
      - any change under src/, tests/, benches/, examples/ (our destination)
      - changes to Cargo.toml, alef.toml (we rewrite both)
      - the script itself (untracked or modified)
    """
    out = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    )
    allow_prefixes = (
        "crates/",
        "src/",
        "tests/",
        "benches/",
        "examples/",
        "Cargo.toml",
        "alef.toml",
        "scripts/collapse-workspace.py",
    )
    dirty = []
    for line in out.stdout.splitlines():
        if not line:
            continue
        path_part = line[3:]
        if " -> " in path_part:
            old, new = path_part.split(" -> ", 1)
            paths = [old, new]
        else:
            paths = [path_part]
        if any(p.startswith(allow_prefixes) for p in paths):
            continue
        dirty.append(line)
    if dirty:
        print(
            "ERROR: working tree has uncommitted changes outside migration scope:",
            file=sys.stderr,
        )
        print("\n".join(dirty), file=sys.stderr)
        sys.exit(1)


def move_tree(src: Path, dst: Path) -> None:
    """git mv src dst, preserving history."""
    dst.parent.mkdir(parents=True, exist_ok=True)
    run(["git", "mv", str(src.relative_to(REPO)), str(dst.relative_to(REPO))])


def move_file(src: Path, dst: Path) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    run(["git", "mv", str(src.relative_to(REPO)), str(dst.relative_to(REPO))])


def rewrite_uses_in_file(path: Path, *, in_tests: bool) -> bool:
    """Rewrite use alef_<crate>:: prefixes in a single source file."""
    table = LIB_PATH_REWRITES if in_tests else CRATE_TO_USE_PATH
    try:
        text = path.read_text()
    except UnicodeDecodeError:
        return False
    new_text = text
    for rust_name in sorted(table.keys(), key=len, reverse=True):
        target = table[rust_name]
        pattern = re.compile(rf"\b{rust_name}::")
        new_text = pattern.sub(target + "::", new_text)
        new_text = re.sub(
            rf"\bextern crate {rust_name};\s*\n",
            "",
            new_text,
        )
    if new_text != text:
        path.write_text(new_text)
        return True
    return False


def rewrite_uses_in_tree(root: Path, *, in_tests: bool) -> int:
    count = 0
    for rs in root.rglob("*.rs"):
        if rewrite_uses_in_file(rs, in_tests=in_tests):
            count += 1
    return count


def move_module(crate: str, dest: str) -> None:
    """Move crates/<crate>/{src,templates,tests,benches,README.md} into the new layout.

    Idempotent: skips a crate whose dest src/ already exists, but still finishes
    cleanup of leftover bits (tests/benches/Cargo.toml) if present.
    """
    crate_dir = CRATES_DIR / crate
    dest_src_root = SRC_DIR / dest
    flat = dest.replace("/", "_")

    if not crate_dir.exists() and not dest_src_root.exists():
        print(f"SKIP: {crate} (neither source nor dest exists)")
        return
    if not crate_dir.exists() and dest_src_root.exists():
        print(f"SKIP: {crate} (already migrated to {dest})")
        return

    dest_src_root.parent.mkdir(parents=True, exist_ok=True)

    src_root = crate_dir / "src"
    if src_root.exists():
        if dest_src_root.exists():
            raise RuntimeError(f"dest already exists: {dest_src_root}")
        move_tree(src_root, dest_src_root)
        lib_rs = dest_src_root / "lib.rs"
        mod_rs = dest_src_root / "mod.rs"
        if lib_rs.exists():
            if mod_rs.exists():
                raise RuntimeError(f"both lib.rs and mod.rs in {dest_src_root}")
            move_file(lib_rs, mod_rs)

    tmpl = crate_dir / "templates"
    if tmpl.exists():
        move_tree(tmpl, dest_src_root / "templates")

    tests = crate_dir / "tests"
    if tests.exists():
        TESTS_DIR.mkdir(exist_ok=True)
        for child in sorted(tests.iterdir()):
            if child.is_dir():
                if child.name == "snapshots":
                    snap_dst = TESTS_DIR / "snapshots"
                    snap_dst.mkdir(exist_ok=True)
                    for snap in sorted(child.iterdir()):
                        new_name = f"{flat}_{snap.name}"
                        move_file(snap, snap_dst / new_name)
                else:
                    move_tree(child, TESTS_DIR / f"{flat}_{child.name}")
            elif child.suffix == ".rs":
                move_file(child, TESTS_DIR / f"{flat}_{child.name}")
            else:
                move_file(child, TESTS_DIR / f"{flat}_{child.name}")

    benches = crate_dir / "benches"
    if benches.exists():
        BENCHES_DIR.mkdir(exist_ok=True)
        for child in sorted(benches.iterdir()):
            move_file(child, BENCHES_DIR / f"{flat}_{child.name}")

    examples = crate_dir / "examples"
    if examples.exists():
        EXAMPLES_DIR = REPO / "examples"
        EXAMPLES_DIR.mkdir(exist_ok=True)
        for child in sorted(examples.iterdir()):
            move_file(child, EXAMPLES_DIR / f"{flat}_{child.name}")

    readme = crate_dir / "README.md"
    if readme.exists():
        run(["git", "rm", str(readme.relative_to(REPO))])

    cargo = crate_dir / "Cargo.toml"
    if cargo.exists():
        run(["git", "rm", str(cargo.relative_to(REPO))])

    if crate_dir.exists():
        for stray in list(crate_dir.iterdir()):
            if stray.is_file():
                run(["git", "rm", str(stray.relative_to(REPO))])
            elif stray.is_dir():
                try:
                    stray.rmdir()
                except OSError:
                    print(f"WARN: leftover dir not removed: {stray}")

    if crate_dir.exists():
        try:
            crate_dir.rmdir()
        except OSError as e:
            print(f"WARN: could not remove {crate_dir}: {e}")


def move_cli_crate() -> None:
    """alef-cli is special — main.rs → src/main.rs, others → src/cli/, build.rs → root."""
    crate_dir = CRATES_DIR / CLI_CRATE
    if not crate_dir.exists():
        return

    src_root = crate_dir / "src"
    cli_dest = SRC_DIR / CLI_MODULE
    cli_dest.mkdir(parents=True, exist_ok=True)

    main_src = src_root / "main.rs"
    if main_src.exists():
        move_file(main_src, SRC_DIR / "main.rs")

    for child in sorted(src_root.iterdir()):
        dest = cli_dest / child.name
        move_tree(child, dest) if child.is_dir() else move_file(child, dest)

    build = crate_dir / "build.rs"
    if build.exists():
        move_file(build, REPO / "build.rs")

    tests = crate_dir / "tests"
    if tests.exists():
        TESTS_DIR.mkdir(exist_ok=True)
        for child in sorted(tests.iterdir()):
            if child.suffix == ".rs":
                move_file(child, TESTS_DIR / f"cli_{child.name}")
            else:
                move_file(child, TESTS_DIR / f"cli_{child.name}")

    for stray in [crate_dir / "Cargo.toml", crate_dir / "README.md"]:
        if stray.exists():
            run(["git", "rm", str(stray.relative_to(REPO))])
    for d in [src_root, crate_dir / "tests", crate_dir]:
        if d.exists():
            try:
                d.rmdir()
            except OSError as e:
                print(f"WARN: leftover {d}: {e}")


def rewrite_main_rs() -> None:
    """src/main.rs declares `mod cli;` etc.; it referenced sibling modules
    (cache, commands, dispatch, pipeline, registry, version_pin) directly.
    After move they live under cli/, so main.rs becomes thin.
    """
    main = SRC_DIR / "main.rs"
    if not main.exists():
        return
    print("INFO: src/main.rs left as-is — will need manual rewrite")


def generate_lib_rs() -> None:
    """Write src/lib.rs re-exporting every module."""
    lib = SRC_DIR / "lib.rs"
    modules = []
    for crate, dest in MODULE_MOVES:
        if "/" in dest:
            continue
        modules.append(dest)
    modules.append("backends")
    modules.append(CLI_MODULE)
    modules.sort()

    content = (
        "//! alef — polyglot binding generator.\n"
        "//!\n"
        "//! Top-level module re-exports for the consolidated `alef` crate.\n"
        "//! Each module corresponds to one of the former workspace member crates\n"
        "//! (alef-core, alef-codegen, ...). See README and CHANGELOG (v0.18.0)\n"
        "//! for the consolidation rationale.\n"
        "\n"
    )
    for m in modules:
        content += f"pub mod {m};\n"

    lib.write_text(content)
    run(["git", "add", "src/lib.rs"])


def generate_backends_mod_rs() -> None:
    """Write src/backends/mod.rs declaring each backend submodule."""
    bk_dir = SRC_DIR / "backends"
    bk_dir.mkdir(parents=True, exist_ok=True)
    mod_rs = bk_dir / "mod.rs"
    if mod_rs.exists():
        print(f"INFO: {mod_rs} already exists, overwriting")
    backends = []
    for crate, dest in MODULE_MOVES:
        if dest.startswith("backends/"):
            backends.append(dest.removeprefix("backends/"))
    backends.sort()
    content = "//! Language-specific binding-generator backends.\n\n"
    for b in backends:
        content += f"pub mod {b};\n"
    mod_rs.write_text(content)
    run(["git", "add", "src/backends/mod.rs"])


def write_root_cargo_toml() -> None:
    """Synthesize a single root Cargo.toml from the per-crate manifests."""
    new = """[package]
name = "alef"
version = "0.18.0"
edition = "2024"
rust-version = "1.85"
license = "MIT"
repository = "https://github.com/xberg-io/alef"
homepage = "https://github.com/xberg-io/alef"
description = "Opinionated polyglot binding generator for Rust libraries"
keywords = ["codegen", "bindings", "ffi", "polyglot", "pyo3"]
categories = ["development-tools::ffi", "development-tools::build-utils"]
readme = "README.md"

[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/v{ version }/alef-{ target }{ archive-suffix }"
bin-dir = "alef-{ target }/{ bin }{ binary-ext }"
pkg-fmt = "tgz"

[package.metadata.binstall.overrides.x86_64-pc-windows-gnu]
pkg-fmt = "zip"

[package.metadata.cargo-machete]
ignored = ["tracing"]

[[bin]]
name = "alef"
path = "src/main.rs"

[lib]
name = "alef"
path = "src/lib.rs"

[dependencies]
ahash = "0.8"
anyhow = "1"
blake3 = "1"
clap = { version = "4", features = ["derive"] }
glob = "0.3"
heck = "0.5"
jsonschema = { version = "0.46", default-features = false, features = ["resolve-file"] }
minijinja = "2"
quote = "1"
rayon = "1"
regex = "1"
semver = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sha2 = "0.11"
similar = "3"
syn = { version = "2", features = ["full", "parsing", "visit"] }
thiserror = "2"
toml = "1.1"
toml_edit = "0.25"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
ureq = { version = "3", features = ["json"] }
walkdir = "2"
which = "8"
zip = { version = "8", default-features = false, features = ["deflate"] }

[dev-dependencies]
criterion = { version = "0.8", features = ["html_reports"] }
insta = { version = "1.47", features = ["redactions"] }
tempfile = "3"
toml = "1.1"
tracing-test = "0.2"

[[bench]]
name = "backends_dart_emit"
harness = false

[[bench]]
name = "backends_gleam_emit"
harness = false

[[bench]]
name = "backends_kotlin_emit"
harness = false

[[bench]]
name = "backends_swift_emit"
harness = false

[[bench]]
name = "backends_zig_emit"
harness = false
"""
    (REPO / "Cargo.toml").write_text(new)
    run(["git", "add", "Cargo.toml"])


def rewrite_all_use_paths() -> None:
    """Rewrite alef_<crate>:: prefixes throughout src/, tests/, benches/."""
    src_changed = rewrite_uses_in_tree(SRC_DIR, in_tests=False)
    tests_changed = rewrite_uses_in_tree(TESTS_DIR, in_tests=True) if TESTS_DIR.exists() else 0
    benches_changed = rewrite_uses_in_tree(BENCHES_DIR, in_tests=True) if BENCHES_DIR.exists() else 0
    print(f"INFO: rewrote use-paths in {src_changed} src files, {tests_changed} tests, {benches_changed} benches")


def update_alef_toml() -> None:
    """Update alef.toml to reference the new package layout."""
    f = REPO / "alef.toml"
    if not f.exists():
        return
    text = f.read_text()
    text = text.replace('name = "alef-cli"', 'name = "alef"')
    text = text.replace(
        'sources = ["crates/alef-cli/src/main.rs"]',
        'sources = ["src/main.rs"]',
    )
    text = re.sub(
        r'alef_version = "[^"]+"',
        'alef_version = "0.18.0"',
        text,
    )
    f.write_text(text)
    run(["git", "add", "alef.toml"])


def main() -> None:
    if not CRATES_DIR.exists():
        print("ERROR: crates/ directory not found — script already run?", file=sys.stderr)
        sys.exit(1)

    assert_clean_tree()

    print("=== STEP 1: move all non-cli crates ===")
    for crate, dest in MODULE_MOVES:
        print(f"\n--- {crate} → src/{dest} ---")
        move_module(crate, dest)

    print("\n=== STEP 2: move alef-cli ===")
    move_cli_crate()

    print("\n=== STEP 3: synthesize lib.rs + backends/mod.rs ===")
    generate_lib_rs()
    generate_backends_mod_rs()

    print("\n=== STEP 4: write new root Cargo.toml ===")
    write_root_cargo_toml()

    print("\n=== STEP 5: rewrite use-paths ===")
    rewrite_all_use_paths()
    run(["git", "add", "-u"])

    print("\n=== STEP 6: update alef.toml ===")
    update_alef_toml()

    print("\n=== STEP 7: cleanup empty crates/ ===")
    if CRATES_DIR.exists():
        try:
            CRATES_DIR.rmdir()
        except OSError as e:
            print(f"WARN: crates/ not empty: {e}")
            for stray in CRATES_DIR.rglob("*"):
                print(f"  stray: {stray}")

    print("\n=== DONE ===")
    print("Next: cargo build, fix errors, cargo test, cargo clippy.")


if __name__ == "__main__":
    main()
