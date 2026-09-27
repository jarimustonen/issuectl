"""Hermetic regression tests for the self-hosted cargo-dist install override.

Run with `python3 tests/release_workflow.py` (requires PyYAML). The generation
check is `python3 scripts/release_workflow.py --check --dist /path/to/pinned/dist`.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = yaml.safe_load((ROOT / '.github/workflows/release.yml').read_text())


class ReleaseWorkflowTest(unittest.TestCase):
    def test_scope_and_downstream(self):
        jobs = WORKFLOW['jobs']
        plan = jobs['plan']['steps']
        self.assertEqual(plan[1]['name'], 'Install dist')
        self.assertEqual(plan[2]['with']['path'], '~/.cargo/bin/dist')
        local = jobs['build-local-artifacts']['steps']
        mac = next(s for s in local if s.get('name') == 'Install dist (self-hosted macOS)')
        hosted = next(s for s in local if s.get('name') == 'Install dist (hosted)')
        self.assertEqual([s.get('name') for s in local if 'Install dist' in s.get('name', '')],
                         ['Install dist (self-hosted macOS)', 'Install dist (hosted)'])
        self.assertEqual(jobs['build-local-artifacts']['runs-on'], '${{ matrix.runner }}')
        self.assertEqual(mac['if'], "${{ matrix.runner == 'self-hosted' && runner.os == 'macOS' }}")
        self.assertEqual(hosted['if'], "${{ matrix.runner != 'self-hosted' || runner.os != 'macOS' }}")
        self.assertEqual(hosted['run'], '${{ matrix.install_dist.run }}')
        self.assertLess(local.index(mac), local.index(hosted))
        self.assertLess(local.index(hosted), next(i for i, s in enumerate(local)
                                                if s.get('name') == 'Build artifacts'))
        self.assertIn('dist build', next(s for s in local if s.get('name') == 'Build artifacts')['run'])

    def test_concurrent_failure_and_wrong_directory(self):
        local = WORKFLOW['jobs']['build-local-artifacts']['steps']
        script = next(s for s in local if s.get('name') == 'Install dist (self-hosted macOS)')['run']
        # Swap only the matrix installer expression; no network or real Cargo home.
        script = script.replace('${{ matrix.install_dist.run }}', 'curl --fake | sh')
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            fakebin = base / 'fakebin'
            fakebin.mkdir()
            curl = fakebin / 'curl'
            curl.write_text('''#!/bin/sh
[ "${CARGO_DIST_NO_MODIFY_PATH:-}" = 1 ] || exit 23
case "$CARGO_DIST_INSTALL_DIR" in "$RUNNER_TEMP"/issuectl-dist.*) ;; *) exit 24 ;; esac
if [ "${FAIL_INSTALL:-0}" = 1 ]; then exit 22; fi
if [ "${IGNORE_INSTALL_DIR:-0}" = 1 ]; then
  printf '%s\\n' 'mkdir -p "$HOME/.cargo/bin"' 'touch "$HOME/.cargo/bin/dist"'
else
  printf '%s\\n' 'mkdir -p "$CARGO_DIST_INSTALL_DIR/bin"' \\
    'printf "#!/bin/sh\\nexit 0\\n" > "$CARGO_DIST_INSTALL_DIR/bin/dist"' \\
    'chmod +x "$CARGO_DIST_INSTALL_DIR/bin/dist"'
fi
''')
            curl.chmod(0o755)
            home = base / 'home'
            home.mkdir()
            env = dict(os.environ, HOME=str(home), RUNNER_TEMP=str(base),
                       PATH=f'{fakebin}:{os.environ["PATH"]}')
            outputs = [base / f'path-{i}' for i in range(4)]
            for output in outputs:
                output.touch()
            procs = [subprocess.Popen(['bash', '-c', script],
                     env=dict(env, GITHUB_PATH=str(output), FAIL_INSTALL='1' if i == 2 else '0',
                              IGNORE_INSTALL_DIR='1' if i == 3 else '0'),
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                     for i, output in enumerate(outputs)]
            results = [proc.communicate() for proc in procs]
            self.assertEqual([p.returncode for p in procs], [0, 0, 22, 1], results)
            roots = [output.read_text().strip() for output in outputs]
            self.assertEqual(roots[2:], ['', ''])
            self.assertEqual(len(set(roots[:2])), 2)
            for root in roots[:2]:
                self.assertTrue(root.startswith(str(base) + '/issuectl-dist.'))
                self.assertTrue((Path(root) / 'dist').is_file())
                downstream = subprocess.run(['bash', '-c', 'command -v dist'],
                                            env=dict(env, PATH=f'{root}:{env["PATH"]}'),
                                            capture_output=True, text=True, check=True)
                self.assertEqual(downstream.stdout.strip(), str(Path(root) / 'dist'))
            self.assertTrue((home / '.cargo/bin/dist').exists(),
                            'wrong-directory mock should prove the guard rejects its install')

    def test_symlink_escape_and_missing_temp(self):
        local = WORKFLOW['jobs']['build-local-artifacts']['steps']
        script = next(s for s in local if s.get('name') == 'Install dist (self-hosted macOS)')['run']
        script = script.replace('${{ matrix.install_dist.run }}',
                                'mkdir -p "$CARGO_DIST_INSTALL_DIR/bin"; '
                                'ln -s "$HOME/.cargo/bin/dist" "$CARGO_DIST_INSTALL_DIR/bin/dist"')
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            home = base / 'home'
            (home / '.cargo/bin').mkdir(parents=True)
            (home / '.cargo/bin/dist').write_text('#!/bin/sh\n')
            (home / '.cargo/bin/dist').chmod(0o755)
            output = base / 'path'
            output.touch()
            env = dict(os.environ, HOME=str(home), RUNNER_TEMP=str(base), GITHUB_PATH=str(output))
            result = subprocess.run(['bash', '-c', script], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(output.read_text(), '')
            result = subprocess.run(['bash', '-c', script], env=dict(env, RUNNER_TEMP=''),
                                    capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(output.read_text(), '')


if __name__ == '__main__':
    unittest.main()
