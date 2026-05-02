# issuectl

CLI for managing markdown-based issues with YAML frontmatter.

Designed to work with the `/issue` Claude Code skill. Issues are stored as
`issues/open/NN-slug/item.md` files.

## AI-First CLI

This is an AI-first CLI tool. See [AGENTS-AI-FIRST-CLI.md](AGENTS-AI-FIRST-CLI.md)
for design principles.

## Install

```sh
cargo install --path .
```

## Usage

### Browse

```
issuectl list                                     List open issues (default)
issuectl ls -a jari                               Filter by assignee
issuectl ls -t bug -p high                        Combine filters
issuectl ls --all                                 Include closed issues
issuectl ls --closed --json                       Closed issues, machine-readable
issuectl show 42                                  Show single issue details
issuectl search moodle                            Keyword search
issuectl stats [--json]                           Summary statistics
```

### Write

```
issuectl new --type bug --title "Login loops" --reporter jari --assignee bob
issuectl new --type epic --title "API v2 migration" --owner cara --priority high
issuectl update 42 --status in-progress
issuectl update 42 --add-commit "abc123:fix login state" --add-label frontend
issuectl close 42                                 Default closing status (fixed for bugs, done otherwise)
issuectl close 42 --status wontfix --commit "abc123:design decision"
```

`new`, `update`, and `close` all validate inputs strictly — invalid `--type`,
`--priority`, or `--status` values are rejected with the list of valid options.
Closing statuses (`done`, `fixed`, `wontfix`, `duplicate`, `cannot-reproduce`,
`obsolete`) automatically move the directory from `open/` to `closed/` and
stamp `closed:` with today's date.

### Maintenance

```
issuectl renumber                                 Renumber issues and fix references
issuectl skill install                            Install /issue skill in current repo
issuectl skill install --force                    Overwrite existing files
```

## Setup for a new repo

```sh
cd my-project
issuectl skill install
```

This creates:
- `issues/AGENTS.md` — issue structure and workflow docs
- `issues/open/` and `issues/closed/` — issue directories
- `.claude/skills/issue/SKILL.md` — the `/issue` Claude Code skill

## Development

```sh
cargo build
cargo run -- list
```

## Renumbering

```sh
issuectl renumber
```

Renumbers issue directories across `issues/open/` and `issues/closed/`, fixes
unambiguous `epic:` and `related:` frontmatter references, updates markdown
`#NN` references in all issue markdown files, updates relative markdown links to
renamed issue directories, and removes issue numbers from internal `# Heading`
titles.

If multiple old directories had the same number, references to that old number
are ambiguous and are reported instead of guessed.

## Planned

- `issuectl dedup` — detect duplicate issues by title/body similarity
