// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "SubscriptionManagerApp",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "subscription-manager-app", targets: ["SubscriptionManagerApp"])
    ],
    targets: [
        .executableTarget(name: "SubscriptionManagerApp")
    ]
)
