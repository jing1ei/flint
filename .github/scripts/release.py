"""Assemble and publish one immutable GitHub Release for a pushed version tag.

The release workflow downloads every platform build into one directory and calls this script.
It refuses to publish anything incomplete:

* every required package must be present exactly once and be non-empty;
* checksum manifests produced by the build jobs must still match their files;
* a release that is already *published* for the tag is never modified (ship a new version
  instead); an unfinished draft left by a failed run is resumed;
* assets are uploaded into a draft, downloaded again and verified before the draft is published.
"""
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

PROJECT = 'Flint'
ROOT = Path(__file__).resolve().parents[2]

# Evidence attached next to the packages (licence texts, FFmpeg build records and the matching
# upstream source). Individually optional: the build jobs decide which apply.
EXTRAS = (
    'ffmpeg-*-source-for-*.tar.*',
    '*-FFMPEG-BUILD.txt',
    'WINDOWS-FFMPEG-*.txt',
    'WRITTEN-OFFER.txt',
    'GPL-*.txt',
    'LGPL-*.txt',
    'WINDOWS.md',
)


def required_assets(version):
    """Package file names a complete release must carry."""
    base = f'{PROJECT}-{version}'
    return [
        f'{base}-macOS-arm64.dmg',
        f'{base}-macOS-arm64.app.zip',
        f'{base}-macOS-x64.dmg',
        f'{base}-macOS-x64.app.zip',
        f'{base}-Windows-x64.exe',
    ]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def read_manifest(manifest):
    """`sha256  name` lines (sha256sum / shasum / Get-FileHash style) as {name: checksum}."""
    entries = {}
    for line in manifest.read_text().splitlines():
        if not line.strip():
            continue
        checksum, name = line.split(maxsplit=1)
        name = name.lstrip('*').removeprefix('./')
        if Path(name).name != name:
            raise ValueError(f'Checksum manifest {manifest.name} names a path: {name}')
        entries[name] = checksum.lower()
    return entries


def prepare(source, output, version, archive=True):
    """Copy a complete, checksummed asset set into `output`, or raise before writing anything."""
    candidates = [p for p in source.rglob('*') if p.is_file()]
    for manifest in candidates:
        if manifest.name.endswith(('.sha256', 'SHA256SUMS.txt')):
            for name, checksum in read_manifest(manifest).items():
                target = manifest.parent / name
                if not target.is_file() or digest(target) != checksum:
                    raise ValueError(f'Input checksum mismatch: {name}')
    selected = {}
    for name in required_assets(version):
        matches = [p for p in candidates if p.name == name]
        if len(matches) != 1 or matches[0].is_symlink() or matches[0].stat().st_size == 0:
            raise ValueError(f'Expected one non-empty {name}; found {matches}')
        selected[name] = matches[0]
    for pattern in EXTRAS:
        for path in candidates:
            if not fnmatch.fnmatchcase(path.name, pattern):
                continue
            if path.is_symlink() or not path.stat().st_size:
                raise ValueError(f'Invalid additional asset: {path.name}')
            if path.name in selected and digest(selected[path.name]) != digest(path):
                raise ValueError(f'Conflicting additional asset: {path.name}')
            selected[path.name] = path
    output.mkdir(parents=True, exist_ok=False)
    for name, path in selected.items():
        shutil.copyfile(path, output / name)
    if archive:
        source_zip = output / f'{PROJECT}-{version}-source.zip'
        subprocess.run(['git', 'archive', '--format=zip', f'--prefix={PROJECT}-{version}/',
                        '-o', str(source_zip.resolve()), 'HEAD'], cwd=ROOT, check=True)
    files = sorted(output.iterdir())
    (output / 'SHA256SUMS.txt').write_text(''.join(f'{digest(p)}  {p.name}\n' for p in files))
    return output


def gh(*args, payload=None):
    result = subprocess.run(['gh', *args], input=json.dumps(payload) if payload is not None else None,
                            text=True, capture_output=True, check=True)
    return result.stdout


def api(endpoint, method='GET', payload=None):
    args = ['api', endpoint, '--method', method]
    if payload is not None:
        args += ['--input', '-']
    text = gh(*args, payload=payload)
    return json.loads(text) if text.strip() else None


def pages(endpoint):
    return [item for page in json.loads(gh('api', endpoint, '--paginate', '--slurp')) for item in page]


def publish(directory, version, sha, repo, notes):
    tag = 'v' + version
    prefix = f'repos/{repo}'
    expected = read_manifest(directory / 'SHA256SUMS.txt')
    for name, checksum in expected.items():
        if digest(directory / name) != checksum:
            raise ValueError(f'Invalid local checksum: {name}')
    expected['SHA256SUMS.txt'] = digest(directory / 'SHA256SUMS.txt')
    if set(expected) != {p.name for p in directory.iterdir()}:
        raise ValueError('Unexpected files outside the checksum manifest')

    target = api(prefix + '/git/ref/tags/' + tag)['object']
    if target['type'] == 'tag':  # annotated tag: follow it to the commit
        target = api(prefix + '/git/tags/' + target['sha'])['object']
    if target['sha'] != sha:
        raise ValueError(f'{tag} points at {target["sha"]}, not the built commit {sha}')

    releases = [r for r in pages(prefix + '/releases?per_page=100') if r['tag_name'] == tag]
    if any(not r['draft'] for r in releases):
        raise ValueError(f'{tag} is already published; release a new version instead')
    if len(releases) > 1:
        raise ValueError(f'Multiple draft releases use {tag}; delete the extras and re-run')
    prerelease = '-' in version
    metadata = dict(name=f'{PROJECT} {version}', body=notes + f'\n\nBuilt from `{sha}`.\n',
                    draft=True, prerelease=prerelease)
    if releases:
        release = api(prefix + f"/releases/{releases[0]['id']}", 'PATCH', metadata)
    else:
        release = api(prefix + '/releases', 'POST', dict(metadata, tag_name=tag))
    gh('release', 'upload', tag, '--repo', repo, '--clobber',
       *(str(directory / name) for name in sorted(expected)))
    # Verify what GitHub stored, including assets resumed from an earlier attempt.
    with tempfile.TemporaryDirectory() as temporary:
        args = ['release', 'download', tag, '--repo', repo, '--dir', temporary]
        for name in expected:
            args += ['--pattern', name]
        gh(*args)
        for name, checksum in expected.items():
            if digest(Path(temporary) / name) != checksum:
                raise ValueError(f'Uploaded checksum mismatch: {name}')
    for asset in pages(prefix + f"/releases/{release['id']}/assets?per_page=100"):
        if asset['name'] not in expected:
            api(prefix + f"/releases/assets/{asset['id']}", 'DELETE')
    api(prefix + f"/releases/{release['id']}", 'PATCH',
        dict(metadata, draft=False, make_latest='false' if prerelease else 'true'))
    print(f'https://github.com/{repo}/releases/tag/{tag}')


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('directory', type=Path)
    parser.add_argument('--version', required=True)
    parser.add_argument('--notes', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    output = prepare(args.directory, args.directory.parent / 'release-ready', args.version)
    if args.prepare_only:
        return
    sha = os.environ['GITHUB_SHA']
    actual = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if actual != sha:
        raise ValueError('The checkout does not match the commit this workflow run built')
    publish(output, args.version, sha, os.environ['GITHUB_REPOSITORY'],
            args.notes.read_text() if args.notes else f'{PROJECT} {args.version}')


if __name__ == '__main__':
    main()
