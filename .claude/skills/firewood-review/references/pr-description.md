# Writing a PR Description

A description passes when a reviewer can tell what is broken or missing from the first line of *Why this should be merged*, and how the change fixes it from the first line of *How this works* (`CONTRIBUTING.md` § Before Requesting a Review). Use the sections of `.github/pull_request_template.md`:

| Section | Contents |
| --- | --- |
| Why this should be merged | Opens with what is broken and why it matters, without describing the fix. |
| How this works | Opens with how the change fixes it. Lists only behavior, limitations, and compatibility effects a reviewer cannot see in the diff. |
| How this was tested | "Required CI checks pass" once they have, plus any other validation run, and important validation not run. |
| Breaking Changes | "None" with the checkboxes removed, or each affected component checked with its compatibility impact. Components and their public interfaces are listed in `RELEASE.md` § Versioning Policy. |

Every claim is one the writer verified against the diff, the code, or CI results. Use vocabulary from the README and codebase; introduce no undefined terms. Keep every section short; anything the diff already shows is left out.
