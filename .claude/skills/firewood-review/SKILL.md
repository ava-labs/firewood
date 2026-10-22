---
name: firewood-review
description: Use when reviewing ava-labs/firewood code changes — pull request or local workspace. Invoke with phrases like "review PR", "review my changes", "review this branch", "firewood review", or "write a better PR description". Handles PR checkout, staged/unstaged changes, Rust toolchain, Go FFI, CI workflow, and multi-agent parallel review.
compatibility: Requires Claude Code with git, cargo/rustup, go, and the gh CLI. Designed for use in the ava-labs/firewood workspace.
allowed-tools: Read Write Bash Agent AskUserQuestion EnterWorktree ExitWorktree mcp__github__pull_request_read mcp__github__list_pull_requests mcp__github__get_commit mcp__github__list_commits
---

# Firewood Code Review

## Overview

Firewood-specific local code review. Dispatches parallel agents covering Rust toolchain validation, Go FFI checks, CI workflow analysis, and deep code quality. Never publishes review output to GitHub unless explicitly instructed.

Code quality review rules are defined in [`CODE_REVIEW.md`](../../../CODE_REVIEW.md) (referenced from the Code Review Guidelines section of `AGENTS.md`). Agents should read it in full before beginning scrutiny.

The review process itself follows [`CONTRIBUTING.md` — Code Review Process](../../../CONTRIBUTING.md#code-review-process), summarized in [Review Standards](#review-standards) below. They govern how findings are worded, which findings are reported, and which verdict is recommended.

**Version control:** the commands below use `git` and `gh`. Another loaded skill may take precedence over this skill's version-control steps; when one says so, use its equivalents for those steps and follow this skill for everything else.

## Review Standards

Apply these to every finding, in every agent and in the final report:

- **Actionable or `nit:`.** Each comment states exactly what change resolves it. A comment that requires no action is prefixed `nit:` so the author knows it can be ignored.
- **Principles, not preferences.** Justify each finding by citing the rule it violates (`CODE_REVIEW.md`, `CONTRIBUTING.md`, `AGENTS.md`, a failing check, or a concrete correctness/performance argument). Drop findings that rest only on taste.
- **Ask when unsure.** If intent is unclear, phrase the finding as a question to the author instead of asserting a defect.
- **Progress over perfection.** A change that is an improvement over the current codebase can be approved with minor comments outstanding. For larger out-of-scope improvements, suggest a follow-up issue or an annotated code `TODO(#issue)` rather than blocking.
- **Respectful and constructive.** Comments guide the author; they do not discourage.

## Writing a PR Description

A PR description passes when a reviewer can tell what is broken (or missing) from the first line of *Why this should be merged* and how the change fixes it from the first line of *How this works* (`CONTRIBUTING.md` § Before Requesting a Review). To write a replacement:

1. Open *Why this should be merged* with what is broken and why it matters, without describing the fix. Open *How this works* with how this change fixes it.
2. Keep every template section short. *How this works* lists only behavior a reviewer cannot see in the diff. *How this was tested* says "Required CI checks pass" once they have, plus any other validation run or not run. *Breaking Changes* is "None" or the checked boxes with their compatibility impact.
3. Verify every claim against the diff, the code, or CI results; drop any claim you cannot verify.
4. Drop anything the diff already shows.

## Phase 0: Setup

### Scratch Directory

Create a session-scoped scratch directory at the start. All fetched API data is written here so subagents can read it without re-fetching or needing extra permissions:

```bash
SCRATCH=/tmp/firewood-review/$$
mkdir -p "$SCRATCH"
```

Pass `$SCRATCH` to every subagent as part of its prompt.

### Determine Review Mode

**PR review** — prompt names a PR number, branch, or URL:

1. **Gate on draft status first.** Run `gh pr view <PR_NUMBER> --json isDraft,state`. If the PR is a draft or its state is not `OPEN`, stop and use `AskUserQuestion` to ask whether to proceed before checking anything out or dispatching agents — draft PRs are usually not ready for review.
2. If worktrees are available and no worktree is already checked out for this PR, create a dedicated worktree via `EnterWorktree` for the PR branch. Otherwise fall back to `gh pr co <PR_NUMBER>`. Create at most one checkout per review - subagents can share worktrees and therefore, should not create worktrees of their own.
3. Skip checkout if the prompt says the workspace is already on the PR branch.
4. Fetch PR metadata — prefer the `github` MCP if available, otherwise fall back to `gh` CLI:

   **MCP (preferred):**

   ```text
   mcp__github__pull_request_read → write JSON to $SCRATCH/pr.json
   ```

   **CLI fallback:**

   ```bash
   gh pr view <PR_NUMBER> \
     --json title,body,comments,reviews,labels,isDraft,additions,deletions,baseRefOid,headRefOid \
     > "$SCRATCH/pr.json"
   gh api repos/ava-labs/firewood/pulls/<PR_NUMBER>/comments \
     > "$SCRATCH/pr-comments.json"
   ```

5. Record the base and head SHAs from the fetched data for use in diffs.
6. **Size check.** Count changed lines per file (`gh pr view <PR_NUMBER> --json files`), excluding generated files (`Cargo.lock`, `MODULE.bazel.lock`, `*.snap`, `ffi/firewood.h`, other lockfiles). If hand-written changes exceed a few hundred lines — more than can be reviewed well in 30–60 minutes — note it for the verdict and propose how the PR could be split.

**Local changes** — prompt says "local changes", "unpublished", or similar:

1. Run `git status --porcelain` to detect staged vs unstaged files.
2. If both staged and unstaged changes exist and the prompt didn't clarify, use `AskUserQuestion`: "Review workspace as-is, or stash unstaged changes first?"
3. If stash: `git stash push --include-untracked -m "firewood-review-stash"`. After synthesis completes, `git stash pop` unconditionally.

**PR description only** — prompt asks for a better, shorter, or rewritten PR description:

1. Skip the draft gate and checkout. Run PR review steps 4–5, fetch the diff with `gh pr diff <PR_NUMBER>`, and fetch CI results with the `ci-review` command below.
2. Head to phase 2 (synthesis) and write a replacement following [Writing a PR Description](#writing-a-pr-description) and present it with a one-line note per claim you dropped or changed.
3. Do not edit the PR unless the user asks.

### Detect Changed File Types

From the diff (`git diff --name-only <base>..<head>` for PR, or `git diff --name-only HEAD` for local), determine which agents to activate:

- Any `*.rs` or `Cargo.*` changes → activate **rust-toolchain**
- Any `ffi/**` Go files → activate **go-toolchain**
- Any `.github/**` changes → activate **ci-review**
- Always activate **code-quality**

Write the changed file list to `$SCRATCH/changed-files.txt`.

## Phase 1: Parallel Agents

Dispatch all applicable agents **in a single message** (parallel execution). Each agent receives: `$SCRATCH` path, the review checkout path, diff output, list of changed files, the [Review Standards](#review-standards), and the instructions from its section below. PR metadata is read from files in `$SCRATCH`.

Every agent also returns its findings in its final response, so a sandbox that blocks writes to `$SCRATCH` does not lose them. For each finding, agents report the evidence that supports it (command output, file and line, CI result) so the synthesis step can verify it.

---

### Agent: rust-toolchain

**Precondition:** Any `*.rs` or `Cargo.*` files changed.

```bash
uname -s
cargo fmt --all --check
cargo fetch --locked
# All subsequent cargo commands: add --frozen --target-dir target/review
```

Every feature set below builds on both platforms. Note that `io-uring` is inert off Linux, so a green macOS `--all-features` run does not exercise `storage/src/linear/io_uring.rs`; only Linux compiles it.

Use `--frozen` after the initial fetch to ensure we're not accidentally pulling new dependencies that aren't covered by the lockfile.

Use `--target-dir target/review` for ALL cargo commands to avoid conflicting with the workspace target dir.

Run `cargo clippy --workspace --all-targets` and `cargo nextest run --workspace --all-targets` for each feature set:

| Feature flags               | macOS | Linux |
| --------------------------- | ----- | ----- |
| (none)                      | ✓     | ✓     |
| `--no-default-features`     | ✓     | ✓     |
| `--features ethhash,logger` | ✓     | ✓     |
| `--all-features`            | ✓     | ✓     |

`--all-features` requires rustc 1.94.1+, above the 1.94.0 workspace MSRV, because it enables `fwdctl`'s `launch` feature and its AWS SDK dependencies declare that floor. If `cargo` reports `rustc 1.94.0 is not supported by the following packages`, that is this constraint and not a defect in the PR under review.

If the PR adds any new Cargo feature, test with and without it across the applicable matrix rows.

**Header drift check:**

```bash
cargo build -p firewood-ffi --frozen --target-dir target/review
git diff --exit-code ffi/firewood.h
```

Non-zero exit = stale header. Write results to `$SCRATCH/rust-toolchain.txt`.

---

### Agent: go-toolchain

**Precondition:** Any Go files in `ffi/` changed.

Go bindings require the FFI build to use the workspace `target/` dir (the `LDFLAGS` in `ffi/firewood.go` reference it directly — do not use `--target-dir` here):

```bash
cargo build -p firewood-ffi -F logger --frozen
cd ffi
./scripts/lint.sh
go test ./... -race
```

Write results to `$SCRATCH/go-toolchain.txt`. Report any lint violations or test failures.

---

### Agent: code-quality

**Always activated.** Read all changed files in full. Do not skim diffs — read the actual source for context. Read PR metadata from `$SCRATCH/pr.json` and `$SCRATCH/pr-comments.json` if present.

Read `CODE_REVIEW.md` in full and apply every rule to the changed code.

Review the PR description against `.github/pull_request_template.md`:

- Every template section is present and follows `CONTRIBUTING.md` § Before Requesting a Review. *How this was tested* notes important validation that was not run. *Breaking Changes* either says "None" or checks each component whose public interface the change breaks — as listed in `RELEASE.md` § Versioning Policy — with a brief compatibility explanation after each checked box. Flag "None" when the diff contains such a break.
- Claims are factual and match the diff; motivation, behavior, scope, and impact are stated.
- The first line of *Why this should be merged* says what is broken, the first line of *How this works* says how the change fixes it, and the rest does not restate the diff. If not, report a Warning and include a replacement written per [Writing a PR Description](#writing-a-pr-description).
- It uses vocabulary from the README and codebase and introduces no undefined terms.
- Labels match `CODE_REVIEW.md` § Labeling (e.g. `breaking-change`).

Additionally:

- Read `CLAUDE.md` and `CONTRIBUTING.md` in full and check whether this PR's changes create any gaps or inconsistencies in those documents. Suggest specific wording updates if helpful.
- If this review session itself introduces new conventions (e.g., a new skill, new workflow), note what should be added to `AGENTS.md` or `CONTRIBUTING.md`.

Write findings to `$SCRATCH/code-quality.txt`.

---

### Agent: ci-review

**Precondition:** Any `.github/**` files changed.

1. Read all changed workflow YAML files in full.
2. **Efficiency:** redundant steps, matrix entries that duplicate coverage, jobs that could parallelize, missing `cache` steps.
3. **Correctness:** wrong trigger events, missing `permissions`, broken `if:` conditionals, misuse of `continue-on-error`, pinned action SHA vs tag inconsistencies.
4. **`act` validation** (if available):

   ```bash
   act --list
   act -n -j <modified-job>
   ```

   Report output. If `act` is unavailable, note it.
5. **Realized CI results** — prefer the `github` MCP if available, otherwise fall back to `gh`:

   **MCP (preferred):**

   ```text
   mcp__github__pull_request_read → extract check_runs/status fields → write to $SCRATCH/ci-checks.json
   ```

   **CLI fallback:**

   ```bash
   gh pr checks <PR_NUMBER> --json name,state,bucket,link,workflow,startedAt,completedAt \
     > "$SCRATCH/ci-checks.json"
   ```

   Compare expected jobs vs actual. Flag failures, unexpected skips, or jobs that didn't run.

Write findings to `$SCRATCH/ci-review.txt`.

---

## Phase 2: Synthesis

After all agents complete, read `$SCRATCH/*.txt` (and the agents' final responses) and aggregate into a single report:

1. **Verify before reporting.** Agent output is a lead, not a conclusion. For every Blocking or Warning finding, confirm it yourself: read the cited code, reproduce the command, or check it against realized CI results (fetch them with the `ci-review` command above if that agent did not run).

   Required CI checks are authoritative for build, lint, and test outcomes; CI covers every platform and feature set that matters, including `io-uring` on Linux. A failure in CI is Blocking. A failure seen only locally is never Blocking — local toolchains and configuration differ from CI — so report it as a Warning, with the command and output, and do not let it change the verdict.

   Drop findings you cannot verify, or downgrade them to questions for the author. Never report an unverified finding as Blocking.

2. Three severity tiers:
   - **Blocking** — must fix before merge (failing required CI checks, `unwrap` in prod, unsafe soundness issues, significant correctness, architecture, design, or maintainability concerns)
   - **Warning** — should fix, non-blocking (failures seen only locally, stale `ffi/firewood.h`, missing edge case tests, minor error context gaps, incomplete PR description)
   - **nit** — optional; the author may ignore it (doc wording, style, AGENTS.md updates, CI efficiency)

   Questions to the author are listed separately from findings.

3. **Recommend a verdict** per `CONTRIBUTING.md` § Code Review Process:
   - **Approve** — the change is an improvement over the current codebase. Warnings and nits may remain as comments.
   - **Comment** — the review is incomplete, or it raises only questions or optional suggestions that need answers before a decision.
   - **Request changes** — at least one verified Blocking finding, or the PR is too large to review in 30–60 minutes (include a proposed split). The same reviewer re-reviews after revisions.

4. Present the report in the current conversation, including any replacement PR description from `code-quality` so the reviewer can offer it to the author. Do not publish to GitHub unless the user explicitly asks.

5. If a worktree was created: offer to exit and clean it up. Confirm no other worktrees or checkouts were created during the review; remove any that were.

6. If changes were stashed: `git stash pop` unconditionally before finishing.

7. Clean up: `rm -rf "$SCRATCH"` after presenting the report.
