// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Proc macros for Firewood.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{ItemFn, ReturnType, parse_macro_input};

/// Arguments for the `#[metrics]` attribute: a single identifier naming a counter constant in
/// `crate::registry`. The corresponding histogram constant must also exist in `crate::registry`
/// under the name `{IDENT}_DURATION_SECONDS`.
struct MetricsArgs {
    ident: syn::Ident,
}

impl Parse for MetricsArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: syn::Ident = input.parse().map_err(|e| {
            syn::Error::new(
                e.span(),
                "expected a registry constant identifier, e.g., #[metrics(MY_OPERATION)]",
            )
        })?;
        if !input.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "unexpected argument; only one identifier is accepted \
                 — the description is declared in the registry doc comment",
            ));
        }
        Ok(MetricsArgs { ident })
    }
}

/// A proc macro attribute that automatically adds metrics instrumentation to functions.
///
/// Wraps a `Result`-returning function with:
/// - A counter `crate::registry::{IDENT}` labeled with `success = "true"` or `"false"`
/// - A histogram `crate::registry::{IDENT}_DURATION_SECONDS` recording the elapsed duration
///
/// Both constants must be declared in `crate::registry` via `firewood_metrics::define_metrics!`;
/// the compiler validates they exist.
///
/// # Usage
/// ```rust,ignore
/// use firewood_macros::metrics;
///
/// // Registry must declare PROPOSAL_COMMITS and PROPOSAL_COMMITS_DURATION_SECONDS
/// #[metrics(PROPOSAL_COMMITS)]
/// fn commit(...) -> Result<(), Error> {
///     // function body
/// }
/// ```
///
/// # Requirements
/// - The function must return a `Result<T, E>` type
/// - `crate::registry::{IDENT}` (counter) and `crate::registry::{IDENT}_DURATION_SECONDS`
///   (histogram) must both be declared in the calling crate's `registry` module
/// - The `metrics` crate must be available in the calling crate
#[proc_macro_attribute]
pub fn metrics(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);

    if args.is_empty() {
        return syn::Error::new_spanned(
            &input_fn,
            "expected a registry constant identifier, e.g., #[metrics(MY_OPERATION)]",
        )
        .to_compile_error()
        .into();
    }

    let parsed_args = match syn::parse::<MetricsArgs>(args) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };

    // Validate that the function returns a Result
    let return_type = match &input_fn.sig.output {
        ReturnType::Type(_, ty) => ty,
        ReturnType::Default => {
            return syn::Error::new_spanned(
                &input_fn.sig,
                "Function must return a Result<T, E> to use #[metrics] attribute",
            )
            .to_compile_error()
            .into();
        }
    };

    let is_result = match return_type.as_ref() {
        syn::Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "Result"),
        _ => false,
    };

    if !is_result {
        return syn::Error::new_spanned(
            return_type,
            "Function must return a Result<T, E> to use #[metrics] attribute",
        )
        .to_compile_error()
        .into();
    }

    let expanded = generate_metrics_wrapper(&input_fn, &parsed_args.ident);
    TokenStream::from(expanded)
}

