# Wiki-LLM maintenance contract

This directory is the durable, model-oriented map of `usque-rs-openwrt`.
Read `wiki/index.md` first, then follow links to the smallest relevant page.
The canonical implementation remains the parent Rust crate; the wiki explains it.

## Layout

- `raw/sources/` holds immutable imported source material, when any exists.
- `raw/assets/` holds immutable imported binary assets, when any exists.
- `wiki/` contains synthesized, maintained knowledge.
- `wiki/index.md` is the complete navigation entry point.
- `wiki/log.md` is append-only maintenance history.

## Editing rules

- Keep one topic per page and use kebab-case file names.
- Prefer pages between 20 and 80 lines.
- Link related pages with `[[relative/path]]` wiki links.
- Every wiki page must end with a `## Sources` section.
- Cite repository paths and commands that substantiate each page.
- Update `wiki/index.md` whenever a page is added, moved, or removed.
- Append to `wiki/log.md`; do not rewrite prior history.
- Treat `raw/` content as immutable after import.

## Security rules

- Never copy secret values, private keys, tokens, or `config.json` contents.
- Refer to `config.json` by path and schema only.
- Keep this public fork free of deployment-specific credentials, private IPs, or local environment topology.

## Sources

- `../README.md` and `../Cargo.toml` define crate intent.
