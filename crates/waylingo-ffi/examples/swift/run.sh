#!/usr/bin/env bash
# Build the library, generate the Swift bindings and run the smoke test on macOS.
# Needs TOMTOM_API_KEY and LLM_API_KEY in the environment (or in .env at the root).
set -euo pipefail
cd "$(dirname "$0")/../../../.."
[ -f .env ] && { set -a; . ./.env; set +a; }
cargo build -q -p waylingo-ffi --release
cargo run -q -p waylingo-ffi --features bindgen --bin uniffi-bindgen -- generate \
  --library target/release/libwaylingo_ffi.dylib --language swift --out-dir bindings/swift
mkdir -p target/swift-smoke
swiftc -parse-as-library -O \
  -I bindings/swift -Xcc -fmodule-map-file=bindings/swift/WaylingoFFI.modulemap \
  -L target/release -lwaylingo_ffi \
  bindings/swift/Waylingo.swift crates/waylingo-ffi/examples/swift/main.swift \
  -o target/swift-smoke/smoke
DYLD_LIBRARY_PATH=target/release target/swift-smoke/smoke