fn generate_metrics_wrapper(input_fn: &ItemFn, ident: &syn::Ident) -> proc_macro2::TokenStream {
    let fn_vis = &input_fn.vis;
    let fn_sig = &input_fn.sig;
    let fn_block = &input_fn.block;
    let fn_attrs = &input_fn.attrs;

    // Histogram constant: {IDENT}_DURATION_SECONDS — must be declared in crate::registry
    let duration_ident = format_ident!("{}_DURATION_SECONDS", ident);

    quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig {
            let __metrics_start = ::std::time::Instant::now();

            let __metrics_result = { #fn_block };

            // Use static label arrays to avoid runtime allocation
            static __METRICS_LABELS_SUCCESS: &[(&str, &str)] = &[("success", "true")];
            static __METRICS_LABELS_ERROR: &[(&str, &str)] = &[("success", "false")];
            let __metrics_labels = if __metrics_result.is_err() {
                __METRICS_LABELS_ERROR
            } else {
                __METRICS_LABELS_SUCCESS
            };

            ::firewood_metrics::firewood_counter!(#ident, __metrics_labels).increment(1);
            ::firewood_metrics::firewood_histogram!(#duration_ident).record(__metrics_start.elapsed().as_secs_f64());

            __metrics_result
        }
    }
}

/// Hash modes a test runs under, parsed from `#[hash_mode(...)]` arguments.
struct HashModeArgs {
    eth: bool,
    merkledb: bool,
}

impl Parse for HashModeArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let idents = Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated(input)?;
        if idents.is_empty() {
            // Bare `#[hash_mode]`: the default is to run under BOTH modes; a
            // single-mode annotation is the deliberate, documented exception.
            return Ok(HashModeArgs {
                eth: true,
                merkledb: true,
            });
        }
        let mut args = HashModeArgs {
            eth: false,
            merkledb: false,
        };
        for ident in &idents {
            let slot = match ident.to_string().as_str() {
                "eth" => &mut args.eth,
                "merkledb" => &mut args.merkledb,
                other => {
                    return Err(syn::Error::new_spanned(
                        ident,
                        format!("unknown hash mode `{other}`; expected `eth` or `merkledb`"),
                    ));
                }
            };
            if *slot {
                return Err(syn::Error::new_spanned(
                    ident,
                    format!("duplicate hash mode `{ident}`"),
                ));
            }
            *slot = true;
        }
        Ok(args)
    }
}

/// Runs a single test body under one or more hash modes, monomorphizing it over
/// the `HashMode` type rather than gating it on a compile-time feature.
///
/// The hash scheme is a runtime, per-database choice (issue #1088), so both
/// `EthHash` and `MerkleDbHash` are compiled into one binary. This macro makes a
/// generic test helper and emits one real `#[test]` wrapper per requested mode,
/// each instantiating the helper with the concrete hash-mode ZST:
///
/// - `#[hash_mode]`                → both wrappers
/// - `#[hash_mode(eth)]`           → `#[test] fn {name}_eth() { {name}::<EthHash>() }`
/// - `#[hash_mode(merkledb)]`      → `#[test] fn {name}_merkledb() { {name}::<MerkleDbHash>() }`
/// - `#[hash_mode(eth, merkledb)]` → both wrappers
///
/// # Contract
///
/// The annotated item MUST be a generic function whose **first type parameter is
/// the hash mode**, e.g. `fn name<H: HashMode>()`. The function MUST NOT carry
/// its own `#[test]` attribute — this macro emits the `#[test]` wrappers itself
/// (a generic function cannot be a `#[test]` directly). The original function is
/// re-emitted once, un-`#[test]`ed, as a generic helper (visibility unchanged); any other
/// attributes (docs, `#[expect(...)]`, …) stay on the helper, while
/// `#[should_panic]` / `#[ignore]` are forwarded onto each wrapper so they
/// behave as expected.
///
/// Value parameters are allowed on the helper, e.g. `fn name<H: HashMode>(a:
/// T, b: U)`. Each wrapper repeats the exact same parameter list and forwards
/// the arguments by name to the helper. Only **plain identifier patterns**
/// are supported (`a: T`, not `(a, b): (T, U)` or `Foo { a, b }: Foo`) —
/// anything else is a compile error, since the macro needs a name to forward
/// under each mode's wrapper.
///
/// # Composing with `#[test_case]`
///
/// This macro composes with the [`test-case`](https://docs.rs/test-case)
/// crate: `#[test_case(...)]` attributes on the helper are moved onto each
/// per-mode wrapper instead of staying on the helper, and `#[test]` is
/// omitted from wrappers that carry `#[test_case]` attrs (`test-case`
/// generates its own test-harness attribute, so adding `#[test]` as well
/// would conflict). For this to work, `#[hash_mode]` MUST be the **outermost**
/// attribute — i.e. listed first, above any `#[test_case(...)]` attrs — so it
/// runs before `test-case` expands and can see its attributes:
///
/// ```rust,ignore
/// #[hash_mode]
/// #[test_case(1, 2; "small")]
/// #[test_case(10, 20; "large")]
/// fn adds<H: HashMode>(a: u32, b: u32) {
///     assert!(a < b);
/// }
/// ```
///
/// The concrete ZST names (`EthHash` / `MerkleDbHash`) are emitted **unqualified**,
/// so each call site must bring them into scope itself, e.g.
/// `use firewood_storage::{EthHash, MerkleDbHash};` (or `use crate::{…}` inside
/// the storage crate). This keeps the macro agnostic to the crate it expands in.
///
/// # Usage
///
/// ```rust,ignore
/// use firewood_macros::hash_mode;
/// use firewood_storage::{EthHash, HashMode};
///
/// #[hash_mode(eth)]
/// fn only_under_ethhash<H: HashMode>() { /* body uses `H` */ }
/// ```
#[proc_macro_attribute]
pub fn hash_mode(args: TokenStream, input: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<HashModeArgs>(args) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };
    let input_fn = parse_macro_input!(input as ItemFn);
    hash_mode_expand(&parsed, input_fn).into()
}

