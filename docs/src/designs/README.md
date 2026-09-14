# Design Documents

Firewood records its designs as living documentation in a single flat directory.
Every design is `YYYY-MM-DD-slug.md`: the date the design was proposed, followed by a
short slug. The date is assigned once, when the file is created, and never changes, so
a design's path stays a stable reference in review threads and cross-links. Freshness
is a separate question, answered by git history: `just design-age` lists designs by
last commit date, oldest first.

A design's lifecycle state lives in its `status` frontmatter field, not in its
location:

- **`draft`**: a work in progress, not yet ready for review.
- **`proposed`**: under review, not yet built.
- **`active`**: describes what the code does today; kept up to date as the code evolves.
- **`superseded`**: replaced by a newer design; kept for history.
- **`rejected`**: considered and declined; kept for the record.

> [!NOTE]
> This is a lightweight convention, not a mandatory gate. A `proposed` design is
> encouraged for significant or non-obvious work, where writing it down sharpens the
> discussion; small or low-risk changes do not need one, and nothing blocks a pull
> request for lacking one. An `active` design may also be written after the fact to
> document something already built.

## Designs

| Design | Proposed | Status |
| --- | --- | --- |
| [mdBook documentation site](2026-06-17-mdbook-documentation-site.md) | 2026-06-17 | active |

Subsystems not yet documented as designs are tracked in
[ava-labs/firewood#2139](https://github.com/ava-labs/firewood/issues/2139).

## Frontmatter

Every design begins with a YAML frontmatter block, stripped from the rendered site by
the in-repo `frontmatter-strip` preprocessor:

| Field | Required | Values |
| --- | --- | --- |
| `title` | yes | Human-readable design title. |
| `status` | yes | `draft`, `proposed`, `active`, `superseded`, or `rejected`. |
| `category` | yes | The Firewood subsystem(s) the design touches: `storage`, `ffi`, `revision-management`, `hashing`, `proposals`, `docs`, or `tooling`. A design spanning several lists them, e.g. `[storage, hashing]`. Extend the list as new subsystems gain designs. |
| `authors` | yes | List of GitHub handles. |
| `tracking-issue` | no | An `owner/repo#N` reference to the issue or PR tracking the work. |

There is no date field because the proposal date is in the filename.

## Propose a design

Run `just new-design <slug>` to scaffold `YYYY-MM-DD-<slug>.md` from `template.md`,
fill it in, and open a pull request.

## Promote a design

When a proposed design is implemented, promote it in place:

1. Flip the frontmatter `status: proposed` to `status: active`.
2. Drop the Unresolved questions section, folding any settled answers into the design.
   Keep Drawbacks: it records what was accepted and why.
3. Rewrite future-tense prose as present tense.
4. Add cross-links to the implementing pull request(s) and commits.
5. Update this index's status marker and the sidebar entry in `docs/src/SUMMARY.md`.
