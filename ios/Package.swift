// swift-tools-version:6.1.0
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "inappbrowser",
    platforms: [
        .iOS(.v15),
    ],
    products: [
        .library(
            name: "inappbrowser",
            type: .static,
            targets: ["inappbrowser"]),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api")
    ],
    targets: [
        .target(
            name: "inappbrowser",
            dependencies: [
                .byName(name: "Tauri")
            ],
            path: "Sources",
            linkerSettings: [
                .linkedFramework("SafariServices"),
                .linkedFramework("UIKit"),
                .linkedFramework("Foundation")
            ]
        )
    ]
)
