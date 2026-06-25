# How These Docs Work

This site is an [mdBook](https://rust-lang.github.io/mdBook/) rooted at `docs/`
(`docs/book.toml`, `docs/src/`).

## Toolchain

- `mdbook` builds the book.
- `mdbook-mermaid` renders Mermaid diagrams; its JS assets are generated at build time
  by `just book-assets` and are git-ignored.
- Callouts use mdBook's native alert syntax (`> [!NOTE]`, `> [!WARNING]`, …) — no
  preprocessor required.
- `mdbook-linkcheck2` runs as a backend during `mdbook build` and validates internal
  links (`follow-web-links = false`).

## Build and serve

- `just book-serve` — serve locally with live reload.
- `just book-build` — build and run the link checker (what CI runs on pull requests).

Install the toolchain as described in
[Development Environment](../getting-started/dev-environment.md).

## Adding or editing a page

1. Add or edit a Markdown file under `docs/src/`.
2. Add it to `docs/src/SUMMARY.md` — a linked entry for real content, or a draft
   chapter (`- [Title]()`) for a page not yet written.
3. Run `just book-build` to validate, then open a pull request.

## Designs

The [Design Documents](../designs/README.md) section describes how to propose and
promote designs, including the `just new-design` scaffolder.
