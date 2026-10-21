# ruff: noqa: S101 ~keep Assertions belong to this generated-code regression fixture.
import ast
import sys
from pathlib import Path
from types import ModuleType


def verify_conformance(native: ModuleType, raised: BaseException, package_root: str) -> None:
    """Check real native descriptors and raised exception fields against the emitted test."""
    assert vars(raised)["vendor"] == "sample-vendor"
    assert not hasattr(type(raised), "vendor")
    root = Path(package_root)
    stub = root / "sample" / "_sample.pyi"
    assert stub.is_file()
    sys.path.insert(0, package_root)
    sys.modules["sample._sample"] = native
    source = root / "sample" / "conformance.py"
    tree = ast.parse(source.read_text(encoding="utf-8"))
    # The shipped stub is present, so the pytest-only missing-installation skip is unreachable. ~keep
    tree.body = [
        node
        for node in tree.body
        if not (isinstance(node, ast.Import) and any(alias.name == "pytest" for alias in node.names))
    ]
    scope: dict[str, object] = {}
    exec(compile(tree, str(source), "exec"), scope)  # noqa: S102 ~keep Execute the emitted conformance functions.
    check = scope["test_stub_declares_only_members_present_at_runtime"]
    assert callable(check)
    check()
    verify_mutations(stub, check)


def verify_mutations(stub: Path, check: object) -> None:
    """Prove methods, native DTO descriptors, and class variables are still examined."""
    assert callable(check)
    original = stub.read_text(encoding="utf-8")
    mutations = [
        ("ContentConfig", "    def missing_native_method(self) -> None: ...", "missing_native_method"),
        ("ContentConfig", "    missing_native_field: str", "missing_native_field"),
        ("RejectedError", "    missing_class_constant: typing.ClassVar[int]", "missing_class_constant"),
    ]
    observed = 0
    try:
        for class_name, declaration, missing in mutations:
            tree = ast.parse(original)
            target = next(node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == class_name)
            lines = original.splitlines(keepends=True)
            lines.insert(target.lineno, f"{declaration}\n")
            stub.write_text("".join(lines), encoding="utf-8")
            try:
                check()
            except AssertionError as error:
                if missing not in str(error):
                    raise AssertionError(f"unexpected conformance failure: {error}") from error
                observed += 1
            else:
                raise AssertionError(f"conformance accepted {class_name}.{missing}")
    finally:
        stub.write_text(original, encoding="utf-8")
    assert observed == len(mutations) == 3
