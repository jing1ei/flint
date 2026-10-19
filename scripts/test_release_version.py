"""Offline regressions for version-manifest agreement (scripts/release_version.py)."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from urllib.parse import urlparse

from release_version import version

ROOT = Path(__file__).resolve().parent.parent
MANIFESTS = ['package.json', 'package-lock.json', 'Cargo.toml', 'Cargo.lock',
             'src-tauri/Cargo.toml', 'src-tauri/tauri.conf.json']


class VersionTests(unittest.TestCase):
    def setUp(self):
        self.version = version(ROOT)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'repo with spaces'
        for name in MANIFESTS:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)

    def test_lockfile_uses_public_npm_registry(self):
        lock = json.loads((ROOT / 'package-lock.json').read_text())
        for name, package in lock['packages'].items():
            if package.get('resolved'):
                self.assertEqual(urlparse(package['resolved']).hostname, 'registry.npmjs.org', name)

    def test_all_versions_agree(self):
        self.assertEqual(version(self.root), self.version)

    def test_stale_npm_lock_is_rejected(self):
        path = self.root / 'package-lock.json'
        data = json.loads(path.read_text())
        data['packages']['']['version'] = self.version + '-drift'
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'disagree'):
            version(self.root)

    def test_stale_cargo_lock_is_rejected(self):
        path = self.root / 'Cargo.lock'
        old = f'name = "flint"\nversion = "{self.version}"'
        self.assertIn(old, path.read_text())
        path.write_text(path.read_text().replace(old, f'name = "flint"\nversion = "{self.version}-drift"'))
        with self.assertRaisesRegex(ValueError, 'disagree'):
            version(self.root)

    def test_invalid_semver_is_rejected(self):
        for bad in ['1.0', '01.0.0', '1.0.0-01', '1.0.0+build']:
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    self.check_everywhere(bad)

    def check_everywhere(self, value):
        """Write `value` into every manifest so only the semver rule can reject it."""
        for name in ['package.json', 'src-tauri/tauri.conf.json']:
            data = json.loads((ROOT / name).read_text())
            data['version'] = value
            (self.root / name).write_text(json.dumps(data))
        lock = json.loads((ROOT / 'package-lock.json').read_text())
        lock['version'] = lock['packages']['']['version'] = value
        (self.root / 'package-lock.json').write_text(json.dumps(lock))
        for name in ['Cargo.toml', 'Cargo.lock']:
            text = (ROOT / name).read_text()
            if name == 'Cargo.toml':
                text = text.replace(f'version = "{self.version}"', f'version = "{value}"', 1)
            else:
                for crate in ['flint', 'convert-core']:
                    text = text.replace(f'name = "{crate}"\nversion = "{self.version}"',
                                        f'name = "{crate}"\nversion = "{value}"')
            (self.root / name).write_text(text)
        return version(self.root)

    def test_valid_prerelease_is_accepted(self):
        self.assertEqual(self.check_everywhere('2.0.0-rc.1'), '2.0.0-rc.1')

    def test_cli_checks_the_tag(self):
        script = ROOT / 'scripts' / 'release_version.py'
        ok = subprocess.run([sys.executable, str(script), 'v' + self.version], capture_output=True, text=True)
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.assertEqual(ok.stdout.strip(), self.version)
        bad = subprocess.run([sys.executable, str(script), 'v0.0.0-other'], capture_output=True, text=True)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn('Tag says', bad.stderr)


if __name__ == '__main__':
    unittest.main()
