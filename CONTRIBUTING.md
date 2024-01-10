# Welcome contributors!

We are eager for contributions and happy you found yourself here.
Please read through this document to familiarize yourself with our
guidelines for contributing to firewood.

# Table of Contents

 * [Quick Links](#Quick Links)
 * [Testing](#Testing)
 * [How to submit changes](#How to submit changes)

# [Quick Links]

 * [Setting up docker](README.docker.md)
 * [Auto-generated documentation](https://ava-labs.github.io/firewood/firewood/)
 * [Issue tracker](https://github.com/ava-labs/firewood/issues)

# [Testing]

After submitting a PR, we'll run all the tests and verify your code meets our submission guidelines. To ensure it's more likely to pass these checks, you should run the following commands locally:

    cargo fmt
    cargo test
    cargo clippy
    cargo doc --no-deps

Resolve any warnings or errors before making your PR.

# [How to submit changes]

To create a PR, fork firewood, and use github to create the PR. We typically prioritize reviews in the middle of our the next work day,
so you should expect a response during the week within 24 hours.

# [How to report a bug]

Please use the [issue tracker](https://github.com/ava-labs/firewood/issues) for reporting issues.

# [First bugs for Contributors]

The [issue tracker](https://github.com/ava-labs/firewood/issues) typically has some issues tagged for first-time contributors. If not,
please reach out. We hope you work on an easy task before tackling a harder one.

# [How to request an enhancement]

Just like bugs, please use the [issue tracker](https://github.com/ava-labs/firewood/issues) for requesting enhancements. Please tag the issue with the "enhancement" tag.

# [Style Guide / Coding Conventions]

We generally follow the same rules that `cargo fmt` and `cargo clippy` will report as warnings, with a few notable exceptions as documented in the associated Cargo.toml file.

By default, we prohibit bare `unwrap` calls and index dereferencing, as there are usually better ways to write this code. In the case where you can't, please use `expect` with a message explaining why it would be a bug, which we currently allow. For more information on our motivation, please read this great article on unwrap: [Using unwrap() in Rust is Okay](https://blog.burntsushi.net/unwrap) by [Andrew Gallant](https://blog.burntsushi.net).

# [Who is involved]

Who is involved? - Open Government's CONTRIBUTING.md has as a name/author, and it might be nice to have a more personal/friendly individual to attact to a project and reach out to with questions. You might list the core contributors and their preferred methods of contact here, or link to a humans.txt file in your root directory (same place as your CONTRIBUTING.md file), which lets people know who they are working. Here is an example of a humans.txt file.

# [Where can I ask for help?]

Please reach out on X (formerly twitter) @rkuris for help or questions!

# Thank you

We'd like to extend a pre-emptive "thank you" for reading through this and submitting your first contribution!
