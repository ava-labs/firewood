# Review Standards

Derived from `CONTRIBUTING.md` § Code Review Process. Apply them to every finding.

## Every finding

- **Actionable or `nit:`.** State exactly what change resolves it. A comment that needs no action starts with `nit:` so the author can ignore it.
- **Principles, not preferences.** Cite what the finding rests on: a rule in `STYLE_GUIDE.md`, `CONTRIBUTING.md`, or `AGENTS.md`; or a concrete correctness, performance, or maintainability argument (for example, duplicated logic, an existing helper that should be reused, or a simpler equivalent), along with the change that resolves it. Drop findings that rest on taste.
- **Evidence.** Give `file:line`, command output, or the CI check, so the finding can be verified.
- **Ask when unsure.** When intent is unclear, write a question for the author instead of asserting a defect.
- **Progress over perfection.** For a larger improvement outside the change's scope, suggest a follow-up issue or a `TODO(#issue)` instead of blocking.
- **Respectful and constructive.** Findings guide the author.

## Severity

| Tier | Meaning | Examples |
| --- | --- | --- |
| Blocking | Must change before merge | panic the lints cannot see (`expect`, `panic!`, `unreachable!`) reachable from input in production; unsound `unsafe`; significant correctness, architecture, design, or maintainability problem |
| Warning | Should change; does not block | local toolchain failure (local review only); stale `ffi/firewood.h`; missing edge-case test; thin error context; PR description that fails [pr-description.md](pr-description.md) |
| nit | Optional | wording, style the linters do not enforce, doc updates, CI efficiency |

In PR review, CI results are reported in the CI summary, not as findings. Questions for the author are listed apart from findings.

## Verdict

| Verdict | When |
| --- | --- |
| Approve | The change improves the codebase. Warnings and nits may remain as comments. |
| Comment | The review is incomplete; it raises only questions or optional suggestions; or the change is too large to review in 30–60 minutes (include a proposed split). |
| Request changes | At least one verified Blocking finding. The same reviewer re-reviews after revisions. |
