"""Offline regression tests for the release publisher (no network, no GitHub)."""
import hashlib
import importlib.util
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location('release', Path(__file__).with_name('release.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

VERSION = '2.3.4'
TAG = 'v' + VERSION


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / 'input'
        self.source.mkdir()
        for name in release.required_assets(VERSION):
            (self.source / name).write_bytes(name.encode())
        (self.source / 'GPL-2.0.txt').write_text('licence')
        self.calls = []
        self.releases = []
        self.tag_sha = 'built'
        self.fail_upload = False
        self.corrupt = False

    def ready(self):
        return release.prepare(self.source, self.root / 'ready', VERSION, archive=False)

    def api(self, endpoint, method='GET', payload=None):
        self.calls.append((method, endpoint, payload))
        if '/git/ref/tags/' in endpoint:
            return {'object': {'type': 'commit', 'sha': self.tag_sha}}
        if '/releases' in endpoint:
            return {'id': 7, 'draft': True}
        return {}

    def pages(self, endpoint):
        if '/assets?' in endpoint:
            return [{'name': 'stale.zip', 'id': 42}]
        return self.releases

    def gh(self, *args, **kwargs):
        self.calls.append(('gh', args, None))
        if args[:2] == ('release', 'upload') and self.fail_upload:
            raise RuntimeError('Upload failed')
        if args[:2] == ('release', 'download'):
            destination = Path(args[args.index('--dir') + 1])
            for path in (self.root / 'ready').iterdir():
                shutil.copyfile(path, destination / path.name)
            if self.corrupt:
                (destination / f'Flint-{VERSION}-Windows-x64.exe').write_bytes(b'corrupt')
        return ''

    def publish(self):
        directory = self.ready()
        with patch.object(release, 'api', self.api), patch.object(release, 'pages', self.pages), \
                patch.object(release, 'gh', self.gh):
            release.publish(directory, VERSION, 'built', 'owner/repo', 'notes')

    def published(self):
        return any(m == 'PATCH' and p and p.get('draft') is False for m, _, p in self.calls)

    def test_checksums_cover_every_asset(self):
        directory = self.ready()
        names = {p.name for p in directory.iterdir()}
        self.assertEqual(names, set(release.required_assets(VERSION)) | {'GPL-2.0.txt', 'SHA256SUMS.txt'})
        for name, checksum in release.read_manifest(directory / 'SHA256SUMS.txt').items():
            self.assertEqual(checksum, hashlib.sha256((directory / name).read_bytes()).hexdigest())

    def test_missing_platform_prepares_nothing(self):
        (self.source / f'Flint-{VERSION}-Windows-x64.exe').unlink()
        with self.assertRaises(ValueError):
            self.ready()
        self.assertFalse((self.root / 'ready').exists())

    def test_empty_package_is_rejected(self):
        (self.source / f'Flint-{VERSION}-macOS-arm64.dmg').write_bytes(b'')
        with self.assertRaises(ValueError):
            self.ready()

    def test_duplicate_package_is_rejected(self):
        (self.source / 'again').mkdir()
        name = f'Flint-{VERSION}-macOS-x64.dmg'
        shutil.copyfile(self.source / name, self.source / 'again' / name)
        with self.assertRaises(ValueError):
            self.ready()

    def test_corrupt_input_manifest_is_rejected(self):
        name = f'Flint-{VERSION}-macOS-arm64.dmg'
        (self.source / 'arm64.sha256').write_text(f'{"0" * 64}  {name}\n')
        with self.assertRaisesRegex(ValueError, 'Input checksum'):
            self.ready()

    def test_new_release_is_verified_before_publication(self):
        self.publish()
        self.assertTrue(self.published())
        upload = next(i for i, c in enumerate(self.calls) if c[0] == 'gh' and c[1][:2] == ('release', 'upload'))
        download = next(i for i, c in enumerate(self.calls) if c[0] == 'gh' and c[1][:2] == ('release', 'download'))
        final = max(i for i, c in enumerate(self.calls) if c[0] == 'PATCH')
        self.assertLess(upload, download)
        self.assertLess(download, final)
        self.assertTrue(any(m == 'DELETE' and e.endswith('/assets/42') for m, e, _ in self.calls))
        self.assertEqual(self.calls[-1][2]['name'], f'Flint {VERSION}')

    def test_draft_from_failed_run_is_resumed(self):
        self.releases = [{'id': 7, 'tag_name': TAG, 'draft': True}]
        self.publish()
        self.assertFalse(any(m == 'POST' and e.endswith('/releases') for m, e, _ in self.calls))
        self.assertTrue(self.published())

    def test_published_release_is_never_modified(self):
        self.releases = [{'id': 7, 'tag_name': TAG, 'draft': False}]
        with self.assertRaisesRegex(ValueError, 'already published'):
            self.publish()
        self.assertFalse(any(m in ('PATCH', 'POST', 'DELETE') for m, _, _ in self.calls))

    def test_tag_on_another_commit_stops_everything(self):
        self.tag_sha = 'other'
        with self.assertRaisesRegex(ValueError, 'not the built commit'):
            self.publish()
        self.assertFalse(any(c[0] == 'gh' for c in self.calls))

    def test_failed_upload_never_publishes(self):
        self.fail_upload = True
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertFalse(self.published())

    def test_corrupt_upload_never_publishes(self):
        self.corrupt = True
        with self.assertRaisesRegex(ValueError, 'Uploaded checksum'):
            self.publish()
        self.assertFalse(self.published())

    def test_local_corruption_never_contacts_github(self):
        directory = self.ready()
        (directory / f'Flint-{VERSION}-Windows-x64.exe').write_bytes(b'bad')
        with patch.object(release, 'api') as api:
            with self.assertRaises(ValueError):
                release.publish(directory, VERSION, 'built', 'owner/repo', '')
            api.assert_not_called()

    def test_prerelease_is_not_marked_latest(self):
        directory = release.prepare(self.source, self.root / 'ready', VERSION, archive=False)
        with patch.object(release, 'api', self.api), patch.object(release, 'pages', self.pages), \
                patch.object(release, 'gh', self.gh):
            for path in list(directory.iterdir()):
                path.rename(directory / path.name.replace(VERSION, VERSION + '-rc.1'))
            (directory / 'SHA256SUMS.txt').write_text(''.join(
                f'{release.digest(p)}  {p.name}\n' for p in sorted(directory.iterdir())
                if p.name != 'SHA256SUMS.txt'))
            release.publish(directory, VERSION + '-rc.1', 'built', 'owner/repo', 'notes')
        final = self.calls[-1][2]
        self.assertTrue(final['prerelease'])
        self.assertEqual(final['make_latest'], 'false')


if __name__ == '__main__':
    unittest.main()
