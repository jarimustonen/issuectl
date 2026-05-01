# issuectl

CLI for managing markdown-based issues with frontmatter.

Designed to pair with the `/issue` Claude Code skill, which keeps issues as
`issues/open/NN-slug/item.md` files with YAML frontmatter.

## Status

Early scaffold. None of the commands are implemented yet.

## Planned commands

| Command | Purpose |
|---------|---------|
| `issuectl list` | List/query issues by frontmatter fields |
| `issuectl dedup` | Detect duplicate issues |
| `issuectl renumber` | Renumber and fix cross-references during merges |
| `issuectl gen-skill` | Generate a `/issue` skill bundle for the current repo |

## Install (later)

```sh
cargo install --path .
```

## Development

```sh
cargo build
cargo run -- list
```
