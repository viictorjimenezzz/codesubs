// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "CodesubsApp",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "codesubs-app", targets: ["CodesubsApp"])
    ],
    targets: [
        .executableTarget(name: "CodesubsApp")
    ]
)
