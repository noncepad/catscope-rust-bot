use proc_macro::TokenStream;

/// No-op for WASI targets: passes the item through unchanged.
#[proc_macro_attribute]
pub fn wasm_expose(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
