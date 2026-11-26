// swift-tools-version:5.9
// Swift Package for Waylingo (iOS and macOS).
//
// The native library is a prebuilt XCFramework attached to each GitHub Release.
// The release workflow (.github/workflows/release.yml) fills in its URL and checksum
// and commits the generated Swift source, so depend on a release tag, not a branch:
//
//     .package(url: "https://github.com/Rohith295/waylingo", from: "0.1.0")
//
// Between releases the URL and checksum below are placeholders and the package
// does not build from main.
import PackageDescription

let package = Package(
    name: "Waylingo",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "Waylingo", targets: ["Waylingo"]),
    ],
    targets: [
        .binaryTarget(name: "WaylingoFFI", url: "https://example.invalid/WaylingoFFI.xcframework.zip", checksum: "0000000000000000000000000000000000000000000000000000000000000000"),
        .target(
            name: "Waylingo",
            dependencies: ["WaylingoFFI"],
            path: "swift/Sources/Waylingo"
        ),
    ]
)
