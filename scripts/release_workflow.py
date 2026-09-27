#!/usr/bin/env python3
"""Generate or check cargo-dist CI with the self-hosted macOS install guard.

Never run `dist generate` against the live workflow: it replaces this override.
Use the pinned cargo-dist from dist-workspace.toml, e.g.:
  python3 scripts/release_workflow.py --check --dist /path/to/dist
  python3 scripts/release_workflow.py --write --dist /path/to/dist
"""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = Path('.github/workflows/release.yml')
ORIGINAL = """      - name: Install dist
        run: ${{ matrix.install_dist.run }}
      # Get the dist-manifest"""
REPLACEMENT = """      # Repository-specific override: the self-hosted ARM64 runner persists its
      # Cargo home between jobs. Keep the generated installer on hosted runners.
      - name: Install dist (self-hosted macOS)
        if: ${{ matrix.runner == 'self-hosted' && runner.os == 'macOS' }}
        shell: bash
        run: |
          set -euo pipefail
          : "${RUNNER_TEMP:?RUNNER_TEMP must be set for isolated dist install}"
          # Atomic allocation permits simultaneous release jobs on one runner.
          export CARGO_DIST_INSTALL_DIR="$(mktemp -d "$RUNNER_TEMP/issuectl-dist.XXXXXXXX")"
          export CARGO_DIST_NO_MODIFY_PATH=1
          ${{ matrix.install_dist.run }}
          export PATH="$CARGO_DIST_INSTALL_DIR/bin:$PATH"
          # Reject a missing binary, an old PATH entry, or a symlink outside the job.
          test "$(command -v dist)" = "$CARGO_DIST_INSTALL_DIR/bin/dist"
          test -x "$CARGO_DIST_INSTALL_DIR/bin/dist"
          test "$(realpath "$(command -v dist)")" = "$CARGO_DIST_INSTALL_DIR/bin/dist"
          echo "$CARGO_DIST_INSTALL_DIR/bin" >> "$GITHUB_PATH"
      - name: Install dist (hosted)
        if: ${{ matrix.runner != 'self-hosted' || runner.os != 'macOS' }}
        run: ${{ matrix.install_dist.run }}
      # Get the dist-manifest"""


def generate(dist: str) -> str:
    # dist discovers the Cargo workspace root; generate outside this workspace.
    with tempfile.TemporaryDirectory(prefix='issuectl-dist-generate-') as tmp:
        with subprocess.Popen(['git', 'archive', 'HEAD'], cwd=ROOT, stdout=subprocess.PIPE) as git:
            subprocess.run(['tar', '-xf', '-', '-C', tmp], stdin=git.stdout, check=True)
            git.stdout.close()
            if git.wait() != 0:
                raise RuntimeError('git archive failed')
        for file in ('Cargo.toml', 'Cargo.lock', 'dist-workspace.toml'):
            shutil.copy2(ROOT / file, Path(tmp) / file)
        subprocess.run([dist, 'generate', '--mode', 'ci'], cwd=tmp, check=True)
        generated = (Path(tmp) / WORKFLOW).read_text()
        if generated.count(ORIGINAL) != 1:
            raise RuntimeError('cargo-dist local installer template changed; review override')
        return generated.replace(ORIGINAL, REPLACEMENT)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    operation = parser.add_mutually_exclusive_group(required=True)
    operation.add_argument('--check', action='store_true')
    operation.add_argument('--write', action='store_true')
    parser.add_argument('--dist', default='dist', help='path to pinned cargo-dist 0.33.0')
    args = parser.parse_args()
    dist = shutil.which(args.dist)
    if not dist:
        parser.error(f'dist binary not found: {args.dist}')
    version = subprocess.run([dist, '--version'], check=True, capture_output=True, text=True).stdout.strip()
    if version != 'cargo-dist 0.33.0':
        parser.error(f'expected cargo-dist 0.33.0, got {version}')
    expected = generate(str(Path(dist).resolve()))
    path = ROOT / WORKFLOW
    if args.write:
        path.write_text(expected)
        print(f'generated {WORKFLOW}')
    elif path.read_text() != expected:
        raise SystemExit(f'{WORKFLOW} differs from cargo-dist generation + macOS override; run --write')
    else:
        print(f'{WORKFLOW}: generation + macOS override match')


if __name__ == '__main__':
    main()
