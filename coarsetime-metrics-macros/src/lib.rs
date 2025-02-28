use quote::quote;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn measure(_attr: TokenStream, contents: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(contents as ItemFn).expect("Failed to parse input");
    let ms_string = input.sig.ident.to_string() + "_ms";
    let count_string = input.sig.ident.to_string() + "_count";

    let name = quote!(_duration);
    input.block.stmts.insert(
        0,
        syn::parse_quote! { let #name = coarsetime::Instant::now(); },
    );

    input
        .block
        .stmts
        .push(syn::parse_quote! { let #name = #name.elapsed().as_millis(); });
    input
        .block
        .stmts
        .push(syn::parse_quote! { counter!(#count_string).increment(1); });
    input
        .block
        .stmts
        .push(syn::parse_quote! { counter!(#ms_string).increment(#name); });
    TokenStream::from(quote::quote!(#input))
}