/// Expands an annotated generic test function into a private helper plus one
/// `#[test]` wrapper per requested mode. Factored out so it can be unit-tested
/// without a `proc_macro` context.
fn hash_mode_expand(args: &HashModeArgs, mut input_fn: ItemFn) -> proc_macro2::TokenStream {
    // The helper must be generic over the hash mode (see the `# Contract`
    // section above). Check this up front and emit a targeted, unconditional
    // error: wrapper bodies are emitted behind `#[test]`, whose contents
    // rustc elides entirely outside `--test` builds, so an error that only
    // surfaced from inside a wrapper (e.g. a bad turbofish) would never be
    // seen by a plain `cargo build`/trybuild `pass`/`compile_fail` check.
    if input_fn.sig.generics.type_params().next().is_none() {
        return syn::Error::new_spanned(
            &input_fn.sig,
            "#[hash_mode] requires a generic function with a hash-mode type parameter, \
             e.g. `fn name<H: HashMode>()`",
        )
        .to_compile_error();
    }

    // Partition attributes: test-behavior attrs and test_case cases ride the
    // wrappers; everything else (docs, #[expect]) stays on the helper. The
    // macro emits the `#[test]` wrappers itself, so any `#[test]` the author
    // left on the helper is dropped outright (a generic fn cannot be a
    // `#[test]` directly).
    let mut forwarded_attrs = Vec::new();
    let mut test_case_attrs = Vec::new();
    input_fn.attrs.retain(|attr| {
        if attr.path().is_ident("test") {
            return false;
        }
        if attr.path().is_ident("should_panic") || attr.path().is_ident("ignore") {
            forwarded_attrs.push(attr.clone());
            return false;
        }
        if attr.path().is_ident("test_case") {
            test_case_attrs.push(attr.clone());
            return false;
        }
        true
    });

    // Value parameters: wrappers repeat them and forward by name. Only plain
    // identifier patterns are supported — anything else is a macro error.
    let mut param_idents = Vec::new();
    for arg in &input_fn.sig.inputs {
        match arg {
            syn::FnArg::Typed(pat_ty) => match &*pat_ty.pat {
                syn::Pat::Ident(pi) => param_idents.push(pi.ident.clone()),
                other => {
                    return syn::Error::new_spanned(
                        other,
                        "#[hash_mode] helpers may only use plain identifier parameters",
                    )
                    .to_compile_error();
                }
            },
            syn::FnArg::Receiver(r) => {
                return syn::Error::new_spanned(r, "#[hash_mode] does not support methods")
                    .to_compile_error();
            }
        }
    }

    let helper_ident = input_fn.sig.ident.clone();
    let params = input_fn.sig.inputs.clone();
    let output = input_fn.sig.output.clone();
    // `#[test]` only when test-case is not supplying its own harness attr.
    let test_attr = test_case_attrs.is_empty().then(|| quote! { #[test] });

    let mut modes: Vec<&'static str> = Vec::new();
    if args.eth {
        modes.push("eth");
    }
    if args.merkledb {
        modes.push("merkledb");
    }

    let wrappers = modes.into_iter().map(|mode| {
        let (suffix, zst) = match mode {
            "eth" => ("eth", format_ident!("EthHash")),
            // parsing guarantees only `eth` / `merkledb` reach here.
            _ => ("merkledb", format_ident!("MerkleDbHash")),
        };
        let wrapper_ident = format_ident!("{}_{}", helper_ident, suffix);
        quote! {
            #(#forwarded_attrs)*
            #(#test_case_attrs)*
            #test_attr
            fn #wrapper_ident(#params) #output {
                #helper_ident::<#zst>(#(#param_idents),*)
            }
        }
    });

    quote! {
        #input_fn
        #(#wrappers)*
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slow_proc_macro_compilation() {
        // Test that the proc macro generates compilable code
        let t = trybuild::TestCases::new();
        t.pass("tests/compile_pass/*.rs");
        t.compile_fail("tests/compile_fail/*.rs");
    }

    #[test]
    fn test_metrics_args_parsing() {
        // Test identifier parsing
        let input = quote::quote! { TEST_METRIC };
        let parsed: MetricsArgs = syn::parse2(input).unwrap();
        assert_eq!(parsed.ident.to_string(), "TEST_METRIC");
    }

    #[test]
    fn test_invalid_args_parsing() {
        // A literal number is not a valid identifier
        let input = quote::quote! { 123 };
        let result: syn::Result<MetricsArgs> = syn::parse2(input);
        assert!(result.is_err());

        // Extra arguments after the ident are not allowed
        let input = quote::quote! { MY_IDENT, extra };
        let result: syn::Result<MetricsArgs> = syn::parse2(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_generated_code_structure() {
        // Test that the proc macro generates the expected code structure
        use syn::{ItemFn, parse_quote};

        let input: ItemFn = parse_quote! {
            fn test_function() -> Result<(), &'static str> {
                Ok(())
            }
        };

        let ident: syn::Ident = syn::parse_str("TEST_METRIC").unwrap();
        let result = generate_metrics_wrapper(&input, &ident);
        let generated_code = result.to_string();

        // Verify key components are present in the generated code
        assert!(generated_code.contains("__METRICS_LABELS_SUCCESS"));
        assert!(generated_code.contains("__METRICS_LABELS_ERROR"));
        assert!(generated_code.contains("TEST_METRIC"));
        assert!(generated_code.contains("TEST_METRIC_DURATION_SECONDS"));
        assert!(
            generated_code.contains("std")
                && generated_code.contains("Instant")
                && generated_code.contains("now")
        );
        assert!(generated_code.contains("counter"));
        assert!(generated_code.contains("histogram"));
    }

    #[test]
    fn test_hash_mode_args_parsing() {
        let eth: HashModeArgs = syn::parse2(quote::quote! { eth }).unwrap();
        assert!(eth.eth && !eth.merkledb);

        let merkledb: HashModeArgs = syn::parse2(quote::quote! { merkledb }).unwrap();
        assert!(!merkledb.eth && merkledb.merkledb);

        let both: HashModeArgs = syn::parse2(quote::quote! { eth, merkledb }).unwrap();
        assert!(both.eth && both.merkledb);
    }

    #[test]
    fn test_hash_mode_bare_args_mean_both_modes() {
        let parsed: HashModeArgs = syn::parse2(quote::quote! {}).unwrap();
        assert!(parsed.eth && parsed.merkledb);
    }

    #[test]
    fn test_hash_mode_invalid_args() {
        // unknown mode
        assert!(syn::parse2::<HashModeArgs>(quote::quote! { sha256 }).is_err());
        // duplicate mode
        assert!(syn::parse2::<HashModeArgs>(quote::quote! { eth, eth }).is_err());
    }

    #[test]
    fn test_hash_mode_expand_emits_per_mode_wrappers() {
        use syn::parse_quote;

        let eth: ItemFn = parse_quote! {
            #[test]
            fn my_test<H: HashMode>() {}
        };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: false,
            },
            eth,
        )
        .to_string();
        // The `#[test]` is stripped from the helper and re-emitted on the wrapper.
        assert!(expanded.contains("fn my_test < H : HashMode >"));
        assert!(expanded.contains("fn my_test_eth ()"));
        assert!(expanded.contains("my_test :: < EthHash > ()"));
        assert!(!expanded.contains("my_test_merkledb"));

        let merkledb: ItemFn = parse_quote! {
            fn my_test<H: HashMode>() {}
        };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: false,
                merkledb: true,
            },
            merkledb,
        )
        .to_string();
        assert!(expanded.contains("fn my_test_merkledb ()"));
        assert!(expanded.contains("my_test :: < MerkleDbHash > ()"));
        assert!(!expanded.contains("my_test_eth"));

        let both: ItemFn = parse_quote! {
            fn my_test<H: HashMode>() {}
        };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: true,
            },
            both,
        )
        .to_string();
        assert!(expanded.contains("fn my_test_eth ()"));
        assert!(expanded.contains("fn my_test_merkledb ()"));
    }

    #[test]
    fn test_hash_mode_forwards_should_panic_to_wrapper() {
        use syn::parse_quote;

        let input: ItemFn = parse_quote! {
            #[should_panic]
            #[test]
            fn boom<H: HashMode>() {}
        };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: false,
            },
            input,
        )
        .to_string();
        // `#[should_panic]` rides on the emitted `#[test]` wrapper, not the helper.
        let wrapper_pos = expanded
            .find("fn boom_eth ()")
            .expect("wrapper should be emitted");
        let should_panic_pos = expanded
            .find("should_panic")
            .expect("should_panic should be forwarded");
        assert!(should_panic_pos < wrapper_pos);
    }

    #[test]
    fn test_hash_mode_forwards_return_type_to_wrapper() {
        use syn::parse_quote;

        let input: ItemFn = parse_quote! {
            fn t<H: HashMode>() -> Result<(), String> { Ok(()) }
        };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: false,
            },
            input,
        )
        .to_string();
        assert!(expanded.contains("fn t_eth () -> Result < () , String >"));
    }

    #[test]
    fn test_hash_mode_moves_test_case_attrs_to_wrappers() {
        use syn::parse_quote;

        let input: ItemFn = parse_quote! {
            #[test_case(1, 2; "small")]
            #[test_case(10, 20; "large")]
            fn t<H: HashMode>(a: u32, b: u32) { let _ = (a, b); }
        };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: true,
            },
            input,
        )
        .to_string();
        // Attrs are gone from the helper (helper renders first) and present twice per wrapper.
        assert_eq!(expanded.matches("test_case").count(), 4);
        // Wrappers keep the value params and forward them.
        assert!(expanded.contains("fn t_eth (a : u32 , b : u32)"));
        assert!(expanded.contains("t :: < EthHash > (a , b)"));
        // No #[test] on test_case wrappers (test-case generates its own).
        assert!(!expanded.contains("# [test] fn t_eth"));
    }

    #[test]
    fn test_hash_mode_plain_value_params_still_get_test_attr() {
        use syn::parse_quote;

        // No test_case attrs + no params -> wrapper keeps plain #[test].
        let input: ItemFn = parse_quote! { fn t<H: HashMode>() {} };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: false,
            },
            input,
        )
        .to_string();
        assert!(expanded.contains("# [test]"));
    }

    #[test]
    fn test_hash_mode_rejects_receiver_param() {
        use syn::parse_quote;

        // syn parses a receiver in a free-fn signature even though rustc would
        // reject it; the macro must turn it into a targeted compile error.
        let input: ItemFn = parse_quote! { fn t<H: HashMode>(&self, a: u32) {} };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: true,
            },
            input,
        )
        .to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("does not support methods"));
    }

    #[test]
    fn test_hash_mode_rejects_lifetime_only_generics() {
        use syn::parse_quote;

        // Lifetime parameters alone don't satisfy the hash-mode contract: the
        // helper needs a *type* parameter to instantiate with the mode ZST.
        let input: ItemFn = parse_quote! { fn t<'a>(x: &'a u32) { let _ = x; } };
        let expanded = hash_mode_expand(
            &HashModeArgs {
                eth: true,
                merkledb: true,
            },
            input,
        )
        .to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("requires a generic function with a hash-mode type parameter"));
    }
}
