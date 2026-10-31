"""Independent checks over the actual selected raw bridge inputs and written outputs."""

import re


def normalized(text):
    lines, blanks = [], 0
    for original_line in text.splitlines():
        line = original_line.rstrip()
        blanks = blanks + 1 if not line else 0
        if blanks <= 2:
            lines.append(line)
    return "\n".join(lines).rstrip("\n") + "\n" if text else ""


def swift_content(text, core):
    head = [line.strip() for line in text.splitlines()[:5]]
    if "import RustBridgeC" not in head:
        text = "import RustBridgeC\n\n" + text
    if "// swift-format-ignore-file" not in head:
        text = "// swift-format-ignore-file\n" + text
    if core:
        for name in ["Identifiable", "Equatable"]:
            text = text.replace("extension RustStr: " + name, "extension RustStr: @retroactive " + name)
        marker = "// alef: RustStringRef.toString() shim"
        if marker in text:
            text = text.split(marker, 1)[0].rstrip("\n") + "\n"
    text = text.replace("    var ptr: UnsafeMutableRawPointer", "    public var ptr: UnsafeMutableRawPointer")
    text = text.replace("    var isOwned: Bool = true", "    public var isOwned: Bool = true")
    return normalized(text)


def c_tokens(text):
    text = re.sub(r"/\*.*?\*/|//[^\n]*", "", text, flags=re.DOTALL)
    lines, block = [], []
    # The shipped header is clang-formatted and SortIncludes reorders each run of adjacent
    # #include lines, so only the order within such a run is ignored; every other token keeps its
    # position.
    for line in [*text.splitlines(), ""]:
        if line.strip().startswith("#include"):
            block.append(line.strip())
            continue
        lines.extend(sorted(block))
        block = []
        lines.append(line)
    return re.findall(r"[A-Za-z_$][A-Za-z0-9_$]*|\d+|[^\s]", "\n".join(lines))


def verify(out, package):
    for name, core in [("SwiftBridgeCore.swift", True), ("xberg-swift.swift", False)]:
        raw = out / name if core else out / "xberg-swift" / name
        actual = package / "Sources/RustBridge" / name
        if actual.read_text() != swift_content(raw.read_text(), core):
            raise RuntimeError("materialized Swift bytes do not match actual OUTDIR transformation: " + name)
    core = (out / "SwiftBridgeCore.h").read_text()
    crate = (out / "xberg-swift/xberg-swift.h").read_text()
    header = (package / "Sources/RustBridgeC/RustBridgeC.h").read_text()
    assembled = "#ifndef RUST_BRIDGE_C_H\n#define RUST_BRIDGE_C_H\n" + core + "\n" + crate + "\n#endif"
    if c_tokens(header) != c_tokens(assembled) or "__swift_bridge__$" not in header:
        raise RuntimeError("materialized header declarations differ from actual raw C inputs")
