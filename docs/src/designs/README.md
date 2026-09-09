# Design Documents

Firewood records its designs as living documentation in a single flat directory.
Every design is `YYYY-MM-DD-slug.md` — the date the design was first written, followed
by a short slug. The date is assigned once, when the file is created, and never
changes afterwards: it records when the design was *proposed*, not when it was last
touched, so a design's path stays a stable reference in review threads and
cross-links. Two designs written on the same day are distinguished by their slugs.

A design's lifecycle state lives in its `status` frontmatter field, not in its
location:

- **`proposed`** — under review, not yet built. A proposal typically lands before
  development so reviewers comment on the document through normal pull-request review.
- **`active`** — reflects what the code does today; a living document, updated as the
  code evolves.
- **`draft`**, **`superseded`**, **`rejected`** — a work in progress, a design replaced
  by a newer one, or a design considered and declined; each kept for the record.

> [!NOTE]
> This is a lightweight convention, not a mandatory gate. A `proposed` design is
> encouraged for significant or non-obvious work, where writing it down sharpens the
> discussion; small or low-risk changes do not need one, and nothing blocks a pull
> request for lacking one. An `active` design may also be written after the fact to
> document something already built. Optimize for "more designs discussed and
> recorded," not "process followed."

## Designs

| Design | Proposed | Status |
| --- | --- | --- |
| [mdBook documentation site](2026-06-17-mdbook-documentation-site.md) | 2026-06-17 | active |

## Designs still to be written

Several core subsystems are documented only in code, not yet as designs. The
backlog — revision management, hashing, proposals & commits, state sync &
reconstruction, and the Go FFI layer — is tracked in
[ava-labs/firewood#2139](https://github.com/ava-labs/firewood/issues/2139).

## Frontmatter

Every design begins with a YAML frontmatter block, stripped from the rendered site by
the in-repo `frontmatter-strip` preprocessor:

| Field | Required | Values |
| --- | --- | --- |
| `title` | yes | Human-readable design title. |
| `status` | yes | `draft`, `proposed`, `active`, `superseded`, or `rejected`. |
| `category` | yes | A Firewood subsystem: `storage`, `ffi`, `revision-management`, `hashing`, `proposals`, `docs`, or `tooling`. |
| `authors` | yes | List of GitHub handles. |
| `tracking-issue` | no | An `owner/repo#N` reference to the issue or PR tracking the work. |

The schema has **no date fields**. A design's proposal date is its filename prefix,
fixed when the file is created; its freshness comes from git history. Neither is a
hand-maintained date inside the document, which would only drift out of sync with
reality. Run `just design-age` to list designs by last git-commit date, oldest first,
to spot stale documents.

## Propose a design

Run `just new-design <slug>` to scaffold `YYYY-MM-DD-<slug>.md` — today's date plus the
slug — from `template.md`, fill it in, and open a pull request.

## Promote a design

When a proposed design is implemented, promote it in place — a status flip, not a file
move, so its `YYYY-MM-DD-slug.md` path stays stable and its date prefix goes on
recording when the design was proposed:

1. Flip the frontmatter `status: proposed` to `status: active`.
2. Drop the proposal-only sections (Drawbacks, Unresolved questions), folding any
   surviving content into the living structure.
3. Rewrite any future-tense prose as present tense — the design now describes reality.
4. Add cross-links to the implementing pull request(s) and commits.
5. Update this index's status marker and `SUMMARY.md`.
