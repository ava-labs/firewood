# Agent brief: code-quality

Review the changed code for correctness and conformance to the repository's rules.

1. Read `STYLE_GUIDE.md` and `AGENTS.md` in full. Review rules also live outside `STYLE_GUIDE.md`, such as `#[repr(C, usize)]` on FFI tagged enums (`AGENTS.md` § Common Development Tasks).
2. Read every file in `$SCRATCH/changed-files.txt` in full from the checkout, not only the diff hunks. Read callers and callees as needed to judge behavior.
3. Look for, in order of weight:
   - correctness: logic and boundary errors, panics reachable on valid input, swallowed or mis-propagated errors, concurrency hazards;
   - soundness of any `unsafe` and of FFI boundaries (`STYLE_GUIDE.md` § FFI and Unsafe Code);
   - performance on hot paths: allocation, copying, complexity;
   - unbounded recursion over trie depth (`AGENTS.md`, "Depth Follows Key Length");
   - tests: new behavior and error paths covered, assertions on behavior rather than implementation; a new Cargo feature is enabled by some profile in `scripts/run-rust-ci.sh`, since CI tests only those profiles;
   - every rule in `STYLE_GUIDE.md`, including lint suppression policy and comment quality.
4. Lints the toolchain enforces, such as `clippy::arithmetic_side_effects`, are judged from clippy or CI output, not by reading operators. Report a pre-existing suppression that covers new code as a nit with its location; cleaning it up is not the author's job.
5. If the change adds a convention, or contradicts `AGENTS.md`, `CONTRIBUTING.md`, or `STYLE_GUIDE.md`, propose the wording update as a nit.

Write findings to `$SCRATCH/code-quality.md` in the form the review standards require, then repeat them in your final message.
