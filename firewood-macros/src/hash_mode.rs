// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Attribute, FnArg, GenericParam, ItemFn, Meta, Pat, TypeParamBound};

/// Retains the generic test body and emits a test wrapper for each hash mode.
pub(crate) fn expand(args: TokenStream, mut function: ItemFn) -> syn::Result<TokenStream> {
    // Validate the input before constructing concrete test signatures.
    if !args.is_empty() {
        return Err(syn::Error::new_spanned(
            args,
            "hash_mode takes no arguments",
        ));
    }
    let arguments = validate_signature(&function.sig)?;

    // Keep body lint settings separate from attributes that register and control tests.
    let mut generic_test_attrs = Vec::new();
    let mut wrapper_attrs = Vec::new();
    let tests = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("test"))
        .count();
    let cases = function
        .attrs
        .iter()
        .filter(|attr| is_test_case(attr))
        .count();

    for attr in &function.attrs {
        if attr.path().is_ident("test")
            || is_test_case(attr)
            || attr.path().is_ident("should_panic")
            || attr.path().is_ident("ignore")
        {
            wrapper_attrs.push(attr.clone());
        } else if attr.path().is_ident("cfg") {
            generic_test_attrs.push(attr.clone());
            wrapper_attrs.push(attr.clone());
        } else if is_lint(&attr.meta) || attr.path().is_ident("doc") {
            generic_test_attrs.push(attr.clone());
        } else {
            return Err(syn::Error::new_spanned(
                attr,
                "unsupported hash_mode attribute; use cfg, lint attributes, test, test_case, should_panic, or ignore",
            ));
        }
    }

    // Each wrapper needs one registration strategy; only test_case supplies arguments.
    if !((tests == 1 && cases == 0) || (tests == 0 && cases > 0)) {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "place hash_mode above exactly one test attribute or one or more test_case attributes",
        ));
    }
    if tests == 1 && !function.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.inputs,
            "hash_mode test arguments require test_case",
        ));
    }

    // Concrete wrappers delegate to the generic body without rewriting its tokens.
    function.attrs = generic_test_attrs;
    let name = &function.sig.ident;
    let wrappers =
        [("eth", quote!(EthHash)), ("merkledb", quote!(MerkleDbHash))].map(|(suffix, mode)| {
            let mut sig = function.sig.clone();
            sig.ident = format_ident!("{name}_{suffix}");
            sig.generics = syn::Generics::default();

            // Forwarding does not mutate arguments; mutable bindings belong to the generic test function.
            for input in &mut sig.inputs {
                if let FnArg::Typed(argument) = input
                    && let Pat::Ident(pattern) = argument.pat.as_mut()
                {
                    pattern.mutability = None;
                }
            }

            quote! {
                #[cfg(test)]
                #(#wrapper_attrs)*
                #sig { #name::<#mode>(#(#arguments),*) }
            }
        });

    Ok(quote! {
        #[cfg(test)]
        #function
        #(#wrappers)*
    })
}

/// Checks that concrete wrappers can call the function and returns its argument names.
fn validate_signature(sig: &syn::Signature) -> syn::Result<Vec<syn::Ident>> {
    let invalid = || {
        syn::Error::new_spanned(
            sig,
            "hash_mode requires a synchronous safe free function with exactly one type parameter bounded by HashMode, no where clause, and concrete argument/return types",
        )
    };

    // Wrappers make ordinary Rust calls and specialize exactly one type parameter.
    if sig.asyncness.is_some()
        || sig.constness.is_some()
        || sig.unsafety.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || sig.generics.where_clause.is_some()
        || sig.generics.params.len() != 1
    {
        return Err(invalid());
    }

    // Require a single HashMode bound without additional generic constraints.
    let Some(GenericParam::Type(parameter)) = sig.generics.params.first() else {
        return Err(invalid());
    };
    if parameter.default.is_some() || !parameter.attrs.is_empty() || parameter.bounds.len() != 1 {
        return Err(invalid());
    }

    let Some(TypeParamBound::Trait(bound)) = parameter.bounds.first() else {
        return Err(invalid());
    };
    if bound.lifetimes.is_some() || !matches!(bound.modifier, syn::TraitBoundModifier::None) {
        return Err(invalid());
    }

    let Some(segment) = bound.path.segments.last() else {
        return Err(invalid());
    };
    if segment.ident != "HashMode" || !segment.arguments.is_empty() {
        return Err(invalid());
    }

    // Collect forwarding names; argument types must remain valid without the mode parameter.
    let mut arguments = Vec::new();
    for input in &sig.inputs {
        let FnArg::Typed(argument) = input else {
            return Err(invalid());
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            return Err(invalid());
        };
        if pattern.by_ref.is_some()
            || pattern.subpat.is_some()
            || !argument.attrs.is_empty()
            || contains_ident(argument.ty.to_token_stream(), &parameter.ident)
        {
            return Err(invalid());
        }
        arguments.push(pattern.ident.clone());
    }

    // The return type is copied unchanged onto both concrete wrappers.
    if contains_ident(sig.output.to_token_stream(), &parameter.ident) {
        return Err(invalid());
    }

    Ok(arguments)
}

/// Searches a token stream, including nested groups, for an identifier.
fn contains_ident(tokens: TokenStream, ident: &syn::Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(candidate) => candidate == *ident,
        proc_macro2::TokenTree::Group(group) => contains_ident(group.stream(), ident),
        _ => false,
    })
}

/// Recognizes bare and crate-qualified `test_case` attributes.
fn is_test_case(attr: &Attribute) -> bool {
    attr.path().is_ident("test_case")
        || attr.path().to_token_stream().to_string() == "test_case :: test_case"
}

/// Recognizes attributes that set a lint level or expectation.
fn is_lint(meta: &Meta) -> bool {
    ["allow", "expect", "warn", "deny", "forbid"]
        .iter()
        .any(|name| meta.path().is_ident(name))
}
