---
name: firewood-review
description: Reviews ava-labs/firewood changes, either a pull request (number, URL, or branch) or uncommitted local work, against the repository's style guide, review process, and CI, and drafts replacement PR descriptions. Use when asked to "review PR 1234", "review my changes", "review this branch", "firewood review", or "write a better PR description".
argument-hint: "[PR number | URL | branch | local] [description]"
compatibility: Requires Claude Code with git, just, cargo/rustup, go, and the gh CLI or GitHub MCP. Designed for the ava-labs/firewood workspace.
allowed-tools: Read Write Bash Agent AskUserQuestion mcp__github__pull_request_read mcp__github__list_pull_requests mcp__github__get_commit mcp__github__list_commits
---

# Firewood Code Review

Review target: `$ARGUMENTS`

Local review of a Firewood change. Present the report in the conversation; never publish anything to GitHub unless the user explicitly asks.

The rules live in the repository, not in this skill:

- `STYLE_GUIDE.md` — what the code must look like.
- `CONTRIBUTING.md` § Code Review Process and § Before Requesting a Review — how findings are worded, which verdict applies, and what a PR description must say.
- `AGENTS.md` — build, lint, and test commands, toolchain selection, and the PR strategy.

[references/review-standards.md](references/review-standards.md) turns those into severity tiers and a verdict rule. Every subagent and the final synthesis apply it.

Version control: the commands below use `git` and `gh`. If another loaded skill says it takes precedence for version-control steps, use its equivalents for those steps and follow this skill for everything else.

## Checklist

Copy this into the task list and update it as you go:

- [ ] 0. Scratch directory created
- [ ] 1. Mode chosen; PR gated, checked out, and metadata fetched (PR mode)
- [ ] 2. Changed files listed; agents selected
- [ ] 3. Toolchain run or CI wait started in the background
- [ ] 4. Review agents dispatched in one message
- [ ] 5. Every Blocking and Warning finding verified
- [ ] 6. Report and verdict presented
- [ ] 7. Stash restored and checkout removed

## 0. Scratch directory

Create a fresh review directory once, inside the harness scratchpad (the session-specific directory the system prompt names as "Scratchpad directory"), or inside `${TMPDIR:-/tmp}` if the harness names none:

```bash
mktemp -d <scratchpad>/firewood-review-<pr-number-or-local>-XXXXXX
```

`mktemp` gives every review its own directory, so a rerun or a second review never reads stale files or collides on `checkout/`. Shell variables do not persist between Bash calls, so paste the printed absolute path into every later command, and into every subagent prompt (§ 4); subagents find the directory only through that path.

`$SCRATCH` below stands for that absolute path. Leave the directory in place when done; the user may want the logs.

## 1. Choose the mode

