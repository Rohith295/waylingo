//! Generates the Swift and Kotlin sources from the built library:
//!
//! ```text
//! cargo build -p waylingo-ffi --release
//! cargo run -p waylingo-ffi --bin uniffi-bindgen -- generate \
//!     --library target/release/libwaylingo_ffi.dylib --language swift --out-dir bindings/swift
//! ```

fn main() {
    uniffi::uniffi_bindgen_main();
}
