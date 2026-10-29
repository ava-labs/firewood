# Agent brief: pr-metadata

Review the pull request as a unit: its description, labels, size, and the conversation so far. Inputs are in `$SCRATCH`: `pr.json`, `pr-files.json` (if present), `pr-reviews.json`, `pr-review-comments.json`, `pr-comments.json`, and `ci-checks.json`.

In local review, the commits under review may differ from the PR's pushed head. Judge the description against the local diff, and report what it would need to say once the local commits are pushed.

1. **Description.** Check the body against [../references/pr-description.md](../references/pr-description.md) and the diff:
   - every template section is present;
   - every claim matches the diff;
   - *Breaking Changes* says "None" only if no component listed in `RELEASE.md` § Versioning Policy has a broken public interface, and each checked box explains its impact.

   If it fails, report a Warning and write a replacement description.
2. **Labels.** Compare against `LABELS.md` § How labels get applied: `area/c-chain` when the change touches C-Chain reexecution or libevm integration; `breaking-change` when a breaking-change box is checked.
3. **Title.** Conventional Commits with an allowed type (`AGENTS.md` § Commit and PR Title Convention).
4. **Size.** More than a few hundred hand-written changed lines (lockfiles, snapshots, and `ffi/firewood.h` excluded): propose a split along independently reviewable seams.
5. **Prior review.** List unresolved review threads and whether the latest commits address them. Do not repeat a finding another reviewer already raised; cite it instead.

Write findings, and any replacement description, to `$SCRATCH/pr-metadata.md` in the form the review standards require, then repeat them in your final message.
