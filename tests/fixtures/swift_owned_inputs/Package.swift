// swift-tools-version: 6.0
import Foundation
import PackageDescription
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let native = root + "/native-target/debug"
let package = Package(
  name: "OwnedValuesProbe",
  platforms: [.macOS("26.0")],
  targets: [
    .systemLibrary(name: "RustBridgeC", path: "packages/swift/Sources/RustBridgeC"),
    .target(name: "RustBridge", dependencies: ["RustBridgeC"], path: "packages/swift/Sources/RustBridge"),
    .target(name: "SampleCore", dependencies: ["RustBridge"], path: "packages/swift/Sources/SampleCore",
      linkerSettings: [.unsafeFlags([native + "/libsample_core_swift.a", "-liconv"])]),
    .executableTarget(name: "Probe", dependencies: ["SampleCore", "RustBridge"], path: "probe"),
    .testTarget(name: "OwnedTests", dependencies: ["SampleCore", "RustBridge"], path: "tests")
  ],
  swiftLanguageModes: [.v5]
)
