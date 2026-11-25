# Agent brief: ci-review

Review changes under `.github/`.

1. Read every changed workflow and action file in full, and `AGENTS.md` § PR Strategy and § Toolchain Selection (already in your context if `CLAUDE.md` is loaded).
2. **Correctness:** the conventions in those `AGENTS.md` sections, plus trigger events, `permissions`, `if:` conditions, and `continue-on-error`. Actions are pinned to a full commit SHA with a version comment, and the pin and comment agree.
3. **Merge gating:** a job that should block merge is listed under `tests-required` in `ci.yaml`.
4. **Efficiency:** redundant steps, matrix entries that duplicate coverage, serial jobs that could run in parallel, missing caches.
5. **Dry run**, if `act` is installed: `act --list` and `act -n -j <changed-job>`. Note when `act` is unavailable.
6. **Realized results:** compare `$SCRATCH/ci-checks.json` (if present) with the jobs the change should produce. Flag failures, unexpected skips, and jobs that did not run.
7. **Triggers the PR's CI never exercises:** list each changed workflow whose `on:` has no `pull_request` trigger (only `push` to `main`, `schedule`, `release`, `workflow_run`, or `workflow_dispatch`). The change first runs there after merge. For each, say whether a manual `gh workflow run <workflow> --ref <branch>` can exercise it before merge, or why it cannot (for example, an `if:` gated on a release tag), and check that the PR description lists it as not run.

Write findings to `$SCRATCH/ci-review.md` in the form the review standards require, then repeat them in your final message.
