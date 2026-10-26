# Welcome contributors

We are eager for contributions and happy you found yourself here.
Please read through this document to familiarize yourself with our
guidelines for contributing to firewood.

## Table of Contents

* [Quick Links](#quick-links)
* [Testing](#testing)
* [Updating the Go version](#updating-the-go-version)
* [How to submit changes](#how-to-submit-changes)
* [Required merge checks](#required-merge-checks)
* [Signing your commits](#signing-your-commits)
* [Code Review Process](#code-review-process)
* [Labels](#labels)
* [Where can I ask for help?](#where-can-i-ask-for-help)

## Quick Links

* [Setting up the devcontainer](.devcontainer/)
* [Auto-generated documentation](https://ava-labs.github.io/firewood/rustdoc/firewood/)
* [Issue tracker](https://github.com/ava-labs/firewood/issues)

## Testing

After submitting a PR, we'll run all the tests and verify your code meets our submission guidelines. To ensure it's more likely to pass these checks, run the same recipes CI runs:

    just fmt
    just prepush-lite

`just fmt` applies formatting; the recipes only check it. `prepush-lite` checks
formatting and TODOs, runs clippy and the tests for one feature profile, and
lints the Markdown and the documentation build.

CI is authoritative for the full matrix, and a change that touches the FFI,
workspace dependencies, or a feature that profile does not enable warrants the
whole thing before you push:

    just prepush

Use `./scripts/run-just.sh <recipe>` in place of `just` if you do not have `just`
installed; the wrapper falls back to Nix and otherwise prints installation
instructions. Resolve any warnings or errors before making your PR.

Use the recipes rather than bare Cargo for anything that builds or runs code.
`rust-toolchain.toml` pins a nightly for `cargo fmt`, clippy, and miri, so a
bare `cargo nextest run` tests on that nightly while CI tests on stable. See
[Toolchain Selection](./AGENTS.md#toolchain-selection) in AGENTS.md.

Also, if you update any versions of packages, notably the MSRV (Minimum Supported Rust Version), you ought to update the nix ffi flake lock file to pin compatible versions of nix packages as well:

    ./scripts/run-just.sh update-ffi-flake

## Updating the Go version

Firewood pins one Go version in several places, and CI fails when they
disagree. To move to a different Go release, follow these steps:

1. Set the `go` directive in `ffi/go.mod`, `ffi/tests/go.mod` and
   `ffi/tools/external/go.mod` to the target version.
2. Set `FIREWOOD_GO_VERSION` in `infra/toolchain-versions.sh`, and set
   `FIREWOOD_GO_LINUX_AMD64_SHA256` to the checksum of
   `go<version>.linux-amd64.tar.gz` from
   <https://go.dev/dl/?mode=json&include=all>.
3. Point the `golang.url` input in `ffi/flake.nix` at an AvalancheGo commit
   whose `nix/go/default.nix` sets `goVersion` to the target version.
   `ffi/flake.nix` takes its Go toolchain from AvalancheGo, so the version bump
   must land in AvalancheGo first. To find the commit, look at the history of
   `nix/go/default.nix` in AvalancheGo. For example, Go 1.26.10 arrived in
   `4f8c6316bed83c3158aff4b4be43855caa14925f`.
4. Run `./scripts/run-just.sh update-ffi-flake` to refresh `ffi/flake.lock`, and
   commit the result. The `ffi-nix` CI job runs the same recipe and fails when it
   changes the lock file or finds a Go version that disagrees with
   `ffi/go.mod`.

## How to submit changes

To create a PR, fork firewood, and use GitHub to create the PR. We typically prioritize reviews in the middle of the next work day,
so you should expect a response during the week within 24 hours.

## Required merge checks

The `tests-required` job in [the CI workflow](.github/workflows/ci.yaml) is the
single CI status required by the `main` branch protection. Its `needs` list is
the version-controlled source of truth for merge-blocking CI jobs.

When adding or removing a merge-blocking job, update the `tests-required.needs`
list in the same pull request. No corresponding GitHub settings change is
needed. The aggregate job uses `always()` and fails when any required job fails
or is skipped, so dependency failures cannot accidentally allow a merge.

## Signing your commits

CI rejects PRs that contain unsigned commits, so configure commit signing
before you open one — GitHub should then show every commit as **Verified**.

The quickest setup is signing with the SSH key you already push with:

    git config --global gpg.format ssh
    git config --global user.signingkey ~/.ssh/id_ed25519.pub
    git config --global commit.gpgsign true

Then add that key to GitHub as a **Signing Key** under *Settings → SSH and GPG
keys*. See GitHub's [signing commits][gh-signing] guide for full details,
including GPG keys and Windows/macOS setup.

[gh-signing]: https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits

## Code Review Process

Code review is a critical part of our development process. It ensures that our codebase remains maintainable, performant, and secure. This document outlines how we approach code reviews at Ava Labs, with responsibilities and expectations for both reviewers and authors.

### For Reviewers

Reviews should be completed or commented within one business day. We have a daily reminder for reviews that have not been reviewed that is posted in slack's #firewood channel.

When reviewing code, your goal is to help the author improve the quality of the change and confirm that it meets our architectural and operational standards. GitHub provides three primary review options:

#### ✅ Accept (Approve)

Use this when the code is an improvement over the current state of the codebase.

* It's okay to request minor changes in comments and still approve the pull request.
* Perfection is not the goal — progress is. If the submitted code is better than what's in production, it's acceptable to approve even if small improvements remain. Consider adding a new issue or request adding a code TODO for larger changes.

#### 💬 Comment (Comment Only)

Use this when your review is incomplete, or you're not ready to approve or reject yet. You should use this if the code is too large to review in a limited amount of time (typically 30-60 minutes). You can also suggest how to break up this diff into a smaller diff.

* This can be helpful for asking clarifying questions, suggesting optional improvements, or flagging issues you're unsure about.
* This state signals that your review is in progress or advisory, not final.
* Make sure your review comments are actionable, or mark them as "nit" which implies that no action is required. If there's any doubt if your comment is actionable, call out exactly what action to take to resolve the comment.

#### ❌ Reject (Request Changes)

Use this when there are significant concerns with the code's correctness, architecture, design, or maintainability.

* A "Reject" signals that the pull request must not be merged until the raised issues are addressed.
* The author is expected to make substantial revisions and return the code for a second round of review by the same reviewer.

#### Best Practices

* Be respectful and constructive. Your comments should guide and empower the author, not discourage them.
* Justify your feedback with principles, not preferences.
* If you're unsure, ask questions rather than assume intent.
* If you're going to nitpick, preface the comment with "nit:". This means the author can choose to ignore the comment.

### For Authors

As the author of a pull request, your responsibility is to ensure the review process is smooth, transparent, and productive.

#### Before Requesting a Review

* Review your own code. Catch obvious issues and clean up unnecessary changes.
* Some code changes are too large to be reviewed quickly. This can happen when the number of lines of new code is more than a few hundred. Consider breaking up your code in this case.
* Write a concise, accurate PR description. Keep claims factual and focus on motivation, behavior, scope, and impact. Avoid implementation details that are apparent from the code unless they help explain the purpose of the PR. Whenever possible, use pre-defined vocabulary from the README or elsewhere in the codebase. Do not introduce new terms in the PR description that are not defined in the codebase.
* Lead with the problem and the fix. Reviewers assume every claim in the description has been checked, so each claim costs review time. A reviewer should be able to tell what is broken from the first line of *Why this should be merged* and how it is fixed from the first line of *How this works*; keep the rest short or leave it out, and do not restate what the diff already shows.
* Use the repository [pull request template](.github/pull_request_template.md):
  * **Why this should be merged:** Explain the problem and why the change is needed.
  * **How this works:** Summarize the behavior and scope of the change. Distinguish between similar but different operations and state important limitations or compatibility effects.
  * **How this was tested:** Summarize tests added and validation run. For most PRs, it is enough to say "Required CI checks pass" once they have passed. Note important validation that was not run.
  * **Breaking Changes:** Check every affected project component and briefly explain the compatibility impact after each checked box. If no components are affected, remove the checkboxes and write "None".
* Add tests and verify they pass locally and in CI.

#### During Review

* Respond to each comment, even if just to acknowledge it.
* Use GitHub's "Resolve" feature when you've addressed feedback. In some cases, to get to the "Resolve" button requires you select "Hide" first, with a reason of "Resolved".
* Don't be afraid to explain your design decisions—but stay open to change.
* If you disagree with a reviewer's suggestion, provide reasoning. If you're sure your response fully resolves the reviewer's suggestion, mark it as resolved.

#### After Review

* When you've made requested changes, clearly indicate it in your comment or commit, and re-request the review.
* If the PR was rejected, wait for explicit re-approval before merging.
* Thank your reviewers—they're helping you ship better code.

## How to report a bug

Please use the [issue tracker](https://github.com/ava-labs/firewood/issues) for reporting issues.

## First time fixes for contributors

The [issue tracker](https://github.com/ava-labs/firewood/issues) typically has some issues tagged for first-time contributors. If not,
please reach out. We hope you work on an easy task before tackling a harder one.

## How to request an enhancement

Just like bugs, please use the [issue tracker](https://github.com/ava-labs/firewood/issues) for requesting enhancements. Please tag the issue with the "enhancement" tag.

## Labels

Issues and pull requests are organized with a namespaced label taxonomy
(`area/*`, `kind/*`, `priority/*`, `status/*`). See [`LABELS.md`](./LABELS.md).
Labels are managed as code in `.github/labels.yml` — edit the manifest, never
the GitHub UI.

## Style Guide / Coding Conventions

See [`STYLE_GUIDE.md`](./STYLE_GUIDE.md).

## Where can I ask for help?

If you have questions or need help, please post them as issues in the [issue tracker](https://github.com/ava-labs/firewood/issues). This allows the community to benefit from the discussion and helps us maintain a searchable knowledge base.

## Thank you

We'd like to extend a pre-emptive "thank you" for reading through this and submitting your first contribution!
