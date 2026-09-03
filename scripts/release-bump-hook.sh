#!/bin/sh
# Regenerate every tracked, repo-local issuectl skill after Shipshape bumps the
# workspace version. Shipshape runs this from its sealed checkout before it
# creates the release commit.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

scratch=$(mktemp -d "${TMPDIR:-/tmp}/issuectl-release-bump.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$scratch/home" "$scratch/target"

# Preserve a repo-authored scaffold and fail rather than smuggling a missing
# scaffold into the release commit through the broader skill installer.
scaffold=issues/AGENTS.md
if [ -e "$scaffold" ]; then
    cp "$scaffold" "$scratch/issues-AGENTS.md"
    scaffold_existed=true
else
    scaffold_existed=false
fi

if [ -n "${CARGO_HOME:-}" ]; then
    cargo_home=$CARGO_HOME
else
    cargo_home=${HOME:?HOME or CARGO_HOME must be set for the release toolchain}/.cargo
fi
if [ -n "${RUSTUP_HOME:-}" ]; then
    rustup_home=$RUSTUP_HOME
else
    rustup_home=${HOME:?HOME or RUSTUP_HOME must be set for the release toolchain}/.rustup
fi

# `cargo run` locates the executable correctly even when Cargo has a configured
# build target. The disposable target guarantees this is the freshly bumped
# binary rather than an artifact from before Shipshape edited Cargo.toml.
HOME="$scratch/home" \
    CARGO_HOME="$cargo_home" \
    RUSTUP_HOME="$rustup_home" \
    CARGO_TARGET_DIR="$scratch/target" \
    cargo run --locked --quiet -p issuectl --bin issuectl -- \
        skill install --agent all --target "$repo_root" --force >/dev/null

if [ "$scaffold_existed" = true ]; then
    cmp -s "$scaffold" "$scratch/issues-AGENTS.md" || {
        echo "release bump hook changed $scaffold" >&2
        exit 1
    }
elif [ -e "$scaffold" ]; then
    echo "release bump hook unexpectedly created $scaffold" >&2
    exit 1
fi

workspace_version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$workspace_version" ] || {
    echo "release bump hook could not read the workspace version" >&2
    exit 1
}
version_marker=$(printf 'This skill was installed for `issuectl %s`' "$workspace_version")
for relative in \
    .claude/skills/issue/SKILL.md \
    .claude/skills/issue-new/SKILL.md \
    .claude/skills/issue-intake/SKILL.md \
    .pi/agent/skills/issue/SKILL.md \
    .pi/agent/skills/issue-new/SKILL.md \
    .pi/agent/skills/issue-intake/SKILL.md \
    .codex/prompts/issue.md \
    .codex/prompts/issue-new.md \
    .codex/prompts/issue-intake.md
do
    grep -F "$version_marker" "$relative" >/dev/null || {
        echo "release bump hook did not regenerate $relative for issuectl $workspace_version" >&2
        exit 1
    }
done
