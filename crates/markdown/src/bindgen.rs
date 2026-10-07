//! `cargo run -p beans-markdown --features bindgen --bin uniffi-bindgen -- generate --library
//! target/debug/libbeans_markdown.dylib --language swift --out-dir …`
fn main() {
    uniffi::uniffi_bindgen_main()
}
