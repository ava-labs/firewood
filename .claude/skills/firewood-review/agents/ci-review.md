# Agent brief: ci-review

Review changes under `.github/`.

1. Read every changed workflow and action file in full, and `AGENTS.md` § PR Strategy and § Toolchain Selection.
2. **Correctness:** the conventions in those `AGENTS.md` sections, plus trigger events, `permissions`, `if:` conditions, and `continue-on-error`. Actions are pinned to a full commit SHA with a version comment, and the pin and comment agree.
3. **Merge gating:** a job that should block merge is listed under `tests-required` in `ci.yaml`.
4. **Efficiency:** redundant steps, matrix entries that duplicate coverage, serial jobs that could run in parallel, missing caches.
5. **Dry run**, if `act` is installed: `act --list` and `act -n -j <changed-job>`. Note when `act` is unavailable.
6. **Realized results:** compare `$SCRATCH/ci-checks.json` (if present) with the jobs the change should produce. Flag failures, unexpected skips, and jobs that did not run.

Write findings to `$SCRATCH/ci-review.md` in the form the review standards require, then repeat them in your final message.
