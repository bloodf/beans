//! `cargo run -p beans-mobile --features bindgen --bin uniffi-bindgen -- generate --library
//! target/debug/libbeans_mobile.dylib --language swift --out-dir …`
fn main() {
    uniffi::uniffi_bindgen_main()
}
