// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Beans",
    platforms: [.macOS(.v14)],
    dependencies: [
        // The updater. A binary xcframework: scripts/app.ts copies the framework into
        // Contents/Frameworks, where the rpath below finds it.
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.9.0"),
    ],
    targets: [
        // The Markdown parser from crates/markdown: the Rust static library and its C header,
        // built into Libraries/ by scripts/app.ts.
        .binaryTarget(
            name: "BeansMarkdownFFI",
            path: "Libraries/BeansMarkdownFFI.xcframework"
        ),
        // The UniFFI Swift bindings over it, generated into Sources/BeansMarkdown by the same step.
        .target(
            name: "BeansMarkdown",
            dependencies: ["BeansMarkdownFFI"],
            path: "Sources/BeansMarkdown",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "Beans",
            dependencies: ["BeansMarkdown", .product(name: "Sparkle", package: "Sparkle")],
            path: "Sources/Beans",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"])
            ]
        ),
        .testTarget(
            name: "BeansTests",
            dependencies: ["Beans"],
            path: "Tests/Notifications",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
