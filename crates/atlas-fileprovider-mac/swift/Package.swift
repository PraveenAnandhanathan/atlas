// swift-tools-version: 5.9
// Build the ATLAS FileProvider app extension.
//
// Prerequisites:
//   - macOS 13+ SDK (Xcode 15+)
//   - libatlas_fileprovider_mac.dylib in the framework search path
//
// Build:
//   swift build -c release
//
// The .appex bundle must be embedded in ATLAS.app under
//   ATLAS.app/Contents/PlugIns/AtlasFileProvider.appex

import PackageDescription

let package = Package(
    name: "AtlasFileProvider",
    platforms: [.macOS(.v13)],
    products: [
        .library(
            name: "AtlasFileProvider",
            type: .dynamic,
            targets: ["AtlasFileProvider"]
        ),
    ],
    targets: [
        .target(
            name: "AtlasFileProvider",
            path: "AtlasFileProvider",
            linkerSettings: [
                // Link against the Rust dylib built by `cargo build -p atlas-fileprovider-mac`
                .linkedLibrary("atlas_fileprovider_mac"),
                .linkedFramework("FileProvider"),
                .linkedFramework("Foundation"),
            ]
        ),
    ]
)
