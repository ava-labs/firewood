# Design Documents

Firewood records its designs as living documentation. A design moves through two
folders, and the folder it lives in is its status:

- **`proposed/`** — a design under review, not yet built. Named
  `NNNN-short-slug.md` with a zero-padded sequence number for ordering and stable
  references in review threads. A proposal typically lands before development so
  reviewers comment on the document through normal pull-request review, but this
  ordering is a recommendation, not a requirement.
- **`active/`** — a design that reflects what the code does today. Named
  `short-slug.md` (no number); it is a living document, updated as the code evolves.

> [!NOTE]
> This is a lightweight convention, not a mandatory gate. A `proposed/` document is
> encouraged for significant or non-obvious designs, where writing it down sharpens
> the discussion; small or low-risk changes do not need one, and nothing blocks a
> pull request for lacking one. You may also write an `active/` design after the fact
> to document something already built. Optimize for "more designs discussed and
> recorded," not "process followed."

## Proposing a design

Run `just new-design <slug>` to scaffold `proposed/NNNN-<slug>.md` from the proposed
template, fill it in, and open a pull request.

## Promotion checklist

When a proposed design is implemented, promote it to `active/`:

1. `git mv proposed/NNNN-slug.md active/slug.md`.
2. Flip the frontmatter `status: proposed` to `status: active`; drop the
   proposal-only sections (Drawbacks, Unresolved questions), folding any survivors
   into the active structure.
3. Rewrite any future-tense prose as present tense — the design now describes
   reality.
4. Add cross-links to the implementing pull request(s) and commits.
5. Register the document in `active/README.md` and update `SUMMARY.md`.

## Templates

- [Proposed design template](templates/proposed.md)
- [Active design template](templates/active.md)
