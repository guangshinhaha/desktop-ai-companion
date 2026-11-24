// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "executor-macos",
    platforms: [.macOS(.v13)],
    targets: [
        .target(name: "ExecutorCore"),
        .executableTarget(name: "executor-macos", dependencies: ["ExecutorCore"]),
        .testTarget(name: "ExecutorCoreTests", dependencies: ["ExecutorCore"]),
    ]
)