| The target names... | Mode |
| --- | --- |
| a PR number, URL, or branch, plus "description" or a request for a better PR description | [PR description only](#pr-description-only) |
| a PR number, URL, or branch | [PR review](#pr-review) |
| "local", "my changes", "unpublished", or nothing with a dirty tree | [Local review](#local-review) |

### PR review

1. **Gate.** `gh pr view <PR> --json isDraft,state`. If it is a draft or not `OPEN`, stop and use `AskUserQuestion` to ask whether to proceed before checking anything out.
2. **Fetch metadata in parallel**, even when the branch is already local: the description, checks, reviews, and comments inform the review and are not in the branch. Skip only when the user says to, no PR number exists, or both the MCP and `gh` are unavailable.

   GitHub MCP (preferred), `mcp__github__pull_request_read` with:

   | `method` | File |
   | --- | --- |
   | `get` | `$SCRATCH/pr.json` |
   | `get_files` | `$SCRATCH/pr-files.json` |
   | `get_check_runs` | `$SCRATCH/ci-checks.json` |
   | `get_reviews` | `$SCRATCH/pr-reviews.json` |
   | `get_review_comments` | `$SCRATCH/pr-review-comments.json` |
   | `get_comments` | `$SCRATCH/pr-comments.json` |

   `gh` fallback:

   ```bash
   gh pr view <PR> --json title,body,state,isDraft,labels,files,additions,deletions,baseRefOid,headRefOid > $SCRATCH/pr.json
   gh pr checks <PR> --json name,state,bucket,link,workflow,completedAt > $SCRATCH/ci-checks.json
   gh api repos/ava-labs/firewood/pulls/<PR>/reviews > $SCRATCH/pr-reviews.json
   gh api graphql -F n=<PR> -f query='query($n:Int!){repository(owner:"ava-labs",name:"firewood"){pullRequest(number:$n){reviewThreads(first:100){nodes{isResolved isOutdated path line comments(first:50){nodes{author{login} body url}}}}}}}' > $SCRATCH/pr-review-comments.json
   gh api repos/ava-labs/firewood/issues/<PR>/comments > $SCRATCH/pr-comments.json
   ```

   Record the base and head SHAs (`baseRefOid`, `headRefOid`) for every diff below.

3. **Check out once.** Use a detached worktree in the scratch directory, which leaves the user's branches and working tree alone:

   ```bash
   git fetch https://github.com/ava-labs/firewood.git pull/<PR>/head && git worktree add --detach $SCRATCH/checkout <headRefOid>
   ```

   Subagents share this checkout and never create their own.
4. **Size** is judged by the pr-metadata agent ([agents/pr-metadata.md](agents/pr-metadata.md) step 4).

### Local review

1. `git status --porcelain`. If both staged and unstaged changes exist and the user has not said which to review, ask: "Review the workspace as-is, or stash unstaged changes first?"
2. To stash: `git stash push --keep-index --include-untracked -m firewood-review-<scratch-dir-name>`, then record the entry's SHA from `git stash list --format='%H %gs'`. The stash stack is shared with every worktree and session, so restore by SHA in step 7, never with a bare `git stash pop`.

Step 3 runs the toolchain unless the working tree matches a commit CI already ran on.

### PR description only

1. Skip the gate and the checkout. Run PR review step 2 and `gh pr diff <PR> > $SCRATCH/pr.diff`.
2. Write a replacement per [references/pr-description.md](references/pr-description.md), verifying each claim against the diff, the code, or `ci-checks.json`.
3. Present it with a one-line note for each claim you dropped or changed. Do not edit the PR unless the user asks. Stop here.

## 2. Select agents

List changed files (`git diff --name-only <base>...<head>`, or `git diff --name-only HEAD` plus untracked files for local) into `$SCRATCH/changed-files.txt`, then select:

| Agent | When | Brief |
| --- | --- | --- |
| code-quality | always | [agents/code-quality.md](agents/code-quality.md) |
| pr-metadata | PR review | [agents/pr-metadata.md](agents/pr-metadata.md) |
| ci-review | any `.github/**` change | [agents/ci-review.md](agents/ci-review.md) |

## 3. Toolchain and CI

Required CI checks are authoritative for build, lint, and test: they cover every platform and feature profile, including `io-uring` on Linux, which macOS never compiles. Build results depend on the tree, not the commit, so a local build adds nothing when CI ran on a commit whose tree matches the tree under review. Compare trees, not SHAs: a clean merge or rebase produces a new commit with the same tree.

```bash
git rev-parse <ci-commit>^{tree} HEAD^{tree}   # the two lines match → same tree
```

In PR review the checkout is `headRefOid`, so the trees match by construction. In local review the tree under review is `HEAD^{tree}` with a clean working tree (uncommitted changes always differ), and `<ci-commit>` is the head of the PR for the checked-out branch (`gh pr view --json number,headRefOid`), if one exists.

**Trees match:** do not build locally unless the user asks. If any check in `ci-checks.json` is pending, wait in a background Bash (`run_in_background`), which notifies once when the checks finish:

```bash
gh pr checks <PR> --watch --interval 60 > /dev/null; gh pr checks <PR> --json name,state,bucket,link,workflow,completedAt > $SCRATCH/ci-checks.json
```

**Trees differ, or CI never ran:** start the local toolchain in the background before dispatching agents, so both run at once. Run it in a background Bash, logging to `$SCRATCH/toolchain.log`:

- `just prepush-lite` by default.
- `just prepush` when the change touches `ffi/`, `Cargo.toml` or `Cargo.lock`, or code gated behind a feature that `debug-ethhash-logger` does not enable (see `AGENTS.md` § PR Strategy).

**Either way, when `ffi/src/**` or `ffi/build.rs` changed:** CI does not check for a stale C header, so check it in the tree under review (`$SCRATCH/checkout` in PR review, the working tree in local review):

```bash
cd <tree-under-review> && just ci-build-ffi && git diff --exit-code ffi/firewood.h
```

`--all-features` needs rustc 1.94.1 or newer (`AGENTS.md` § Toolchain Floor). `rustc 1.94.0 is not supported by the following packages` is a local toolchain problem, not a defect in the change.

## 4. Dispatch review agents

Dispatch every selected agent in **one message** so they run in parallel, as `general-purpose` subagents. Subagents do not see this conversation, so each prompt contains:

- the absolute paths of the checkout and `$SCRATCH`, and the PR number or "local";
- the base and head SHAs (or "working tree");
- the instruction to read its brief and [references/review-standards.md](references/review-standards.md) by absolute path (this skill's directory is `${CLAUDE_SKILL_DIR}`) and follow both;
- its output file, `$SCRATCH/<agent>.md`.

Each brief tells its agent to repeat its findings in its final message, so a sandbox that blocks writes to `$SCRATCH` does not lose them.

## 5. Verify

Agent findings are leads, not conclusions. For every Blocking and Warning finding, read the cited code, reproduce the command, or check it against `ci-checks.json`. Downgrade what you cannot confirm to a question for the author, or drop it. Never report an unverified finding as Blocking.

In PR review, CI status goes in the § 6 CI summary only, never in a finding: a failing required check already blocks the merge, and GitHub, not a local run, is the authority on it. In local review there is no CI yet, so report each local failure as a Warning with its command and output.

## 6. Report

Present, in this order:

1. **Verdict** — Approve, Comment, or Request changes, with a one-sentence reason (rule in [references/review-standards.md](references/review-standards.md)).
2. **Blocking**, **Warning**, and **nit** findings, each with `file:line`, the rule it rests on, and the change that resolves it.
3. **Questions for the author.**
4. **CI and toolchain summary** — required checks, local commands run, and anything not run.
5. **Replacement PR description**, if pr-metadata wrote one.

## 7. Clean up

- Stashed: `git stash apply <sha>`, even if the review failed partway; then find the entry's current `stash@{n}` by its message and drop it.
- Checkout created: `git worktree remove $SCRATCH/checkout`. Confirm with `git worktree list` that no other checkout was created; remove any that was.
