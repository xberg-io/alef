"""Shared SwiftPM test prefix for the positive package and both strict link controls."""


def command(executable):
    # XCTest imports add a private SwiftUICore hint rejected by the SDK's allowed-client list. ~keep
    # Disable only that framework hint at compilation; keep all linker diagnostics fatal. ~keep
    # https://github.com/swiftlang/swift/blob/swift-6.3.3-RELEASE/test/ClangImporter/autolinking.swift ~keep
    return [
        executable,
        "test",
        "-Xswiftc",
        "-Xfrontend",
        "-Xswiftc",
        "-disable-autolink-framework",
        "-Xswiftc",
        "-Xfrontend",
        "-Xswiftc",
        "SwiftUICore",
    ]
