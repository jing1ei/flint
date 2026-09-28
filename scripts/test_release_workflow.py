"""Offline version and tag regressions using disposable Git repositories."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from urllib.parse import urlparse
from release_version import version

ROOT = Path(__file__).resolve().parent.parent

class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.version = version(ROOT)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'repo with spaces'
        self.root.mkdir()
        for name in ['package.json','package-lock.json','Cargo.toml','Cargo.lock',
                     'src-tauri/Cargo.toml','src-tauri/tauri.conf.json']:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, stderr=subprocess.DEVNULL, text=True).strip()

    def init_git(self):
        remote = Path(self.temp.name) / 'remote.git'
        subprocess.run(['git','init','--bare',str(remote)], check=True, capture_output=True)
        self.git('init')
        self.git('config','user.name','Release Test')
        self.git('config','user.email','release-test@example.invalid')
        self.git('add','.')
        self.git('commit','-m','initial')
        self.git('remote','add','origin',str(remote))
        return self.git('rev-parse','HEAD')

    def prepare(self, sha, tag=None):
        tag = tag or 'v' + self.version
        output = Path(self.temp.name) / 'output'
        output.write_text('')
        run = subprocess.run(['bash',str(ROOT/'scripts/prepare_release_tag.sh')],
            cwd=self.root, env={**os.environ,'SOURCE_SHA':sha,'TAG':tag,'GITHUB_OUTPUT':str(output)},
            capture_output=True,text=True)
        return run, output.read_text()

    def test_lockfile_uses_public_npm_registry(self):
        lock = json.loads((ROOT / 'package-lock.json').read_text())
        for name, package in lock['packages'].items():
            if package.get('resolved'):
                self.assertEqual(urlparse(package['resolved']).hostname,
                                 'registry.npmjs.org', name)

    def test_all_versions_agree(self):
        self.assertEqual(version(self.root),self.version)

    def test_stale_lock_prevents_release(self):
        path=self.root/'package-lock.json'
        data=json.loads(path.read_text());data['packages']['']['version']=self.version+'-drift'
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError,'disagree'):version(self.root)

    def test_rust_lock_drift_prevents_release(self):
        path=self.root/'Cargo.lock'
        path.write_text(path.read_text().replace(f'name = "flint"\nversion = "{self.version}"',f'name = "flint"\nversion = "{self.version}-drift"'))
        with self.assertRaisesRegex(ValueError,'disagree'):version(self.root)

    def test_new_tag_and_failed_release_rerun_use_same_commit(self):
        sha=self.init_git()
        run,output=self.prepare(sha)
        self.assertEqual(run.returncode,0,run.stderr)
        self.assertEqual(output,'release=true\n')
        self.assertEqual(self.git('rev-parse',f'v{self.version}^{{commit}}'),sha)
        remote=self.git('ls-remote','origin',f'refs/tags/v{self.version}^{{}}')
        self.assertTrue(remote.startswith(sha))
        run,output=self.prepare(sha)
        self.assertEqual(run.returncode,0,run.stderr)
        self.assertEqual(output,'release=true\n')

    def test_existing_version_on_other_commit_is_not_moved(self):
        first=self.init_git();self.prepare(first)
        self.git('commit','--allow-empty','-m','second')
        run,output=self.prepare(self.git('rev-parse','HEAD'))
        self.assertEqual(run.returncode,0,run.stderr)
        self.assertEqual(output,'release=false\n')
        self.assertEqual(self.git('rev-parse',f'v{self.version}^{{commit}}'),first)

    def test_wrong_checkout_cannot_be_tagged(self):
        first=self.init_git()
        self.git('commit','--allow-empty','-m','second')
        run,output=self.prepare(first)
        self.assertNotEqual(run.returncode,0)
        self.assertEqual(output,'')
        self.assertEqual(self.git('tag','--list'),'')

if __name__=='__main__':unittest.main()
