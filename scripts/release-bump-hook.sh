#!/bin/sh
# Regenerate every tracked, repo-local issuectl skill after Shipshape bumps the
# workspace version. Shipshape runs this from its sealed checkout before it
# creates the release commit.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/issuectl-release-bump.XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
mkdir -p "$scratch/home" "$scratch/target"

# Tests may supply the already-built issuectl. A real release builds the bumped
# binary in a disposable target so templates see the new CARGO_PKG_VERSION.
if [ -n "${ISSUECTL_RELEASE_HOOK_BIN:-}" ]; then
    issuectl_bin=$ISSUECTL_RELEASE_HOOK_BIN
else
    operator_home=${HOME:?HOME must be set for the release build toolchain}
    cargo_home=${CARGO_HOME:-$operator_home/.cargo}
    rustup_home=${RUSTUP_HOME:-$operator_home/.rustup}
    HOME="$scratch/home" \
        CARGO_HOME="$cargo_home" \
        RUSTUP_HOME="$rustup_home" \
        CARGO_TARGET_DIR="$scratch/target" \
        cargo build --locked --quiet -p issuectl
    issuectl_bin="$scratch/target/debug/issuectl"
fi

cd "$repo_root"
HOME="$scratch/home" CARGO_TARGET_DIR="$scratch/target" \
    "$issuectl_bin" skill install --agent all --target "$repo_root" --force >/dev/null
